use std::path::PathBuf;
use thiserror::Error;

use crate::config::ConfigError;
use crate::render_builder::RenderBuilder;
use crate::substitute::SubstituteError;
use crate::template::TemplateError;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum OutputFormat {
    Json,
    Yaml,
}

pub struct RenderOptions {
    pub template: String,
    pub palette: String,
    pub format: OutputFormat,
    pub strict: bool,
    pub env_allow: bool,
    pub env_mask_logs: bool,
    pub config_dir: PathBuf,
}

pub struct RenderOutput {
    pub data: String,
    pub lines: usize,
}

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("config error: {0}")]
    Config(#[from] ConfigError),
    #[error("template error: {0}")]
    Template(#[from] TemplateError),
    #[error("substitution error: {0}")]
    Substitute(#[from] SubstituteError),
    #[error("invalid template name: {name}")]
    InvalidTemplateName { name: String },
    #[error("palette not found: {name}")]
    MissingPalette { name: String },
    #[error("failed to serialize output to json")]
    SerializeJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to serialize output to yaml")]
    SerializeYaml {
        #[source]
        source: serde_yaml::Error,
    },
}

pub fn render(options: RenderOptions) -> Result<RenderOutput, RenderError> {
    RenderBuilder::new(options)
        .load_configs()?
        .resolve_palette()?
        .resolve_template_path()?
        .load_template()?
        .apply_alias_models()?
        .build_mapping()?
        .substitute()?
        .serialize()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use serde_json::Value;
    use tempfile::TempDir;

    use super::{OutputFormat, RenderOptions, render};

    const SAMPLE_YAML: &str = r#"
palettes:
  default:
    agents:
      build:
        model: openrouter/openai/gpt-4o
        variant: mini
        reasoning: true
"#;

    const JSON_TEMPLATE: &str = r#"{
  "agent": {
    "build": {
      "model": "{{build}}",
      "variant": "{{build-variant}}"
    }
  },
  "description": "Build uses {{build}}"
}
"#;

    const YAML_TEMPLATE: &str = r#"
agent:
  build:
    model: "{{build}}"
    variant: "{{build-variant}}"
description: "Build uses {{build}}"
"#;

    fn write_config(config_dir: &Path) {
        fs::write(config_dir.join("model-configs.yaml"), SAMPLE_YAML)
            .expect("write model-configs.yaml");
        let template_dir = config_dir.join("template.d");
        fs::create_dir_all(&template_dir).expect("create template dir");
        fs::write(template_dir.join("default.json"), JSON_TEMPLATE).expect("write template");
    }

    #[test]
    fn render_resolves_template_name() {
        let config_dir = TempDir::new().expect("config dir");
        write_config(config_dir.path());

        let output = render(RenderOptions {
            template: "default".to_string(),
            palette: "default".to_string(),
            format: OutputFormat::Json,
            strict: false,
            env_allow: false,
            env_mask_logs: false,
            config_dir: config_dir.path().to_path_buf(),
        })
        .expect("render");

        let value: Value = serde_json::from_str(&output.data).expect("parse json");
        assert_eq!(value["agent"]["build"]["model"], "openrouter/openai/gpt-4o");
        assert_eq!(value["agent"]["build"]["variant"], "mini");
        assert_eq!(value["agent"]["build"]["reasoningEffort"], "high");
        assert_eq!(value["description"], "Build uses openrouter/openai/gpt-4o");
        assert_eq!(output.lines, output.data.lines().count());
    }

    #[test]
    fn render_accepts_template_path_with_extension() {
        let config_dir = TempDir::new().expect("config dir");
        fs::write(config_dir.path().join("model-configs.yaml"), SAMPLE_YAML)
            .expect("write model-configs.yaml");

        let template_path = config_dir.path().join("custom.yaml");
        fs::write(&template_path, YAML_TEMPLATE).expect("write yaml template");

        let output = render(RenderOptions {
            template: template_path.to_string_lossy().to_string(),
            palette: "default".to_string(),
            format: OutputFormat::Json,
            strict: false,
            env_allow: false,
            env_mask_logs: false,
            config_dir: config_dir.path().to_path_buf(),
        })
        .expect("render");

        let value: Value = serde_json::from_str(&output.data).expect("parse json");
        assert_eq!(value["agent"]["build"]["model"], "openrouter/openai/gpt-4o");
        assert_eq!(value["description"], "Build uses openrouter/openai/gpt-4o");
    }

    #[test]
    fn render_outputs_yaml() {
        let config_dir = TempDir::new().expect("config dir");
        write_config(config_dir.path());

        let output = render(RenderOptions {
            template: "default".to_string(),
            palette: "default".to_string(),
            format: OutputFormat::Yaml,
            strict: false,
            env_allow: false,
            env_mask_logs: false,
            config_dir: config_dir.path().to_path_buf(),
        })
        .expect("render");

        let yaml_value: serde_yaml::Value = serde_yaml::from_str(&output.data).expect("parse yaml");
        let json_value = serde_json::to_value(yaml_value).expect("convert yaml");
        assert_eq!(
            json_value["agent"]["build"]["model"],
            "openrouter/openai/gpt-4o"
        );
        assert_eq!(json_value["agent"]["build"]["variant"], "mini");
    }
}
