use std::path::PathBuf;

use dialoguer::theme::ColorfulTheme;
use dialoguer::{Confirm, Input, MultiSelect};
use thiserror::Error;

use crate::config::ConfigError;
use crate::options::RunOptions;
use crate::substitute::SubstituteError;
use crate::template::TemplateError;
use crate::wizard_builder::WizardBuilder;

#[derive(Debug, Error)]
pub enum WizardError {
    #[error("config error: {0}")]
    Config(#[from] ConfigError),
    #[error("template error: {0}")]
    Template(#[from] TemplateError),
    #[error("substitution error: {0}")]
    Substitute(#[from] SubstituteError),
    #[error("invalid template name: {name} (use base names without extensions)")]
    InvalidTemplateName { name: String },
    #[error("palette not found: {name}")]
    MissingPalette { name: String },
    #[error("output already exists: {path}")]
    OutputExists { path: PathBuf },
    #[error("wizard aborted by user")]
    Aborted,
    #[error("failed to write output at {path}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialize output")]
    Serialize {
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to start editor: {editor}")]
    EditorStart { editor: String },
    #[error("editor exited with a failure status")]
    EditorFailed,
    #[error("prompt error: {0}")]
    Prompt(String),
}

pub struct WizardOptions {
    pub template: Option<String>,
    pub palette: Option<String>,
    pub out: PathBuf,
    pub force: bool,
    pub run_options: RunOptions,
    pub config_dir: PathBuf,
}

pub fn run(options: WizardOptions) -> Result<(), WizardError> {
    let mut prompter = DialoguerPrompter::new();
    run_with_prompter(options, &mut prompter)
}

pub(crate) fn run_with_prompter(
    options: WizardOptions,
    prompter: &mut dyn WizardPrompter,
) -> Result<(), WizardError> {
    WizardBuilder::new(options, prompter).run()
}

pub(crate) trait WizardPrompter {
    fn input(&mut self, prompt: &str, default: Option<&str>) -> Result<String, WizardError>;
    fn confirm(&mut self, prompt: &str, default: bool) -> Result<bool, WizardError>;
    fn multi_select(&mut self, prompt: &str, options: &[String])
    -> Result<Vec<usize>, WizardError>;
    fn allow_editor_prompt(&self) -> bool;
}

struct DialoguerPrompter {
    theme: ColorfulTheme,
}

impl DialoguerPrompter {
    fn new() -> Self {
        Self {
            theme: ColorfulTheme::default(),
        }
    }
}

impl WizardPrompter for DialoguerPrompter {
    fn input(&mut self, prompt: &str, default: Option<&str>) -> Result<String, WizardError> {
        let input = Input::with_theme(&self.theme).with_prompt(prompt);
        let input = if let Some(default) = default {
            input.default(default.to_string())
        } else {
            input
        };
        input
            .interact_text()
            .map_err(|err| WizardError::Prompt(err.to_string()))
    }

    fn confirm(&mut self, prompt: &str, default: bool) -> Result<bool, WizardError> {
        Confirm::with_theme(&self.theme)
            .with_prompt(prompt)
            .default(default)
            .interact()
            .map_err(|err| WizardError::Prompt(err.to_string()))
    }

    fn multi_select(
        &mut self,
        prompt: &str,
        options: &[String],
    ) -> Result<Vec<usize>, WizardError> {
        if options.is_empty() {
            return Ok(Vec::new());
        }
        MultiSelect::with_theme(&self.theme)
            .with_prompt(prompt)
            .items(options)
            .interact()
            .map_err(|err| WizardError::Prompt(err.to_string()))
    }

    fn allow_editor_prompt(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, VecDeque};
    use std::fs;

    use serde_json::Value as JsonValue;
    use tempfile::TempDir;

    use super::{WizardError, WizardOptions, WizardPrompter, run_with_prompter};
    use crate::config::{AgentConfig, ModelConfigs, Palette};
    use crate::options::RunOptions;

    struct StubPrompter {
        inputs: VecDeque<String>,
        confirms: VecDeque<bool>,
        selections: VecDeque<Vec<usize>>,
    }

    impl StubPrompter {
        fn new() -> Self {
            Self {
                inputs: VecDeque::new(),
                confirms: VecDeque::new(),
                selections: VecDeque::new(),
            }
        }

        fn push_input(mut self, value: &str) -> Self {
            self.inputs.push_back(value.to_string());
            self
        }

        fn push_confirm(mut self, value: bool) -> Self {
            self.confirms.push_back(value);
            self
        }

        fn push_selection(mut self, value: Vec<usize>) -> Self {
            self.selections.push_back(value);
            self
        }
    }

    impl WizardPrompter for StubPrompter {
        fn input(&mut self, _prompt: &str, default: Option<&str>) -> Result<String, WizardError> {
            match self.inputs.pop_front() {
                Some(value) => Ok(value),
                None => Ok(default.unwrap_or("").to_string()),
            }
        }

        fn confirm(&mut self, _prompt: &str, default: bool) -> Result<bool, WizardError> {
            Ok(self.confirms.pop_front().unwrap_or(default))
        }

        fn multi_select(
            &mut self,
            _prompt: &str,
            _options: &[String],
        ) -> Result<Vec<usize>, WizardError> {
            Ok(self.selections.pop_front().unwrap_or_default())
        }

        fn allow_editor_prompt(&self) -> bool {
            false
        }
    }

    fn write_template(dir: &TempDir, name: &str, contents: &str) {
        let template_dir = dir.path().join("template.d");
        fs::create_dir_all(&template_dir).expect("create template dir");
        fs::write(template_dir.join(name), contents).expect("write template");
    }

    fn write_palettes(dir: &TempDir, configs: &ModelConfigs) {
        let data = serde_yaml::to_string(configs).expect("serialize configs");
        fs::write(dir.path().join("model-configs.yaml"), data).expect("write palettes");
    }

    #[test]
    fn wizard_writes_output_after_confirm() {
        let config_dir = TempDir::new().expect("config dir");
        write_template(
            &config_dir,
            "default.json",
            r#"{"agent": {"build": {"model": "{{build}}", "variant": "{{build-variant}}"}}, "name": "{{project-name}}"}"#,
        );
        write_palettes(
            &config_dir,
            &ModelConfigs {
                palettes: BTreeMap::from([(
                    "default".to_string(),
                    Palette {
                        agents: BTreeMap::from([(
                            "build".to_string(),
                            AgentConfig {
                                model: "openrouter/build".to_string(),
                                variant: Some("mini".to_string()),
                                reasoning: None,
                            },
                        )]),
                        mapping: BTreeMap::new(),
                    },
                )]),
            },
        );
        let out_path = config_dir.path().join("opencode.json");

        let mut prompter = StubPrompter::new()
            .push_input("default")
            .push_input("default")
            .push_selection(vec![])
            .push_input("demo")
            .push_confirm(true);

        run_with_prompter(
            WizardOptions {
                template: Some("default".to_string()),
                palette: Some("default".to_string()),
                out: out_path.clone(),
                force: false,
                run_options: RunOptions {
                    strict: true,
                    env_allow: true,
                    env_mask_logs: true,
                },
                config_dir: config_dir.path().to_path_buf(),
            },
            &mut prompter,
        )
        .expect("wizard run");

        let data = fs::read_to_string(out_path).expect("read output");
        let value: JsonValue = serde_json::from_str(&data).expect("parse json");
        assert_eq!(value["agent"]["build"]["model"], "openrouter/build");
        assert_eq!(value["agent"]["build"]["variant"], "mini");
        assert_eq!(value["name"], "demo");
    }

    #[test]
    fn wizard_aborts_without_confirmation() {
        let config_dir = TempDir::new().expect("config dir");
        write_template(
            &config_dir,
            "default.json",
            r#"{"agent": {"build": {"model": "{{build}}"}}}"#,
        );
        write_palettes(
            &config_dir,
            &ModelConfigs {
                palettes: BTreeMap::from([(
                    "default".to_string(),
                    Palette {
                        agents: BTreeMap::from([(
                            "build".to_string(),
                            AgentConfig {
                                model: "openrouter/build".to_string(),
                                variant: None,
                                reasoning: None,
                            },
                        )]),
                        mapping: BTreeMap::new(),
                    },
                )]),
            },
        );
        let out_path = config_dir.path().join("opencode.json");

        let mut prompter = StubPrompter::new()
            .push_input("default")
            .push_input("default")
            .push_selection(vec![])
            .push_confirm(false);

        let err = run_with_prompter(
            WizardOptions {
                template: Some("default".to_string()),
                palette: Some("default".to_string()),
                out: out_path.clone(),
                force: false,
                run_options: RunOptions::default(),
                config_dir: config_dir.path().to_path_buf(),
            },
            &mut prompter,
        )
        .expect_err("wizard aborted");

        match err {
            WizardError::Aborted => {}
            other => panic!("unexpected error: {other:?}"),
        }
        assert!(!out_path.exists());
    }

    #[test]
    fn wizard_applies_agent_override() {
        let config_dir = TempDir::new().expect("config dir");
        write_template(
            &config_dir,
            "default.json",
            r#"{"agent": {"build": {"model": "{{build}}", "variant": "{{build-variant}}"}}}"#,
        );
        write_palettes(
            &config_dir,
            &ModelConfigs {
                palettes: BTreeMap::from([(
                    "default".to_string(),
                    Palette {
                        agents: BTreeMap::from([(
                            "build".to_string(),
                            AgentConfig {
                                model: "openrouter/build".to_string(),
                                variant: Some("mini".to_string()),
                                reasoning: None,
                            },
                        )]),
                        mapping: BTreeMap::new(),
                    },
                )]),
            },
        );
        let out_path = config_dir.path().join("opencode.json");

        let mut prompter = StubPrompter::new()
            .push_input("default")
            .push_input("default")
            .push_selection(vec![0])
            .push_input("openrouter/override")
            .push_input("nano")
            .push_confirm(true);

        run_with_prompter(
            WizardOptions {
                template: Some("default".to_string()),
                palette: Some("default".to_string()),
                out: out_path.clone(),
                force: false,
                run_options: RunOptions::default(),
                config_dir: config_dir.path().to_path_buf(),
            },
            &mut prompter,
        )
        .expect("wizard run");

        let data = fs::read_to_string(out_path).expect("read output");
        let value: JsonValue = serde_json::from_str(&data).expect("parse json");
        assert_eq!(value["agent"]["build"]["model"], "openrouter/override");
        assert_eq!(value["agent"]["build"]["variant"], "nano");
    }
}
