//! Typestate builder for the `render` command.
//!
//! [`RenderBuilder`] uses the typestate pattern to enforce a compile-time
//! valid ordering of steps when rendering a template to stdout (as JSON or
//! YAML) without writing a file to disk.
//!
//! # State-transition diagram
//!
//! ```text
//!   ┌───────────┐
//!   │   Start   │
//!   └─────┬─────┘
//!         │ load_configs()
//!         v
//!  ┌───────────────────────┐
//!  │    ConfigsLoaded      │
//!  └──────────┬────────────┘
//!             │ resolve_palette()
//!             v
//!  ┌───────────────────────┐
//!  │   PaletteResolved     │
//!  └──────────┬────────────┘
//!             │ resolve_template_path()
//!             v
//!  ┌───────────────────────┐
//!  │ TemplatePathResolved  │
//!  └──────────┬────────────┘
//!             │ load_template()
//!             v
//!  ┌───────────────────────┐
//!  │    TemplateLoaded     │
//!  └──────────┬────────────┘
//!             │ apply_alias_models()
//!             v
//!  ┌───────────────────────┐
//!  │   AliasesApplied      │
//!  └──────────┬────────────┘
//!             │ build_mapping()
//!             v
//!  ┌───────────────────────┐
//!  │    MappingBuilt       │
//!  └──────────┬────────────┘
//!             │ resolve_env_vars()
//!             v
//!  ┌───────────────────────┐
//!  │    EnvResolved        │
//!  └──────────┬────────────┘
//!             │ substitute()
//!             v
//!  ┌───────────────────────┐
//!  │     Substituted       │
//!  └──────────┬────────────┘
//!             │ serialize()
//!             v
//!      RenderOutput
//! ```
//!
//! Unlike [`super::create_builder::CreateBuilder`], the render builder does
//! not write to disk. It returns a [`RenderOutput`](crate::render::RenderOutput)
//! containing the serialized string and a line count.

use std::collections::HashMap;
use std::path::PathBuf;

use regex::Regex;
use serde_json::Value;

use crate::config::{ModelConfigs, Palette, load_model_configs};
use crate::env_resolve::{Allow, EnvResolver};
use crate::render::{OutputFormat, RenderError, RenderOptions, RenderOutput};
use crate::substitute::{SubstituteError, substitute};
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
pub(crate) struct EnvResolved {
    template_value: Value,
    mapping: HashMap<String, Value>,
}
pub(crate) struct Substituted {
    template_value: Value,
}

impl RenderBuilder<Start> {
    pub(crate) fn new(options: RenderOptions) -> Self {
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
    /// Resolve `env:*` placeholders found in the template via [`EnvResolver`].
    ///
    /// Resolved values are merged directly into the main `mapping` as
    /// `Value::String` entries so that the substitution step can use the
    /// unified mapping without a separate env side-channel.
    ///
    /// When `env_allow` is `false`, this is a no-op pass-through.
    pub(crate) fn resolve_env_vars(self) -> Result<RenderBuilder<EnvResolved>, RenderError> {
        let RenderBuilder { options, state } = self;
        let MappingBuilt {
            template_value,
            mut mapping,
        } = state;

        if options.env_allow {
            let env_placeholders = collect_env_placeholders(&template_value);
            if !env_placeholders.is_empty() {
                let resolver = EnvResolver::new(Allow::All, options.strict, options.env_mask_logs);
                let resolved = resolver
                    .resolve(&env_placeholders)
                    .map_err(resolve_to_render_error)?;
                for (key, value) in &resolved {
                    mapping.insert(key.clone(), Value::String(value.clone()));
                }
            }
        }

        Ok(RenderBuilder {
            options,
            state: EnvResolved {
                template_value,
                mapping,
            },
        })
    }

    /// Backward-compatible substitute that internally chains through env
    /// resolution.  This keeps the call chain in [`crate::render::render()`]
    /// (`…build_mapping()?.substitute()?.serialize()`) working without edits
    /// to `src/render.rs`.
    pub(crate) fn substitute(self) -> Result<RenderBuilder<Substituted>, RenderError> {
        self.resolve_env_vars()?.substitute()
    }
}

impl RenderBuilder<EnvResolved> {
    /// Substitute placeholders in the template using the unified mapping
    /// (model-config entries merged with any resolved environment values).
    pub(crate) fn substitute(self) -> Result<RenderBuilder<Substituted>, RenderError> {
        let RenderBuilder { options, state } = self;
        let EnvResolved {
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

/// Scan a JSON value for all `env:*` placeholder keys and return a mapping
/// suitable for [`EnvResolver::resolve`].
///
/// Each entry maps `"env:VAR"` → `"env:VAR"` so that the resolver sees the
/// `env:` prefix in the *value*, strips it, and resolves the OS variable.
/// The result is keyed by the original placeholder key (`"env:VAR"`) which
/// lines up with what `substitute_with_env` expects.
fn collect_env_placeholders(value: &Value) -> HashMap<String, String> {
    let re = Regex::new(r"\{\{\s*([^\}]+?)\s*\}\}").expect("placeholder regex");
    let mut result = HashMap::new();
    collect_env_walk(value, &re, &mut result);
    result
}

fn collect_env_walk(value: &Value, re: &Regex, out: &mut HashMap<String, String>) {
    match value {
        Value::String(s) => {
            for cap in re.captures_iter(s) {
                let key = cap.get(1).map(|m| m.as_str().trim()).unwrap_or("");
                if key.starts_with("env:") {
                    out.insert(key.to_string(), key.to_string());
                }
            }
        }
        Value::Object(map) => {
            for v in map.values() {
                collect_env_walk(v, re, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_env_walk(item, re, out);
            }
        }
        _ => {}
    }
}

/// Map [`crate::env_resolve::ResolveError`] to [`RenderError`] by converting
/// to [`SubstituteError::MissingPlaceholder`] with a stable `env:{var}` key.
///
/// We cannot add variants to `RenderError` (render.rs is out of scope), so we
/// re-use the existing `Substitute` variant.  The key carries structured
/// `env:{var}` semantics rather than the raw `Display` output of the error so
/// that consumers can parse or match on it predictably.
fn resolve_to_render_error(err: crate::env_resolve::ResolveError) -> RenderError {
    use crate::env_resolve::ResolveError;
    let key = match err {
        ResolveError::NotAllowed { var } => format!("env:{var} (requires --env-allow)"),
        ResolveError::MissingEnvVar { var } => format!("env:{var}"),
    };
    RenderError::Substitute(SubstituteError::MissingPlaceholder { key })
}

// Template resolution is centralized in template.rs to keep behavior consistent.

#[cfg(test)]
mod tests {
    use std::env;
    use std::ffi::OsString;
    use std::fs;
    use std::path::Path;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use serde_json::Value;
    use tempfile::TempDir;

    use crate::render::{OutputFormat, RenderError, RenderOptions};

    use super::RenderBuilder;

    // ------------------------------------------------------------------
    // Environment safety helpers
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
    // Fixtures
    // ------------------------------------------------------------------

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

    const ENV_TEMPLATE: &str = r#"{
  "agent": {
    "build": {
      "model": "{{build}}",
      "variant": "{{build-variant}}",
      "apiKey": "{{env:OCFG_TEST_API_KEY}}"
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

    // ------------------------------------------------------------------
    // Existing tests (unchanged)
    // ------------------------------------------------------------------

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

    // ------------------------------------------------------------------
    // New env-resolution tests
    // ------------------------------------------------------------------

    #[test]
    fn env_allowed_and_present_resolves_value() {
        let _lock = env_lock();
        let guard = EnvVarGuard::new("OCFG_TEST_API_KEY");
        guard.set("sk-test-secret-123");

        let config_dir = TempDir::new().expect("config dir");
        write_model_configs(config_dir.path(), SAMPLE_YAML);
        write_template(config_dir.path(), "default.json", ENV_TEMPLATE);

        let mut opts = default_options(config_dir.path());
        opts.env_allow = true;

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
            .resolve_env_vars()
            .expect("resolve env vars")
            .substitute()
            .expect("substitute")
            .serialize()
            .expect("serialize");

        let value: Value = serde_json::from_str(&output.data).expect("parse json");
        assert_eq!(
            value["agent"]["build"]["apiKey"], "sk-test-secret-123",
            "env placeholder should be resolved"
        );
        assert_eq!(value["agent"]["build"]["model"], "openrouter/openai/gpt-4o");
    }

    #[test]
    fn env_not_allowed_leaves_placeholder() {
        let config_dir = TempDir::new().expect("config dir");
        write_model_configs(config_dir.path(), SAMPLE_YAML);
        write_template(config_dir.path(), "default.json", ENV_TEMPLATE);

        let mut opts = default_options(config_dir.path());
        opts.env_allow = false;

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
            .resolve_env_vars()
            .expect("resolve env vars")
            .substitute()
            .expect("substitute")
            .serialize()
            .expect("serialize");

        let value: Value = serde_json::from_str(&output.data).expect("parse json");
        assert_eq!(
            value["agent"]["build"]["apiKey"], "{{env:OCFG_TEST_API_KEY}}",
            "env placeholder should be left unresolved when env_allow is false"
        );
    }

    #[test]
    fn env_strict_missing_var_returns_error() {
        let _lock = env_lock();
        let guard = EnvVarGuard::new("OCFG_TEST_API_KEY");
        guard.remove();

        let config_dir = TempDir::new().expect("config dir");
        write_model_configs(config_dir.path(), SAMPLE_YAML);
        write_template(config_dir.path(), "default.json", ENV_TEMPLATE);

        let mut opts = default_options(config_dir.path());
        opts.env_allow = true;
        opts.strict = true;

        let result = RenderBuilder::new(opts)
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
            .resolve_env_vars();

        assert!(
            result.is_err(),
            "strict mode + missing env var should error"
        );
    }

    #[test]
    fn env_non_strict_missing_var_leaves_placeholder() {
        let _lock = env_lock();
        let guard = EnvVarGuard::new("OCFG_TEST_API_KEY");
        guard.remove();

        let config_dir = TempDir::new().expect("config dir");
        write_model_configs(config_dir.path(), SAMPLE_YAML);
        write_template(config_dir.path(), "default.json", ENV_TEMPLATE);

        let mut opts = default_options(config_dir.path());
        opts.env_allow = true;
        opts.strict = false;

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
            .resolve_env_vars()
            .expect("resolve env vars")
            .substitute()
            .expect("substitute")
            .serialize()
            .expect("serialize");

        let value: Value = serde_json::from_str(&output.data).expect("parse json");
        assert_eq!(
            value["agent"]["build"]["apiKey"], "{{env:OCFG_TEST_API_KEY}}",
            "non-strict + missing env var should leave placeholder"
        );
    }

    #[test]
    fn env_mask_logs_does_not_affect_output() {
        let _lock = env_lock();
        let guard = EnvVarGuard::new("OCFG_TEST_API_KEY");
        guard.set("sk-secret-value-abc");

        let config_dir = TempDir::new().expect("config dir");
        write_model_configs(config_dir.path(), SAMPLE_YAML);
        write_template(config_dir.path(), "default.json", ENV_TEMPLATE);

        let mut opts = default_options(config_dir.path());
        opts.env_allow = true;
        opts.env_mask_logs = true;

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
            .resolve_env_vars()
            .expect("resolve env vars")
            .substitute()
            .expect("substitute")
            .serialize()
            .expect("serialize");

        let value: Value = serde_json::from_str(&output.data).expect("parse json");
        assert_eq!(
            value["agent"]["build"]["apiKey"], "sk-secret-value-abc",
            "env_mask_logs must not affect the rendered output"
        );
    }

    #[test]
    fn backward_compat_substitute_on_mapping_built() {
        // Verify that calling .substitute() directly on MappingBuilt still
        // works (chains through resolve_env_vars internally).
        let _lock = env_lock();
        let guard = EnvVarGuard::new("OCFG_TEST_API_KEY");
        guard.set("compat-value");

        let config_dir = TempDir::new().expect("config dir");
        write_model_configs(config_dir.path(), SAMPLE_YAML);
        write_template(config_dir.path(), "default.json", ENV_TEMPLATE);

        let mut opts = default_options(config_dir.path());
        opts.env_allow = true;

        // Use the backward-compatible .substitute() on MappingBuilt
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
        assert_eq!(
            value["agent"]["build"]["apiKey"], "compat-value",
            "backward-compat substitute() should resolve env vars"
        );
    }
}
