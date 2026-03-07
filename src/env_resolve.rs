use std::collections::HashMap;
use std::env;

use thiserror::Error;

/// Prefix that identifies environment-variable placeholders in mapping values.
const ENV_PREFIX: &str = "env:";

/// Error conditions returned by [`EnvResolver::resolve`].
#[derive(Debug, Error)]
pub(crate) enum ResolveError {
    #[error("environment variable not found: {var}")]
    MissingEnvVar { var: String },
}

/// Controls which environment variables the resolver is permitted to capture.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum Allow {
    /// Every OS environment variable may be captured.
    All,
    /// No environment variables may be captured.
    None,
    /// Only environment variables whose names appear in the list may be captured.
    List(Vec<String>),
}

/// Reads OS environment variables for mapping entries whose value starts with
/// [`ENV_PREFIX`] and enforces an allow-policy plus strict-mode semantics.
pub(crate) struct EnvResolver {
    allow: Allow,
    strict: bool,
}

impl EnvResolver {
    /// Build a resolver from runtime settings.
    pub(crate) fn new(allow: Allow, strict: bool) -> Self {
        Self { allow, strict }
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
                Allow::All => {}
                Allow::None => continue,
                Allow::List(permitted) => {
                    if !permitted.iter().any(|p| p == var_name) {
                        continue;
                    }
                }
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

/// Mask the middle characters of `value` with `*`, keeping the first and last
/// characters visible when the string is long enough.
///
/// - empty → `""`
/// - length 1 → `"*"`
/// - length 2 → `"**"`
/// - length ≥ 3 → first char + `*` repeated (len − 2) times + last char
///
/// The function operates on Unicode scalar values (`char`), so multi-byte
/// characters are handled correctly.
#[allow(dead_code)]
pub(crate) fn format_masked(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    match chars.len() {
        0 => String::new(),
        1 | 2 => "*".repeat(chars.len()),
        n => {
            let mut masked = String::with_capacity(value.len());
            masked.push(chars[0]);
            for _ in 0..n - 2 {
                masked.push('*');
            }
            masked.push(chars[n - 1]);
            masked
        }
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
        let resolver = EnvResolver::new(Allow::All, false);
        let m = mapping(&[("build", "gpt-4"), ("code-variant", "fast")]);
        let result = resolver.resolve(&m).expect("resolve");
        assert!(result.is_empty());
    }

    /// 2. Allow all + var present → captured at original key.
    #[test]
    fn allow_all_captures_present_var() {
        let _lock = env_lock();
        let guard = EnvVarGuard::new("OCFG_TEST_A");
        guard.set("hello");

        let resolver = EnvResolver::new(Allow::All, false);
        let m = mapping(&[("apiKey", "env:OCFG_TEST_A")]);
        let result = resolver.resolve(&m).expect("resolve");

        assert_eq!(result.get("apiKey").unwrap(), "hello");
    }

    /// 3. Missing var non-strict → skipped.
    #[test]
    fn missing_var_non_strict_skipped() {
        let _lock = env_lock();
        let guard = EnvVarGuard::new("OCFG_TEST_MISSING");
        guard.remove();

        let resolver = EnvResolver::new(Allow::All, false);
        let m = mapping(&[("apiKey", "env:OCFG_TEST_MISSING")]);
        let result = resolver.resolve(&m).expect("resolve");

        assert!(result.is_empty());
    }

    /// 4. Missing var strict → MissingEnvVar error.
    #[test]
    fn missing_var_strict_returns_error() {
        let _lock = env_lock();
        let guard = EnvVarGuard::new("OCFG_TEST_STRICT");
        guard.remove();

        let resolver = EnvResolver::new(Allow::All, true);
        let m = mapping(&[("apiKey", "env:OCFG_TEST_STRICT")]);
        let err = resolver.resolve(&m).unwrap_err();

        assert!(
            matches!(err, ResolveError::MissingEnvVar { ref var } if var == "OCFG_TEST_STRICT"),
            "expected MissingEnvVar, got {err:?}",
        );
    }

    /// 5. Partial capture — one present env value, one missing → only present
    ///    inserted.
    #[test]
    fn partial_capture_only_present_inserted() {
        let _lock = env_lock();
        let guard_present = EnvVarGuard::new("OCFG_TEST_PRESENT");
        let guard_absent = EnvVarGuard::new("OCFG_TEST_ABSENT");
        guard_present.set("found");
        guard_absent.remove();

        let resolver = EnvResolver::new(Allow::All, false);
        let m = mapping(&[
            ("key_a", "env:OCFG_TEST_PRESENT"),
            ("key_b", "env:OCFG_TEST_ABSENT"),
        ]);
        let result = resolver.resolve(&m).expect("resolve");

        assert_eq!(result.len(), 1);
        assert_eq!(result.get("key_a").unwrap(), "found");
        assert!(!result.contains_key("key_b"));
    }

    // ------------------------------------------------------------------
    // Allow::None tests
    // ------------------------------------------------------------------

    /// 6. Allow::None rejects all env entries even when the variable is set.
    #[test]
    fn allow_none_rejects_all() {
        let _lock = env_lock();
        let guard = EnvVarGuard::new("OCFG_TEST_NONE");
        guard.set("should_not_appear");

        let resolver = EnvResolver::new(Allow::None, false);
        let m = mapping(&[("apiKey", "env:OCFG_TEST_NONE")]);
        let result = resolver.resolve(&m).expect("resolve");

        assert!(result.is_empty(), "Allow::None must produce empty result");
    }

    // ------------------------------------------------------------------
    // Allow::List tests
    // ------------------------------------------------------------------

    /// 7. Allow::List permits only listed variable names.
    #[test]
    fn allow_list_permits_whitelisted() {
        let _lock = env_lock();
        let guard = EnvVarGuard::new("OCFG_TEST_LISTED");
        guard.set("permitted_value");

        let resolver = EnvResolver::new(Allow::List(vec!["OCFG_TEST_LISTED".to_string()]), false);
        let m = mapping(&[("key", "env:OCFG_TEST_LISTED")]);
        let result = resolver.resolve(&m).expect("resolve");

        assert_eq!(result.get("key").unwrap(), "permitted_value");
    }

    /// 8. Allow::List rejects variables not in the list.
    #[test]
    fn allow_list_rejects_unlisted() {
        let _lock = env_lock();
        let guard = EnvVarGuard::new("OCFG_TEST_UNLISTED");
        guard.set("secret");

        let resolver = EnvResolver::new(Allow::List(vec!["SOME_OTHER_VAR".to_string()]), false);
        let m = mapping(&[("key", "env:OCFG_TEST_UNLISTED")]);
        let result = resolver.resolve(&m).expect("resolve");

        assert!(
            result.is_empty(),
            "unlisted variable must be excluded from result"
        );
    }

    /// 9. Allow::List with mixed entries — only permitted ones appear.
    #[test]
    fn allow_list_mixed_entries() {
        let _lock = env_lock();
        let guard_a = EnvVarGuard::new("OCFG_TEST_ALLOW_A");
        let guard_b = EnvVarGuard::new("OCFG_TEST_ALLOW_B");
        guard_a.set("val_a");
        guard_b.set("val_b");

        let resolver = EnvResolver::new(Allow::List(vec!["OCFG_TEST_ALLOW_A".to_string()]), false);
        let m = mapping(&[
            ("a", "env:OCFG_TEST_ALLOW_A"),
            ("b", "env:OCFG_TEST_ALLOW_B"),
        ]);
        let result = resolver.resolve(&m).expect("resolve");

        assert_eq!(result.len(), 1);
        assert_eq!(result.get("a").unwrap(), "val_a");
        assert!(!result.contains_key("b"));
    }

    // ------------------------------------------------------------------
    // format_masked tests
    // ------------------------------------------------------------------

    /// 10. Masking an empty string returns empty.
    #[test]
    fn mask_empty() {
        assert_eq!(format_masked(""), "");
    }

    /// 11. Masking a single character returns `*`.
    #[test]
    fn mask_single_char() {
        assert_eq!(format_masked("x"), "*");
    }

    /// 12. Masking two characters returns `**`.
    #[test]
    fn mask_two_chars() {
        assert_eq!(format_masked("ab"), "**");
    }

    /// 13. Masking three characters keeps first and last, masks middle.
    #[test]
    fn mask_three_chars() {
        assert_eq!(format_masked("abc"), "a*c");
    }

    /// 14. Masking a longer string keeps first and last, masks all middle.
    #[test]
    fn mask_long_string() {
        assert_eq!(format_masked("secret_key"), "s********y");
    }

    /// 15. Masking works correctly with multi-byte UTF-8 characters.
    #[test]
    fn mask_unicode() {
        // "αβγ" — three 2-byte Greek chars
        assert_eq!(format_masked("αβγ"), "α*γ");
        // "日本語" — three 3-byte CJK chars
        assert_eq!(format_masked("日本語"), "日*語");
    }
}
