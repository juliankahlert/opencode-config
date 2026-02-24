use std::collections::HashMap;
use std::marker::PhantomData;

use serde_json::Value;

use crate::config::{Palette, load_model_configs};
use crate::create::{CreateError, CreateOptions};
use crate::substitute::substitute;
use crate::template::{
    apply_alias_models, build_mapping, is_valid_template_name, load_template,
    resolve_template_name_path, write_json_pretty,
};

pub struct CreateBuilder<State> {
    options: CreateOptions,
    state: State,
    _state: PhantomData<State>,
}

pub struct Start;
pub struct PaletteSelected {
    palette: Palette,
}
pub struct TemplateLoaded {
    palette: Palette,
    template_value: Value,
}
pub struct AliasesApplied {
    palette: Palette,
    template_value: Value,
}
pub struct MappingBuilt {
    template_value: Value,
    mapping: HashMap<String, Value>,
}
pub struct FinalReady {
    template_value: Value,
}

impl<State> CreateBuilder<State> {
    fn transition<Next>(self, state: Next) -> CreateBuilder<Next> {
        CreateBuilder {
            options: self.options,
            state,
            _state: PhantomData,
        }
    }
}

impl CreateBuilder<Start> {
    pub fn new(options: CreateOptions) -> Self {
        Self {
            options,
            state: Start,
            _state: PhantomData,
        }
    }

    pub fn run(self) -> Result<(), CreateError> {
        let builder = self.warn_env_flags()?;
        let builder = builder.ensure_output()?;
        let builder = builder.select_palette()?;
        let builder = builder.load_template()?;
        let builder = builder.apply_aliases()?;
        let builder = builder.build_mapping()?;
        let builder = builder.substitute_placeholders()?;
        builder.write_output()
    }

    pub fn warn_env_flags(self) -> Result<Self, CreateError> {
        if self.options.run_options.env_allow {
            eprintln!("warning: --env-allow is currently unsupported for create/switch");
        }
        if self.options.run_options.env_mask_logs {
            eprintln!("warning: --env-mask-logs is currently unsupported for create/switch");
        }
        Ok(self)
    }

    pub fn ensure_output(self) -> Result<Self, CreateError> {
        if self.options.out.exists() && !self.options.force {
            return Err(CreateError::OutputExists {
                path: self.options.out.clone(),
            });
        }
        Ok(self)
    }

    pub fn select_palette(self) -> Result<CreateBuilder<PaletteSelected>, CreateError> {
        let configs = load_model_configs(&self.options.config_dir)?;
        let palette = configs
            .palettes
            .get(&self.options.palette)
            .ok_or_else(|| CreateError::MissingPalette {
                name: self.options.palette.clone(),
            })?
            .clone();
        Ok(self.transition(PaletteSelected { palette }))
    }
}

impl CreateBuilder<PaletteSelected> {
    pub fn load_template(self) -> Result<CreateBuilder<TemplateLoaded>, CreateError> {
        let CreateBuilder {
            options,
            state,
            _state: _,
        } = self;

        if !is_valid_template_name(&options.template) {
            return Err(CreateError::InvalidTemplateName {
                name: options.template.clone(),
            });
        }
        let template_path = resolve_template_name_path(&options.config_dir, &options.template);
        let template_value = load_template(&template_path)?;
        Ok(CreateBuilder {
            options,
            state: TemplateLoaded {
                palette: state.palette,
                template_value,
            },
            _state: PhantomData,
        })
    }
}

impl CreateBuilder<TemplateLoaded> {
    pub fn apply_aliases(self) -> Result<CreateBuilder<AliasesApplied>, CreateError> {
        let CreateBuilder {
            options,
            state,
            _state: _,
        } = self;

        let TemplateLoaded {
            palette,
            mut template_value,
        } = state;
        apply_alias_models(&mut template_value, &palette);
        Ok(CreateBuilder {
            options,
            state: AliasesApplied {
                palette,
                template_value,
            },
            _state: PhantomData,
        })
    }
}

impl CreateBuilder<AliasesApplied> {
    pub fn build_mapping(self) -> Result<CreateBuilder<MappingBuilt>, CreateError> {
        let CreateBuilder {
            options,
            state,
            _state: _,
        } = self;

        let AliasesApplied {
            palette,
            template_value,
        } = state;
        let mapping = build_mapping(&palette);
        Ok(CreateBuilder {
            options,
            state: MappingBuilt {
                template_value,
                mapping,
            },
            _state: PhantomData,
        })
    }
}

impl CreateBuilder<MappingBuilt> {
    pub fn substitute_placeholders(self) -> Result<CreateBuilder<FinalReady>, CreateError> {
        let CreateBuilder {
            options,
            state,
            _state: _,
        } = self;

        let MappingBuilt {
            mut template_value,
            mapping,
        } = state;
        substitute(&mut template_value, &mapping, options.run_options.strict)?;
        Ok(CreateBuilder {
            options,
            state: FinalReady { template_value },
            _state: PhantomData,
        })
    }
}

impl CreateBuilder<FinalReady> {
    pub fn write_output(self) -> Result<(), CreateError> {
        let CreateBuilder {
            options,
            state,
            _state: _,
        } = self;

        write_json_pretty(
            &options.out,
            &state.template_value,
            |source| CreateError::Serialize { source },
            |source, path| CreateError::Write { path, source },
        )
    }
}
