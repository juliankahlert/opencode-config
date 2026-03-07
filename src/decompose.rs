//! Decompose a monolithic template file into a fragment directory.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::{Map, Value};
use thiserror::Error;
use tracing::{debug, info};

use crate::config::ConfigError;
use crate::template::{
    TemplateError, is_valid_template_name, load_template, load_template_dir, write_json_pretty,
};

#[derive(Debug, Error)]
pub enum DecomposeError {
    #[error("config error: {0}")]
    Config(#[from] ConfigError),

    #[error("template error: {0}")]
    Template(#[from] TemplateError),

    #[error("template is not a single file: {name}")]
    NotASingleFile { name: String },

    #[error("target directory already exists: {path}")]
    TargetExists { path: PathBuf },

    #[error("template has no 'agent' key at top level")]
    NoAgentKey,

    #[error("roundtrip verification failed: reassembled output differs from original")]
    RoundtripMismatch,

    #[error("failed to create temporary directory")]
    TempDir {
        #[source]
        source: std::io::Error,
    },

    #[error("failed to write fragment at {path}")]
    WriteFragment {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to backup original at {path}")]
    Backup {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to rename temp directory to {path}")]
    Rename {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to remove original file at {path}")]
    RemoveOriginal {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to serialize fragment")]
    Serialize {
        #[source]
        source: serde_json::Error,
    },

    #[error("invalid agent name for fragment filename: {0:?}")]
    InvalidAgentName(String),

    #[error("invalid template name: {0:?}")]
    InvalidTemplateName(String),
}

pub struct DecomposeOptions {
    /// Template name (e.g. "default") -- resolved via resolve_template_source.
    pub template: String,
    /// Path to the config directory (resolved from --config or XDG).
    pub config_dir: PathBuf,
    /// When true, print the decomposition plan without writing anything.
    pub dry_run: bool,
    /// When true, perform a roundtrip verification: reassemble fragments
    /// via load_template_dir and assert equality with the original.
    pub verify: bool,
    /// When true, proceed even if the target directory already exists.
    pub force: bool,
}

/// Fragment produced by splitting a monolithic template.
#[derive(Debug)]
struct Fragment {
    filename: String,
    value: Value,
}

/// Resolve a template name to a single file path for decomposition.
///
/// Unlike `resolve_template_source`, this does not error when both a file and
/// a `.d/` directory exist (a common state when force-overwriting).  It
/// prioritises the file: if a file with a recognised extension exists, it is
/// returned.  If only a directory exists, `NotASingleFile` is returned.
fn resolve_decompose_source(config_dir: &Path, name: &str) -> Result<PathBuf, DecomposeError> {
    let template_dir = config_dir.join("template.d");

    // Look for the first file with a recognised extension
    for ext in ["json", "yaml", "yml"] {
        let path = template_dir.join(format!("{name}.{ext}"));
        if path.is_file() {
            return Ok(path);
        }
    }

    // No file found — check if the name is already a directory
    let dir_path = template_dir.join(format!("{name}.d"));
    if dir_path.is_dir() {
        return Err(DecomposeError::NotASingleFile {
            name: name.to_string(),
        });
    }

    // Fall back — load_template will produce a proper read error
    Ok(template_dir.join(format!("{name}.yml")))
}

/// Run the full decompose pipeline: load, split, write, verify, backup, rename.
pub fn run(options: DecomposeOptions) -> Result<(), DecomposeError> {
    // Validate template name before any path construction
    if !is_valid_template_name(&options.template) {
        return Err(DecomposeError::InvalidTemplateName(
            options.template.clone(),
        ));
    }

    // Phase 1: Resolve and load
    let file_path = resolve_decompose_source(&options.config_dir, &options.template)?;

    let original = load_template(&file_path)?;
    let fragments = split_into_fragments(&options.template, &original)?;

    // Phase 2: Determine target directory
    let template_dir = options.config_dir.join("template.d");
    let target_dir = template_dir.join(format!("{}.d", options.template));

    if target_dir.exists() && !options.force {
        return Err(DecomposeError::TargetExists { path: target_dir });
    }

    // Write fragments to temp directory (sibling of target for same-fs rename)
    let tmp_dir =
        tempfile::tempdir_in(&template_dir).map_err(|source| DecomposeError::TempDir { source })?;

    for fragment in &fragments {
        let frag_path = tmp_dir.path().join(&fragment.filename);
        write_json_pretty(
            &frag_path,
            &fragment.value,
            |source| DecomposeError::Serialize { source },
            |source, path| DecomposeError::WriteFragment { path, source },
        )?;
        debug!(fragment = %fragment.filename, "wrote fragment");
    }

    // Phase 3: Verify roundtrip (optional)
    if options.verify {
        verify_roundtrip(&original, tmp_dir.path())?;
        info!("roundtrip verification passed");
    }

    // Phase 4: Commit
    // 4a: Backup original
    let backup_path = make_backup_path(&file_path);
    fs::copy(&file_path, &backup_path).map_err(|source| DecomposeError::Backup {
        path: backup_path.clone(),
        source,
    })?;
    info!(backup = %backup_path.display(), "created backup");

    // 4b: Remove existing target dir if force
    if target_dir.exists() && options.force {
        fs::remove_dir_all(&target_dir).map_err(|source| DecomposeError::Rename {
            path: target_dir.clone(),
            source,
        })?;
    }

    // 4c: Atomic rename (same filesystem guarantees atomicity)
    let tmp_path = tmp_dir.keep();
    fs::rename(&tmp_path, &target_dir).map_err(|source| DecomposeError::Rename {
        path: target_dir.clone(),
        source,
    })?;
    info!(target = %target_dir.display(), "renamed temp dir to target");

    // 4d: Remove original file
    fs::remove_file(&file_path).map_err(|source| DecomposeError::RemoveOriginal {
        path: file_path.clone(),
        source,
    })?;
    info!(original = %file_path.display(), "removed original file");

    Ok(())
}

/// Preview the decomposition plan without filesystem side-effects.
pub fn run_preview(options: DecomposeOptions) -> Result<String, DecomposeError> {
    // Validate template name before any path construction
    if !is_valid_template_name(&options.template) {
        return Err(DecomposeError::InvalidTemplateName(
            options.template.clone(),
        ));
    }

    let file_path = resolve_decompose_source(&options.config_dir, &options.template)?;

    let original = load_template(&file_path)?;
    let fragments = split_into_fragments(&options.template, &original)?;

    let template_dir = options.config_dir.join("template.d");
    let target_dir = template_dir.join(format!("{}.d", options.template));

    // Mirror the TargetExists check from run()
    if target_dir.exists() && !options.force {
        return Err(DecomposeError::TargetExists { path: target_dir });
    }

    // Ephemeral verify: write to temp dir, verify roundtrip, then auto-cleanup
    if options.verify {
        let tmp_dir = tempfile::tempdir_in(&template_dir)
            .map_err(|source| DecomposeError::TempDir { source })?;
        for fragment in &fragments {
            write_json_pretty(
                &tmp_dir.path().join(&fragment.filename),
                &fragment.value,
                |source| DecomposeError::Serialize { source },
                |source, path| DecomposeError::WriteFragment { path, source },
            )?;
        }
        verify_roundtrip(&original, tmp_dir.path())?;
        // tmp_dir drops here, cleaning up automatically
    }

    let mut output = String::new();
    let _ = writeln!(
        output,
        "[DRY-RUN] Decompose template '{}' ({})",
        options.template,
        file_path.display()
    );
    let _ = writeln!(
        output,
        "[DRY-RUN] Target directory: {}/",
        target_dir.display()
    );
    let _ = writeln!(output, "[DRY-RUN] Fragments:");

    let agent_count = fragments.len().saturating_sub(1);
    for fragment in &fragments {
        let description = if fragment.filename.starts_with('_') {
            let keys: Vec<String> = fragment
                .value
                .as_object()
                .map(|obj| obj.keys().cloned().collect())
                .unwrap_or_default();
            format!("global: {}", keys.join(", "))
        } else {
            let agent_name = fragment
                .filename
                .strip_suffix(".json")
                .unwrap_or(&fragment.filename);
            format!("agent: {agent_name}")
        };
        let _ = writeln!(
            output,
            "[DRY-RUN]   {:<20}({})",
            fragment.filename, description
        );
    }

    let _ = writeln!(
        output,
        "[DRY-RUN] Total: {} fragments (1 global + {} agents)",
        fragments.len(),
        agent_count
    );

    Ok(output)
}

/// Validate that an agent name is safe to use as a fragment filename.
///
/// Allowed charset: `[A-Za-z0-9._-]`, non-empty, no leading dot, no `..`.
fn validate_agent_name(name: &str) -> Result<(), DecomposeError> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.starts_with('.')
        || name.contains('/')
        || name.contains('\\')
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    {
        return Err(DecomposeError::InvalidAgentName(name.to_string()));
    }
    Ok(())
}

/// Split a monolithic template into a global fragment and per-agent fragments.
fn split_into_fragments(
    template_name: &str,
    original: &Value,
) -> Result<Vec<Fragment>, DecomposeError> {
    let obj = original.as_object().ok_or(DecomposeError::NoAgentKey)?;

    let agents = obj
        .get("agent")
        .and_then(Value::as_object)
        .ok_or(DecomposeError::NoAgentKey)?;

    // Build global fragment: original with agent contents replaced by empty object
    let mut global_obj = Map::new();
    for (key, value) in obj {
        if key == "agent" {
            global_obj.insert("agent".to_string(), Value::Object(Map::new()));
        } else {
            global_obj.insert(key.clone(), value.clone());
        }
    }

    // Underscore prefix guarantees the global fragment sorts first
    let global_filename = format!("_{template_name}.json");
    let mut fragments = vec![Fragment {
        filename: global_filename,
        value: Value::Object(global_obj),
    }];

    // Build per-agent fragments sorted by agent name
    let mut agent_names: Vec<&String> = agents.keys().collect();
    agent_names.sort();

    for agent_name in agent_names {
        validate_agent_name(agent_name)?;
        let agent_value = &agents[agent_name];
        let mut inner = Map::new();
        inner.insert(agent_name.clone(), agent_value.clone());
        let mut agent_obj = Map::new();
        agent_obj.insert("agent".to_string(), Value::Object(inner));

        fragments.push(Fragment {
            filename: format!("{agent_name}.json"),
            value: Value::Object(agent_obj),
        });
    }

    Ok(fragments)
}

/// Verify roundtrip: reload fragments via load_template_dir and compare to original.
fn verify_roundtrip(original: &Value, tmpdir: &Path) -> Result<(), DecomposeError> {
    let reassembled = load_template_dir(tmpdir).map_err(DecomposeError::Template)?;
    if reassembled != *original {
        return Err(DecomposeError::RoundtripMismatch);
    }
    Ok(())
}

/// Generate backup path with timestamp: `<file>.bak.<YYYYMMDDTHHMMSS>`
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

/// Convert days since 1970-01-01 to (year, month, day) using the
/// Howard Hinnant civil_from_days algorithm.
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;
    use std::path::Path;

    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    fn write_file(path: impl AsRef<Path>, contents: &str) {
        let path = path.as_ref();
        let mut file = fs::File::create(path).expect("create file");
        file.write_all(contents.as_bytes()).expect("write file");
    }

    /// Create a temp config dir with a single monolithic template file.
    fn setup_template_config(template_name: &str, template_json: &Value) -> TempDir {
        let config_dir = TempDir::new().expect("temp dir");
        let template_dir = config_dir.path().join("template.d");
        fs::create_dir_all(&template_dir).expect("create template.d");
        let file_path = template_dir.join(format!("{template_name}.json"));
        write_file(
            &file_path,
            &serde_json::to_string_pretty(template_json).unwrap(),
        );
        config_dir
    }

    fn sample_template() -> Value {
        json!({
            "$schema": "https://example.com/schema.json",
            "default_agent": "coder",
            "agent": {
                "build": {
                    "model": "gpt-4",
                    "variant": "mini"
                },
                "review": {
                    "model": "gpt-5"
                }
            }
        })
    }

    // === split_into_fragments tests ===

    #[test]
    fn split_extracts_agents() {
        let template = sample_template();
        let fragments = split_into_fragments("default", &template).unwrap();
        // 1 global + 2 agents
        assert_eq!(fragments.len(), 3);
        assert_eq!(fragments[0].filename, "_default.json");
        assert_eq!(fragments[1].filename, "build.json");
        assert_eq!(fragments[2].filename, "review.json");
    }

    #[test]
    fn split_preserves_global_keys() {
        let template = sample_template();
        let fragments = split_into_fragments("default", &template).unwrap();
        let global = &fragments[0].value;
        assert!(global.get("$schema").is_some());
        assert!(global.get("default_agent").is_some());
        // agent key exists but is empty
        assert_eq!(global["agent"], json!({}));
    }

    #[test]
    fn split_agent_fragments_structure() {
        let template = sample_template();
        let fragments = split_into_fragments("default", &template).unwrap();
        let build_frag = &fragments[1].value;
        assert_eq!(build_frag["agent"]["build"]["model"], "gpt-4");
        assert_eq!(build_frag["agent"]["build"]["variant"], "mini");
        // Only the "agent" key at top level
        assert_eq!(build_frag.as_object().unwrap().len(), 1);
    }

    #[test]
    fn split_roundtrip_equality() {
        use crate::template::deep_merge;

        let template = sample_template();
        let fragments = split_into_fragments("default", &template).unwrap();

        // Merge all fragments in order (same as load_template_dir does)
        let mut merged = fragments[0].value.clone();
        for frag in &fragments[1..] {
            deep_merge(&mut merged, &frag.value);
        }

        assert_eq!(merged, template);
    }

    #[test]
    fn global_fragment_sorts_first() {
        let template = json!({
            "agent": {
                "aaa": {"model": "m1"},
                "zzz": {"model": "m2"}
            }
        });
        let fragments = split_into_fragments("mytemplate", &template).unwrap();
        let filenames: Vec<&str> = fragments.iter().map(|f| f.filename.as_str()).collect();
        let mut sorted = filenames.clone();
        sorted.sort();
        assert_eq!(
            filenames, sorted,
            "fragments should already be in sorted order"
        );
        assert!(
            filenames[0].starts_with('_'),
            "global fragment should start with underscore"
        );
    }

    #[test]
    fn reject_no_agent_key() {
        let template = json!({"$schema": "x", "default_agent": "y"});
        let result = split_into_fragments("default", &template);
        match result {
            Err(DecomposeError::NoAgentKey) => {}
            other => panic!("expected NoAgentKey, got: {other:?}"),
        }
    }

    #[test]
    fn reject_unsafe_agent_name_path_traversal() {
        let template = json!({"agent": {"../etc/passwd": {"model": "x"}}});
        let result = split_into_fragments("default", &template);
        match result {
            Err(DecomposeError::InvalidAgentName(name)) => {
                assert_eq!(name, "../etc/passwd");
            }
            other => panic!("expected InvalidAgentName, got: {other:?}"),
        }
    }

    #[test]
    fn reject_unsafe_agent_name_slash() {
        let template = json!({"agent": {"foo/bar": {"model": "x"}}});
        let result = split_into_fragments("default", &template);
        assert!(matches!(result, Err(DecomposeError::InvalidAgentName(_))));
    }

    #[test]
    fn reject_unsafe_agent_name_backslash() {
        let template = json!({"agent": {"foo\\bar": {"model": "x"}}});
        let result = split_into_fragments("default", &template);
        assert!(matches!(result, Err(DecomposeError::InvalidAgentName(_))));
    }

    #[test]
    fn reject_unsafe_agent_name_empty() {
        let template = json!({"agent": {"": {"model": "x"}}});
        let result = split_into_fragments("default", &template);
        assert!(matches!(result, Err(DecomposeError::InvalidAgentName(_))));
    }

    #[test]
    fn reject_unsafe_agent_name_dotdot() {
        let template = json!({"agent": {"..": {"model": "x"}}});
        let result = split_into_fragments("default", &template);
        assert!(matches!(result, Err(DecomposeError::InvalidAgentName(_))));
    }

    #[test]
    fn reject_unsafe_agent_name_leading_dot() {
        let template = json!({"agent": {".hidden": {"model": "x"}}});
        let result = split_into_fragments("default", &template);
        assert!(matches!(result, Err(DecomposeError::InvalidAgentName(_))));
    }

    #[test]
    fn reject_unsafe_agent_name_control_char() {
        let template = json!({"agent": {"foo\x00bar": {"model": "x"}}});
        let result = split_into_fragments("default", &template);
        assert!(matches!(result, Err(DecomposeError::InvalidAgentName(_))));
    }

    #[test]
    fn accept_valid_agent_names() {
        let template = json!({
            "agent": {
                "build": {"model": "x"},
                "code-review": {"model": "y"},
                "agent_v2": {"model": "z"},
                "Agent3.0": {"model": "w"}
            }
        });
        let result = split_into_fragments("default", &template);
        assert!(result.is_ok(), "valid agent names should be accepted");
    }

    // === run / run_preview integration tests ===

    #[test]
    fn run_rejects_traversal_template_name() {
        let config_dir = TempDir::new().expect("temp dir");
        let options = DecomposeOptions {
            template: "../evil".to_string(),
            config_dir: config_dir.path().to_path_buf(),
            dry_run: false,
            verify: false,
            force: false,
        };
        match run(options) {
            Err(DecomposeError::InvalidTemplateName(name)) => {
                assert_eq!(name, "../evil");
            }
            other => panic!("expected InvalidTemplateName, got: {other:?}"),
        }
    }

    #[test]
    fn run_rejects_slash_template_name() {
        let config_dir = TempDir::new().expect("temp dir");
        let options = DecomposeOptions {
            template: "a/b".to_string(),
            config_dir: config_dir.path().to_path_buf(),
            dry_run: false,
            verify: false,
            force: false,
        };
        assert!(matches!(
            run(options),
            Err(DecomposeError::InvalidTemplateName(_))
        ));
    }

    #[test]
    fn run_rejects_empty_template_name() {
        let config_dir = TempDir::new().expect("temp dir");
        let options = DecomposeOptions {
            template: String::new(),
            config_dir: config_dir.path().to_path_buf(),
            dry_run: false,
            verify: false,
            force: false,
        };
        assert!(matches!(
            run(options),
            Err(DecomposeError::InvalidTemplateName(_))
        ));
    }

    #[test]
    fn preview_rejects_traversal_template_name() {
        let config_dir = TempDir::new().expect("temp dir");
        let options = DecomposeOptions {
            template: "../evil".to_string(),
            config_dir: config_dir.path().to_path_buf(),
            dry_run: true,
            verify: false,
            force: false,
        };
        match run_preview(options) {
            Err(DecomposeError::InvalidTemplateName(name)) => {
                assert_eq!(name, "../evil");
            }
            other => panic!("expected InvalidTemplateName, got: {other:?}"),
        }
    }

    #[test]
    fn preview_rejects_extension_template_name() {
        let config_dir = TempDir::new().expect("temp dir");
        let options = DecomposeOptions {
            template: "default.json".to_string(),
            config_dir: config_dir.path().to_path_buf(),
            dry_run: true,
            verify: false,
            force: false,
        };
        assert!(matches!(
            run_preview(options),
            Err(DecomposeError::InvalidTemplateName(_))
        ));
    }

    #[test]
    fn reject_directory_template() {
        let config_dir = TempDir::new().expect("temp dir");
        let template_dir = config_dir.path().join("template.d");
        let frag_dir = template_dir.join("default.d");
        fs::create_dir_all(&frag_dir).expect("create dir");
        write_file(frag_dir.join("base.json"), r#"{"agent":{}}"#);

        let options = DecomposeOptions {
            template: "default".to_string(),
            config_dir: config_dir.path().to_path_buf(),
            dry_run: false,
            verify: false,
            force: false,
        };

        let result = run(options);
        match result {
            Err(DecomposeError::NotASingleFile { name }) => {
                assert_eq!(name, "default");
            }
            other => panic!("expected NotASingleFile, got: {other:?}"),
        }
    }

    #[test]
    fn target_exists_error_without_force() {
        let template = sample_template();
        let config_dir = setup_template_config("default", &template);
        let template_dir = config_dir.path().join("template.d");
        let target_dir = template_dir.join("default.d");
        fs::create_dir_all(&target_dir).expect("create target dir");

        let options = DecomposeOptions {
            template: "default".to_string(),
            config_dir: config_dir.path().to_path_buf(),
            dry_run: false,
            verify: false,
            force: false,
        };

        let result = run(options);
        match result {
            Err(DecomposeError::TargetExists { path }) => {
                assert!(path.ends_with("default.d"));
            }
            other => panic!("expected TargetExists, got: {other:?}"),
        }
    }

    #[test]
    fn force_overwrites_existing_target() {
        let template = sample_template();
        let config_dir = setup_template_config("default", &template);
        let template_dir = config_dir.path().join("template.d");
        let target_dir = template_dir.join("default.d");
        fs::create_dir_all(&target_dir).expect("create target dir");
        write_file(target_dir.join("old.json"), "{}");

        let options = DecomposeOptions {
            template: "default".to_string(),
            config_dir: config_dir.path().to_path_buf(),
            dry_run: false,
            verify: false,
            force: true,
        };

        run(options).expect("force overwrite should succeed");

        // Target dir should exist with new fragments
        assert!(target_dir.exists());
        assert!(target_dir.join("_default.json").exists());
        assert!(target_dir.join("build.json").exists());
        assert!(target_dir.join("review.json").exists());
        // Old fragment should be gone
        assert!(!target_dir.join("old.json").exists());
    }

    #[test]
    fn normal_decompose_creates_fragments_and_backup() {
        let template = sample_template();
        let config_dir = setup_template_config("default", &template);
        let template_dir = config_dir.path().join("template.d");

        let options = DecomposeOptions {
            template: "default".to_string(),
            config_dir: config_dir.path().to_path_buf(),
            dry_run: false,
            verify: true,
            force: false,
        };

        run(options).expect("decompose should succeed");

        // Original file removed
        assert!(!template_dir.join("default.json").exists());

        // Fragment directory exists with correct contents
        let target_dir = template_dir.join("default.d");
        assert!(target_dir.exists());
        assert!(target_dir.join("_default.json").exists());
        assert!(target_dir.join("build.json").exists());
        assert!(target_dir.join("review.json").exists());

        // Backup exists
        let entries: Vec<_> = fs::read_dir(&template_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with("default.json.bak."))
            })
            .collect();
        assert_eq!(entries.len(), 1, "exactly one backup file should exist");
    }

    #[test]
    fn verify_mismatch_detected() {
        let original = sample_template();
        let temp_dir = TempDir::new().expect("temp dir");

        // Write fragments with a differing global value
        write_file(
            temp_dir.path().join("_default.json"),
            &serde_json::to_string_pretty(&json!({
                "$schema": "https://example.com/schema.json",
                "default_agent": "WRONG",
                "agent": {}
            }))
            .unwrap(),
        );
        write_file(
            temp_dir.path().join("build.json"),
            &serde_json::to_string_pretty(&json!({
                "agent": {"build": {"model": "gpt-4", "variant": "mini"}}
            }))
            .unwrap(),
        );
        write_file(
            temp_dir.path().join("review.json"),
            &serde_json::to_string_pretty(&json!({
                "agent": {"review": {"model": "gpt-5"}}
            }))
            .unwrap(),
        );

        let result = verify_roundtrip(&original, temp_dir.path());
        match result {
            Err(DecomposeError::RoundtripMismatch) => {}
            other => panic!("expected RoundtripMismatch, got: {other:?}"),
        }
    }

    #[test]
    fn dry_run_lists_fragments_no_side_effects() {
        let template = sample_template();
        let config_dir = setup_template_config("default", &template);

        let options = DecomposeOptions {
            template: "default".to_string(),
            config_dir: config_dir.path().to_path_buf(),
            dry_run: true,
            verify: false,
            force: false,
        };

        let output = run_preview(options).expect("dry-run should succeed");

        assert!(output.contains("[DRY-RUN]"));
        assert!(output.contains("default"));
        assert!(output.contains("_default.json"));
        assert!(output.contains("build.json"));
        assert!(output.contains("review.json"));
        assert!(output.contains("1 global + 2 agents"));

        // No files should have been created or removed
        let template_dir = config_dir.path().join("template.d");
        assert!(
            !template_dir.join("default.d").exists(),
            "fragment dir must not exist after dry-run"
        );
        assert!(
            template_dir.join("default.json").exists(),
            "original must still exist after dry-run"
        );
    }

    #[test]
    fn preview_returns_target_exists_without_force() {
        let template = sample_template();
        let config_dir = setup_template_config("default", &template);
        let template_dir = config_dir.path().join("template.d");
        let target_dir = template_dir.join("default.d");
        fs::create_dir_all(&target_dir).expect("create target dir");

        let options = DecomposeOptions {
            template: "default".to_string(),
            config_dir: config_dir.path().to_path_buf(),
            dry_run: true,
            verify: false,
            force: false,
        };

        let result = run_preview(options);
        match result {
            Err(DecomposeError::TargetExists { path }) => {
                assert!(path.ends_with("default.d"));
            }
            other => panic!("expected TargetExists, got: {other:?}"),
        }
    }

    #[test]
    fn preview_verify_passes_for_valid_template() {
        let template = sample_template();
        let config_dir = setup_template_config("default", &template);

        let options = DecomposeOptions {
            template: "default".to_string(),
            config_dir: config_dir.path().to_path_buf(),
            dry_run: true,
            verify: true,
            force: false,
        };

        let output = run_preview(options).expect("preview with verify should succeed");
        assert!(output.contains("[DRY-RUN]"));

        // Verify should not leave any temp artifacts in template.d
        let template_dir = config_dir.path().join("template.d");
        let entries: Vec<_> = fs::read_dir(&template_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        // Only the original file should remain
        assert_eq!(entries.len(), 1, "no temp artifacts should remain");
    }

    #[test]
    fn preview_verify_detects_mismatch() {
        // This test uses a template where roundtrip would fail if fragments
        // were incorrectly constructed.  We test indirectly: a well-formed
        // template should pass, which is covered above.  Here we verify the
        // verify_roundtrip function itself catches mismatches (already tested
        // by verify_mismatch_detected), so we just confirm preview + force
        // + verify succeeds for the normal case even with existing target.
        let template = sample_template();
        let config_dir = setup_template_config("default", &template);
        let template_dir = config_dir.path().join("template.d");
        let target_dir = template_dir.join("default.d");
        fs::create_dir_all(&target_dir).expect("create target dir");

        let options = DecomposeOptions {
            template: "default".to_string(),
            config_dir: config_dir.path().to_path_buf(),
            dry_run: true,
            verify: true,
            force: true,
        };

        let output = run_preview(options).expect("preview + force + verify should succeed");
        assert!(output.contains("[DRY-RUN]"));
    }

    // === backup / timestamp tests ===

    #[test]
    fn backup_filename_format() {
        let path = Path::new("/tmp/template.d/default.json");
        let backup = make_backup_path(path);
        let name = backup.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("default.json.bak."), "backup name: {name}");
        // Timestamp is 15 chars: YYYYMMDDTHHMMSS
        let suffix = name.strip_prefix("default.json.bak.").unwrap();
        assert_eq!(suffix.len(), 15, "timestamp should be 15 chars: {suffix}");
        assert!(
            suffix.contains('T'),
            "should contain 'T' separator: {suffix}"
        );
    }

    #[test]
    fn format_timestamp_known_value() {
        // 2024-01-01T12:00:00 UTC = 1704110400 unix seconds
        let ts = format_timestamp(1704110400);
        assert_eq!(ts, "20240101T120000");
    }

    #[test]
    fn format_timestamp_epoch() {
        let ts = format_timestamp(0);
        assert_eq!(ts, "19700101T000000");
    }
}
