use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::predicate;
use serde_json::Value;
use tempfile::TempDir;

const SAMPLE_YAML: &str = r#"
palettes:
  default:
    agents:
      build:
        model: openrouter/openai/gpt-4o
        variant: mini
"#;

const TEMPLATE_OK: &str = r#"{
  "agent": {
    "build": {
      "model": "{{build}}",
      "variant": "{{build-variant}}"
    }
  }
}
"#;

const TEMPLATE_BAD: &str = r#"{
  "agent": {
    "build": {
      "model": "{{missing}}"
    }
  }
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
    fs::write(template_dir.join("ok.json"), TEMPLATE_OK).expect("write ok template");
    fs::write(template_dir.join("bad.json"), TEMPLATE_BAD).expect("write bad template");
}

#[test]
fn integration_validate_success() {
    let config_home = TempDir::new().expect("config home");
    let config_dir = config_dir_from_home(&config_home);
    write_config(&config_dir);
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .arg("validate")
        .arg("--templates")
        .arg("template.d/ok.json")
        .assert()
        .success();
}

#[test]
fn integration_validate_reports_missing_placeholders() {
    let config_home = TempDir::new().expect("config home");
    let config_dir = config_dir_from_home(&config_home);
    write_config(&config_dir);
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    let assert = cmd
        .current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .arg("validate")
        .arg("--format")
        .arg("json")
        .arg("--templates")
        .arg("template.d/bad.json")
        .assert()
        .failure();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout utf8");
    let report: Value = serde_json::from_str(stdout.trim()).expect("parse json");
    assert!(report["errors"].as_array().is_some());
    assert!(!report["errors"].as_array().unwrap().is_empty());
    assert!(
        report["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["kind"] == "unknown-placeholder")
    );
}

#[test]
fn integration_validate_text_output() {
    let config_home = TempDir::new().expect("config home");
    let config_dir = config_dir_from_home(&config_home);
    write_config(&config_dir);
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .arg("validate")
        .arg("--templates")
        .arg("template.d/bad.json")
        .assert()
        .failure()
        .stdout(predicate::str::contains("unknown placeholder"));
}
