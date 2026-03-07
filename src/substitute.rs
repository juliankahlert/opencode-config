use std::collections::HashMap;

use regex::{Captures, Regex};
use serde_json::Value;
use thiserror::Error;
use tracing::trace;

#[derive(Debug, Error)]
pub enum SubstituteError {
    #[error("missing placeholder: {key}")]
    MissingPlaceholder { key: String },
    #[error("placeholder {key} requires string value")]
    NonStringInStringPlaceholder { key: String },
    #[error("failed to compile placeholder regex")]
    Regex {
        #[source]
        source: regex::Error,
    },
}

pub fn substitute(
    value: &mut Value,
    mapping: &HashMap<String, Value>,
    strict: bool,
) -> Result<(), SubstituteError> {
    substitute_with_env(value, mapping, None, strict)
}

/// Substitute placeholders using a model-config mapping merged with
/// optional environment entries.  Model-config keys take precedence.
///
/// Environment values are accepted as `String` and always injected as
/// `Value::String`, enforcing the env-is-always-string contract at the
/// type level.
pub fn substitute_with_env(
    value: &mut Value,
    mapping: &HashMap<String, Value>,
    env_mapping: Option<&HashMap<String, String>>,
    strict: bool,
) -> Result<(), SubstituteError> {
    trace!(
        mapping_keys = mapping.len(),
        has_env = env_mapping.is_some(),
        strict,
        "starting substitution"
    );
    match env_mapping {
        Some(env) => {
            let mut merged: HashMap<String, Value> = env
                .iter()
                .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                .collect();
            merged.extend(mapping.iter().map(|(k, v)| (k.clone(), v.clone())));
            SubstituteBuilder::new(&merged)?.strict(strict).apply(value)
        }
        None => SubstituteBuilder::new(mapping)?.strict(strict).apply(value),
    }
}

pub struct SubstituteBuilder<'a> {
    mapping: &'a HashMap<String, Value>,
    strict: bool,
    regex: Regex,
}

impl<'a> SubstituteBuilder<'a> {
    pub fn new(mapping: &'a HashMap<String, Value>) -> Result<Self, SubstituteError> {
        let regex = Regex::new(r"\{\{\s*([^\}]+?)\s*\}\}")
            .map_err(|source| SubstituteError::Regex { source })?;
        Ok(Self {
            mapping,
            strict: false,
            regex,
        })
    }

    pub fn strict(mut self, strict: bool) -> Self {
        self.strict = strict;
        self
    }

    pub fn apply(&self, value: &mut Value) -> Result<(), SubstituteError> {
        self.substitute_value(value)
    }

    fn substitute_value(&self, value: &mut Value) -> Result<(), SubstituteError> {
        match value {
            Value::Object(map) => {
                let keys: Vec<String> = map.keys().cloned().collect();
                for key in keys {
                    if let Some(child) = map.get_mut(&key) {
                        let remove = self.substitute_in_value(child, &key)?;
                        if remove {
                            map.remove(&key);
                        }
                    }
                }
            }
            Value::Array(items) => {
                for item in items {
                    self.substitute_value(item)?;
                }
            }
            Value::String(text) => {
                if let Some(replacement) = self.resolve_placeholder_value(text)? {
                    *value = replacement;
                } else {
                    let new_text = self.substitute_string(text)?;
                    *text = new_text;
                }
            }
            _ => {}
        }

        Ok(())
    }

    fn substitute_in_value(&self, value: &mut Value, key: &str) -> Result<bool, SubstituteError> {
        match value {
            Value::String(text) => {
                if let Some(placeholder) = self.missing_variant_placeholder(text, key) {
                    if self.strict {
                        return Err(SubstituteError::MissingPlaceholder { key: placeholder });
                    }
                    return Ok(true);
                }

                if let Some(replacement) = self.resolve_placeholder_value(text)? {
                    *value = replacement;
                } else {
                    let new_text = self.substitute_string(text)?;
                    *text = new_text;
                }
                Ok(false)
            }
            Value::Object(_) | Value::Array(_) => {
                self.substitute_value(value)?;
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    fn missing_variant_placeholder(&self, text: &str, key: &str) -> Option<String> {
        let trimmed = text.trim();
        let captures = self.regex.captures(trimmed)?;
        if captures.get(0)?.as_str() != trimmed {
            return None;
        }
        if key != "variant" {
            return None;
        }
        let placeholder = captures.get(1)?.as_str().trim();
        if !placeholder.ends_with("-variant") {
            return None;
        }
        if self.mapping.contains_key(placeholder) {
            return None;
        }
        Some(placeholder.to_string())
    }

    fn substitute_string(&self, text: &str) -> Result<String, SubstituteError> {
        let mut missing = None;
        let mut non_string = None;
        let replaced = self.regex.replace_all(text, |caps: &Captures| {
            let key = caps.get(1).map(|m| m.as_str().trim()).unwrap_or("");
            if let Some(value) = self.mapping.get(key) {
                match value.as_str() {
                    Some(value) => value.to_string(),
                    None => {
                        if non_string.is_none() {
                            non_string = Some(key.to_string());
                        }
                        value.to_string()
                    }
                }
            } else {
                if missing.is_none() {
                    missing = Some(key.to_string());
                }
                caps.get(0).map(|m| m.as_str()).unwrap_or("").to_string()
            }
        });

        if self.strict {
            if let Some(key) = missing {
                return Err(SubstituteError::MissingPlaceholder { key });
            }
            if let Some(key) = non_string {
                return Err(SubstituteError::NonStringInStringPlaceholder { key });
            }
        }

        Ok(replaced.to_string())
    }

    fn resolve_placeholder_value(&self, text: &str) -> Result<Option<Value>, SubstituteError> {
        let trimmed = text.trim();
        let captures = match self.regex.captures(trimmed) {
            Some(captures) => captures,
            None => return Ok(None),
        };
        if captures.get(0).map(|m| m.as_str()) != Some(trimmed) {
            return Ok(None);
        }
        let key = captures.get(1).map(|m| m.as_str().trim()).unwrap_or("");
        if let Some(value) = self.mapping.get(key) {
            return Ok(Some(value.clone()));
        }
        if self.strict {
            return Err(SubstituteError::MissingPlaceholder {
                key: key.to_string(),
            });
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::{Value, json};

    use super::{SubstituteError, substitute, substitute_with_env};

    fn mapping() -> HashMap<String, Value> {
        HashMap::from([
            (
                "build".to_string(),
                Value::String("openrouter/build".to_string()),
            ),
            (
                "build-variant".to_string(),
                Value::String("mini".to_string()),
            ),
            ("count".to_string(), json!(3)),
            ("flag".to_string(), json!(true)),
        ])
    }

    #[test]
    fn replaces_placeholders() {
        let mut value = json!({
            "agent": {
                "model": "{{build}}",
                "variant": "{{build-variant}}"
            }
        });
        substitute(&mut value, &mapping(), false).expect("substitute");

        assert_eq!(value["agent"]["model"], "openrouter/build");
        assert_eq!(value["agent"]["variant"], "mini");
    }

    #[test]
    fn missing_variant_removes_key() {
        let mut value = json!({
            "agent": {
                "model": "{{build}}",
                "variant": "{{review-variant}}",
                "other": "{{review-variant}}"
            }
        });
        substitute(&mut value, &mapping(), false).expect("substitute");

        assert_eq!(value["agent"]["model"], "openrouter/build");
        assert!(!value["agent"].as_object().unwrap().contains_key("variant"));
        assert_eq!(value["agent"]["other"], "{{review-variant}}");
    }

    #[test]
    fn replaces_inside_longer_string() {
        let mut value = json!({
            "description": "Model {{build}} is ready"
        });
        substitute(&mut value, &mapping(), false).expect("substitute");
        assert_eq!(value["description"], "Model openrouter/build is ready");
    }

    #[test]
    fn strict_errors_on_missing() {
        let mut value = json!({
            "agent": {
                "model": "{{build}}",
                "missing": "{{review}}"
            }
        });

        let error = substitute(&mut value, &mapping(), true).expect_err("error");
        match error {
            SubstituteError::MissingPlaceholder { key } => {
                assert_eq!(key, "review");
            }
            SubstituteError::NonStringInStringPlaceholder { .. } => {
                panic!("unexpected non-string error")
            }
            SubstituteError::Regex { .. } => panic!("unexpected regex error"),
        }
    }

    #[test]
    fn replaces_node_with_non_string_value() {
        let mut value = json!({
            "count": "{{count}}",
            "flag": "{{flag}}"
        });

        substitute(&mut value, &mapping(), false).expect("substitute");

        assert_eq!(value["count"], json!(3));
        assert_eq!(value["flag"], json!(true));
    }

    #[test]
    fn strict_errors_on_non_string_in_string() {
        let mut value = json!({
            "message": "Count is {{count}}"
        });

        let error = substitute(&mut value, &mapping(), true).expect_err("error");
        match error {
            SubstituteError::NonStringInStringPlaceholder { key } => {
                assert_eq!(key, "count");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn permissive_stringifies_non_string_in_string() {
        let mut value = json!({
            "message": "Count is {{count}}"
        });

        substitute(&mut value, &mapping(), false).expect("substitute");

        assert_eq!(value["message"], "Count is 3");
    }

    #[test]
    fn env_only_placeholder() {
        let mut value = json!({
            "host": "{{db-host}}"
        });
        let mapping = HashMap::new();
        let env_map = HashMap::from([("db-host".to_string(), "localhost".to_string())]);

        substitute_with_env(&mut value, &mapping, Some(&env_map), false).expect("substitute");

        assert_eq!(value["host"], "localhost");
    }

    #[test]
    fn model_config_wins_over_env() {
        let mut value = json!({ "model": "{{build}}" });

        let mapping = HashMap::from([(
            "build".to_string(),
            Value::String("openrouter/build".to_string()),
        )]);
        let env_map = HashMap::from([("build".to_string(), "env-override/build".to_string())]);

        substitute_with_env(&mut value, &mapping, Some(&env_map), false).expect("substitute");

        assert_eq!(value["model"], "openrouter/build");
    }

    #[test]
    fn env_fills_gap() {
        let mut value = json!({
            "model": "{{build}}",
            "host": "{{db-host}}"
        });

        let mapping = HashMap::from([(
            "build".to_string(),
            Value::String("openrouter/build".to_string()),
        )]);
        let env_map = HashMap::from([("db-host".to_string(), "db.example.com".to_string())]);

        substitute_with_env(&mut value, &mapping, Some(&env_map), false).expect("substitute");

        assert_eq!(value["model"], "openrouter/build");
        assert_eq!(value["host"], "db.example.com");
    }

    #[test]
    fn strict_rejects_missing_env() {
        let mut value = json!({
            "host": "{{db-host}}"
        });

        let mapping = HashMap::new();
        let env_map: HashMap<String, String> = HashMap::new();

        let error = substitute_with_env(&mut value, &mapping, Some(&env_map), true)
            .expect_err("should fail");

        match error {
            SubstituteError::MissingPlaceholder { key } => {
                assert_eq!(key, "db-host");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn env_none_fallback() {
        let mut value = json!({
            "model": "{{build}}",
            "missing": "{{unknown}}"
        });

        let mapping = HashMap::from([(
            "build".to_string(),
            Value::String("openrouter/build".to_string()),
        )]);

        substitute_with_env(&mut value, &mapping, None, false).expect("substitute");

        assert_eq!(value["model"], "openrouter/build");
        assert_eq!(value["missing"], "{{unknown}}");
    }

    #[test]
    fn env_permissive_unresolved_env_left_intact() {
        // With an empty env mapping and strict=false, an unresolved
        // env:-prefixed placeholder should remain verbatim in the output.
        let mut value = json!({
            "key": "{{env:SOME_VAR}}"
        });
        let mapping = HashMap::new();
        let env_map: HashMap<String, String> = HashMap::new();

        substitute_with_env(&mut value, &mapping, Some(&env_map), false)
            .expect("permissive substitute");

        assert_eq!(
            value["key"], "{{env:SOME_VAR}}",
            "unresolved env placeholder should remain in permissive mode"
        );
    }

    #[test]
    fn env_prefix_not_resolved_without_env_mapping() {
        // substitute() (no env mapping) must not resolve env:-prefixed
        // placeholders — they are opaque keys handled by the builder layer.
        let mut value = json!({
            "model": "{{build}}",
            "apiKey": "{{env:MY_VAR}}"
        });

        substitute(&mut value, &mapping(), false).expect("substitute");

        assert_eq!(value["model"], "openrouter/build");
        assert_eq!(
            value["apiKey"], "{{env:MY_VAR}}",
            "env: placeholder must not be resolved without env mapping"
        );
    }

    #[test]
    fn env_value_is_always_string() {
        // The env_mapping type is HashMap<String, String>, so non-string
        // values cannot be passed.  Verify that numeric-looking env values
        // are injected as Value::String, not Value::Number.
        let mut value = json!({
            "port": "{{db-port}}",
            "flag": "{{db-flag}}"
        });

        let mapping = HashMap::new();
        let env_map = HashMap::from([
            ("db-port".to_string(), "3306".to_string()),
            ("db-flag".to_string(), "true".to_string()),
        ]);

        substitute_with_env(&mut value, &mapping, Some(&env_map), false).expect("substitute");

        assert!(
            value["port"].is_string(),
            "numeric env value must be String"
        );
        assert_eq!(value["port"], "3306");
        assert!(
            !value["port"].is_number(),
            "env value must not become Number"
        );

        assert!(
            value["flag"].is_string(),
            "boolean-like env value must be String"
        );
        assert_eq!(value["flag"], "true");
        assert!(
            !value["flag"].is_boolean(),
            "env value must not become Bool"
        );
    }
}
