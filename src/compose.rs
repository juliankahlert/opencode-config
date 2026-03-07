//! Compose fragmented template directories back into a single monolithic file.
//!
//! The compose module merges per-agent JSON/YAML fragments from an input
//! directory into a single output file, detecting and resolving conflicts
//! on overlapping scalar paths.

use std::collections::HashMap;
use std::fs;
use std::io::{IsTerminal, Write as _};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::{Map, Value};
use tempfile::NamedTempFile;
use thiserror::Error;
use tracing::{debug, info, warn};

use crate::config::ConfigError;
use crate::options::RunOptions;
use crate::template::{TemplateError, deep_merge, is_template_extension, load_template};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Conflict resolution strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conflict {
    /// Fail immediately on any conflict.
    Error,
    /// Last fragment (lexicographic order) wins silently.
    LastWins,
    /// Prompt the user interactively; falls back to `LastWins` on non-TTY.
    Interactive,
}

/// Options for the compose pipeline.
pub struct ComposeOptions {
    pub input_dir: PathBuf,
    pub out: PathBuf,
    pub dry_run: bool,
    pub backup: bool,
    pub pretty: bool,
    pub verify: bool,
    pub force: bool,
    pub conflict: Conflict,
    pub run_options: RunOptions,
    pub config_dir: PathBuf,
}

#[derive(Debug, Error)]
pub enum ComposeError {
    #[error("config error: {0}")]
    Config(#[from] ConfigError),

    #[error("template error: {0}")]
    Template(#[from] TemplateError),

    #[error("conflict at path `{path}`: {sources:?}")]
    Conflict { path: String, sources: Vec<String> },

    #[error("output already exists: {}", path.display())]
    OutputExists { path: PathBuf },

    #[error("failed to write output at {}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to create backup at {}", path.display())]
    Backup {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to serialize output")]
    Serialize {
        #[source]
        source: serde_json::Error,
    },

    #[error("roundtrip verification failed for {}", path.display())]
    RoundtripMismatch { path: PathBuf },

    #[error("prompt cancelled by user")]
    Cancelled,

    #[error("prompt error: {0}")]
    Prompt(String),

    #[error("failed to read {}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("input directory not found: {}", path.display())]
    InputDirNotFound { path: PathBuf },

    #[error("no template fragments found in {}", path.display())]
    EmptyInputDir { path: PathBuf },

    #[error(
        "template `{name}` resolved to a file, not a fragment directory; \
         run `decompose {name}` first to split it into fragments"
    )]
    NotAFragmentDir { name: String },
}

/// Records which source file contributed a value at a JSON path.
#[derive(Debug, Clone)]
pub struct ProvenanceEntry {
    /// Dot-separated JSON path (e.g. `agent.build.model`).
    pub path: String,
    /// Source fragment filename.
    pub source: String,
    /// The value set at this path by this fragment.
    pub value: Value,
}

/// A candidate value during conflict resolution.
#[derive(Debug, Clone)]
pub struct ConflictCandidate {
    /// The JSON value proposed by this fragment.
    pub value: Value,
    /// Source filename (e.g. `02-review.json`).
    pub source: String,
}

/// Result of conflict resolution: the index of the winning candidate.
pub type ConflictResult = Result<usize, ComposeError>;

/// Trait for resolving conflicts between fragment values at the same JSON path.
pub(crate) trait ConflictResolver {
    fn resolve(&mut self, path: &str, candidates: &[ConflictCandidate]) -> ConflictResult;
}

/// Orchestrates the compose pipeline: load fragments, detect and resolve
/// conflicts, merge, serialize, and write output.
pub struct Composer {
    options: ComposeOptions,
}

// ---------------------------------------------------------------------------
// Conflict resolver implementations
// ---------------------------------------------------------------------------

struct ErrorResolver;

impl ConflictResolver for ErrorResolver {
    fn resolve(&mut self, path: &str, candidates: &[ConflictCandidate]) -> ConflictResult {
        Err(ComposeError::Conflict {
            path: path.to_string(),
            sources: candidates.iter().map(|c| c.source.clone()).collect(),
        })
    }
}

struct LastWinsResolver;

impl ConflictResolver for LastWinsResolver {
    fn resolve(&mut self, _path: &str, candidates: &[ConflictCandidate]) -> ConflictResult {
        Ok(candidates.len() - 1)
    }
}

struct InteractiveResolver;

impl ConflictResolver for InteractiveResolver {
    fn resolve(&mut self, path: &str, candidates: &[ConflictCandidate]) -> ConflictResult {
        use dialoguer::Select;
        use dialoguer::theme::ColorfulTheme;

        let items: Vec<String> = candidates
            .iter()
            .map(|c| format!("{}: {}", c.source, c.value))
            .collect();

        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt(format!("Conflict at {path}:"))
            .items(&items)
            .default(0)
            .interact_opt()
            .map_err(|e| ComposeError::Prompt(e.to_string()))?;

        match selection {
            Some(index) => Ok(index),
            None => Err(ComposeError::Cancelled),
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Run the compose pipeline: load fragments, resolve conflicts, write output.
///
/// When [`ComposeOptions::dry_run`] is `true`, the pipeline runs merge and
/// serialization but skips the filesystem write entirely.
pub fn run(options: ComposeOptions) -> Result<(), ComposeError> {
    let composer = Composer::new(options);
    let merged = composer.compose()?;
    let content = composer.serialize(&merged)?;

    if composer.options.dry_run {
        info!(
            output = %composer.options.out.display(),
            bytes = content.len(),
            "dry-run: skipping write"
        );
        return Ok(());
    }

    composer.write_output(&content)
}

/// Preview the composed output without writing files.
pub fn run_preview(options: ComposeOptions) -> Result<String, ComposeError> {
    let composer = Composer::new(options);
    let merged = composer.compose()?;
    composer.serialize(&merged)
}

// ---------------------------------------------------------------------------
// Composer implementation
// ---------------------------------------------------------------------------

impl Composer {
    pub fn new(options: ComposeOptions) -> Self {
        Self { options }
    }

    /// Load, merge, and resolve conflicts across all fragments.
    pub fn compose(&self) -> Result<Value, ComposeError> {
        let fragments = self.load_fragments()?;
        let provenance = build_provenance(&fragments);
        let conflicts = detect_conflicts(&provenance);

        let resolutions = if !conflicts.is_empty() {
            let mut resolver = self.make_resolver();
            info!(count = conflicts.len(), "conflicts detected");
            resolve_all_conflicts(&conflicts, resolver.as_mut())?
        } else {
            HashMap::new()
        };

        // Deep merge all fragments (last-wins by default)
        let mut merged = Value::Object(Map::new());
        for (_, value) in &fragments {
            deep_merge(&mut merged, value);
        }

        // Override conflicted paths with resolved winning values
        for (path, value) in &resolutions {
            set_at_path(&mut merged, path, value.clone());
        }

        Ok(merged)
    }

    /// Serialize the merged value as JSON.
    pub fn serialize(&self, value: &Value) -> Result<String, ComposeError> {
        let mut output = if self.options.pretty {
            serde_json::to_string_pretty(value)
        } else {
            serde_json::to_string(value)
        }
        .map_err(|source| ComposeError::Serialize { source })?;

        // Trailing newline for pretty output
        if self.options.pretty && !output.ends_with('\n') {
            output.push('\n');
        }

        Ok(output)
    }

    /// Write the composed output to disk with backup and verification.
    fn write_output(&self, content: &str) -> Result<(), ComposeError> {
        let out = &self.options.out;

        if out.exists() && !self.options.force {
            return Err(ComposeError::OutputExists { path: out.clone() });
        }

        // Create backup before overwriting
        if self.options.backup && out.exists() {
            let backup_path = make_backup_path(out);
            fs::copy(out, &backup_path).map_err(|source| ComposeError::Backup {
                path: backup_path.clone(),
                source,
            })?;
            info!(backup = %backup_path.display(), "created backup");
        }

        // Atomic write: tempfile in same directory then rename
        let parent = out.parent().unwrap_or(Path::new("."));
        let parent = if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        };

        let mut tmp = NamedTempFile::new_in(parent).map_err(|source| ComposeError::Write {
            path: out.clone(),
            source,
        })?;

        tmp.write_all(content.as_bytes())
            .map_err(|source| ComposeError::Write {
                path: out.clone(),
                source,
            })?;

        tmp.persist(out).map_err(|e| ComposeError::Write {
            path: out.clone(),
            source: e.error,
        })?;

        info!(output = %out.display(), "wrote composed output");

        // Roundtrip verification
        if self.options.verify {
            verify_roundtrip(out, content)?;
            info!("roundtrip verification passed");
        }

        Ok(())
    }

    /// Load all template fragments from the input directory in lex order.
    fn load_fragments(&self) -> Result<Vec<(String, Value)>, ComposeError> {
        let dir = &self.options.input_dir;

        if !dir.is_dir() {
            return Err(ComposeError::InputDirNotFound { path: dir.clone() });
        }

        let mut entries: Vec<_> = fs::read_dir(dir)
            .map_err(|source| ComposeError::Read {
                path: dir.clone(),
                source,
            })?
            .map(|entry| {
                entry.map_err(|source| ComposeError::Read {
                    path: dir.clone(),
                    source,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        entries.retain(|entry| entry.path().is_file() && is_template_extension(&entry.path()));

        entries.sort_by_key(|e| e.file_name());

        if entries.is_empty() {
            return Err(ComposeError::EmptyInputDir { path: dir.clone() });
        }

        info!(
            dir = %dir.display(),
            fragment_count = entries.len(),
            "loading compose fragments"
        );

        let mut fragments = Vec::with_capacity(entries.len());
        for entry in &entries {
            let filename = entry.file_name().to_string_lossy().to_string();
            let value = load_template(&entry.path())?;
            let value = maybe_wrap_agent(&filename, value);
            debug!(fragment = %filename, "loaded fragment");
            fragments.push((filename, value));
        }

        Ok(fragments)
    }

    /// Build the appropriate conflict resolver based on options and environment.
    fn make_resolver(&self) -> Box<dyn ConflictResolver> {
        match self.options.conflict {
            Conflict::Error => Box::new(ErrorResolver),
            Conflict::LastWins => Box::new(LastWinsResolver),
            Conflict::Interactive => {
                if std::io::stdin().is_terminal() {
                    Box::new(InteractiveResolver)
                } else {
                    warn!(
                        "non-interactive terminal; falling back to last-wins \
                         for conflict resolution"
                    );
                    Box::new(LastWinsResolver)
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Per-agent wrapping
// ---------------------------------------------------------------------------

/// Extract agent name from a numbered fragment filename.
///
/// Matches `<digits>-<agent-name>.<ext>` and returns `<agent-name>`.
fn extract_agent_name(filename: &str) -> Option<&str> {
    let stem = Path::new(filename).file_stem()?.to_str()?;
    let dash_pos = stem.find('-')?;
    let prefix = &stem[..dash_pos];
    if prefix.is_empty() || !prefix.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let name = &stem[dash_pos + 1..];
    if name.is_empty() {
        return None;
    }
    Some(name)
}

/// Wrap a fragment under `agent.<name>` when the filename matches
/// `<NN>-<agent-name>.<ext>` and the fragment does not already contain
/// an `agent` key.  Global fragments (underscore prefix) are never wrapped.
fn maybe_wrap_agent(filename: &str, value: Value) -> Value {
    // Global fragments are never wrapped
    if let Some(stem) = Path::new(filename).file_stem().and_then(|s| s.to_str())
        && stem.starts_with('_')
    {
        return value;
    }

    let agent_name = match extract_agent_name(filename) {
        Some(name) => name.to_string(),
        None => return value,
    };

    // Already nested under `agent` — leave untouched
    if value
        .as_object()
        .is_some_and(|obj| obj.contains_key("agent"))
    {
        return value;
    }

    let mut agent_inner = Map::new();
    agent_inner.insert(agent_name, value);
    let mut root = Map::new();
    root.insert("agent".to_string(), Value::Object(agent_inner));
    Value::Object(root)
}

// ---------------------------------------------------------------------------
// Provenance and conflict detection
// ---------------------------------------------------------------------------

/// Walk the JSON tree depth-first and record every leaf (scalar/array) with
/// its source filename.
fn collect_leaves(
    prefix: &str,
    value: &Value,
    source: &str,
    provenance: &mut HashMap<String, Vec<ProvenanceEntry>>,
) {
    match value {
        Value::Object(obj) => {
            for (key, val) in obj {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                collect_leaves(&path, val, source, provenance);
            }
        }
        _ => {
            // Leaf node: scalar, array, or null
            provenance
                .entry(prefix.to_string())
                .or_default()
                .push(ProvenanceEntry {
                    path: prefix.to_string(),
                    source: source.to_string(),
                    value: value.clone(),
                });
        }
    }
}

/// Build a provenance map from all loaded fragments.
fn build_provenance(fragments: &[(String, Value)]) -> HashMap<String, Vec<ProvenanceEntry>> {
    let mut provenance: HashMap<String, Vec<ProvenanceEntry>> = HashMap::new();
    for (filename, value) in fragments {
        collect_leaves("", value, filename, &mut provenance);
    }
    provenance
}

/// Identify conflicting paths: those with 2+ entries holding different values.
fn detect_conflicts(
    provenance: &HashMap<String, Vec<ProvenanceEntry>>,
) -> Vec<(String, Vec<ConflictCandidate>)> {
    let mut conflicts = Vec::new();

    for (path, entries) in provenance {
        if entries.len() < 2 {
            continue;
        }
        let first = &entries[0].value;
        if entries.iter().all(|e| &e.value == first) {
            continue; // identical values — no conflict
        }

        let candidates: Vec<ConflictCandidate> = entries
            .iter()
            .map(|e| ConflictCandidate {
                value: e.value.clone(),
                source: e.source.clone(),
            })
            .collect();
        conflicts.push((path.clone(), candidates));
    }

    // Deterministic ordering for reproducible output
    conflicts.sort_by(|a, b| a.0.cmp(&b.0));
    conflicts
}

/// Resolve every conflict via the given resolver and return a map of
/// path → winning value.
fn resolve_all_conflicts(
    conflicts: &[(String, Vec<ConflictCandidate>)],
    resolver: &mut dyn ConflictResolver,
) -> Result<HashMap<String, Value>, ComposeError> {
    let mut resolutions = HashMap::new();
    for (path, candidates) in conflicts {
        let idx = resolver.resolve(path, candidates)?;
        resolutions.insert(path.clone(), candidates[idx].value.clone());
    }
    Ok(resolutions)
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Set a value at a dot-separated JSON path inside `root`.
///
/// Assumes all intermediate objects exist (true after `deep_merge`).
fn set_at_path(root: &mut Value, path: &str, new_value: Value) {
    let parts: Vec<&str> = path.split('.').collect();
    if parts.is_empty() {
        return;
    }

    let mut current = root;
    for part in &parts[..parts.len() - 1] {
        current = match current.as_object_mut().and_then(|obj| obj.get_mut(*part)) {
            Some(v) => v,
            None => return, // intermediate path missing — should not happen
        };
    }

    if let Some(obj) = current.as_object_mut() {
        obj.insert(parts[parts.len() - 1].to_string(), new_value);
    }
}

// ---------------------------------------------------------------------------
// Backup and verification
// ---------------------------------------------------------------------------

/// Generate a backup path with timestamp suffix: `<file>.bak.<YYYYMMDDTHHMMSS>`.
fn make_backup_path(original: &Path) -> PathBuf {
    let now = SystemTime::now();
    let duration = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let timestamp = format_timestamp(duration.as_secs());

    let mut backup = original.as_os_str().to_owned();
    backup.push(format!(".bak.{timestamp}"));
    PathBuf::from(backup)
}

/// Format unix seconds as `YYYYMMDDTHHMMSS` (UTC).
fn format_timestamp(secs: u64) -> String {
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;
    let (year, month, day) = days_to_ymd(days);
    format!("{year:04}{month:02}{day:02}T{hours:02}{minutes:02}{seconds:02}")
}

/// Howard Hinnant civil_from_days: convert days since 1970-01-01 to (Y, M, D).
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    let z = days as i64 + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year as u64, m, d)
}

/// Read back a written file and compare against expected content.
fn verify_roundtrip(path: &Path, expected_content: &str) -> Result<(), ComposeError> {
    let readback = fs::read_to_string(path).map_err(|source| ComposeError::Read {
        path: path.to_path_buf(),
        source,
    })?;

    let written: Value =
        serde_json::from_str(&readback).map_err(|source| ComposeError::Serialize { source })?;
    let expected: Value = serde_json::from_str(expected_content)
        .map_err(|source| ComposeError::Serialize { source })?;

    if written != expected {
        return Err(ComposeError::RoundtripMismatch {
            path: path.to_path_buf(),
        });
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    // ------ tracing capture helper -------------------------------------------

    /// A writer backed by a shared byte buffer, used to capture `tracing` output
    /// in tests.
    #[derive(Clone)]
    struct SharedWriter(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for SharedWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for SharedWriter {
        type Writer = SharedWriter;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// Install a per-thread tracing subscriber that writes to `buf` and return
    /// a guard that restores the previous subscriber on drop.
    fn capture_tracing(buf: &Arc<Mutex<Vec<u8>>>) -> tracing::subscriber::DefaultGuard {
        use tracing_subscriber::fmt;

        let writer = SharedWriter(Arc::clone(buf));
        let subscriber = fmt::Subscriber::builder()
            .with_writer(writer)
            .with_max_level(tracing::Level::WARN)
            .with_ansi(false)
            .finish();
        tracing::subscriber::set_default(subscriber)
    }

    // ------ test utilities ---------------------------------------------------

    fn write_file(path: impl AsRef<Path>, contents: &str) {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dir");
        }
        let mut file = fs::File::create(path).expect("create file");
        file.write_all(contents.as_bytes()).expect("write file");
    }

    fn default_options(input_dir: PathBuf, out: PathBuf, config_dir: PathBuf) -> ComposeOptions {
        ComposeOptions {
            input_dir,
            out,
            dry_run: false,
            backup: false,
            pretty: true,
            verify: false,
            force: false,
            conflict: Conflict::Error,
            run_options: RunOptions::default(),
            config_dir,
        }
    }

    // === extract_agent_name ===

    #[test]
    fn extract_agent_name_numbered() {
        assert_eq!(extract_agent_name("01-build.json"), Some("build"));
        assert_eq!(extract_agent_name("02-review.yaml"), Some("review"));
        assert_eq!(extract_agent_name("99-security.yml"), Some("security"));
        assert_eq!(
            extract_agent_name("1-code-review.json"),
            Some("code-review")
        );
    }

    #[test]
    fn extract_agent_name_no_match() {
        assert_eq!(extract_agent_name("_global.json"), None);
        assert_eq!(extract_agent_name("overrides.json"), None);
        assert_eq!(extract_agent_name("build.json"), None);
        assert_eq!(extract_agent_name("01-.json"), None);
        assert_eq!(extract_agent_name("-build.json"), None);
    }

    // === maybe_wrap_agent ===

    #[test]
    fn wrap_agent_numbered_fragment() {
        let value = json!({"model": "gpt-4"});
        let wrapped = maybe_wrap_agent("01-build.json", value);
        assert_eq!(wrapped, json!({"agent": {"build": {"model": "gpt-4"}}}));
    }

    #[test]
    fn wrap_agent_already_nested() {
        let value = json!({"agent": {"build": {"model": "gpt-4"}}});
        let result = maybe_wrap_agent("01-build.json", value.clone());
        assert_eq!(result, value, "should not double-wrap");
    }

    #[test]
    fn wrap_agent_global_no_wrap() {
        let value = json!({"$schema": "https://example.com"});
        let result = maybe_wrap_agent("_global.json", value.clone());
        assert_eq!(result, value);
    }

    #[test]
    fn wrap_agent_non_numbered_no_wrap() {
        let value = json!({"model": "gpt-4"});
        let result = maybe_wrap_agent("overrides.json", value.clone());
        assert_eq!(result, value);
    }

    // === provenance and conflict detection ===

    #[test]
    fn conflict_detected_different_values() {
        let fragments = vec![
            (
                "01-build.json".to_string(),
                json!({"agent": {"build": {"model": "gpt-4"}}}),
            ),
            (
                "02-override.json".to_string(),
                json!({"agent": {"build": {"model": "gpt-5"}}}),
            ),
        ];
        let provenance = build_provenance(&fragments);
        let conflicts = detect_conflicts(&provenance);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].0, "agent.build.model");
        assert_eq!(conflicts[0].1.len(), 2);
    }

    #[test]
    fn no_conflict_identical_values() {
        let fragments = vec![
            (
                "01-a.json".to_string(),
                json!({"agent": {"build": {"model": "gpt-4"}}}),
            ),
            (
                "02-b.json".to_string(),
                json!({"agent": {"build": {"model": "gpt-4"}}}),
            ),
        ];
        let provenance = build_provenance(&fragments);
        let conflicts = detect_conflicts(&provenance);
        assert!(conflicts.is_empty(), "identical values should not conflict");
    }

    #[test]
    fn no_conflict_disjoint_keys() {
        let fragments = vec![
            (
                "01-build.json".to_string(),
                json!({"agent": {"build": {"model": "gpt-4"}}}),
            ),
            (
                "02-review.json".to_string(),
                json!({"agent": {"review": {"model": "gpt-5"}}}),
            ),
        ];
        let provenance = build_provenance(&fragments);
        let conflicts = detect_conflicts(&provenance);
        assert!(conflicts.is_empty(), "disjoint keys should merge cleanly");
    }

    #[test]
    fn conflict_on_different_arrays() {
        let fragments = vec![
            ("01-a.json".to_string(), json!({"tags": ["fast"]})),
            ("02-b.json".to_string(), json!({"tags": ["safe"]})),
        ];
        let provenance = build_provenance(&fragments);
        let conflicts = detect_conflicts(&provenance);
        assert_eq!(conflicts.len(), 1, "different arrays should conflict");
        assert_eq!(conflicts[0].0, "tags");
    }

    // === resolvers ===

    #[test]
    fn last_wins_resolver_picks_last() {
        let mut resolver = LastWinsResolver;
        let candidates = vec![
            ConflictCandidate {
                value: json!("gpt-4"),
                source: "a.json".to_string(),
            },
            ConflictCandidate {
                value: json!("gpt-5"),
                source: "b.json".to_string(),
            },
            ConflictCandidate {
                value: json!("gpt-6"),
                source: "c.json".to_string(),
            },
        ];
        let result = resolver.resolve("test.path", &candidates).unwrap();
        assert_eq!(result, 2);
    }

    #[test]
    fn error_resolver_returns_error() {
        let mut resolver = ErrorResolver;
        let candidates = vec![
            ConflictCandidate {
                value: json!("gpt-4"),
                source: "a.json".to_string(),
            },
            ConflictCandidate {
                value: json!("gpt-5"),
                source: "b.json".to_string(),
            },
        ];
        let result = resolver.resolve("test.path", &candidates);
        match result {
            Err(ComposeError::Conflict { path, sources }) => {
                assert_eq!(path, "test.path");
                assert_eq!(sources, vec!["a.json", "b.json"]);
            }
            other => panic!("expected Conflict error, got: {other:?}"),
        }
    }

    // === interactive fallback on non-TTY ===

    #[test]
    fn interactive_fallback_non_tty() {
        // In tests, stdin is not a terminal, so Interactive falls back to
        // LastWins and emits a tracing warning.
        let buf = Arc::new(Mutex::new(Vec::new()));
        let _guard = capture_tracing(&buf);

        let temp_dir = TempDir::new().expect("temp dir");
        let input_dir = temp_dir.path().join("fragments");
        fs::create_dir_all(&input_dir).expect("create dir");

        write_file(
            input_dir.join("01-build.json"),
            r#"{"agent": {"build": {"model": "gpt-4"}}}"#,
        );
        write_file(
            input_dir.join("02-override.json"),
            r#"{"agent": {"build": {"model": "gpt-5"}}}"#,
        );

        let options = ComposeOptions {
            input_dir,
            out: temp_dir.path().join("out.json"),
            conflict: Conflict::Interactive,
            force: true,
            ..default_options(
                PathBuf::new(),
                PathBuf::new(),
                temp_dir.path().to_path_buf(),
            )
        };

        let composer = Composer::new(options);
        let merged = composer
            .compose()
            .expect("compose should succeed with non-TTY interactive fallback");
        assert_eq!(merged["agent"]["build"]["model"], "gpt-5");

        let log_data = buf.lock().unwrap();
        let captured = String::from_utf8_lossy(&log_data);
        assert!(
            captured.contains("non-interactive terminal"),
            "expected non-TTY fallback warning in captured logs, got: {captured}"
        );
    }

    // === full pipeline ===

    #[test]
    fn compose_merges_non_conflicting_fragments() {
        let temp_dir = TempDir::new().expect("temp dir");
        let input_dir = temp_dir.path().join("fragments");
        fs::create_dir_all(&input_dir).expect("create dir");

        write_file(
            input_dir.join("_global.json"),
            r#"{"$schema": "https://example.com", "agent": {}}"#,
        );
        write_file(
            input_dir.join("01-build.json"),
            r#"{"agent": {"build": {"model": "gpt-4"}}}"#,
        );
        write_file(
            input_dir.join("02-review.json"),
            r#"{"agent": {"review": {"model": "gpt-5"}}}"#,
        );

        let out = temp_dir.path().join("opencode.json");
        let mut options = default_options(input_dir, out.clone(), temp_dir.path().to_path_buf());
        options.verify = true;

        run(options).expect("compose should succeed");

        let content = fs::read_to_string(&out).expect("read output");
        let parsed: Value = serde_json::from_str(&content).expect("parse output");
        assert_eq!(parsed["$schema"], "https://example.com");
        assert_eq!(parsed["agent"]["build"]["model"], "gpt-4");
        assert_eq!(parsed["agent"]["review"]["model"], "gpt-5");
    }

    #[test]
    fn compose_with_auto_wrapping() {
        let temp_dir = TempDir::new().expect("temp dir");
        let input_dir = temp_dir.path().join("fragments");
        fs::create_dir_all(&input_dir).expect("create dir");

        // This fragment has no `agent` key — auto-wrapping should apply
        write_file(
            input_dir.join("01-build.json"),
            r#"{"model": "gpt-4", "variant": "mini"}"#,
        );

        let out = temp_dir.path().join("opencode.json");
        let options = default_options(input_dir, out.clone(), temp_dir.path().to_path_buf());

        run(options).expect("compose should succeed");

        let content = fs::read_to_string(&out).expect("read output");
        let parsed: Value = serde_json::from_str(&content).expect("parse output");
        assert_eq!(parsed["agent"]["build"]["model"], "gpt-4");
        assert_eq!(parsed["agent"]["build"]["variant"], "mini");
    }

    #[test]
    fn last_wins_resolves_conflicts() {
        let temp_dir = TempDir::new().expect("temp dir");
        let input_dir = temp_dir.path().join("fragments");
        fs::create_dir_all(&input_dir).expect("create dir");

        write_file(
            input_dir.join("01-build.json"),
            r#"{"agent": {"build": {"model": "gpt-4"}}}"#,
        );
        write_file(
            input_dir.join("99-overrides.json"),
            r#"{"agent": {"build": {"model": "gpt-5"}}}"#,
        );

        let mut options = default_options(
            input_dir,
            temp_dir.path().join("out.json"),
            temp_dir.path().to_path_buf(),
        );
        options.conflict = Conflict::LastWins;

        let preview = run_preview(options).expect("preview");
        let parsed: Value = serde_json::from_str(&preview).expect("parse");
        assert_eq!(parsed["agent"]["build"]["model"], "gpt-5");
    }

    #[test]
    fn error_mode_rejects_conflicts() {
        let temp_dir = TempDir::new().expect("temp dir");
        let input_dir = temp_dir.path().join("fragments");
        fs::create_dir_all(&input_dir).expect("create dir");

        write_file(
            input_dir.join("01-build.json"),
            r#"{"agent": {"build": {"model": "gpt-4"}}}"#,
        );
        write_file(
            input_dir.join("99-overrides.json"),
            r#"{"agent": {"build": {"model": "gpt-5"}}}"#,
        );

        let options = default_options(
            input_dir,
            temp_dir.path().join("out.json"),
            temp_dir.path().to_path_buf(),
        );

        let result = run_preview(options);
        match result {
            Err(ComposeError::Conflict { path, sources }) => {
                assert_eq!(path, "agent.build.model");
                assert_eq!(sources.len(), 2);
            }
            other => panic!("expected Conflict error, got: {other:?}"),
        }
    }

    // === output behaviour ===

    #[test]
    fn dry_run_does_not_write() {
        let temp_dir = TempDir::new().expect("temp dir");
        let input_dir = temp_dir.path().join("fragments");
        fs::create_dir_all(&input_dir).expect("create dir");

        write_file(input_dir.join("base.json"), r#"{"key": "value"}"#);

        let out = temp_dir.path().join("opencode.json");
        let options = default_options(input_dir, out.clone(), temp_dir.path().to_path_buf());

        let preview = run_preview(options).expect("preview should succeed");
        assert!(preview.contains("value"));
        assert!(
            !out.exists(),
            "output file must not be created during dry-run"
        );
    }

    #[test]
    fn run_dry_run_skips_write() {
        let temp_dir = TempDir::new().expect("temp dir");
        let input_dir = temp_dir.path().join("fragments");
        fs::create_dir_all(&input_dir).expect("create dir");

        write_file(input_dir.join("base.json"), r#"{"key": "value"}"#);

        let out = temp_dir.path().join("opencode.json");
        let mut options = default_options(input_dir, out.clone(), temp_dir.path().to_path_buf());
        options.dry_run = true;

        run(options).expect("run with dry_run should succeed");
        assert!(
            !out.exists(),
            "run() with dry_run=true must not create output file"
        );
    }

    #[test]
    fn output_exists_without_force() {
        let temp_dir = TempDir::new().expect("temp dir");
        let input_dir = temp_dir.path().join("fragments");
        fs::create_dir_all(&input_dir).expect("create dir");

        write_file(input_dir.join("base.json"), r#"{"key": "value"}"#);

        let out = temp_dir.path().join("opencode.json");
        write_file(&out, "existing content");

        let options = default_options(input_dir, out.clone(), temp_dir.path().to_path_buf());

        let result = run(options);
        match result {
            Err(ComposeError::OutputExists { path }) => {
                assert_eq!(path, out);
            }
            other => panic!("expected OutputExists, got: {other:?}"),
        }
    }

    #[test]
    fn backup_created_on_overwrite() {
        let temp_dir = TempDir::new().expect("temp dir");
        let input_dir = temp_dir.path().join("fragments");
        fs::create_dir_all(&input_dir).expect("create dir");

        write_file(input_dir.join("base.json"), r#"{"key": "new"}"#);

        let out = temp_dir.path().join("opencode.json");
        write_file(&out, r#"{"key": "old"}"#);

        let mut options = default_options(input_dir, out.clone(), temp_dir.path().to_path_buf());
        options.backup = true;
        options.force = true;

        run(options).expect("compose should succeed");

        // Verify backup was created
        let entries: Vec<_> = fs::read_dir(temp_dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with("opencode.json.bak."))
            })
            .collect();
        assert_eq!(entries.len(), 1, "exactly one backup file should exist");

        let backup_content = fs::read_to_string(entries[0].path()).expect("read backup");
        assert_eq!(backup_content, r#"{"key": "old"}"#);

        let new_content = fs::read_to_string(&out).expect("read output");
        let parsed: Value = serde_json::from_str(&new_content).expect("parse output");
        assert_eq!(parsed["key"], "new");
    }

    #[test]
    fn roundtrip_verification_passes() {
        let temp_dir = TempDir::new().expect("temp dir");
        let input_dir = temp_dir.path().join("fragments");
        fs::create_dir_all(&input_dir).expect("create dir");

        write_file(
            input_dir.join("base.json"),
            r#"{"agent": {"build": {"model": "gpt-4"}}}"#,
        );

        let out = temp_dir.path().join("opencode.json");
        let mut options = default_options(input_dir, out.clone(), temp_dir.path().to_path_buf());
        options.verify = true;

        run(options).expect("compose with verify should succeed");
        assert!(out.exists());
    }

    #[test]
    fn pretty_output_has_newlines() {
        let temp_dir = TempDir::new().expect("temp dir");
        let input_dir = temp_dir.path().join("fragments");
        fs::create_dir_all(&input_dir).expect("create dir");

        write_file(input_dir.join("base.json"), r#"{"key": "value"}"#);

        let mut options = default_options(
            input_dir,
            temp_dir.path().join("out.json"),
            temp_dir.path().to_path_buf(),
        );
        options.pretty = true;

        let preview = run_preview(options).expect("preview");
        assert!(
            preview.contains('\n'),
            "pretty output should contain newlines"
        );
        assert!(
            preview.contains("  "),
            "pretty output should contain indentation"
        );
        assert!(
            preview.ends_with('\n'),
            "pretty output should end with newline"
        );
    }

    #[test]
    fn minified_output_compact() {
        let temp_dir = TempDir::new().expect("temp dir");
        let input_dir = temp_dir.path().join("fragments");
        fs::create_dir_all(&input_dir).expect("create dir");

        write_file(input_dir.join("base.json"), r#"{"key": "value"}"#);

        let mut options = default_options(
            input_dir,
            temp_dir.path().join("out.json"),
            temp_dir.path().to_path_buf(),
        );
        options.pretty = false;

        let preview = run_preview(options).expect("preview");
        assert!(
            !preview.contains('\n'),
            "minified output must not contain newlines"
        );
        assert!(
            !preview.contains("  "),
            "minified output must not contain indentation"
        );
    }

    // === error propagation ===

    #[test]
    fn empty_input_dir_error() {
        let temp_dir = TempDir::new().expect("temp dir");
        let input_dir = temp_dir.path().join("empty");
        fs::create_dir_all(&input_dir).expect("create dir");

        let options = default_options(
            input_dir.clone(),
            temp_dir.path().join("out.json"),
            temp_dir.path().to_path_buf(),
        );

        let result = run_preview(options);
        match result {
            Err(ComposeError::EmptyInputDir { path }) => {
                assert_eq!(path, input_dir);
            }
            other => panic!("expected EmptyInputDir, got: {other:?}"),
        }
    }

    #[test]
    fn input_dir_not_found_error() {
        let temp_dir = TempDir::new().expect("temp dir");
        let input_dir = temp_dir.path().join("nonexistent");

        let options = default_options(
            input_dir.clone(),
            temp_dir.path().join("out.json"),
            temp_dir.path().to_path_buf(),
        );

        let result = run_preview(options);
        match result {
            Err(ComposeError::InputDirNotFound { path }) => {
                assert_eq!(path, input_dir);
            }
            other => panic!("expected InputDirNotFound, got: {other:?}"),
        }
    }

    // === set_at_path ===

    #[test]
    fn set_at_path_overwrites_leaf() {
        let mut root = json!({"agent": {"build": {"model": "gpt-4"}}});
        set_at_path(&mut root, "agent.build.model", json!("gpt-5"));
        assert_eq!(root["agent"]["build"]["model"], "gpt-5");
    }

    #[test]
    fn set_at_path_top_level_key() {
        let mut root = json!({"theme": "light"});
        set_at_path(&mut root, "theme", json!("dark"));
        assert_eq!(root["theme"], "dark");
    }
}
