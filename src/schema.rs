use std::fs;
use std::path::PathBuf;

use serde_json::{Map, Value, json};
use thiserror::Error;

use crate::config::{ConfigError, Palette, load_model_configs};

#[derive(Debug, Clone)]
pub struct SchemaGenerateOptions {
    pub palette: Option<String>,
    pub out_dir: PathBuf,
    pub config_dir: PathBuf,
}

#[derive(Debug, Error)]
pub enum SchemaError {
    #[error("palette name is required")]
    MissingPaletteName,
    #[error("palette not found: {name}")]
    MissingPalette { name: String },
    #[error("config error: {0}")]
    Config(#[from] ConfigError),
    #[error("failed to write schema to {path}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialize schema")]
    Serialize {
        #[source]
        source: serde_json::Error,
    },
}

pub fn generate_schema_file(options: SchemaGenerateOptions) -> Result<PathBuf, SchemaError> {
    let palette_name = options.palette.ok_or(SchemaError::MissingPaletteName)?;
    let configs = load_model_configs(&options.config_dir)?;
    let palette =
        configs
            .palettes
            .get(&palette_name)
            .ok_or_else(|| SchemaError::MissingPalette {
                name: palette_name.clone(),
            })?;

    let schema = build_schema(palette);
    fs::create_dir_all(&options.out_dir).map_err(|source| SchemaError::Write {
        path: options.out_dir.clone(),
        source,
    })?;

    let file_name = format!("opencode.{palette_name}.schema.json");
    let path = options.out_dir.join(file_name);
    let data = serde_json::to_string_pretty(&schema)
        .map_err(|source| SchemaError::Serialize { source })?;
    fs::write(&path, data).map_err(|source| SchemaError::Write {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

fn build_schema(palette: &Palette) -> Value {
    let mut agent_props = Map::new();
    for agent in palette.agents.keys() {
        agent_props.insert(
            agent.clone(),
            json!({
                "type": "object",
                "properties": {
                    "model": { "type": "string" },
                    "variant": { "type": "string" },
                    "reasoningEffort": { "type": "string" },
                    "textVerbosity": { "type": "string" }
                },
                "additionalProperties": true
            }),
        );
    }

    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "opencode.json",
        "type": "object",
        "properties": {
            "agent": {
                "type": "object",
                "properties": agent_props,
                "additionalProperties": true
            }
        },
        "additionalProperties": true
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::{SchemaGenerateOptions, generate_schema_file};

    #[test]
    fn generate_schema_writes_file() {
        let config_dir = TempDir::new().expect("config dir");
        let out_dir = TempDir::new().expect("out dir");
        let yaml = r#"
palettes:
  default:
    agents:
      build:
        model: openrouter/openai/gpt-4o
"#;
        fs::write(config_dir.path().join("model-configs.yaml"), yaml)
            .expect("write model-configs.yaml");

        let path = generate_schema_file(SchemaGenerateOptions {
            palette: Some("default".to_string()),
            out_dir: out_dir.path().to_path_buf(),
            config_dir: config_dir.path().to_path_buf(),
        })
        .expect("generate schema");

        let data = fs::read_to_string(&path).expect("read schema");
        let value: serde_json::Value = serde_json::from_str(&data).expect("parse schema");
        assert_eq!(
            value["properties"]["agent"]["properties"]["build"]["properties"]["model"]["type"],
            "string"
        );
    }
}
