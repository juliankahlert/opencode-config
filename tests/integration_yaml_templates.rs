use std::fs;
use std::path::Path;

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;
use tempfile::TempDir;

const SAMPLE_YAML: &str = r#"
palettes:
  github:
    agents:
      build:
        model: openrouter/openai/gpt-4o
        variant: mini
        reasoning: true
      review:
        model: openrouter/openai/gpt-4o
        reasoning:
          effort: medium
          text_verbosity: low
"#;

const DEFAULT_TEMPLATE_JSON: &str = r#"{
  "agent": {
    "build": {
      "model": "{{build}}",
      "variant": "{{build-variant}}"
    },
    "review": {
      "model": "{{review}}",
      "variant": "{{review-variant}}"
    }
  },
  "description": "Build uses {{build}}"
}
"#;

const DEFAULT_TEMPLATE_YAML: &str = r#"
agent:
  build:
    model: "{{build}}"
    variant: "{{build-variant}}"
  review:
    model: "{{review}}"
    variant: "{{review-variant}}"
description: "Build uses {{build}}"
"#;

fn write_config(config_dir: &Path) {
    fs::write(config_dir.join("model-configs.yaml"), SAMPLE_YAML)
        .expect("write model-configs.yaml");

    let template_dir = config_dir.join("template.d");
    fs::create_dir_all(&template_dir).expect("create template dir");
    fs::write(template_dir.join("json.json"), DEFAULT_TEMPLATE_JSON).expect("write json template");
    fs::write(template_dir.join("yaml.yaml"), DEFAULT_TEMPLATE_YAML).expect("write yaml template");
    fs::write(template_dir.join("yml.yml"), DEFAULT_TEMPLATE_YAML).expect("write yml template");
}

fn run_create(config_home: &Path, template: &str) -> Value {
    let work_dir = TempDir::new().expect("work dir");
    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", config_home)
        .arg("create")
        .arg(template)
        .arg("github")
        .assert()
        .success();

    let output_path = work_dir.path().join("opencode.json");
    let data = fs::read_to_string(&output_path).expect("read output");
    serde_json::from_str(&data).expect("parse json")
}

#[test]
fn yaml_and_yml_templates_match_json_output() {
    let config_home = TempDir::new().expect("config home");
    let config_dir = config_home.path().join("opencode-config.d");
    fs::create_dir_all(&config_dir).expect("create config dir");
    write_config(&config_dir);

    let json_value = run_create(config_home.path(), "json");
    let yaml_value = run_create(config_home.path(), "yaml");
    let yml_value = run_create(config_home.path(), "yml");

    assert_eq!(json_value, yaml_value);
    assert_eq!(json_value, yml_value);

    assert_eq!(
        json_value["agent"]["build"]["model"],
        "openrouter/openai/gpt-4o"
    );
    assert_eq!(json_value["agent"]["build"]["variant"], "mini");
    assert_eq!(json_value["agent"]["build"]["reasoningEffort"], "high");
    assert!(json_value["agent"]["build"].get("textVerbosity").is_none());
    assert_eq!(
        json_value["agent"]["review"]["model"],
        "openrouter/openai/gpt-4o"
    );
    assert!(json_value["agent"]["review"].get("variant").is_none());
    assert_eq!(json_value["agent"]["review"]["reasoningEffort"], "medium");
    assert_eq!(json_value["agent"]["review"]["textVerbosity"], "low");
    assert_eq!(
        json_value["description"],
        "Build uses openrouter/openai/gpt-4o"
    );
}
