use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde_json::Value;
use serde_yaml;
use thiserror::Error;

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
}

pub struct TemplateLoader<State> {
    path: PathBuf,
    state: State,
}

struct TemplateSource;

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

impl TemplateLoader<TemplateSource> {
    pub fn from_path(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            state: TemplateSource,
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

pub(crate) fn resolve_template_name_path(config_dir: &Path, name: &str) -> PathBuf {
    let template_dir = config_dir.join("template.d");
    let json_path = template_dir.join(format!("{name}.json"));
    if json_path.exists() {
        return json_path;
    }
    let yaml_path = template_dir.join(format!("{name}.yaml"));
    if yaml_path.exists() {
        return yaml_path;
    }
    template_dir.join(format!("{name}.yml"))
}

pub(crate) fn resolve_template_path_allowing_path(config_dir: &Path, template: &str) -> PathBuf {
    if should_resolve_template_name(template) {
        resolve_template_name_path(config_dir, template)
    } else {
        PathBuf::from(template)
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
        TemplateError, apply_alias_models, build_mapping, is_valid_template_name, list_templates,
        load_template,
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
}
