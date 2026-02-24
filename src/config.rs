use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to locate config directory")]
    MissingConfigDir,
    #[error("failed to read model configs at {path}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse model configs at {path}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ModelConfigs {
    pub palettes: BTreeMap<String, Palette>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Palette {
    pub agents: BTreeMap<String, AgentConfig>,
    #[serde(default)]
    pub mapping: BTreeMap<String, Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    pub model: String,
    #[serde(default)]
    pub variant: Option<String>,
    #[serde(default)]
    pub reasoning: Option<Reasoning>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub enum Reasoning {
    Bool(bool),
    Object(ReasoningCfg),
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReasoningCfg {
    pub effort: Option<String>,
    pub text_verbosity: Option<String>,
}

pub fn resolve_config_dir(override_dir: Option<PathBuf>) -> Result<PathBuf, ConfigError> {
    if let Some(path) = override_dir {
        return Ok(path);
    }

    let dirs = BaseDirs::new().ok_or(ConfigError::MissingConfigDir)?;
    Ok(dirs.config_dir().join("opencode-config.d"))
}

pub fn load_model_configs(config_dir: &Path) -> Result<ModelConfigs, ConfigError> {
    let path = config_dir.join("model-configs.yaml");
    let data = fs::read_to_string(&path).map_err(|source| ConfigError::Read {
        path: path.clone(),
        source,
    })?;
    serde_yaml::from_str(&data).map_err(|source| ConfigError::Parse { path, source })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;

    use tempfile::TempDir;

    use super::{ConfigError, load_model_configs};

    const SAMPLE_YAML: &str = r#"
palettes:
  github:
    agents:
      build:
        model: openrouter/openai/gpt-4o
        variant: mini
        reasoning:
          effort: high
          text_verbosity: low
      review:
        model: openrouter/openai/gpt-4o
        reasoning: true
    mapping:
      build-count: 3
      build-flags:
        - fast
        - safe
  default:
    agents:
      build:
        model: openrouter/openai/gpt-4.1
        reasoning: false
"#;

    #[test]
    fn load_model_configs_parses_yaml() {
        let temp_dir = TempDir::new().expect("temp dir");
        let file_path = temp_dir.path().join("model-configs.yaml");
        let mut file = fs::File::create(&file_path).expect("create file");
        file.write_all(SAMPLE_YAML.as_bytes()).expect("write yaml");

        let configs = load_model_configs(temp_dir.path()).expect("load configs");
        assert_eq!(configs.palettes.len(), 2);
        let github = configs.palettes.get("github").expect("github palette");
        assert_eq!(github.agents.len(), 2);
        let build = github.agents.get("build").expect("build agent");
        assert_eq!(build.model, "openrouter/openai/gpt-4o");
        assert_eq!(build.variant.as_deref(), Some("mini"));
        assert_eq!(
            build.reasoning,
            Some(super::Reasoning::Object(super::ReasoningCfg {
                effort: Some("high".to_string()),
                text_verbosity: Some("low".to_string()),
            }))
        );
        let review = github.agents.get("review").expect("review agent");
        assert_eq!(review.model, "openrouter/openai/gpt-4o");
        assert_eq!(review.variant, None);
        assert_eq!(review.reasoning, Some(super::Reasoning::Bool(true)));
        assert_eq!(
            github.mapping.get("build-count"),
            Some(&serde_json::json!(3))
        );
        assert_eq!(
            github.mapping.get("build-flags"),
            Some(&serde_json::json!(["fast", "safe"]))
        );
    }

    #[test]
    fn load_model_configs_reports_yaml_error() {
        let temp_dir = TempDir::new().expect("temp dir");
        let file_path = temp_dir.path().join("model-configs.yaml");
        let mut file = fs::File::create(&file_path).expect("create file");
        file.write_all(b"palettes: [").expect("write yaml");

        let error = load_model_configs(temp_dir.path()).expect_err("should error");
        match error {
            ConfigError::Parse { .. } => {}
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
