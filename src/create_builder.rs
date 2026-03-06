//! Typestate builder for the `create` command.
//!
//! [`CreateBuilder`] uses the typestate pattern to enforce a compile-time
//! valid ordering of steps when generating an `opencode.json` file from a
//! template and a model-config palette.
//!
//! # State-transition diagram
//!
//! ```text
//!   ┌───────────┐
//!   │   Start   │
//!   └─────┬─────┘
//!         │ ensure_output()   [self-transition]
//!         │ select_palette()
//!         v
//!  ┌──────────────────┐
//!  │ PaletteSelected  │
//!  └────────┬─────────┘
//!           │ load_template()
//!           v
//!  ┌──────────────────┐
//!  │ TemplateLoaded   │
//!  └────────┬─────────┘
//!           │ apply_aliases()
//!           v
//!  ┌──────────────────┐
//!  │ AliasesApplied   │
//!  └────────┬─────────┘
//!           │ build_mapping()
//!           v
//!  ┌──────────────────┐
//!  │  MappingBuilt    │
//!  └────────┬─────────┘
//!           │ resolve_env_vars()
//!           v
//!  ┌──────────────────┐
//!  │  EnvResolved     │
//!  └────────┬─────────┘
//!           │ substitute_placeholders()
//!           v
//!  ┌──────────────────┐
//!  │   FinalReady     │
//!  └────────┬─────────┘
//!           │ write_output()
//!           v
//!       Result<()>
//! ```
//!
//! The convenience method [`CreateBuilder::run`] drives the entire chain
//! from `Start` through `FinalReady`, returning once the output file is
//! written.

use std::collections::HashMap;

use regex::Regex;
use serde_json::Value;

use crate::config::{Palette, load_model_configs};
use crate::create::{CreateError, CreateOptions};
use crate::env_resolve::{Allow, EnvResolver, ResolveError};
use crate::substitute::{SubstituteError, substitute};
use crate::template::{
    apply_alias_models, build_mapping, is_valid_template_name, load_template,
    resolve_template_name_path, write_json_pretty,
};

pub struct CreateBuilder<State> {
    options: CreateOptions,
    state: State,
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
pub struct EnvResolved {
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
        }
    }
}

impl CreateBuilder<Start> {
    pub fn new(options: CreateOptions) -> Self {
        Self {
            options,
            state: Start,
        }
    }

    pub fn run(self) -> Result<(), CreateError> {
        let builder = self.ensure_output()?;
        let builder = builder.select_palette()?;
        let builder = builder.load_template()?;
        let builder = builder.apply_aliases()?;
        let builder = builder.build_mapping()?;
        let builder = builder.resolve_env_vars()?;
        let builder = builder.substitute_placeholders()?;
        builder.write_output()
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
        let CreateBuilder { options, state } = self;

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
        })
    }
}

impl CreateBuilder<TemplateLoaded> {
    pub fn apply_aliases(self) -> Result<CreateBuilder<AliasesApplied>, CreateError> {
        let CreateBuilder { options, state } = self;

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
        })
    }
}

impl CreateBuilder<AliasesApplied> {
    pub fn build_mapping(self) -> Result<CreateBuilder<MappingBuilt>, CreateError> {
        let CreateBuilder { options, state } = self;

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
        })
    }
}

impl CreateBuilder<MappingBuilt> {
    /// Resolve `{{env:VAR}}` placeholders by looking up environment variables.
    ///
    /// When `env_allow` is active, the template is scanned for placeholder
    /// keys starting with `env:`.  Each matching variable is looked up in the
    /// OS environment via [`EnvResolver`] and the resolved values are merged
    /// directly into the main `mapping` as `Value::String` entries.
    ///
    /// When `env_allow` is **not** active the step is a no-op pass-through —
    /// any `{{env:…}}` placeholders are left for the substitution engine to
    /// handle (left unresolved in permissive mode, error in strict mode).
    pub fn resolve_env_vars(self) -> Result<CreateBuilder<EnvResolved>, CreateError> {
        let CreateBuilder { options, state } = self;

        let MappingBuilt {
            template_value,
            mut mapping,
        } = state;

        if options.run_options.env_allow {
            let env_keys = collect_env_placeholder_keys(&template_value);
            if !env_keys.is_empty() {
                let synthetic: HashMap<String, String> =
                    env_keys.into_iter().map(|k| (k.clone(), k)).collect();

                let resolver = EnvResolver::new(
                    Allow::All,
                    options.run_options.strict,
                    options.run_options.env_mask_logs,
                );

                let resolved = resolver.resolve(&synthetic).map_err(|e| match e {
                    ResolveError::MissingEnvVar { var } => {
                        CreateError::Substitute(SubstituteError::MissingPlaceholder {
                            key: format!("env:{var}"),
                        })
                    }
                    ResolveError::NotAllowed { var } => {
                        CreateError::Substitute(SubstituteError::MissingPlaceholder {
                            key: format!("env:{var}"),
                        })
                    }
                })?;

                for (key, value) in resolved {
                    mapping.insert(key, Value::String(value));
                }
            }
        }

        Ok(CreateBuilder {
            options,
            state: EnvResolved {
                template_value,
                mapping,
            },
        })
    }
}

impl CreateBuilder<EnvResolved> {
    pub fn substitute_placeholders(self) -> Result<CreateBuilder<FinalReady>, CreateError> {
        let CreateBuilder { options, state } = self;

        let EnvResolved {
            mut template_value,
            mapping,
        } = state;
        substitute(&mut template_value, &mapping, options.run_options.strict)?;
        Ok(CreateBuilder {
            options,
            state: FinalReady { template_value },
        })
    }
}

impl CreateBuilder<FinalReady> {
    pub fn write_output(self) -> Result<(), CreateError> {
        let CreateBuilder { options, state } = self;

        write_json_pretty(
            &options.out,
            &state.template_value,
            |source| CreateError::Serialize { source },
            |source, path| CreateError::Write { path, source },
        )
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Collect all unique `env:VAR` placeholder keys from a JSON value.
///
/// Scans strings recursively for `{{ env:VAR }}` patterns and returns the
/// inner key (e.g. `env:MY_VAR`) with surrounding whitespace trimmed.
fn collect_env_placeholder_keys(value: &Value) -> Vec<String> {
    let re = Regex::new(r"\{\{\s*(env:[^\}]+?)\s*\}\}").expect("env placeholder regex");
    let mut keys = Vec::new();
    collect_env_keys(value, &re, &mut keys);
    keys.sort();
    keys.dedup();
    keys
}

fn collect_env_keys(value: &Value, re: &Regex, keys: &mut Vec<String>) {
    match value {
        Value::String(s) => {
            for cap in re.captures_iter(s) {
                if let Some(m) = cap.get(1) {
                    keys.push(m.as_str().trim().to_string());
                }
            }
        }
        Value::Object(map) => {
            for v in map.values() {
                collect_env_keys(v, re, keys);
            }
        }
        Value::Array(arr) => {
            for v in arr {
                collect_env_keys(v, re, keys);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::env;
    use std::ffi::OsString;
    use std::fs;
    use std::path::Path;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use tempfile::TempDir;

    use super::{CreateBuilder, Start};
    use crate::config::{AgentConfig, ModelConfigs, Palette};
    use crate::create::{CreateError, CreateOptions};
    use crate::options::RunOptions;

    // ------------------------------------------------------------------
    // Environment safety helpers (mirrors src/options.rs pattern)
    // ------------------------------------------------------------------

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn env_lock() -> MutexGuard<'static, ()> {
        ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("lock env")
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvVarGuard {
        fn new(key: &'static str) -> Self {
            let previous = env::var_os(key);
            Self { key, previous }
        }

        fn set(&self, value: &str) {
            unsafe {
                env::set_var(self.key, value);
            }
        }

        fn remove(&self) {
            unsafe {
                env::remove_var(self.key);
            }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(value) = self.previous.take() {
                unsafe {
                    env::set_var(self.key, value);
                }
            } else {
                unsafe {
                    env::remove_var(self.key);
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // Test helpers
    // ------------------------------------------------------------------

    fn write_template(dir: &TempDir, name: &str, contents: &str) {
        let template_dir = dir.path().join("template.d");
        fs::create_dir_all(&template_dir).expect("create template dir");
        fs::write(template_dir.join(name), contents).expect("write template");
    }

    fn write_model_configs(dir: &TempDir, configs: &ModelConfigs) {
        let data = serde_yaml::to_string(configs).expect("serialize configs");
        fs::write(dir.path().join("model-configs.yaml"), data).expect("write model-configs");
    }

    fn default_run_options() -> RunOptions {
        RunOptions {
            strict: false,
            env_allow: false,
            env_mask_logs: false,
        }
    }

    fn make_options(
        config_dir: &Path,
        out: &Path,
        template: &str,
        palette: &str,
        force: bool,
    ) -> CreateOptions {
        CreateOptions {
            template: template.to_string(),
            palette: palette.to_string(),
            out: out.to_path_buf(),
            force,
            run_options: default_run_options(),
            config_dir: config_dir.to_path_buf(),
        }
    }

    fn make_options_with_run(
        config_dir: &Path,
        out: &Path,
        template: &str,
        palette: &str,
        run_options: RunOptions,
    ) -> CreateOptions {
        CreateOptions {
            template: template.to_string(),
            palette: palette.to_string(),
            out: out.to_path_buf(),
            force: false,
            run_options,
            config_dir: config_dir.to_path_buf(),
        }
    }

    fn single_agent_configs() -> ModelConfigs {
        ModelConfigs {
            palettes: BTreeMap::from([(
                "default".to_string(),
                Palette {
                    agents: BTreeMap::from([(
                        "build".to_string(),
                        AgentConfig {
                            model: "openrouter/openai/gpt-4o".to_string(),
                            variant: Some("mini".to_string()),
                            reasoning: None,
                        },
                    )]),
                    mapping: BTreeMap::new(),
                },
            )]),
        }
    }

    // ------------------------------------------------------------------
    // Existing tests
    // ------------------------------------------------------------------

    #[test]
    fn new_builder_stores_options() {
        let config_dir = TempDir::new().expect("config dir");
        let out_path = config_dir.path().join("opencode.json");
        let options = make_options(config_dir.path(), &out_path, "default", "default", false);

        // Builder stores options — ensure_output succeeds when output does not exist
        let builder = CreateBuilder::new(options);
        let builder = builder
            .ensure_output()
            .expect("ensure_output should pass for non-existing path");

        // Verify the builder is still in Start state and options are retained by
        // calling ensure_output again (it consumes self, so we already validated above).
        // A second check: force=true path also works, proving options.force was stored.
        let config_dir2 = TempDir::new().expect("config dir 2");
        let out_path2 = config_dir2.path().join("opencode.json");
        fs::write(&out_path2, "existing").expect("write existing");
        let options2 = make_options(
            config_dir2.path(),
            &out_path2,
            "default",
            "default",
            true, // force
        );
        let builder2 = CreateBuilder::new(options2);
        builder2
            .ensure_output()
            .expect("ensure_output should pass when force=true");

        // Ensure builder from first call is still usable (it was moved, so this
        // just confirms the first ensure_output returned Ok with a valid builder).
        drop(builder);
    }

    #[test]
    fn output_exists_error_when_not_forced() {
        let config_dir = TempDir::new().expect("config dir");
        let out_path = config_dir.path().join("opencode.json");
        fs::write(&out_path, "existing content").expect("write existing");

        let options = make_options(
            config_dir.path(),
            &out_path,
            "default",
            "default",
            false, // force = false
        );
        let builder = CreateBuilder::new(options);
        match builder.ensure_output() {
            Err(CreateError::OutputExists { path }) => {
                assert_eq!(path, out_path);
            }
            Err(other) => panic!("expected OutputExists, got: {other:?}"),
            Ok(_) => panic!("expected error when output exists and force=false"),
        }
    }

    #[test]
    fn missing_palette_error() {
        let config_dir = TempDir::new().expect("config dir");
        write_model_configs(&config_dir, &single_agent_configs());
        let out_path = config_dir.path().join("opencode.json");

        let options = make_options(
            config_dir.path(),
            &out_path,
            "default",
            "nonexistent", // palette not in model-configs
            false,
        );
        let builder = CreateBuilder::<Start>::new(options);
        match builder.select_palette() {
            Err(CreateError::MissingPalette { name }) => {
                assert_eq!(name, "nonexistent");
            }
            Err(other) => panic!("expected MissingPalette, got: {other:?}"),
            Ok(_) => panic!("expected error for missing palette"),
        }
    }

    #[test]
    fn end_to_end_run_writes_substituted_output() {
        let config_dir = TempDir::new().expect("config dir");
        write_model_configs(&config_dir, &single_agent_configs());
        write_template(
            &config_dir,
            "default.json",
            r#"{
                "agent": {
                    "build": {
                        "model": "{{agent-build-model}}",
                        "variant": "{{agent-build-variant}}"
                    }
                }
            }"#,
        );

        let work_dir = TempDir::new().expect("work dir");
        let out_path = work_dir.path().join("opencode.json");

        let options = make_options(config_dir.path(), &out_path, "default", "default", false);
        CreateBuilder::new(options)
            .run()
            .expect("run should succeed");

        assert!(out_path.exists(), "output file should be written");
        let data = fs::read_to_string(&out_path).expect("read output");
        let value: serde_json::Value = serde_json::from_str(&data).expect("parse json");
        assert_eq!(value["agent"]["build"]["model"], "openrouter/openai/gpt-4o");
        assert_eq!(value["agent"]["build"]["variant"], "mini");
    }

    #[test]
    fn invalid_template_name_error() {
        let config_dir = TempDir::new().expect("config dir");
        write_model_configs(&config_dir, &single_agent_configs());
        let out_path = config_dir.path().join("opencode.json");

        // Template name with path separator
        let options = make_options(
            config_dir.path(),
            &out_path,
            "foo/bar", // invalid — contains path separator
            "default",
            false,
        );
        let builder = CreateBuilder::new(options);
        // Must reach PaletteSelected state first via select_palette
        let builder = builder.select_palette().expect("select_palette");
        match builder.load_template() {
            Err(CreateError::InvalidTemplateName { name }) => {
                assert_eq!(name, "foo/bar");
            }
            Err(other) => panic!("expected InvalidTemplateName, got: {other:?}"),
            Ok(_) => panic!("expected error for invalid template name"),
        }

        // Also test template name with extension (another invalid form)
        let options2 = make_options(
            config_dir.path(),
            &out_path,
            "default.json", // invalid — has extension
            "default",
            false,
        );
        let builder2 = CreateBuilder::new(options2);
        let builder2 = builder2.select_palette().expect("select_palette");
        match builder2.load_template() {
            Err(CreateError::InvalidTemplateName { name }) => {
                assert_eq!(name, "default.json");
            }
            Err(other) => panic!("expected InvalidTemplateName, got: {other:?}"),
            Ok(_) => panic!("expected error for template name with extension"),
        }
    }

    // ------------------------------------------------------------------
    // Env-resolution tests
    // ------------------------------------------------------------------

    #[test]
    fn env_resolved_with_env_allow() {
        let _lock = env_lock();
        let guard = EnvVarGuard::new("OCFG_CREATE_TEST_A");
        guard.set("secret-value");

        let config_dir = TempDir::new().expect("config dir");
        write_model_configs(&config_dir, &single_agent_configs());
        write_template(
            &config_dir,
            "envtest.json",
            r#"{
                "agent": {
                    "build": {
                        "model": "{{agent-build-model}}",
                        "apiKey": "{{env:OCFG_CREATE_TEST_A}}"
                    }
                }
            }"#,
        );

        let work_dir = TempDir::new().expect("work dir");
        let out_path = work_dir.path().join("opencode.json");

        let options = make_options_with_run(
            config_dir.path(),
            &out_path,
            "envtest",
            "default",
            RunOptions {
                strict: false,
                env_allow: true,
                env_mask_logs: false,
            },
        );

        CreateBuilder::new(options)
            .run()
            .expect("run should succeed");

        let data = fs::read_to_string(&out_path).expect("read output");
        let value: serde_json::Value = serde_json::from_str(&data).expect("parse json");
        assert_eq!(value["agent"]["build"]["model"], "openrouter/openai/gpt-4o");
        assert_eq!(value["agent"]["build"]["apiKey"], "secret-value");
    }

    #[test]
    fn env_not_resolved_without_env_allow() {
        let _lock = env_lock();
        let guard = EnvVarGuard::new("OCFG_CREATE_TEST_B");
        guard.set("should-not-appear");

        let config_dir = TempDir::new().expect("config dir");
        write_model_configs(&config_dir, &single_agent_configs());
        write_template(
            &config_dir,
            "envtest.json",
            r#"{
                "agent": {
                    "build": {
                        "model": "{{agent-build-model}}",
                        "apiKey": "{{env:OCFG_CREATE_TEST_B}}"
                    }
                }
            }"#,
        );

        let work_dir = TempDir::new().expect("work dir");
        let out_path = work_dir.path().join("opencode.json");

        let options = make_options_with_run(
            config_dir.path(),
            &out_path,
            "envtest",
            "default",
            RunOptions {
                strict: false,
                env_allow: false,
                env_mask_logs: false,
            },
        );

        CreateBuilder::new(options)
            .run()
            .expect("run should succeed in permissive mode");

        let data = fs::read_to_string(&out_path).expect("read output");
        let value: serde_json::Value = serde_json::from_str(&data).expect("parse json");
        // env placeholder should be left unresolved
        assert_eq!(
            value["agent"]["build"]["apiKey"],
            "{{env:OCFG_CREATE_TEST_B}}"
        );
    }

    #[test]
    fn env_strict_errors_on_missing_var() {
        let _lock = env_lock();
        let guard = EnvVarGuard::new("OCFG_CREATE_TEST_C");
        guard.remove();

        let config_dir = TempDir::new().expect("config dir");
        write_model_configs(&config_dir, &single_agent_configs());
        write_template(
            &config_dir,
            "envtest.json",
            r#"{"apiKey": "{{env:OCFG_CREATE_TEST_C}}"}"#,
        );

        let work_dir = TempDir::new().expect("work dir");
        let out_path = work_dir.path().join("opencode.json");

        let options = make_options_with_run(
            config_dir.path(),
            &out_path,
            "envtest",
            "default",
            RunOptions {
                strict: true,
                env_allow: true,
                env_mask_logs: false,
            },
        );

        let err = CreateBuilder::new(options)
            .run()
            .expect_err("should fail on missing env var in strict mode");

        let msg = err.to_string();
        assert!(
            msg.contains("env") && msg.contains("OCFG_CREATE_TEST_C"),
            "error should mention the missing env var, got: {msg}",
        );
    }

    #[test]
    fn env_missing_var_permissive_leaves_placeholder() {
        let _lock = env_lock();
        let guard = EnvVarGuard::new("OCFG_CREATE_TEST_D");
        guard.remove();

        let config_dir = TempDir::new().expect("config dir");
        write_model_configs(&config_dir, &single_agent_configs());
        write_template(
            &config_dir,
            "envtest.json",
            r#"{"apiKey": "{{env:OCFG_CREATE_TEST_D}}"}"#,
        );

        let work_dir = TempDir::new().expect("work dir");
        let out_path = work_dir.path().join("opencode.json");

        let options = make_options_with_run(
            config_dir.path(),
            &out_path,
            "envtest",
            "default",
            RunOptions {
                strict: false,
                env_allow: true,
                env_mask_logs: false,
            },
        );

        CreateBuilder::new(options)
            .run()
            .expect("permissive mode should succeed");

        let data = fs::read_to_string(&out_path).expect("read output");
        let value: serde_json::Value = serde_json::from_str(&data).expect("parse json");
        assert_eq!(
            value["apiKey"], "{{env:OCFG_CREATE_TEST_D}}",
            "unresolved env placeholder should be left as-is",
        );
    }

    #[test]
    fn env_no_placeholders_passthrough() {
        // Template without env: placeholders should pass through unchanged.
        let config_dir = TempDir::new().expect("config dir");
        write_model_configs(&config_dir, &single_agent_configs());
        write_template(
            &config_dir,
            "plain.json",
            r#"{"model": "{{agent-build-model}}"}"#,
        );

        let work_dir = TempDir::new().expect("work dir");
        let out_path = work_dir.path().join("opencode.json");

        let options = make_options_with_run(
            config_dir.path(),
            &out_path,
            "plain",
            "default",
            RunOptions {
                strict: false,
                env_allow: true,
                env_mask_logs: false,
            },
        );

        CreateBuilder::new(options)
            .run()
            .expect("run should succeed");

        let data = fs::read_to_string(&out_path).expect("read output");
        let value: serde_json::Value = serde_json::from_str(&data).expect("parse json");
        assert_eq!(value["model"], "openrouter/openai/gpt-4o");
    }

    #[test]
    fn env_mask_logs_does_not_affect_output() {
        let _lock = env_lock();
        let guard = EnvVarGuard::new("OCFG_CREATE_TEST_MASK");
        guard.set("sk-abc123secret");

        let config_dir = TempDir::new().expect("config dir");
        write_model_configs(&config_dir, &single_agent_configs());
        write_template(
            &config_dir,
            "envtest.json",
            r#"{"apiKey": "{{env:OCFG_CREATE_TEST_MASK}}"}"#,
        );

        let work_dir = TempDir::new().expect("work dir");
        let out_path = work_dir.path().join("opencode.json");

        let options = make_options_with_run(
            config_dir.path(),
            &out_path,
            "envtest",
            "default",
            RunOptions {
                strict: false,
                env_allow: true,
                env_mask_logs: true,
            },
        );

        CreateBuilder::new(options)
            .run()
            .expect("run should succeed");

        let data = fs::read_to_string(&out_path).expect("read output");
        let value: serde_json::Value = serde_json::from_str(&data).expect("parse json");
        // Output must contain the full value, not a masked version.
        assert_eq!(value["apiKey"], "sk-abc123secret");
    }

    #[test]
    fn env_inline_placeholder_resolved() {
        let _lock = env_lock();
        let guard = EnvVarGuard::new("OCFG_CREATE_TEST_INLINE");
        guard.set("example.com");

        let config_dir = TempDir::new().expect("config dir");
        write_model_configs(&config_dir, &single_agent_configs());
        write_template(
            &config_dir,
            "envtest.json",
            r#"{"url": "https://{{env:OCFG_CREATE_TEST_INLINE}}/api"}"#,
        );

        let work_dir = TempDir::new().expect("work dir");
        let out_path = work_dir.path().join("opencode.json");

        let options = make_options_with_run(
            config_dir.path(),
            &out_path,
            "envtest",
            "default",
            RunOptions {
                strict: false,
                env_allow: true,
                env_mask_logs: false,
            },
        );

        CreateBuilder::new(options)
            .run()
            .expect("run should succeed");

        let data = fs::read_to_string(&out_path).expect("read output");
        let value: serde_json::Value = serde_json::from_str(&data).expect("parse json");
        assert_eq!(value["url"], "https://example.com/api");
    }
}
