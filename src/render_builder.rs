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
