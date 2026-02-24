use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::predicate;
use serde_json::{Value, json};
use tempfile::TempDir;

const SAMPLE_YAML: &str = r#"
palettes:
  default:
    agents:
      build:
        model: openrouter/openai/gpt-4o
    mapping:
      payload:
        nested:
          - a
          - b
        enabled: false
      count: 3
"#;

const TEMPLATE: &str = r#"{
  "agent": {
    "build": {
      "model": "{{build}}"
    }
  },
  "payload": "{{payload}}",
  "description": "Build uses {{build}}",
  "message": "Count is {{count}}"
}
"#;

fn config_dir_from_home(config_home: &TempDir) -> PathBuf {
    config_home.path().join("opencode-config.d")
}

fn write_config(config_dir: &Path) {
    fs::create_dir_all(config_dir).expect("create config dir");
    fs::write(config_dir.join("model-configs.yaml"), SAMPLE_YAML)
        .expect("write model-configs.yaml");

    let template_dir = config_dir.join("template.d");
    fs::create_dir_all(&template_dir).expect("create template dir");
    fs::write(template_dir.join("default.json"), TEMPLATE).expect("write template");
}

fn run_create(config_home: &TempDir, work_dir: &TempDir, strict: bool) -> Value {
    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .arg("create")
        .arg("default")
        .arg("default");
    if strict {
        cmd.arg("--strict");
    }
    cmd.assert().success();

    let output_path = work_dir.path().join("opencode.json");
    let data = fs::read_to_string(&output_path).expect("read output");
    serde_json::from_str(&data).expect("parse json")
}

#[test]
fn integration_nonstring_object_into_template() {
    let config_home = TempDir::new().expect("config home");
    let config_dir = config_dir_from_home(&config_home);
    write_config(&config_dir);
    let work_dir = TempDir::new().expect("work dir");

    let output = run_create(&config_home, &work_dir, false);
    assert_eq!(
        output["agent"]["build"]["model"],
        "openrouter/openai/gpt-4o"
    );
    assert_eq!(
        output["payload"],
        json!({"nested": ["a", "b"], "enabled": false})
    );
    assert_eq!(output["message"], "Count is 3");
}

#[test]
fn integration_nonstring_in_string_strict_vs_permissive() {
    let config_home = TempDir::new().expect("config home");
    let config_dir = config_dir_from_home(&config_home);
    write_config(&config_dir);
    let work_dir = TempDir::new().expect("work dir");

    let output = run_create(&config_home, &work_dir, false);
    assert_eq!(output["description"], "Build uses openrouter/openai/gpt-4o");

    let strict_dir = TempDir::new().expect("strict work dir");
    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(strict_dir.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .arg("create")
        .arg("default")
        .arg("default")
        .arg("--strict")
        .assert()
        .failure()
        .stderr(predicate::str::contains("requires string value"));
}
