use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde_json::Value;
use serde_yaml;
use thiserror::Error;
use tracing::{debug, info};

use crate::config::{Palette, Reasoning};

#[derive(Debug, Error)]
pub enum TemplateError {
    #[error("failed to read template at {path}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse template at {path}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to parse yaml template at {path}")]
    ParseYaml {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("failed to convert yaml template at {path}")]
    ConvertYaml {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("unsupported template extension at {path}: {extension}")]
    UnsupportedExtension { path: PathBuf, extension: String },
    #[error("template directory not found: {path}")]
    MissingDir { path: PathBuf },
    #[error("ambiguous template \"{name}\": both {file_path} and {dir_path} exist; remove one")]
    AmbiguousTemplate {
        name: String,
        file_path: PathBuf,
        dir_path: PathBuf,
    },
    #[error("template directory is empty: {path}")]
    EmptyTemplateDir { path: PathBuf },
}

/// Discriminant for whether a template name resolved to a single file or a fragment directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateSource {
    /// A single template file (`.json`, `.yaml`, or `.yml`).
    File(PathBuf),
    /// A directory of template fragments (`<name>.d/`).
    Directory(PathBuf),
}

pub struct TemplateLoader<State> {
    path: PathBuf,
    state: State,
}

struct TemplateSourceState;

struct TemplateRead {
    data: String,
}

struct TemplateParsed {
    parsed: ParsedTemplate,
}

enum ParsedTemplate {
    Json(Value),
    Yaml(serde_yaml::Value),
}

impl TemplateLoader<TemplateSourceState> {
    pub fn from_path(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            state: TemplateSourceState,
        }
    }

    pub fn read(self) -> Result<TemplateLoader<TemplateRead>, TemplateError> {
        let data = fs::read_to_string(&self.path).map_err(|source| TemplateError::Read {
            path: self.path.clone(),
            source,
        })?;
        Ok(TemplateLoader {
            path: self.path,
            state: TemplateRead { data },
        })
    }
}

impl TemplateLoader<TemplateRead> {
    pub fn parse(self) -> Result<TemplateLoader<TemplateParsed>, TemplateError> {
        let extension = self
            .path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase);

        let parsed = match extension.as_deref() {
            Some("json") => {
                let value = serde_json::from_str(&self.state.data).map_err(|source| {
                    TemplateError::Parse {
                        path: self.path.clone(),
                        source,
                    }
                })?;
                ParsedTemplate::Json(value)
            }
            Some("yaml") | Some("yml") => {
                let yaml_value: serde_yaml::Value = serde_yaml::from_str(&self.state.data)
                    .map_err(|source| TemplateError::ParseYaml {
                        path: self.path.clone(),
                        source,
                    })?;
                ParsedTemplate::Yaml(yaml_value)
            }
            Some(extension) => {
                return Err(TemplateError::UnsupportedExtension {
                    path: self.path.clone(),
                    extension: extension.to_string(),
                });
            }
            None => {
                return Err(TemplateError::UnsupportedExtension {
                    path: self.path.clone(),
                    extension: "<none>".to_string(),
                });
            }
        };

        Ok(TemplateLoader {
            path: self.path,
            state: TemplateParsed { parsed },
        })
    }
}

impl TemplateLoader<TemplateParsed> {
    pub fn to_json(self) -> Result<Value, TemplateError> {
        match self.state.parsed {
            ParsedTemplate::Json(value) => Ok(value),
            ParsedTemplate::Yaml(yaml_value) => {
                serde_json::to_value(yaml_value).map_err(|source| TemplateError::ConvertYaml {
                    path: self.path.clone(),
                    source,
                })
            }
        }
    }
}

pub fn load_template(path: &Path) -> Result<Value, TemplateError> {
    TemplateLoader::from_path(path).read()?.parse()?.to_json()
}

pub(crate) fn is_template_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| matches!(ext.to_ascii_lowercase().as_str(), "json" | "yaml" | "yml"))
}

fn find_template_file(template_dir: &Path, name: &str) -> Option<PathBuf> {
    for ext in ["json", "yaml", "yml"] {
        let path = template_dir.join(format!("{name}.{ext}"));
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

/// Returns `true` when `path` is a directory that contains at least one
/// template fragment with a recognised extension (`.json`, `.yaml`, `.yml`).
pub fn is_template_dir(path: &Path) -> bool {
    path.is_dir() && count_template_files(path) > 0
}

fn count_template_files(dir: &Path) -> usize {
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_file() && is_template_extension(&e.path()))
                .count()
        })
        .unwrap_or(0)
}

pub(crate) fn resolve_template_source(
    config_dir: &Path,
    name: &str,
) -> Result<TemplateSource, TemplateError> {
    let template_dir = config_dir.join("template.d");
    let file_path = find_template_file(&template_dir, name);
    let dir_path = template_dir.join(format!("{name}.d"));
    let dir_exists = dir_path.is_dir();

    let result = match (file_path, dir_exists) {
        (Some(file_path), true) => Err(TemplateError::AmbiguousTemplate {
            name: name.to_string(),
            file_path,
            dir_path,
        }),
        (Some(file_path), false) => Ok(TemplateSource::File(file_path)),
        (None, true) => {
            if count_template_files(&dir_path) == 0 {
                Err(TemplateError::EmptyTemplateDir { path: dir_path })
            } else {
                Ok(TemplateSource::Directory(dir_path))
            }
        }
        (None, false) => {
            // Backward compat: fall back to .yml path (may not exist)
            Ok(TemplateSource::File(
                template_dir.join(format!("{name}.yml")),
            ))
        }
    };

    match &result {
        Ok(source) => info!(name, ?source, "template source resolved"),
        Err(err) => debug!(name, %err, "template source resolution failed"),
    }

    result
}

/// Recursively merge `overlay` into `base`, mutating `base` in place.
///
/// - Objects: recurse into each key; overlay keys are added or merged.
/// - Arrays, scalars, null: overlay replaces base.
pub fn deep_merge(base: &mut Value, overlay: &Value) {
    if let (Some(base_obj), Some(overlay_obj)) = (base.as_object_mut(), overlay.as_object()) {
        for (key, overlay_val) in overlay_obj {
            match base_obj.get_mut(key) {
                Some(base_val) => deep_merge(base_val, overlay_val),
                None => {
                    base_obj.insert(key.clone(), overlay_val.clone());
                }
            }
        }
    } else {
        *base = overlay.clone();
    }
}

/// Load all JSON/YAML fragments from `dir`, merge them in lexicographic
/// filename order, and return the combined Value.
pub fn load_template_dir(dir: &Path) -> Result<Value, TemplateError> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .map_err(|source| TemplateError::Read {
            path: dir.to_path_buf(),
            source,
        })?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_file() && is_template_extension(&entry.path()))
        .collect();

    entries.sort_by_key(|e| e.file_name());

    if entries.is_empty() {
        return Err(TemplateError::EmptyTemplateDir {
            path: dir.to_path_buf(),
        });
    }

    info!(
        dir = %dir.display(),
        fragment_count = entries.len(),
        "loading template directory"
    );

    let mut merged = load_template(&entries[0].path())?;

    for entry in &entries[1..] {
        let overlay = load_template(&entry.path())?;
        deep_merge(&mut merged, &overlay);
        debug!(fragment = %entry.path().display(), "merged template fragment");
    }

    Ok(merged)
}

/// Unified template loader: dispatches to `load_template` for files
/// or `load_template_dir` for directories.
pub fn load_template_or_dir(source: &TemplateSource) -> Result<Value, TemplateError> {
    match source {
        TemplateSource::File(path) => load_template(path),
        TemplateSource::Directory(dir) => load_template_dir(dir),
    }
}

pub(crate) fn is_valid_template_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }

    if name.contains('/') || name.contains('\\') {
        return false;
    }

    let mut components = Path::new(name).components();
    match components.next() {
        Some(std::path::Component::Normal(_)) if components.next().is_none() => {}
        _ => return false,
    }

    if Path::new(name).extension().is_some() {
        return false;
    }

    true
}

pub(crate) fn resolve_template_source_allowing_path(
    config_dir: &Path,
    template: &str,
) -> Result<TemplateSource, TemplateError> {
    if should_resolve_template_name(template) {
        resolve_template_source(config_dir, template)
    } else {
        Ok(TemplateSource::File(PathBuf::from(template)))
    }
}

pub(crate) fn write_json_pretty<E>(
    path: &Path,
    value: &Value,
    map_serialize: impl FnOnce(serde_json::Error) -> E,
    map_write: impl FnOnce(std::io::Error, PathBuf) -> E,
) -> Result<(), E> {
    let data = serde_json::to_string_pretty(value).map_err(map_serialize)?;
    fs::write(path, data).map_err(|source| map_write(source, path.to_path_buf()))
}

fn should_resolve_template_name(template: &str) -> bool {
    if template.contains('/') || template.contains('\\') {
        return false;
    }

    Path::new(template).extension().is_none()
}

pub fn list_templates(config_dir: &Path) -> Result<Vec<String>, TemplateError> {
    let template_dir = config_dir.join("template.d");
    let entries = fs::read_dir(&template_dir).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            TemplateError::MissingDir {
                path: template_dir.clone(),
            }
        } else {
            TemplateError::Read {
                path: template_dir.clone(),
                source,
            }
        }
    })?;

    let mut names = BTreeSet::new();
    for entry in entries {
        let entry = entry.map_err(|source| TemplateError::Read {
            path: template_dir.clone(),
            source,
        })?;
        let path = entry.path();
        if let Some(extension) = path.extension().and_then(|ext| ext.to_str())
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        {
            let extension = extension.to_ascii_lowercase();
            if matches!(extension.as_str(), "json" | "yaml" | "yml") {
                names.insert(stem.to_string());
            }
        }
        // Detect `.d` directories: e.g. `template.d/foo.d/`
        if path.is_dir()
            && let Some(dir_name) = path.file_name().and_then(|n| n.to_str())
            && let Some(stem) = dir_name.strip_suffix(".d")
            && !stem.is_empty()
            && count_template_files(&path) > 0
        {
            names.insert(stem.to_string());
        }
    }
    Ok(names.into_iter().collect())
}

pub fn apply_alias_models(value: &mut Value, palette: &Palette) {
    let Some(agents) = value.get_mut("agent").and_then(Value::as_object_mut) else {
        return;
    };

    let regex = match Regex::new(r"^\{\{\s*([^\}]+?)\s*\}\}$") {
        Ok(regex) => regex,
        Err(_) => return,
    };

    for agent in agents.values_mut() {
        let Some(agent_obj) = agent.as_object_mut() else {
            continue;
        };

        let alias = match agent_obj.get("model") {
            Some(Value::String(model)) => {
                let captures = match regex.captures(model) {
                    Some(captures) => captures,
                    None => continue,
                };
                let alias = match captures.get(1) {
                    Some(alias) => alias.as_str().trim(),
                    None => continue,
                };
                alias.to_string()
            }
            _ => continue,
        };

        let resolved_alias = if palette.agents.contains_key(&alias) {
            Some(alias)
        } else {
            alias
                .strip_prefix("agent-")
                .and_then(|value| value.strip_suffix("-model"))
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .filter(|value| palette.agents.contains_key(value))
        };

        let Some(resolved_alias) = resolved_alias else {
            continue;
        };

        let Some(config) = palette.agents.get(&resolved_alias) else {
            continue;
        };

        agent_obj.insert("model".to_string(), Value::String(config.model.clone()));
        if let Some(variant) = &config.variant {
            agent_obj.insert("variant".to_string(), Value::String(variant.clone()));
        }

        if let Some(reasoning) = &config.reasoning {
            match reasoning {
                Reasoning::Bool(true) => {
                    agent_obj.insert(
                        "reasoningEffort".to_string(),
                        Value::String("high".to_string()),
                    );
                }
                Reasoning::Bool(false) => {}
                Reasoning::Object(cfg) => {
                    if let Some(effort) = &cfg.effort {
                        agent_obj
                            .insert("reasoningEffort".to_string(), Value::String(effort.clone()));
                    }
                    if let Some(text_verbosity) = &cfg.text_verbosity {
                        agent_obj.insert(
                            "textVerbosity".to_string(),
                            Value::String(text_verbosity.clone()),
                        );
                    }
                }
            }
        }
    }
}

pub fn build_mapping(palette: &Palette) -> HashMap<String, Value> {
    let mut mapping = HashMap::new();
    for (key, value) in &palette.mapping {
        mapping.insert(key.clone(), value.clone());
    }
    for (agent, config) in &palette.agents {
        mapping.insert(agent.clone(), Value::String(config.model.clone()));
        mapping.insert(
            format!("agent-{agent}-model"),
            Value::String(config.model.clone()),
        );
        if let Some(variant) = &config.variant {
            mapping.insert(format!("{agent}-variant"), Value::String(variant.clone()));
            mapping.insert(
                format!("agent-{agent}-variant"),
                Value::String(variant.clone()),
            );
        }
        if let Some(reasoning) = &config.reasoning {
            match reasoning {
                Reasoning::Bool(true) => {
                    mapping.insert(
                        format!("agent-{agent}-reasoning-effort"),
                        Value::String("high".to_string()),
                    );
                }
                Reasoning::Bool(false) => {}
                Reasoning::Object(cfg) => {
                    if let Some(effort) = &cfg.effort {
                        mapping.insert(
                            format!("agent-{agent}-reasoning-effort"),
                            Value::String(effort.clone()),
                        );
                    }
                    if let Some(text_verbosity) = &cfg.text_verbosity {
                        mapping.insert(
                            format!("agent-{agent}-text-verbosity"),
                            Value::String(text_verbosity.clone()),
                        );
                    }
                }
            }
        }
    }
    mapping
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;
    use std::path::Path;

    use serde_json::json;
    use tempfile::TempDir;

    use super::{
        TemplateError, TemplateSource, apply_alias_models, build_mapping, deep_merge,
        is_template_dir, is_valid_template_name, list_templates, load_template, load_template_dir,
        load_template_or_dir, resolve_template_source,
    };
    use crate::config::{AgentConfig, Palette, Reasoning, ReasoningCfg};
    #[test]
    fn load_template_reads_json() {
        let temp_dir = TempDir::new().expect("temp dir");
        let file_path = temp_dir.path().join("default.json");
        let mut file = fs::File::create(&file_path).expect("create file");
        file.write_all(br#"{"agent": {"name": "build"}}"#)
            .expect("write json");

        let value = load_template(&file_path).expect("load template");
        assert_eq!(value["agent"]["name"], "build");
    }

    #[test]
    fn load_template_reads_yaml() {
        let temp_dir = TempDir::new().expect("temp dir");
        let file_path = temp_dir.path().join("default.yaml");
        let mut file = fs::File::create(&file_path).expect("create file");
        file.write_all(
            br#"
agent:
  name: build
  enabled: true
  count: 2
  flags:
    - fast
    - safe
"#,
        )
        .expect("write yaml");

        let value = load_template(&file_path).expect("load template");
        assert_eq!(value["agent"]["name"], "build");
        assert_eq!(value["agent"]["enabled"], true);
        assert_eq!(value["agent"]["count"], 2);
        assert_eq!(value["agent"]["flags"], json!(["fast", "safe"]));
    }

    #[test]
    fn load_template_rejects_unsupported_extension() {
        let temp_dir = TempDir::new().expect("temp dir");
        let file_path = temp_dir.path().join("default.toml");
        let mut file = fs::File::create(&file_path).expect("create file");
        file.write_all(br#"[agent]\nname = \"build\""#)
            .expect("write toml");

        let err = load_template(&file_path).expect_err("unsupported extension");
        match err {
            TemplateError::UnsupportedExtension { extension, .. } => {
                assert_eq!(extension, "toml");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn build_mapping_includes_models_and_variants() {
        let mut agents = std::collections::BTreeMap::new();
        agents.insert(
            "build".to_string(),
            AgentConfig {
                model: "openrouter/build".to_string(),
                variant: Some("mini".to_string()),
                reasoning: None,
            },
        );
        agents.insert(
            "review".to_string(),
            AgentConfig {
                model: "openrouter/review".to_string(),
                variant: None,
                reasoning: None,
            },
        );
        let palette = Palette {
            agents,
            mapping: std::collections::BTreeMap::from([(
                "build-config".to_string(),
                json!({"model": "openrouter/build", "variant": "mini"}),
            )]),
        };
        let mapping = build_mapping(&palette);

        assert_eq!(mapping.get("build").unwrap(), &json!("openrouter/build"));
        assert_eq!(mapping.get("build-variant").unwrap(), &json!("mini"));
        assert_eq!(
            mapping.get("build-config").unwrap(),
            &json!({"model": "openrouter/build", "variant": "mini"})
        );
        assert_eq!(
            mapping.get("agent-build-model").unwrap(),
            &json!("openrouter/build")
        );
        assert_eq!(mapping.get("agent-build-variant").unwrap(), &json!("mini"));
        assert_eq!(mapping.get("review").unwrap(), &json!("openrouter/review"));
        assert!(!mapping.contains_key("review-variant"));
        assert_eq!(
            mapping.get("agent-review-model").unwrap(),
            &json!("openrouter/review")
        );
        assert!(!mapping.contains_key("agent-review-variant"));
    }

    #[test]
    fn apply_alias_models_overwrites_model_variant_and_reasoning() {
        let mut agents = std::collections::BTreeMap::new();
        agents.insert(
            "build".to_string(),
            AgentConfig {
                model: "openrouter/build".to_string(),
                variant: Some("mini".to_string()),
                reasoning: Some(Reasoning::Bool(true)),
            },
        );
        agents.insert(
            "review".to_string(),
            AgentConfig {
                model: "openrouter/review".to_string(),
                variant: None,
                reasoning: Some(Reasoning::Object(ReasoningCfg {
                    effort: Some("medium".to_string()),
                    text_verbosity: Some("low".to_string()),
                })),
            },
        );
        let palette = Palette {
            agents,
            mapping: std::collections::BTreeMap::new(),
        };

        let mut value = json!({
            "agent": {
                "build": {
                    "model": "{{ build }}"
                },
                "review": {
                    "model": "{{review}}",
                    "variant": "existing",
                    "reasoningEffort": "low",
                    "textVerbosity": "high"
                },
                "other": {
                    "model": "{{missing}}",
                    "variant": "keep"
                },
                "noop": {
                    "model": "openrouter/noop"
                }
            }
        });

        apply_alias_models(&mut value, &palette);

        assert_eq!(value["agent"]["build"]["model"], "openrouter/build");
        assert_eq!(value["agent"]["build"]["variant"], "mini");
        assert_eq!(value["agent"]["build"]["reasoningEffort"], "high");
        assert!(value["agent"]["build"].get("textVerbosity").is_none());

        assert_eq!(value["agent"]["review"]["model"], "openrouter/review");
        assert_eq!(value["agent"]["review"]["variant"], "existing");
        assert_eq!(value["agent"]["review"]["reasoningEffort"], "medium");
        assert_eq!(value["agent"]["review"]["textVerbosity"], "low");

        assert_eq!(value["agent"]["other"]["model"], "{{missing}}");
        assert_eq!(value["agent"]["other"]["variant"], "keep");
        assert_eq!(value["agent"]["noop"]["model"], "openrouter/noop");
    }

    #[test]
    fn list_templates_filters_supported_extensions() {
        let temp_dir = TempDir::new().expect("temp dir");
        let template_dir = temp_dir.path().join("template.d");
        fs::create_dir_all(&template_dir).expect("create template dir");

        write_file(template_dir.join("default.json"), "{}");
        write_file(template_dir.join("alt.yaml"), "{}");
        write_file(template_dir.join("other.yml"), "{}");
        write_file(template_dir.join("ignored.txt"), "{}");
        write_file(template_dir.join("another.toml"), "{}");

        let names = list_templates(temp_dir.path()).expect("list templates");
        assert_eq!(names, vec!["alt", "default", "other"]);
    }

    fn write_file(path: impl AsRef<Path>, contents: &str) {
        let path = path.as_ref();
        let mut file = fs::File::create(path).expect("create file");
        file.write_all(contents.as_bytes()).expect("write file");
    }

    #[test]
    fn validate_template_name_accepts_base_names() {
        for name in ["default", "my-template", "my_template"] {
            assert!(is_valid_template_name(name));
        }
    }

    #[test]
    fn validate_template_name_rejects_paths_and_extensions() {
        for name in [
            "default.json",
            "default.yaml",
            "default.yml",
            "foo/bar",
            "../default",
            "./default",
            "foo\\bar",
            "",
        ] {
            assert!(!is_valid_template_name(name));
        }
    }

    // === deep_merge tests ===

    #[test]
    fn deep_merge_disjoint_objects() {
        let mut base = json!({"a": 1});
        let overlay = json!({"b": 2});
        deep_merge(&mut base, &overlay);
        assert_eq!(base, json!({"a": 1, "b": 2}));
    }

    #[test]
    fn deep_merge_nested_recursion() {
        let mut base = json!({"agent": {"build": {"model": "gpt-4"}}});
        let overlay = json!({"agent": {"build": {"variant": "mini"}}});
        deep_merge(&mut base, &overlay);
        assert_eq!(
            base,
            json!({"agent": {"build": {"model": "gpt-4", "variant": "mini"}}})
        );
    }

    #[test]
    fn deep_merge_scalar_override() {
        let mut base = json!({"a": 1});
        let overlay = json!({"a": 2});
        deep_merge(&mut base, &overlay);
        assert_eq!(base, json!({"a": 2}));
    }

    #[test]
    fn deep_merge_array_replacement() {
        let mut base = json!({"a": [1, 2]});
        let overlay = json!({"a": [3]});
        deep_merge(&mut base, &overlay);
        assert_eq!(base, json!({"a": [3]}));
    }

    #[test]
    fn deep_merge_null_overlay() {
        let mut base = json!({"a": 1});
        let overlay = json!({"a": null});
        deep_merge(&mut base, &overlay);
        assert_eq!(base, json!({"a": null}));
    }

    #[test]
    fn deep_merge_type_mismatch_overlay_wins() {
        let mut base = json!({"a": {"b": 1}});
        let overlay = json!({"a": "string"});
        deep_merge(&mut base, &overlay);
        assert_eq!(base, json!({"a": "string"}));
    }

    #[test]
    fn deep_merge_empty_overlay_noop() {
        let mut base = json!({"a": 1});
        let overlay = json!({});
        deep_merge(&mut base, &overlay);
        assert_eq!(base, json!({"a": 1}));
    }

    #[test]
    fn deep_merge_deeply_nested() {
        let mut base = json!({"l1": {"l2": {"l3": {"l4": {"l5": "original"}}}}});
        let overlay = json!({"l1": {"l2": {"l3": {"l4": {"l5": "replaced"}}}}});
        deep_merge(&mut base, &overlay);
        assert_eq!(base["l1"]["l2"]["l3"]["l4"]["l5"], "replaced");
    }

    // === resolve_template_source tests ===

    #[test]
    fn resolve_file_only() {
        let temp_dir = TempDir::new().expect("temp dir");
        let template_dir = temp_dir.path().join("template.d");
        fs::create_dir_all(&template_dir).expect("create dir");
        write_file(template_dir.join("foo.json"), r#"{"agent":{}}"#);

        let result = resolve_template_source(temp_dir.path(), "foo").expect("resolve");
        assert_eq!(result, TemplateSource::File(template_dir.join("foo.json")));
    }

    #[test]
    fn resolve_dir_only() {
        let temp_dir = TempDir::new().expect("temp dir");
        let template_dir = temp_dir.path().join("template.d");
        let dir_path = template_dir.join("foo.d");
        fs::create_dir_all(&dir_path).expect("create dir");
        write_file(dir_path.join("base.json"), r#"{"agent":{}}"#);

        let result = resolve_template_source(temp_dir.path(), "foo").expect("resolve");
        assert_eq!(result, TemplateSource::Directory(dir_path));
    }

    #[test]
    fn resolve_ambiguous_error() {
        let temp_dir = TempDir::new().expect("temp dir");
        let template_dir = temp_dir.path().join("template.d");
        let dir_path = template_dir.join("foo.d");
        fs::create_dir_all(&dir_path).expect("create dir");
        write_file(template_dir.join("foo.json"), r#"{"agent":{}}"#);
        write_file(dir_path.join("base.json"), r#"{"agent":{}}"#);

        let result = resolve_template_source(temp_dir.path(), "foo");
        match result {
            Err(TemplateError::AmbiguousTemplate {
                name,
                file_path,
                dir_path: dp,
            }) => {
                assert_eq!(name, "foo");
                assert!(file_path.ends_with("foo.json"));
                assert!(dp.ends_with("foo.d"));
            }
            other => panic!("expected AmbiguousTemplate, got: {other:?}"),
        }
    }

    #[test]
    fn resolve_empty_dir_error() {
        let temp_dir = TempDir::new().expect("temp dir");
        let template_dir = temp_dir.path().join("template.d");
        let dir_path = template_dir.join("foo.d");
        fs::create_dir_all(&dir_path).expect("create dir");

        let result = resolve_template_source(temp_dir.path(), "foo");
        match result {
            Err(TemplateError::EmptyTemplateDir { path }) => {
                assert!(path.ends_with("foo.d"));
            }
            other => panic!("expected EmptyTemplateDir, got: {other:?}"),
        }
    }

    #[test]
    fn resolve_neither_fallback() {
        let temp_dir = TempDir::new().expect("temp dir");
        let template_dir = temp_dir.path().join("template.d");
        fs::create_dir_all(&template_dir).expect("create dir");

        let result = resolve_template_source(temp_dir.path(), "foo").expect("resolve");
        assert_eq!(result, TemplateSource::File(template_dir.join("foo.yml")));
    }

    #[test]
    fn resolve_ambiguous_yaml() {
        let temp_dir = TempDir::new().expect("temp dir");
        let template_dir = temp_dir.path().join("template.d");
        let dir_path = template_dir.join("foo.d");
        fs::create_dir_all(&dir_path).expect("create dir");
        write_file(template_dir.join("foo.yaml"), "agent: {}");
        write_file(dir_path.join("bar.json"), r#"{"agent":{}}"#);

        let result = resolve_template_source(temp_dir.path(), "foo");
        match result {
            Err(TemplateError::AmbiguousTemplate {
                name, file_path, ..
            }) => {
                assert_eq!(name, "foo");
                assert!(file_path.ends_with("foo.yaml"));
            }
            other => panic!("expected AmbiguousTemplate, got: {other:?}"),
        }
    }

    #[test]
    fn resolve_file_priority_json_over_yaml() {
        let temp_dir = TempDir::new().expect("temp dir");
        let template_dir = temp_dir.path().join("template.d");
        fs::create_dir_all(&template_dir).expect("create dir");
        write_file(template_dir.join("foo.json"), r#"{"agent":{}}"#);
        write_file(template_dir.join("foo.yaml"), "agent: {}");

        let result = resolve_template_source(temp_dir.path(), "foo").expect("resolve");
        assert_eq!(result, TemplateSource::File(template_dir.join("foo.json")));
    }

    #[test]
    fn resolve_dir_no_templates() {
        let temp_dir = TempDir::new().expect("temp dir");
        let template_dir = temp_dir.path().join("template.d");
        let dir_path = template_dir.join("foo.d");
        fs::create_dir_all(&dir_path).expect("create dir");
        write_file(dir_path.join("readme.txt"), "not a template");

        let result = resolve_template_source(temp_dir.path(), "foo");
        match result {
            Err(TemplateError::EmptyTemplateDir { path }) => {
                assert!(path.ends_with("foo.d"));
            }
            other => panic!("expected EmptyTemplateDir, got: {other:?}"),
        }
    }

    // === load_template_dir tests ===

    #[test]
    fn load_template_dir_ordering() {
        let temp_dir = TempDir::new().expect("temp dir");
        let dir = temp_dir.path();
        write_file(dir.join("b.json"), r#"{"key": "b"}"#);
        write_file(dir.join("a.json"), r#"{"key": "a"}"#);

        let result = load_template_dir(dir).expect("load dir");
        // a.json is base (alphabetically first), b.json overlays → "b" wins
        assert_eq!(result["key"], "b");
    }

    #[test]
    fn load_template_dir_empty_error() {
        let temp_dir = TempDir::new().expect("temp dir");
        let result = load_template_dir(temp_dir.path());
        match result {
            Err(TemplateError::EmptyTemplateDir { .. }) => {}
            other => panic!("expected EmptyTemplateDir, got: {other:?}"),
        }
    }

    #[test]
    fn load_template_dir_ignores_non_templates() {
        let temp_dir = TempDir::new().expect("temp dir");
        let dir = temp_dir.path();
        write_file(dir.join("README.md"), "# readme");
        write_file(dir.join(".gitkeep"), "");

        let result = load_template_dir(dir);
        match result {
            Err(TemplateError::EmptyTemplateDir { .. }) => {}
            other => panic!("expected EmptyTemplateDir, got: {other:?}"),
        }
    }

    #[test]
    fn load_template_dir_three_fragments() {
        let temp_dir = TempDir::new().expect("temp dir");
        let dir = temp_dir.path();
        write_file(
            dir.join("01-base.json"),
            r#"{"agent": {"build": {"model": "gpt-4"}}, "extra": "keep"}"#,
        );
        write_file(
            dir.join("02-override.json"),
            r#"{"agent": {"build": {"variant": "mini"}}}"#,
        );
        write_file(
            dir.join("03-final.json"),
            r#"{"agent": {"build": {"model": "gpt-5"}}}"#,
        );

        let result = load_template_dir(dir).expect("load dir");
        assert_eq!(result["agent"]["build"]["model"], "gpt-5");
        assert_eq!(result["agent"]["build"]["variant"], "mini");
        assert_eq!(result["extra"], "keep");
    }

    #[test]
    fn load_template_dir_mixed_extensions() {
        let temp_dir = TempDir::new().expect("temp dir");
        let dir = temp_dir.path();
        write_file(
            dir.join("01-base.json"),
            r#"{"agent": {"build": {"model": "gpt-4"}}}"#,
        );
        write_file(
            dir.join("02-override.yaml"),
            "agent:\n  build:\n    variant: mini",
        );

        let result = load_template_dir(dir).expect("load dir");
        assert_eq!(result["agent"]["build"]["model"], "gpt-4");
        assert_eq!(result["agent"]["build"]["variant"], "mini");
    }

    // === load_template_or_dir tests ===

    #[test]
    fn load_template_or_dir_dispatches_file() {
        let temp_dir = TempDir::new().expect("temp dir");
        let file_path = temp_dir.path().join("test.json");
        write_file(&file_path, r#"{"key": "value"}"#);

        let source = TemplateSource::File(file_path);
        let result = load_template_or_dir(&source).expect("load");
        assert_eq!(result["key"], "value");
    }

    #[test]
    fn load_template_or_dir_dispatches_directory() {
        let temp_dir = TempDir::new().expect("temp dir");
        let dir = temp_dir.path().join("fragments");
        fs::create_dir_all(&dir).expect("create dir");
        write_file(dir.join("base.json"), r#"{"key": "value"}"#);

        let source = TemplateSource::Directory(dir);
        let result = load_template_or_dir(&source).expect("load");
        assert_eq!(result["key"], "value");
    }

    // === list_templates with directories ===

    #[test]
    fn list_templates_includes_dirs() {
        let temp_dir = TempDir::new().expect("temp dir");
        let template_dir = temp_dir.path().join("template.d");
        fs::create_dir_all(&template_dir).expect("create dir");

        write_file(template_dir.join("alpha.json"), "{}");
        let beta_dir = template_dir.join("beta.d");
        fs::create_dir_all(&beta_dir).expect("create dir");
        write_file(beta_dir.join("base.json"), "{}");

        let names = list_templates(temp_dir.path()).expect("list");
        assert_eq!(names, vec!["alpha", "beta"]);
    }

    #[test]
    fn list_templates_skips_empty_dirs() {
        let temp_dir = TempDir::new().expect("temp dir");
        let template_dir = temp_dir.path().join("template.d");
        fs::create_dir_all(&template_dir).expect("create dir");

        write_file(template_dir.join("alpha.json"), "{}");
        let empty_dir = template_dir.join("empty.d");
        fs::create_dir_all(&empty_dir).expect("create dir");

        let names = list_templates(temp_dir.path()).expect("list");
        assert_eq!(names, vec!["alpha"]);
    }

    // === load_template_dir with _global fragment ===

    #[test]
    fn load_template_dir_global_plus_named_fragments() {
        let temp_dir = TempDir::new().expect("temp dir");
        let dir = temp_dir.path();
        write_file(
            dir.join("_global.json"),
            r#"{"agent": {"build": {"model": "gpt-4"}}, "theme": "dark"}"#,
        );
        write_file(
            dir.join("review.json"),
            r#"{"agent": {"review": {"model": "gpt-5"}}}"#,
        );

        let result = load_template_dir(dir).expect("load dir");
        // _global.json is base (alphabetically first), review.json overlays
        assert_eq!(result["agent"]["build"]["model"], "gpt-4");
        assert_eq!(result["agent"]["review"]["model"], "gpt-5");
        assert_eq!(result["theme"], "dark");
    }

    // === is_template_dir tests ===

    #[test]
    fn is_template_dir_true_for_dir_with_fragment() {
        let temp_dir = TempDir::new().expect("temp dir");
        let dir = temp_dir.path();
        write_file(dir.join("base.json"), r#"{"key": "value"}"#);

        assert!(is_template_dir(dir));
    }

    #[test]
    fn is_template_dir_false_for_file() {
        let temp_dir = TempDir::new().expect("temp dir");
        let file_path = temp_dir.path().join("template.json");
        write_file(&file_path, r#"{"key": "value"}"#);

        assert!(!is_template_dir(&file_path));
    }

    #[test]
    fn is_template_dir_false_for_empty_dir() {
        let temp_dir = TempDir::new().expect("temp dir");
        assert!(!is_template_dir(temp_dir.path()));
    }
}
