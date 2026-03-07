use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::Value;
use thiserror::Error;

/// Ordered map of fragment filename → JSON content.
/// `BTreeMap` ensures deterministic iteration order.
pub type FragmentMap = BTreeMap<String, Value>;

#[derive(Debug, Error)]
pub enum MappingError {
    #[error("template is not a JSON object")]
    NotAnObject,

    #[error("agent key is not a JSON object")]
    AgentNotAnObject,

    #[error("agent entry '{name}' is not a JSON object")]
    AgentEntryNotAnObject { name: String },

    #[error("agent name '{name}' is not a valid filename")]
    InvalidAgentName { name: String },

    #[error("template name '{name}' is not a valid filename")]
    InvalidTemplateName { name: String },

    #[error("fragment name collision: '{name}' appears as both global and agent")]
    FragmentCollision { name: String },

    #[error("failed to write fragment at {path}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Abstraction for template decomposition strategies.
pub trait FragmentMapper {
    /// Decompose a monolithic template into named fragments.
    fn decompose(&self, template_name: &str, value: &Value) -> Result<FragmentMap, MappingError>;

    /// Validate that a set of fragments are structurally well-formed.
    fn validate(&self, fragments: &FragmentMap) -> Result<(), MappingError>;
}

pub struct DefaultMapping;

impl FragmentMapper for DefaultMapping {
    fn decompose(&self, template_name: &str, value: &Value) -> Result<FragmentMap, MappingError> {
        if !crate::template::is_valid_template_name(template_name) {
            return Err(MappingError::InvalidTemplateName {
                name: template_name.to_string(),
            });
        }

        let obj = value.as_object().ok_or(MappingError::NotAnObject)?;
        let mut fragments = FragmentMap::new();

        // Build global fragment: all keys except "agent"
        let global: serde_json::Map<String, Value> = obj
            .iter()
            .filter(|(k, _)| k.as_str() != "agent")
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        let global_filename = format!("{template_name}.json");

        // Always emit the global fragment so downstream directory loading
        // sees at least one file, even when the global content is empty.
        fragments.insert(global_filename.clone(), Value::Object(global));

        // Build per-agent fragments
        if let Some(agents) = obj.get("agent") {
            let agents_obj = agents.as_object().ok_or(MappingError::AgentNotAnObject)?;

            for (agent_name, agent_value) in agents_obj {
                validate_agent_name(agent_name)?;

                if !agent_value.is_object() {
                    return Err(MappingError::AgentEntryNotAnObject {
                        name: agent_name.clone(),
                    });
                }

                let fragment_name = format!("{agent_name}.json");
                if fragment_name == global_filename {
                    return Err(MappingError::FragmentCollision {
                        name: agent_name.clone(),
                    });
                }

                let wrapped = serde_json::json!({
                    "agent": { agent_name.clone(): agent_value.clone() }
                });

                fragments.insert(fragment_name, wrapped);
            }
        }

        Ok(fragments)
    }

    fn validate(&self, fragments: &FragmentMap) -> Result<(), MappingError> {
        for value in fragments.values() {
            // Every fragment must be a JSON object
            let obj = value.as_object().ok_or(MappingError::NotAnObject)?;

            // If fragment contains an "agent" key, validate the wrapping structure
            if let Some(agent_val) = obj.get("agent") {
                let agent_obj = agent_val
                    .as_object()
                    .ok_or(MappingError::AgentNotAnObject)?;

                // Agent wrapper must contain exactly one entry
                if agent_obj.len() != 1 {
                    // Use AgentNotAnObject — the wrapping is malformed
                    return Err(MappingError::AgentNotAnObject);
                }

                let (agent_name, inner) = agent_obj.iter().next().expect("exactly one entry");
                validate_agent_name(agent_name)?;

                if !inner.is_object() {
                    return Err(MappingError::AgentEntryNotAnObject {
                        name: agent_name.clone(),
                    });
                }
            }
        }

        Ok(())
    }
}

fn validate_agent_name(name: &str) -> Result<(), MappingError> {
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || name.starts_with('.')
    {
        return Err(MappingError::InvalidAgentName {
            name: name.to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn decompose_simple_template() {
        let template = json!({
            "systemPrompt": "You are helpful",
            "theme": "dark",
            "agent": {
                "build": { "model": "gpt-4o", "variant": "mini" },
                "review": { "model": "gpt-4o" }
            }
        });

        let mapper = DefaultMapping;
        let fragments = mapper.decompose("default", &template).expect("decompose");

        assert_eq!(fragments.len(), 3);
        assert!(fragments.contains_key("default.json"));
        assert!(fragments.contains_key("build.json"));
        assert!(fragments.contains_key("review.json"));

        // Global fragment has no agent key
        let global = &fragments["default.json"];
        assert!(global.get("agent").is_none());
        assert_eq!(global["systemPrompt"], "You are helpful");
        assert_eq!(global["theme"], "dark");

        // Agent fragments are wrapped
        let build = &fragments["build.json"];
        assert_eq!(build["agent"]["build"]["model"], "gpt-4o");
        assert_eq!(build["agent"]["build"]["variant"], "mini");

        let review = &fragments["review.json"];
        assert_eq!(review["agent"]["review"]["model"], "gpt-4o");
    }

    #[test]
    fn decompose_no_agent_key() {
        let template = json!({
            "systemPrompt": "You are helpful",
            "theme": "dark"
        });

        let mapper = DefaultMapping;
        let fragments = mapper.decompose("default", &template).expect("decompose");

        assert_eq!(fragments.len(), 1);
        assert!(fragments.contains_key("default.json"));
        assert_eq!(fragments["default.json"]["systemPrompt"], "You are helpful");
        assert_eq!(fragments["default.json"]["theme"], "dark");
    }

    #[test]
    fn decompose_agent_only() {
        let template = json!({
            "agent": {
                "build": { "model": "gpt-4o" }
            }
        });

        let mapper = DefaultMapping;
        let fragments = mapper.decompose("default", &template).expect("decompose");

        // Global fragment is always emitted, even when empty
        assert_eq!(fragments.len(), 2);
        assert!(fragments.contains_key("default.json"));
        assert_eq!(fragments["default.json"], json!({}));
        assert!(fragments.contains_key("build.json"));
        assert_eq!(fragments["build.json"]["agent"]["build"]["model"], "gpt-4o");
    }

    #[test]
    fn decompose_rejects_non_object() {
        let mapper = DefaultMapping;

        let array = json!([1, 2, 3]);
        let err = mapper.decompose("default", &array).unwrap_err();
        assert!(matches!(err, MappingError::NotAnObject));

        let string = json!("hello");
        let err = mapper.decompose("default", &string).unwrap_err();
        assert!(matches!(err, MappingError::NotAnObject));

        let number = json!(42);
        let err = mapper.decompose("default", &number).unwrap_err();
        assert!(matches!(err, MappingError::NotAnObject));

        let null = json!(null);
        let err = mapper.decompose("default", &null).unwrap_err();
        assert!(matches!(err, MappingError::NotAnObject));
    }

    #[test]
    fn decompose_rejects_non_object_agent() {
        let template = json!({
            "agent": "not-an-object"
        });

        let mapper = DefaultMapping;
        let err = mapper.decompose("default", &template).unwrap_err();
        assert!(matches!(err, MappingError::AgentNotAnObject));
    }

    #[test]
    fn decompose_rejects_invalid_agent_name_slash() {
        let template = json!({
            "agent": {
                "foo/bar": { "model": "gpt-4o" }
            }
        });

        let mapper = DefaultMapping;
        let err = mapper.decompose("default", &template).unwrap_err();
        assert!(matches!(err, MappingError::InvalidAgentName { .. }));
    }

    #[test]
    fn decompose_rejects_invalid_agent_name_backslash() {
        let template = json!({
            "agent": {
                "foo\\bar": { "model": "gpt-4o" }
            }
        });

        let mapper = DefaultMapping;
        let err = mapper.decompose("default", &template).unwrap_err();
        assert!(matches!(err, MappingError::InvalidAgentName { .. }));
    }

    #[test]
    fn decompose_rejects_invalid_agent_name_dotdot() {
        let template = json!({
            "agent": {
                "foo..bar": { "model": "gpt-4o" }
            }
        });

        let mapper = DefaultMapping;
        let err = mapper.decompose("default", &template).unwrap_err();
        assert!(matches!(err, MappingError::InvalidAgentName { .. }));
    }

    #[test]
    fn decompose_rejects_invalid_agent_name_leading_dot() {
        let template = json!({
            "agent": {
                ".hidden": { "model": "gpt-4o" }
            }
        });

        let mapper = DefaultMapping;
        let err = mapper.decompose("default", &template).unwrap_err();
        assert!(matches!(err, MappingError::InvalidAgentName { .. }));
    }

    #[test]
    fn decompose_rejects_invalid_agent_name_empty() {
        let template = json!({
            "agent": {
                "": { "model": "gpt-4o" }
            }
        });

        let mapper = DefaultMapping;
        let err = mapper.decompose("default", &template).unwrap_err();
        assert!(matches!(err, MappingError::InvalidAgentName { .. }));
    }

    #[test]
    fn decompose_detects_collision() {
        // Template name "build" collides with agent name "build"
        let template = json!({
            "systemPrompt": "You are helpful",
            "agent": {
                "build": { "model": "gpt-4o" }
            }
        });

        let mapper = DefaultMapping;
        let err = mapper.decompose("build", &template).unwrap_err();
        assert!(matches!(err, MappingError::FragmentCollision { .. }));
    }

    #[test]
    fn decompose_rejects_non_object_agent_entry() {
        let template = json!({
            "agent": {
                "build": "not-an-object"
            }
        });

        let mapper = DefaultMapping;
        let err = mapper.decompose("default", &template).unwrap_err();
        assert!(matches!(err, MappingError::AgentEntryNotAnObject { ref name } if name == "build"));
    }

    #[test]
    fn round_trip_simple() {
        use crate::template::deep_merge;

        let original = json!({
            "systemPrompt": "You are helpful",
            "agent": {
                "build": { "model": "gpt-4o" },
                "review": { "model": "gpt-5" }
            }
        });

        let mapper = DefaultMapping;
        let fragments = mapper.decompose("default", &original).expect("decompose");

        // Simulate load_template_dir: merge in BTreeMap order
        let mut merged = Value::Object(Default::default());
        for value in fragments.values() {
            deep_merge(&mut merged, value);
        }

        assert_eq!(merged, original);
    }

    #[test]
    fn round_trip_complex() {
        use crate::template::deep_merge;

        let original = json!({
            "systemPrompt": "Complex prompt",
            "theme": "dark",
            "options": {
                "verbose": true,
                "level": 3
            },
            "agent": {
                "build": {
                    "model": "gpt-4o",
                    "variant": "mini",
                    "config": {
                        "maxTokens": 4096,
                        "temperature": 0.7
                    }
                },
                "review": {
                    "model": "gpt-5",
                    "reasoningEffort": "high"
                },
                "deploy": {
                    "model": "claude-4",
                    "nested": {
                        "deep": {
                            "value": true
                        }
                    }
                }
            }
        });

        let mapper = DefaultMapping;
        let fragments = mapper.decompose("default", &original).expect("decompose");

        assert_eq!(fragments.len(), 4); // global + 3 agents

        let mut merged = Value::Object(Default::default());
        for value in fragments.values() {
            deep_merge(&mut merged, value);
        }

        assert_eq!(merged, original);
    }

    #[test]
    fn validate_empty_fragments() {
        let mapper = DefaultMapping;
        let fragments = FragmentMap::new();
        mapper.validate(&fragments).expect("empty map is valid");
    }

    #[test]
    fn validate_accepts_well_formed_fragments() {
        let mapper = DefaultMapping;
        let mut fragments = FragmentMap::new();
        fragments.insert(
            "default.json".to_string(),
            json!({"systemPrompt": "test", "theme": "dark"}),
        );
        fragments.insert(
            "build.json".to_string(),
            json!({"agent": {"build": {"model": "gpt-4o"}}}),
        );
        mapper.validate(&fragments).expect("well-formed is valid");
    }

    #[test]
    fn validate_catches_non_object_fragment() {
        let mapper = DefaultMapping;
        let mut fragments = FragmentMap::new();
        fragments.insert("bad.json".to_string(), json!("not an object"));

        let err = mapper.validate(&fragments).unwrap_err();
        assert!(matches!(err, MappingError::NotAnObject));
    }

    #[test]
    fn validate_catches_malformed_agent_wrapping_not_object() {
        let mapper = DefaultMapping;
        let mut fragments = FragmentMap::new();
        // agent key is not an object
        fragments.insert("bad.json".to_string(), json!({"agent": "not-an-object"}));

        let err = mapper.validate(&fragments).unwrap_err();
        assert!(matches!(err, MappingError::AgentNotAnObject));
    }

    #[test]
    fn validate_catches_malformed_agent_wrapping_multiple_entries() {
        let mapper = DefaultMapping;
        let mut fragments = FragmentMap::new();
        // agent wrapper has two entries instead of exactly one
        fragments.insert(
            "bad.json".to_string(),
            json!({"agent": {"build": {"model": "a"}, "review": {"model": "b"}}}),
        );

        let err = mapper.validate(&fragments).unwrap_err();
        assert!(matches!(err, MappingError::AgentNotAnObject));
    }

    #[test]
    fn validate_catches_malformed_agent_wrapping_empty() {
        let mapper = DefaultMapping;
        let mut fragments = FragmentMap::new();
        // agent wrapper is empty object
        fragments.insert("bad.json".to_string(), json!({"agent": {}}));

        let err = mapper.validate(&fragments).unwrap_err();
        assert!(matches!(err, MappingError::AgentNotAnObject));
    }

    #[test]
    fn validate_catches_invalid_agent_name_in_fragment() {
        let mapper = DefaultMapping;
        let mut fragments = FragmentMap::new();
        fragments.insert(
            "bad.json".to_string(),
            json!({"agent": {"../evil": {"model": "gpt-4o"}}}),
        );

        let err = mapper.validate(&fragments).unwrap_err();
        assert!(matches!(err, MappingError::InvalidAgentName { .. }));
    }

    #[test]
    fn validate_catches_non_object_agent_entry_in_fragment() {
        let mapper = DefaultMapping;
        let mut fragments = FragmentMap::new();
        fragments.insert(
            "bad.json".to_string(),
            json!({"agent": {"build": "not-an-object"}}),
        );

        let err = mapper.validate(&fragments).unwrap_err();
        assert!(matches!(err, MappingError::AgentEntryNotAnObject { ref name } if name == "build"));
    }

    #[test]
    fn decompose_empty_object_produces_global_fragment() {
        let template = json!({});

        let mapper = DefaultMapping;
        let fragments = mapper.decompose("default", &template).expect("decompose");

        assert_eq!(fragments.len(), 1);
        assert!(fragments.contains_key("default.json"));
        assert_eq!(fragments["default.json"], json!({}));
    }

    #[test]
    fn decompose_empty_agent_object_produces_global_fragment() {
        let template = json!({"agent": {}});

        let mapper = DefaultMapping;
        let fragments = mapper.decompose("default", &template).expect("decompose");

        assert_eq!(fragments.len(), 1);
        assert!(fragments.contains_key("default.json"));
        assert_eq!(fragments["default.json"], json!({}));
    }

    #[test]
    fn decompose_rejects_invalid_template_name_traversal() {
        let mapper = DefaultMapping;
        let template = json!({"key": "value"});

        let err = mapper.decompose("../x", &template).unwrap_err();
        assert!(matches!(err, MappingError::InvalidTemplateName { ref name } if name == "../x"));
    }

    #[test]
    fn decompose_rejects_invalid_template_name_slash() {
        let mapper = DefaultMapping;
        let template = json!({"key": "value"});

        let err = mapper.decompose("a/b", &template).unwrap_err();
        assert!(matches!(err, MappingError::InvalidTemplateName { ref name } if name == "a/b"));
    }

    #[test]
    fn decompose_rejects_invalid_template_name_empty() {
        let mapper = DefaultMapping;
        let template = json!({"key": "value"});

        let err = mapper.decompose("", &template).unwrap_err();
        assert!(matches!(err, MappingError::InvalidTemplateName { ref name } if name.is_empty()));
    }

    #[test]
    fn decompose_rejects_invalid_template_name_with_extension() {
        let mapper = DefaultMapping;
        let template = json!({"key": "value"});

        let err = mapper.decompose("default.json", &template).unwrap_err();
        assert!(
            matches!(err, MappingError::InvalidTemplateName { ref name } if name == "default.json")
        );
    }
}
