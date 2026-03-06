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

/// A single finding produced by JSON Schema validation.
#[derive(Debug, Clone)]
pub struct SchemaFinding {
    /// JSON Pointer path to the invalid value (e.g. `"/agent/build/model"`).
    pub instance_path: String,
    /// JSON Pointer path into the schema that triggered the error.
    pub schema_path: String,
    /// Human-readable description of the violation.
    pub message: String,
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
    #[error("invalid schema: {message}")]
    InvalidSchema { message: String },
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

pub(crate) fn build_schema(palette: &Palette) -> Value {
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

/// Validate `instance` against `schema` using JSON Schema.
///
/// Compiles the schema once via the `jsonschema` crate and maps each
/// validation error into a [`SchemaFinding`] with path and message fields.
///
/// Returns an empty `Vec` when the instance is valid.
/// Returns one `SchemaFinding` per violation when invalid.
/// Returns [`SchemaError::InvalidSchema`] if the schema itself cannot be compiled.
pub fn validate_against_schema(
    schema: &Value,
    instance: &Value,
) -> Result<Vec<SchemaFinding>, SchemaError> {
    let validator = jsonschema::validator_for(schema).map_err(|e| SchemaError::InvalidSchema {
        message: e.to_string(),
    })?;

    let findings = validator
        .iter_errors(instance)
        .map(|err| SchemaFinding {
            instance_path: err.instance_path.to_string(),
            schema_path: err.schema_path.to_string(),
            message: err.to_string(),
        })
        .collect();

    Ok(findings)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use tempfile::TempDir;

    use super::{SchemaGenerateOptions, generate_schema_file, validate_against_schema};

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

    #[test]
    fn valid_document_passes() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            }
        });
        let instance = json!({ "name": "ok" });

        let findings = validate_against_schema(&schema, &instance).expect("schema should compile");
        assert!(findings.is_empty(), "expected no findings for valid doc");
    }

    #[test]
    fn invalid_type_produces_finding() {
        let schema = json!({
            "type": "object",
            "properties": {
                "model": { "type": "string" }
            }
        });
        let instance = json!({ "model": 42 });

        let findings = validate_against_schema(&schema, &instance).expect("schema should compile");
        assert!(!findings.is_empty(), "expected at least one finding");
        assert_eq!(findings[0].instance_path, "/model");
        assert!(
            findings[0].message.contains("string"),
            "message should mention 'string', got: {}",
            findings[0].message
        );
    }

    #[test]
    fn missing_required_field_produces_finding() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            },
            "required": ["name"]
        });
        let instance = json!({});

        let findings = validate_against_schema(&schema, &instance).expect("schema should compile");
        assert!(!findings.is_empty(), "expected at least one finding");
        assert!(
            findings.iter().any(|f| f.message.contains("required")),
            "expected a finding mentioning 'required', got: {:?}",
            findings
        );
    }

    #[test]
    fn malformed_schema_returns_error() {
        let schema = json!({ "type": "not-a-real-type" });
        let instance = json!({});

        let result = validate_against_schema(&schema, &instance);
        assert!(result.is_err(), "expected Err for malformed schema");
        let err = result.unwrap_err();
        assert!(
            matches!(err, super::SchemaError::InvalidSchema { .. }),
            "expected InvalidSchema variant, got: {err:?}"
        );
    }

    #[test]
    fn multiple_violations_return_multiple_findings() {
        let schema = json!({
            "type": "object",
            "properties": {
                "first": { "type": "string" },
                "second": { "type": "string" }
            }
        });
        let instance = json!({ "first": 1, "second": 2 });

        let findings = validate_against_schema(&schema, &instance).expect("schema should compile");
        assert!(
            findings.len() >= 2,
            "expected at least 2 findings, got {}",
            findings.len()
        );
    }
}
