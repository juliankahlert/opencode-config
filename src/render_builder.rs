use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::Value;

use crate::config::{ModelConfigs, Palette, load_model_configs};
use crate::render::{OutputFormat, RenderError, RenderOptions, RenderOutput};
use crate::substitute::substitute;
use crate::template::{
    apply_alias_models, build_mapping, load_template, resolve_template_path_allowing_path,
};

pub(crate) struct RenderBuilder<State> {
    options: RenderOptions,
    state: State,
}

pub(crate) struct Start;
pub(crate) struct ConfigsLoaded {
    configs: ModelConfigs,
}
pub(crate) struct PaletteResolved {
    palette: Palette,
}
pub(crate) struct TemplatePathResolved {
    palette: Palette,
    template_path: PathBuf,
}
pub(crate) struct TemplateLoaded {
    palette: Palette,
    template_value: Value,
}
pub(crate) struct AliasesApplied {
    palette: Palette,
    template_value: Value,
}
pub(crate) struct MappingBuilt {
    template_value: Value,
    mapping: HashMap<String, Value>,
}
pub(crate) struct Substituted {
    template_value: Value,
}

impl RenderBuilder<Start> {
    pub(crate) fn new(options: RenderOptions) -> Self {
        if options.env_allow {
            eprintln!("warning: --env-allow is currently a no-op");
        }
        if options.env_mask_logs {
            eprintln!("warning: --env-mask-logs is currently a no-op");
        }
        Self {
            options,
            state: Start,
        }
    }

    pub(crate) fn load_configs(self) -> Result<RenderBuilder<ConfigsLoaded>, RenderError> {
        let configs = load_model_configs(&self.options.config_dir)?;
        Ok(RenderBuilder {
            options: self.options,
            state: ConfigsLoaded { configs },
        })
    }
}

impl RenderBuilder<ConfigsLoaded> {
    pub(crate) fn resolve_palette(self) -> Result<RenderBuilder<PaletteResolved>, RenderError> {
        let RenderBuilder { options, state } = self;
        let palette = state
            .configs
            .palettes
            .get(&options.palette)
            .ok_or(RenderError::MissingPalette {
                name: options.palette.clone(),
            })?
            .clone();
        Ok(RenderBuilder {
            options,
            state: PaletteResolved { palette },
        })
    }
}

impl RenderBuilder<PaletteResolved> {
    pub(crate) fn resolve_template_path(
        self,
    ) -> Result<RenderBuilder<TemplatePathResolved>, RenderError> {
        let RenderBuilder { options, state } = self;
        if options.template.is_empty() {
            return Err(RenderError::InvalidTemplateName {
                name: options.template.clone(),
            });
        }
        let template_path =
            resolve_template_path_allowing_path(&options.config_dir, &options.template);
        Ok(RenderBuilder {
            options,
            state: TemplatePathResolved {
                palette: state.palette,
                template_path,
            },
        })
    }
}

impl RenderBuilder<TemplatePathResolved> {
    pub(crate) fn load_template(self) -> Result<RenderBuilder<TemplateLoaded>, RenderError> {
        let RenderBuilder { options, state } = self;
        let template_value = load_template(&state.template_path)?;
        Ok(RenderBuilder {
            options,
            state: TemplateLoaded {
                palette: state.palette,
                template_value,
            },
        })
    }
}

impl RenderBuilder<TemplateLoaded> {
    pub(crate) fn apply_alias_models(self) -> Result<RenderBuilder<AliasesApplied>, RenderError> {
        let RenderBuilder { options, state } = self;
        let TemplateLoaded {
            palette,
            mut template_value,
        } = state;
        apply_alias_models(&mut template_value, &palette);
        Ok(RenderBuilder {
            options,
            state: AliasesApplied {
                palette,
                template_value,
            },
        })
    }
}

impl RenderBuilder<AliasesApplied> {
    pub(crate) fn build_mapping(self) -> Result<RenderBuilder<MappingBuilt>, RenderError> {
        let RenderBuilder { options, state } = self;
        let AliasesApplied {
            palette,
            template_value,
        } = state;
        let mapping = build_mapping(&palette);
        Ok(RenderBuilder {
            options,
            state: MappingBuilt {
                template_value,
                mapping,
            },
        })
    }
}

impl RenderBuilder<MappingBuilt> {
    pub(crate) fn substitute(self) -> Result<RenderBuilder<Substituted>, RenderError> {
        let RenderBuilder { options, state } = self;
        let MappingBuilt {
            mut template_value,
            mapping,
        } = state;
        substitute(&mut template_value, &mapping, options.strict)?;
        Ok(RenderBuilder {
            options,
            state: Substituted { template_value },
        })
    }
}

impl RenderBuilder<Substituted> {
    pub(crate) fn serialize(self) -> Result<RenderOutput, RenderError> {
        let RenderBuilder { options, state } = self;
        let data = serialize_output(&state.template_value, options.format)?;
        let lines = data.lines().count();
        Ok(RenderOutput { data, lines })
    }
}

fn serialize_output(value: &Value, format: OutputFormat) -> Result<String, RenderError> {
    match format {
        OutputFormat::Json => serde_json::to_string_pretty(value)
            .map_err(|source| RenderError::SerializeJson { source }),
        OutputFormat::Yaml => {
            serde_yaml::to_string(value).map_err(|source| RenderError::SerializeYaml { source })
        }
    }
}

// Template resolution is centralized in template.rs to keep behavior consistent.

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use serde_json::Value;
    use tempfile::TempDir;

    use crate::render::{OutputFormat, RenderError, RenderOptions};

    use super::RenderBuilder;

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
}"#;

    fn write_model_configs(config_dir: &Path, yaml: &str) {
        fs::write(config_dir.join("model-configs.yaml"), yaml).expect("write model-configs.yaml");
    }

    fn write_template(config_dir: &Path, name: &str, content: &str) {
        let template_dir = config_dir.join("template.d");
        fs::create_dir_all(&template_dir).expect("create template dir");
        fs::write(template_dir.join(name), content).expect("write template");
    }

    fn default_options(config_dir: &Path) -> RenderOptions {
        RenderOptions {
            template: "default".to_string(),
            palette: "default".to_string(),
            format: OutputFormat::Json,
            strict: false,
            env_allow: false,
            env_mask_logs: false,
            config_dir: config_dir.to_path_buf(),
        }
    }

    #[test]
    fn end_to_end_render_outputs_json() {
        let config_dir = TempDir::new().expect("config dir");
        write_model_configs(config_dir.path(), SAMPLE_YAML);
        write_template(config_dir.path(), "default.json", JSON_TEMPLATE);

        let output = RenderBuilder::new(default_options(config_dir.path()))
            .load_configs()
            .expect("load configs")
            .resolve_palette()
            .expect("resolve palette")
            .resolve_template_path()
            .expect("resolve template path")
            .load_template()
            .expect("load template")
            .apply_alias_models()
            .expect("apply alias models")
            .build_mapping()
            .expect("build mapping")
            .substitute()
            .expect("substitute")
            .serialize()
            .expect("serialize");

        let value: Value = serde_json::from_str(&output.data).expect("parse json");
        assert_eq!(value["agent"]["build"]["model"], "openrouter/openai/gpt-4o");
        assert_eq!(value["agent"]["build"]["variant"], "mini");
        assert_eq!(value["description"], "Build uses openrouter/openai/gpt-4o");
        assert!(output.lines > 0);
    }

    #[test]
    fn missing_palette_returns_error() {
        let config_dir = TempDir::new().expect("config dir");
        write_model_configs(config_dir.path(), SAMPLE_YAML);

        let mut opts = default_options(config_dir.path());
        opts.palette = "nonexistent".to_string();

        let result = RenderBuilder::new(opts)
            .load_configs()
            .expect("load configs")
            .resolve_palette();

        match result {
            Err(RenderError::MissingPalette { name }) => {
                assert_eq!(name, "nonexistent");
            }
            Err(other) => panic!("expected MissingPalette, got: {other:?}"),
            Ok(_) => panic!("expected error, got Ok"),
        }
    }

    #[test]
    fn yaml_output_format_serializes_yaml() {
        let config_dir = TempDir::new().expect("config dir");
        write_model_configs(config_dir.path(), SAMPLE_YAML);
        write_template(config_dir.path(), "default.json", JSON_TEMPLATE);

        let mut opts = default_options(config_dir.path());
        opts.format = OutputFormat::Yaml;

        let output = RenderBuilder::new(opts)
            .load_configs()
            .expect("load configs")
            .resolve_palette()
            .expect("resolve palette")
            .resolve_template_path()
            .expect("resolve template path")
            .load_template()
            .expect("load template")
            .apply_alias_models()
            .expect("apply alias models")
            .build_mapping()
            .expect("build mapping")
            .substitute()
            .expect("substitute")
            .serialize()
            .expect("serialize");

        // Verify output is valid YAML (not JSON)
        let yaml_value: serde_yaml::Value = serde_yaml::from_str(&output.data).expect("parse yaml");
        let json_value = serde_json::to_value(yaml_value).expect("convert to json");
        assert_eq!(
            json_value["agent"]["build"]["model"],
            "openrouter/openai/gpt-4o"
        );
        assert_eq!(json_value["agent"]["build"]["variant"], "mini");

        // JSON parsing should fail or differ since serde_yaml output is not JSON
        assert!(
            serde_json::from_str::<Value>(&output.data).is_err(),
            "YAML output should not be valid JSON"
        );
    }

    #[test]
    fn template_path_resolves_filesystem_path() {
        let config_dir = TempDir::new().expect("config dir");
        write_model_configs(config_dir.path(), SAMPLE_YAML);

        // Write template to an arbitrary path outside template.d
        let template_path = config_dir.path().join("custom-template.json");
        fs::write(&template_path, JSON_TEMPLATE).expect("write custom template");

        let mut opts = default_options(config_dir.path());
        opts.template = template_path.to_string_lossy().to_string();

        let output = RenderBuilder::new(opts)
            .load_configs()
            .expect("load configs")
            .resolve_palette()
            .expect("resolve palette")
            .resolve_template_path()
            .expect("resolve template path")
            .load_template()
            .expect("load template")
            .apply_alias_models()
            .expect("apply alias models")
            .build_mapping()
            .expect("build mapping")
            .substitute()
            .expect("substitute")
            .serialize()
            .expect("serialize");

        let value: Value = serde_json::from_str(&output.data).expect("parse json");
        assert_eq!(value["agent"]["build"]["model"], "openrouter/openai/gpt-4o");
        assert_eq!(value["description"], "Build uses openrouter/openai/gpt-4o");
    }
}
