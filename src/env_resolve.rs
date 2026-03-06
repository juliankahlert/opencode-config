// Module not yet wired into the builder pipeline (integration is a follow-up
// task).  Items are pub(crate) but currently only exercised by unit tests.
#![allow(dead_code)]

use std::collections::HashMap;
use std::env;

use thiserror::Error;

/// Prefix that identifies environment-variable placeholders in mapping values.
const ENV_PREFIX: &str = "env:";

/// Error conditions returned by [`EnvResolver::resolve`].
#[derive(Debug, Error)]
pub(crate) enum ResolveError {
    #[error("env placeholders require --env-allow: {var}")]
    NotAllowed { var: String },

    #[error("environment variable not found: {var}")]
    MissingEnvVar { var: String },
}

/// Controls which environment variables the resolver is permitted to capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Allow {
    /// Env resolution is disabled — any `env:` value triggers an error.
    None,
    /// Every OS environment variable may be captured.
    All,
    /// Only the named variables may be captured.
    List(Vec<String>),
}

/// Reads OS environment variables for mapping entries whose value starts with
/// [`ENV_PREFIX`] and enforces an allow-policy plus strict-mode semantics.
pub(crate) struct EnvResolver {
    allow: Allow,
    strict: bool,
    mask_logs: bool,
}

impl EnvResolver {
    /// Build a resolver from runtime settings.
    pub(crate) fn new(allow: Allow, strict: bool, mask_logs: bool) -> Self {
        Self {
            allow,
            strict,
            mask_logs,
        }
    }

    /// Whether resolved values should be masked in diagnostic output.
    pub(crate) fn mask_logs(&self) -> bool {
        self.mask_logs
    }

    /// Walk `mappings` for entries whose **value** starts with `env:`, look up
    /// the corresponding OS environment variable, and return the resolved
    /// pairs keyed by the original mapping key.
    ///
    /// Entries whose value does **not** start with `env:` are silently ignored
    /// and excluded from the result.
    ///
    /// # Errors
    ///
    /// * [`ResolveError::NotAllowed`] — the allow policy forbids the variable.
    /// * [`ResolveError::MissingEnvVar`] — strict mode is on and the variable
    ///   is not set.
    pub(crate) fn resolve(
        &self,
        mappings: &HashMap<String, String>,
    ) -> Result<HashMap<String, String>, ResolveError> {
        let mut result = HashMap::new();

        for (key, value) in mappings {
            let var_name = match value.strip_prefix(ENV_PREFIX) {
                Some(name) => name,
                None => continue,
            };

            // Enforce allow policy.
            match &self.allow {
                Allow::None => {
                    return Err(ResolveError::NotAllowed {
                        var: var_name.to_string(),
                    });
                }
                Allow::List(allowed) => {
                    if !allowed.iter().any(|a| a == var_name) {
                        return Err(ResolveError::NotAllowed {
                            var: var_name.to_string(),
                        });
                    }
                }
                Allow::All => {}
            }

            // Look up OS environment.
            match env::var(var_name) {
                Ok(resolved) => {
                    result.insert(key.clone(), resolved);
                }
                Err(_) => {
                    if self.strict {
                        return Err(ResolveError::MissingEnvVar {
                            var: var_name.to_string(),
                        });
                    }
                    // Non-strict: silently skip missing variables.
                }
            }
        }

        Ok(result)
    }
}

/// Return a display-safe representation of `value`.
///
/// Values longer than 3 characters are masked after the first 3 chars.
/// Shorter values are fully replaced with `***`.
pub(crate) fn format_masked(value: &str) -> String {
    if value.len() <= 3 {
        "***".to_string()
    } else {
        format!("{}***", &value[..3])
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::env;
    use std::ffi::OsString;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use super::{Allow, EnvResolver, ResolveError, format_masked};

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
    // Helper to build a mapping from slices
    // ------------------------------------------------------------------

    fn mapping(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    // ------------------------------------------------------------------
    // Tests
    // ------------------------------------------------------------------

    /// 1. No env values in the mapping → empty result.
    #[test]
    fn no_env_values_returns_empty() {
        let resolver = EnvResolver::new(Allow::All, false, false);
        let m = mapping(&[("build", "gpt-4"), ("code-variant", "fast")]);
        let result = resolver.resolve(&m).expect("resolve");
        assert!(result.is_empty());
    }

    /// 2. Allow denied when a value is `env:VAR` → NotAllowed error.
    #[test]
    fn allow_denied_returns_not_allowed() {
        let resolver = EnvResolver::new(Allow::None, false, false);
        let m = mapping(&[("apiKey", "env:SECRET_KEY")]);
        let err = resolver.resolve(&m).unwrap_err();
        assert!(
            matches!(err, ResolveError::NotAllowed { ref var } if var == "SECRET_KEY"),
            "expected NotAllowed, got {err:?}",
        );
    }

    /// 3. Allow all + var present → captured at original key.
    #[test]
    fn allow_all_captures_present_var() {
        let _lock = env_lock();
        let guard = EnvVarGuard::new("OCFG_TEST_A");
        guard.set("hello");

        let resolver = EnvResolver::new(Allow::All, false, false);
        let m = mapping(&[("apiKey", "env:OCFG_TEST_A")]);
        let result = resolver.resolve(&m).expect("resolve");

        assert_eq!(result.get("apiKey").unwrap(), "hello");
    }

    /// 4. Allow list + var present → captured.
    #[test]
    fn allow_list_captures_present_var() {
        let _lock = env_lock();
        let guard = EnvVarGuard::new("OCFG_TEST_B");
        guard.set("world");

        let resolver = EnvResolver::new(Allow::List(vec!["OCFG_TEST_B".to_string()]), false, false);
        let m = mapping(&[("secret", "env:OCFG_TEST_B")]);
        let result = resolver.resolve(&m).expect("resolve");

        assert_eq!(result.get("secret").unwrap(), "world");
        drop(guard);
    }

    /// 5. Missing var non-strict → skipped.
    #[test]
    fn missing_var_non_strict_skipped() {
        let _lock = env_lock();
        let guard = EnvVarGuard::new("OCFG_TEST_MISSING");
        guard.remove();

        let resolver = EnvResolver::new(Allow::All, false, false);
        let m = mapping(&[("apiKey", "env:OCFG_TEST_MISSING")]);
        let result = resolver.resolve(&m).expect("resolve");

        assert!(result.is_empty());
    }

    /// 6. Missing var strict → MissingEnvVar error.
    #[test]
    fn missing_var_strict_returns_error() {
        let _lock = env_lock();
        let guard = EnvVarGuard::new("OCFG_TEST_STRICT");
        guard.remove();

        let resolver = EnvResolver::new(Allow::All, true, false);
        let m = mapping(&[("apiKey", "env:OCFG_TEST_STRICT")]);
        let err = resolver.resolve(&m).unwrap_err();

        assert!(
            matches!(err, ResolveError::MissingEnvVar { ref var } if var == "OCFG_TEST_STRICT"),
            "expected MissingEnvVar, got {err:?}",
        );
    }

    /// 7. Partial capture — one present env value, one missing → only present
    ///    inserted.
    #[test]
    fn partial_capture_only_present_inserted() {
        let _lock = env_lock();
        let guard_present = EnvVarGuard::new("OCFG_TEST_PRESENT");
        let guard_absent = EnvVarGuard::new("OCFG_TEST_ABSENT");
        guard_present.set("found");
        guard_absent.remove();

        let resolver = EnvResolver::new(Allow::All, false, false);
        let m = mapping(&[
            ("key_a", "env:OCFG_TEST_PRESENT"),
            ("key_b", "env:OCFG_TEST_ABSENT"),
        ]);
        let result = resolver.resolve(&m).expect("resolve");

        assert_eq!(result.len(), 1);
        assert_eq!(result.get("key_a").unwrap(), "found");
        assert!(!result.contains_key("key_b"));
    }

    /// 8. format_masked long value → first 3 chars + ***.
    #[test]
    fn format_masked_long_value() {
        assert_eq!(format_masked("sk-abc123"), "sk-***");
    }

    /// 9. format_masked short value → ***.
    #[test]
    fn format_masked_short_value() {
        assert_eq!(format_masked("ab"), "***");
        assert_eq!(format_masked("abc"), "***");
        assert_eq!(format_masked(""), "***");
    }
}
