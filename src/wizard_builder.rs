use std::collections::{BTreeSet, HashMap};
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use regex::Regex;
use serde_json::Value;
use tempfile::TempDir;

use crate::config::{Palette, load_model_configs};
use crate::substitute::substitute;
use crate::template::{
    apply_alias_models, build_mapping, is_valid_template_name, list_templates, load_template,
    resolve_template_name_path, write_json_pretty,
};
use crate::wizard::{WizardError, WizardOptions, WizardPrompter};

#[doc(hidden)]
pub struct WizardBuilder<'a, S> {
    options: WizardOptions,
    prompter: &'a mut dyn WizardPrompter,
    state: S,
}

pub(crate) struct Start;
pub(crate) struct TemplateSelected {
    template_name: String,
}
pub(crate) struct PaletteSelected {
    template_name: String,
    palette: Palette,
}
pub(crate) struct TemplateLoaded {
    palette: Palette,
    template_value: Value,
}
pub(crate) struct OverridesApplied {
    palette: Palette,
    template_value: Value,
}
#[doc(hidden)]
pub struct MappingBuilt {
    template_value: Value,
    mapping: HashMap<String, Value>,
}
pub(crate) struct Substituted {
    template_value: Value,
}
pub(crate) struct DraftWritten {
    template_value: Value,
    temp_dir: TempDir,
    draft_path: PathBuf,
}
pub(crate) struct FinalReady {
    final_value: Value,
}

impl<'a> WizardBuilder<'a, Start> {
    pub(crate) fn new(options: WizardOptions, prompter: &'a mut dyn WizardPrompter) -> Self {
        Self {
            options,
            prompter,
            state: Start,
        }
    }

    pub(crate) fn run(self) -> Result<(), WizardError> {
        if self.options.run_options.env_allow || self.options.run_options.env_mask_logs {
            eprintln!("warning: env placeholders are not supported in the wizard");
        }

        if self.options.out.exists() && !self.options.force {
            return Err(WizardError::OutputExists {
                path: self.options.out.clone(),
            });
        }

        let builder = self.select_template()?;
        let builder = builder.select_palette()?;
        let builder = builder.load_template()?;
        let builder = builder.apply_overrides()?;
        let builder = builder.build_mapping()?;
        let builder = builder.substitute_placeholders()?;
        let builder = builder.write_draft()?;
        let builder = builder.maybe_open_editor()?;
        builder.finalize_write()
    }

    pub(crate) fn select_template(
        self,
    ) -> Result<WizardBuilder<'a, TemplateSelected>, WizardError> {
        let WizardBuilder {
            options,
            prompter,
            state: _,
        } = self;
        let templates = list_templates(&options.config_dir).unwrap_or_default();
        let template_set: BTreeSet<String> = templates.into_iter().collect();
        let template = prompt_with_default(prompter, "Template name", options.template.as_deref())?;
        if template.trim().is_empty() {
            return Err(WizardError::InvalidTemplateName { name: template });
        }
        let template = template.trim().to_string();
        if !is_valid_template_name(&template) {
            return Err(WizardError::InvalidTemplateName { name: template });
        }
        if !template_set.is_empty() && !template_set.contains(&template) {
            return Err(WizardError::InvalidTemplateName { name: template });
        }
        Ok(WizardBuilder {
            options,
            prompter,
            state: TemplateSelected {
                template_name: template,
            },
        })
    }
}

impl<'a> WizardBuilder<'a, TemplateSelected> {
    pub(crate) fn select_palette(self) -> Result<WizardBuilder<'a, PaletteSelected>, WizardError> {
        let WizardBuilder {
            options,
            prompter,
            state,
        } = self;
        let configs = load_model_configs(&options.config_dir)?;
        let palette_input =
            prompt_with_default(prompter, "Palette name", options.palette.as_deref())?;
        if palette_input.trim().is_empty() {
            return Err(WizardError::MissingPalette {
                name: palette_input,
            });
        }
        let palette_name = palette_input.trim().to_string();
        let palette = configs
            .palettes
            .get(&palette_name)
            .ok_or_else(|| WizardError::MissingPalette {
                name: palette_name.clone(),
            })?
            .clone();
        Ok(WizardBuilder {
            options,
            prompter,
            state: PaletteSelected {
                template_name: state.template_name,
                palette,
            },
        })
    }
}

impl<'a> WizardBuilder<'a, PaletteSelected> {
    pub(crate) fn load_template(self) -> Result<WizardBuilder<'a, TemplateLoaded>, WizardError> {
        let WizardBuilder {
            options,
            prompter,
            state,
        } = self;
        if !is_valid_template_name(&state.template_name) {
            return Err(WizardError::InvalidTemplateName {
                name: state.template_name,
            });
        }
        let template_path = resolve_template_name_path(&options.config_dir, &state.template_name);
        let template_value = load_template(&template_path)?;
        Ok(WizardBuilder {
            options,
            prompter,
            state: TemplateLoaded {
                palette: state.palette,
                template_value,
            },
        })
    }
}

impl<'a> WizardBuilder<'a, TemplateLoaded> {
    pub(crate) fn apply_overrides(
        self,
    ) -> Result<WizardBuilder<'a, OverridesApplied>, WizardError> {
        let WizardBuilder {
            options,
            prompter,
            state,
        } = self;
        let TemplateLoaded {
            mut palette,
            mut template_value,
        } = state;
        apply_agent_overrides(prompter, &mut palette)?;
        apply_alias_models(&mut template_value, &palette);
        Ok(WizardBuilder {
            options,
            prompter,
            state: OverridesApplied {
                palette,
                template_value,
            },
        })
    }
}

impl<'a> WizardBuilder<'a, OverridesApplied> {
    pub(crate) fn build_mapping(self) -> Result<WizardBuilder<'a, MappingBuilt>, WizardError> {
        let WizardBuilder {
            options,
            prompter,
            state,
        } = self;
        let OverridesApplied {
            palette,
            template_value,
        } = state;
        let mut mapping = build_mapping(&palette);
        let placeholders = collect_placeholders(&template_value)?;
        for key in placeholders {
            if mapping.contains_key(&key) {
                continue;
            }
            if key.trim().starts_with("env:") {
                continue;
            }
            let value = prompt_placeholder_value(prompter, &key, options.run_options.strict)?;
            if let Some(value) = value {
                mapping.insert(key, value);
            }
        }
        Ok(WizardBuilder {
            options,
            prompter,
            state: MappingBuilt {
                template_value,
                mapping,
            },
        })
    }
}

impl<'a> WizardBuilder<'a, MappingBuilt> {
    pub(crate) fn substitute_placeholders(
        self,
    ) -> Result<WizardBuilder<'a, Substituted>, WizardError> {
        let WizardBuilder {
            options,
            prompter,
            state,
        } = self;
        let MappingBuilt {
            mut template_value,
            mapping,
        } = state;
        substitute(&mut template_value, &mapping, options.run_options.strict)?;
        Ok(WizardBuilder {
            options,
            prompter,
            state: Substituted { template_value },
        })
    }
}

impl<'a> WizardBuilder<'a, Substituted> {
    pub(crate) fn write_draft(self) -> Result<WizardBuilder<'a, DraftWritten>, WizardError> {
        let WizardBuilder {
            options,
            prompter,
            state,
        } = self;
        let temp_dir = TempDir::new().map_err(|source| WizardError::Write {
            path: std::env::temp_dir(),
            source,
        })?;
        let draft_path = temp_dir.path().join("draft.json");
        write_json_pretty(
            &draft_path,
            &state.template_value,
            |source| WizardError::Serialize { source },
            |source, path| WizardError::Write { path, source },
        )?;
        Ok(WizardBuilder {
            options,
            prompter,
            state: DraftWritten {
                template_value: state.template_value,
                temp_dir,
                draft_path,
            },
        })
    }
}

impl<'a> WizardBuilder<'a, DraftWritten> {
    pub(crate) fn maybe_open_editor(self) -> Result<WizardBuilder<'a, FinalReady>, WizardError> {
        let WizardBuilder {
            options,
            prompter,
            state,
        } = self;
        let DraftWritten {
            template_value,
            temp_dir: _temp_dir,
            draft_path,
        } = state;
        if let Some(editor) = resolve_editor().filter(|_| prompter.allow_editor_prompt()) {
            let open_editor = prompter.confirm("Open draft in editor?", true)?;
            if open_editor {
                open_in_editor(&editor, &draft_path)?;
            }
        }
        let final_value = if draft_path.exists() {
            load_template(&draft_path)?
        } else {
            template_value
        };
        Ok(WizardBuilder {
            options,
            prompter,
            state: FinalReady { final_value },
        })
    }
}

impl<'a> WizardBuilder<'a, FinalReady> {
    pub(crate) fn finalize_write(self) -> Result<(), WizardError> {
        let WizardBuilder {
            options,
            prompter,
            state,
        } = self;
        finalize_write(prompter, options, state.final_value)
    }
}

fn finalize_write(
    prompter: &mut dyn WizardPrompter,
    options: WizardOptions,
    final_value: Value,
) -> Result<(), WizardError> {
    let confirm = prompter.confirm(
        &format!("Write output to {}?", options.out.display()),
        false,
    )?;
    if !confirm {
        return Err(WizardError::Aborted);
    }
    write_json_pretty(
        &options.out,
        &final_value,
        |source| WizardError::Serialize { source },
        |source, path| WizardError::Write { path, source },
    )?;
    Ok(())
}

fn prompt_with_default(
    prompter: &mut dyn WizardPrompter,
    prompt: &str,
    default: Option<&str>,
) -> Result<String, WizardError> {
    let value = prompter.input(prompt, default)?;
    if let Some(default) = default.filter(|_| value.trim().is_empty()) {
        return Ok(default.to_string());
    }
    Ok(value)
}

fn apply_agent_overrides(
    prompter: &mut dyn WizardPrompter,
    palette: &mut Palette,
) -> Result<(), WizardError> {
    if palette.agents.is_empty() {
        return Ok(());
    }
    let agent_names: Vec<String> = palette.agents.keys().cloned().collect();
    let selected = prompter.multi_select("Select agents to override", &agent_names)?;
    for index in selected {
        let name = agent_names
            .get(index)
            .ok_or_else(|| WizardError::Prompt("invalid agent selection".to_string()))?
            .clone();
        let agent = palette
            .agents
            .get(&name)
            .cloned()
            .ok_or_else(|| WizardError::Prompt("missing agent".to_string()))?;
        let model = prompt_with_default(
            prompter,
            &format!("Model for agent {name}"),
            Some(agent.model.as_str()),
        )?;
        let variant_default = agent.variant.as_deref().unwrap_or("");
        let variant_input = prompter.input(
            &format!("Variant for agent {name} (use '-' to clear)"),
            Some(variant_default),
        )?;
        let variant = if variant_input.trim() == "-" {
            None
        } else if variant_input.trim().is_empty() {
            agent.variant.clone()
        } else {
            Some(variant_input.trim().to_string())
        };
        palette.agents.insert(
            name,
            crate::config::AgentConfig {
                model: model.trim().to_string(),
                variant,
                reasoning: agent.reasoning.clone(),
            },
        );
    }
    Ok(())
}

fn prompt_placeholder_value(
    prompter: &mut dyn WizardPrompter,
    key: &str,
    strict: bool,
) -> Result<Option<Value>, WizardError> {
    let prompt = format!("Value for placeholder {key}");
    let value = prompter.input(&prompt, None)?;
    if value.trim().is_empty() {
        if strict {
            return Err(WizardError::Prompt(format!(
                "missing value for placeholder {key} in strict mode"
            )));
        }
        return Ok(None);
    }
    match serde_json::from_str::<Value>(&value) {
        Ok(parsed) => Ok(Some(parsed)),
        Err(_) => Ok(Some(Value::String(value))),
    }
}

fn collect_placeholders(value: &Value) -> Result<BTreeSet<String>, WizardError> {
    let regex = Regex::new(r"\{\{\s*([^\}]+?)\s*\}\}")
        .map_err(|err| WizardError::Prompt(err.to_string()))?;
    let mut placeholders = BTreeSet::new();
    collect_placeholders_inner(value, &regex, &mut placeholders);
    Ok(placeholders)
}

fn collect_placeholders_inner(value: &Value, regex: &Regex, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for child in map.values() {
                collect_placeholders_inner(child, regex, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_placeholders_inner(item, regex, out);
            }
        }
        Value::String(text) => {
            for capture in regex.captures_iter(text) {
                if let Some(key) = capture.get(1) {
                    out.insert(key.as_str().trim().to_string());
                }
            }
        }
        _ => {}
    }
}

fn resolve_editor() -> Option<String> {
    env::var("EDITOR")
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn open_in_editor(editor: &str, path: &Path) -> Result<(), WizardError> {
    let parts = split_command(editor);
    let (program, args) = parts
        .split_first()
        .ok_or_else(|| WizardError::EditorStart {
            editor: editor.to_string(),
        })?;
    let status = Command::new(program)
        .args(args)
        .arg(path)
        .status()
        .map_err(|_| WizardError::EditorStart {
            editor: editor.to_string(),
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(WizardError::EditorFailed)
    }
}

fn split_command(command: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut chars = command.chars().peekable();
    let mut quote = None;
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            '\'' | '"' => {
                if quote == Some(ch) {
                    quote = None;
                } else if quote.is_none() {
                    quote = Some(ch);
                } else {
                    current.push(ch);
                }
            }
            ' ' | '\t' if quote.is_none() => {
                if !current.is_empty() {
                    parts.push(current.clone());
                    current.clear();
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}
