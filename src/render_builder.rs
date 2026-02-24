use std::collections::HashMap;
use std::marker::PhantomData;
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
    configs: Option<ModelConfigs>,
    palette: Option<Palette>,
    template_path: Option<PathBuf>,
    template_value: Option<Value>,
    mapping: Option<HashMap<String, Value>>,
    _state: PhantomData<State>,
}

pub(crate) struct Start;
pub(crate) struct ConfigsLoaded;
pub(crate) struct PaletteResolved;
pub(crate) struct TemplatePathResolved;
pub(crate) struct TemplateLoaded;
pub(crate) struct AliasesApplied;
pub(crate) struct MappingBuilt;
pub(crate) struct Substituted;

impl<State> RenderBuilder<State> {
    fn transition<Next>(self) -> RenderBuilder<Next> {
        RenderBuilder {
            options: self.options,
            configs: self.configs,
            palette: self.palette,
            template_path: self.template_path,
            template_value: self.template_value,
            mapping: self.mapping,
            _state: PhantomData,
        }
    }
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
            configs: None,
            palette: None,
            template_path: None,
            template_value: None,
            mapping: None,
            _state: PhantomData,
        }
    }

    pub(crate) fn load_configs(self) -> Result<RenderBuilder<ConfigsLoaded>, RenderError> {
        let configs = load_model_configs(&self.options.config_dir)?;
        let mut builder = self.transition();
        builder.configs = Some(configs);
        Ok(builder)
    }
}

impl RenderBuilder<ConfigsLoaded> {
    pub(crate) fn resolve_palette(self) -> Result<RenderBuilder<PaletteResolved>, RenderError> {
        let palette_name = self.options.palette.clone();
        let configs = self.configs.as_ref().ok_or(RenderError::MissingPalette {
            name: palette_name.clone(),
        })?;
        let palette = configs
            .palettes
            .get(&palette_name)
            .ok_or(RenderError::MissingPalette { name: palette_name })?
            .clone();
        let mut builder = self.transition();
        builder.palette = Some(palette);
        Ok(builder)
    }
}

impl RenderBuilder<PaletteResolved> {
    pub(crate) fn resolve_template_path(
        self,
    ) -> Result<RenderBuilder<TemplatePathResolved>, RenderError> {
        if self.options.template.is_empty() {
            return Err(RenderError::InvalidTemplateName {
                name: self.options.template.clone(),
            });
        }
        let template_path =
            resolve_template_path_allowing_path(&self.options.config_dir, &self.options.template);
        let mut builder = self.transition();
        builder.template_path = Some(template_path);
        Ok(builder)
    }
}

impl RenderBuilder<TemplatePathResolved> {
    pub(crate) fn load_template(self) -> Result<RenderBuilder<TemplateLoaded>, RenderError> {
        let template_path =
            self.template_path
                .as_ref()
                .ok_or(RenderError::InvalidTemplateName {
                    name: self.options.template.clone(),
                })?;
        let template_value = load_template(template_path)?;
        let mut builder = self.transition();
        builder.template_value = Some(template_value);
        Ok(builder)
    }
}

impl RenderBuilder<TemplateLoaded> {
    pub(crate) fn apply_alias_models(self) -> Result<RenderBuilder<AliasesApplied>, RenderError> {
        let mut builder = self;
        let palette = builder
            .palette
            .as_ref()
            .ok_or(RenderError::MissingPalette {
                name: builder.options.palette.clone(),
            })?;
        let mut template_value =
            builder
                .template_value
                .take()
                .ok_or(RenderError::InvalidTemplateName {
                    name: builder.options.template.clone(),
                })?;
        apply_alias_models(&mut template_value, palette);
        builder.template_value = Some(template_value);
        Ok(builder.transition())
    }
}

impl RenderBuilder<AliasesApplied> {
    pub(crate) fn build_mapping(self) -> Result<RenderBuilder<MappingBuilt>, RenderError> {
        let mut builder = self;
        let palette = builder
            .palette
            .as_ref()
            .ok_or(RenderError::MissingPalette {
                name: builder.options.palette.clone(),
            })?;
        let mapping = build_mapping(palette);
        builder.mapping = Some(mapping);
        Ok(builder.transition())
    }
}

impl RenderBuilder<MappingBuilt> {
    pub(crate) fn substitute(self) -> Result<RenderBuilder<Substituted>, RenderError> {
        let mut builder = self;
        let mapping = builder
            .mapping
            .as_ref()
            .ok_or(RenderError::MissingPalette {
                name: builder.options.palette.clone(),
            })?;
        let mut template_value =
            builder
                .template_value
                .take()
                .ok_or(RenderError::InvalidTemplateName {
                    name: builder.options.template.clone(),
                })?;
        substitute(&mut template_value, mapping, builder.options.strict)?;
        builder.template_value = Some(template_value);
        Ok(builder.transition())
    }
}

impl RenderBuilder<Substituted> {
    pub(crate) fn serialize(self) -> Result<RenderOutput, RenderError> {
        let template_value = self
            .template_value
            .ok_or(RenderError::InvalidTemplateName {
                name: self.options.template.clone(),
            })?;
        let data = serialize_output(&template_value, self.options.format)?;
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
