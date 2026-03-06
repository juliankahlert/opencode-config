use std::env;
use std::fs;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RunOptions {
    pub strict: bool,
    pub env_allow: bool,
    pub env_mask_logs: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct RunOptionsConfig {
    #[serde(default)]
    pub strict: Option<bool>,
    #[serde(default)]
    pub env_allow: Option<bool>,
    #[serde(default)]
    pub env_mask_logs: Option<bool>,
}

struct CliStep;
struct ConfigStep;
struct EnvStep;
struct DefaultStep;

struct RunOptionsBuilder<Stage> {
    strict: Option<bool>,
    env_allow: Option<bool>,
    env_mask_logs: Option<bool>,
    _stage: PhantomData<Stage>,
}

impl RunOptionsBuilder<CliStep> {
    fn new() -> Self {
        Self {
            strict: None,
            env_allow: None,
            env_mask_logs: None,
            _stage: PhantomData,
        }
    }

    fn apply_cli(
        mut self,
        strict: Option<bool>,
        env_allow: Option<bool>,
        env_mask_logs: Option<bool>,
    ) -> RunOptionsBuilder<ConfigStep> {
        self.strict = strict;
        self.env_allow = env_allow;
        self.env_mask_logs = env_mask_logs;
        RunOptionsBuilder {
            strict: self.strict,
            env_allow: self.env_allow,
            env_mask_logs: self.env_mask_logs,
            _stage: PhantomData,
        }
    }
}

impl RunOptionsBuilder<ConfigStep> {
    fn apply_config(mut self, config: Option<&RunOptionsConfig>) -> RunOptionsBuilder<EnvStep> {
        if let Some(config) = config {
            if self.strict.is_none() {
                self.strict = config.strict;
            }
            if self.env_allow.is_none() {
                self.env_allow = config.env_allow;
            }
            if self.env_mask_logs.is_none() {
                self.env_mask_logs = config.env_mask_logs;
            }
        }
        RunOptionsBuilder {
            strict: self.strict,
            env_allow: self.env_allow,
            env_mask_logs: self.env_mask_logs,
            _stage: PhantomData,
        }
    }
}

impl RunOptionsBuilder<EnvStep> {
    fn apply_env(mut self, strict: Option<bool>) -> RunOptionsBuilder<DefaultStep> {
        if self.strict.is_none() {
            self.strict = strict;
        }
        RunOptionsBuilder {
            strict: self.strict,
            env_allow: self.env_allow,
            env_mask_logs: self.env_mask_logs,
            _stage: PhantomData,
        }
    }

    fn env_flag_sources(self) -> (Option<bool>, Option<bool>) {
        (self.env_allow, self.env_mask_logs)
    }
}

impl RunOptionsBuilder<DefaultStep> {
    fn build(self) -> RunOptions {
        RunOptions {
            strict: self.strict.unwrap_or(false),
            env_allow: self.env_allow.unwrap_or(false),
            env_mask_logs: self.env_mask_logs.unwrap_or(false),
        }
    }
}

#[derive(Debug, Error)]
pub enum OptionsError {
    #[error("failed to read run options at {path}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse run options at {path}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("invalid boolean value for {var}: {value}")]
    InvalidEnv { var: &'static str, value: String },
}

pub fn resolve_run_options(
    cli_strict: Option<bool>,
    cli_env_allow: Option<bool>,
    cli_env_mask_logs: Option<bool>,
    config_dir: &Path,
) -> Result<RunOptions, OptionsError> {
    let config = load_run_options_config(config_dir)?;
    let env_strict = strict_from_env()?;

    let options = RunOptionsBuilder::new()
        .apply_cli(cli_strict, cli_env_allow, cli_env_mask_logs)
        .apply_config(config.as_ref())
        .apply_env(env_strict)
        .build();

    Ok(options)
}

pub fn resolve_env_flag_sources(
    cli_env_allow: Option<bool>,
    cli_env_mask_logs: Option<bool>,
    config_dir: &Path,
) -> Result<(Option<bool>, Option<bool>), OptionsError> {
    let config = load_run_options_config(config_dir)?;
    let sources = RunOptionsBuilder::new()
        .apply_cli(None, cli_env_allow, cli_env_mask_logs)
        .apply_config(config.as_ref())
        .env_flag_sources();
    Ok(sources)
}

fn load_run_options_config(config_dir: &Path) -> Result<Option<RunOptionsConfig>, OptionsError> {
    let path = config_dir.join("config.yaml");
    let data = match fs::read_to_string(&path) {
        Ok(data) => data,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(OptionsError::Read {
                path: path.clone(),
                source,
            });
        }
    };

    serde_yaml::from_str(&data)
        .map(Some)
        .map_err(|source| OptionsError::Parse { path, source })
}

fn strict_from_env() -> std::result::Result<Option<bool>, OptionsError> {
    let value = match env::var("OPENCODE_STRICT") {
        Ok(value) => value,
        Err(env::VarError::NotPresent) => return Ok(None),
        Err(env::VarError::NotUnicode(_)) => {
            return Err(OptionsError::InvalidEnv {
                var: "OPENCODE_STRICT",
                value: "<non-unicode>".to_string(),
            });
        }
    };

    let normalized = value.trim().to_ascii_lowercase();
    let parsed = match normalized.as_str() {
        "1" | "true" | "yes" | "on" => true,
        "0" | "false" | "no" | "off" => false,
        _ => {
            return Err(OptionsError::InvalidEnv {
                var: "OPENCODE_STRICT",
                value,
            });
        }
    };

    Ok(Some(parsed))
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::ffi::OsString;
    use std::fs;
    use std::path::Path;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use tempfile::TempDir;

    use super::{OptionsError, resolve_run_options};

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

    fn write_config(config_dir: &Path, strict: Option<bool>) {
        if let Some(strict) = strict {
            let data = format!("strict: {strict}\n");
            fs::write(config_dir.join("config.yaml"), data).expect("write config.yaml");
        }
    }

    fn write_full_config(
        config_dir: &Path,
        strict: Option<bool>,
        env_allow: Option<bool>,
        env_mask_logs: Option<bool>,
    ) {
        let mut data = String::new();
        if let Some(v) = strict {
            data.push_str(&format!("strict: {v}\n"));
        }
        if let Some(v) = env_allow {
            data.push_str(&format!("env_allow: {v}\n"));
        }
        if let Some(v) = env_mask_logs {
            data.push_str(&format!("env_mask_logs: {v}\n"));
        }
        fs::write(config_dir.join("config.yaml"), data).expect("write config.yaml");
    }

    #[test]
    fn resolve_run_options_respects_precedence() {
        let _lock = env_lock();
        let env_guard = EnvVarGuard::new("OPENCODE_STRICT");
        env_guard.remove();

        let config_home = TempDir::new().expect("config home");
        let config_dir = config_home.path();

        let options = resolve_run_options(None, None, None, config_dir).expect("resolve");
        assert!(!options.strict);

        env_guard.set("1");
        let options = resolve_run_options(None, None, None, config_dir).expect("resolve");
        assert!(options.strict);

        write_config(config_dir, Some(false));
        let options = resolve_run_options(None, None, None, config_dir).expect("resolve");
        assert!(!options.strict);

        let options = resolve_run_options(Some(true), None, None, config_dir).expect("resolve");
        assert!(options.strict);
    }

    #[test]
    fn strict_env_parsing_accepts_boolean_values() {
        let _lock = env_lock();
        let env_guard = EnvVarGuard::new("OPENCODE_STRICT");

        env_guard.set("on");
        let config_home = TempDir::new().expect("config home");
        let options = resolve_run_options(None, None, None, config_home.path()).expect("resolve");
        assert!(options.strict);

        env_guard.set("0");
        let options = resolve_run_options(None, None, None, config_home.path()).expect("resolve");
        assert!(!options.strict);
    }

    #[test]
    fn strict_env_rejects_invalid_values() {
        let _lock = env_lock();
        let env_guard = EnvVarGuard::new("OPENCODE_STRICT");
        env_guard.set("maybe");

        let config_home = TempDir::new().expect("config home");
        let result = resolve_run_options(None, None, None, config_home.path());
        match result {
            Err(OptionsError::InvalidEnv { var, value }) => {
                assert_eq!(var, "OPENCODE_STRICT");
                assert_eq!(value, "maybe");
            }
            other => panic!("unexpected result: {other:?}"),
        }
    }

    #[test]
    fn env_allow_and_env_mask_logs_precedence() {
        let _lock = env_lock();
        let env_guard = EnvVarGuard::new("OPENCODE_STRICT");
        env_guard.remove();

        let config_home = TempDir::new().expect("config home");
        let config_dir = config_home.path();

        // Defaults: both false when nothing is configured
        let options = resolve_run_options(None, None, None, config_dir).expect("resolve defaults");
        assert!(!options.env_allow, "default env_allow should be false");
        assert!(
            !options.env_mask_logs,
            "default env_mask_logs should be false"
        );

        // Config sets both true
        write_full_config(config_dir, None, Some(true), Some(true));
        let options =
            resolve_run_options(None, None, None, config_dir).expect("resolve config true");
        assert!(options.env_allow, "config env_allow=true should propagate");
        assert!(
            options.env_mask_logs,
            "config env_mask_logs=true should propagate"
        );

        // CLI false overrides config true
        let options = resolve_run_options(None, Some(false), Some(false), config_dir)
            .expect("resolve CLI override");
        assert!(
            !options.env_allow,
            "CLI env_allow=false should override config"
        );
        assert!(
            !options.env_mask_logs,
            "CLI env_mask_logs=false should override config"
        );

        // CLI true works without any config file
        let empty_dir = TempDir::new().expect("empty config home");
        let options = resolve_run_options(None, Some(true), Some(true), empty_dir.path())
            .expect("resolve CLI only");
        assert!(
            options.env_allow,
            "CLI env_allow=true should work without config"
        );
        assert!(
            options.env_mask_logs,
            "CLI env_mask_logs=true should work without config"
        );
    }
}
