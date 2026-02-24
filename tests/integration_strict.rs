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
"#;

const TEMPLATE: &str = r#"{
  "agent": {
    "build": {
      "model": "{{build}}",
      "variant": "{{build-variant}}"
    }
  }
}
"#;

fn config_dir_from_home(config_home: &TempDir) -> PathBuf {
    config_home.path().join("opencode-config.d")
}

fn write_config(config_dir: &Path, strict_setting: Option<bool>) {
    fs::create_dir_all(config_dir).expect("create config dir");
    fs::write(config_dir.join("model-configs.yaml"), SAMPLE_YAML)
        .expect("write model-configs.yaml");

    let template_dir = config_dir.join("template.d");
    fs::create_dir_all(&template_dir).expect("create template dir");
    fs::write(template_dir.join("default.json"), TEMPLATE).expect("write template");

    if let Some(strict) = strict_setting {
        let config_yaml = format!("strict: {strict}\n");
        fs::write(config_dir.join("config.yaml"), config_yaml).expect("write config.yaml");
    }
}

fn read_output(work_dir: &TempDir) -> Value {
    let output_path = work_dir.path().join("opencode.json");
    let data = fs::read_to_string(&output_path).expect("read output");
    serde_json::from_str(&data).expect("parse json")
}

#[test]
fn strict_default_is_permissive() {
    let config_home = TempDir::new().expect("config home");
    let config_dir = config_dir_from_home(&config_home);
    write_config(&config_dir, None);
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .arg("create")
        .arg("default")
        .arg("default")
        .assert()
        .success();

    let value = read_output(&work_dir);
    assert_eq!(value["agent"]["build"]["model"], "openrouter/openai/gpt-4o");
    assert!(value["agent"]["build"].get("variant").is_none());
}

#[test]
fn strict_env_enables_errors() {
    let config_home = TempDir::new().expect("config home");
    let config_dir = config_dir_from_home(&config_home);
    write_config(&config_dir, None);
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .env("OPENCODE_STRICT", "1")
        .arg("create")
        .arg("default")
        .arg("default")
        .assert()
        .failure()
        .stderr(predicate::str::contains("missing placeholder"));
}

#[test]
fn strict_config_enables_errors() {
    let config_home = TempDir::new().expect("config home");
    let config_dir = config_dir_from_home(&config_home);
    write_config(&config_dir, Some(true));
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .arg("create")
        .arg("default")
        .arg("default")
        .assert()
        .failure()
        .stderr(predicate::str::contains("missing placeholder"));
}

#[test]
fn strict_config_overrides_env() {
    let config_home = TempDir::new().expect("config home");
    let config_dir = config_dir_from_home(&config_home);
    write_config(&config_dir, Some(false));
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .env("OPENCODE_STRICT", "1")
        .arg("create")
        .arg("default")
        .arg("default")
        .assert()
        .success();

    let value = read_output(&work_dir);
    assert!(value["agent"]["build"].get("variant").is_none());
}

#[test]
fn strict_cli_overrides_config() {
    let config_home = TempDir::new().expect("config home");
    let config_dir = config_dir_from_home(&config_home);
    write_config(&config_dir, Some(false));
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .arg("create")
        .arg("default")
        .arg("default")
        .arg("--strict")
        .assert()
        .failure()
        .stderr(predicate::str::contains("missing placeholder"));
}

#[test]
fn strict_cli_overrides_env() {
    let config_home = TempDir::new().expect("config home");
    let config_dir = config_dir_from_home(&config_home);
    write_config(&config_dir, None);
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .env("OPENCODE_STRICT", "1")
        .arg("create")
        .arg("default")
        .arg("default")
        .arg("--no-strict")
        .assert()
        .success();

    let value = read_output(&work_dir);
    assert!(value["agent"]["build"].get("variant").is_none());
}

#[test]
fn strict_cli_overrides_config_true() {
    let config_home = TempDir::new().expect("config home");
    let config_dir = config_dir_from_home(&config_home);
    write_config(&config_dir, Some(true));
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .arg("create")
        .arg("default")
        .arg("default")
        .arg("--no-strict")
        .assert()
        .success();

    let value = read_output(&work_dir);
    assert!(value["agent"]["build"].get("variant").is_none());
}
