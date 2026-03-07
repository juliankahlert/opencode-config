use std::fs;
use std::path::Path;

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::predicate;
use serde_json::Value;
use tempfile::TempDir;

/// Palette without `variant` on build agent so alias resolution does not
/// overwrite the template's variant field — lets us test merge semantics
/// independently.
const SAMPLE_YAML: &str = r#"
palettes:
  github:
    agents:
      build:
        model: openrouter/openai/gpt-4o
      review:
        model: openrouter/openai/gpt-4o
    mapping:
      build-variant: mini
"#;

const DIR_BASE_TEMPLATE: &str = r#"{
  "agent": {
    "build": {
      "model": "{{build}}",
      "variant": "{{build-variant}}"
    },
    "review": {
      "model": "{{review}}"
    }
  }
}"#;

const DIR_OVERRIDE_FRAGMENT: &str = r#"{
  "agent": {
    "build": {
      "variant": "large"
    }
  }
}"#;

const LEGACY_TEMPLATE: &str = r#"{
  "agent": {
    "build": {
      "model": "{{build}}"
    }
  }
}"#;

fn write_dir_config(config_dir: &Path) {
    fs::write(config_dir.join("model-configs.yaml"), SAMPLE_YAML)
        .expect("write model-configs.yaml");
    let template_dir = config_dir.join("template.d");
    fs::create_dir_all(&template_dir).expect("create template dir");

    // Directory template: "dirtest"
    let dir_template = template_dir.join("dirtest.d");
    fs::create_dir_all(&dir_template).expect("create dirtest.d");
    fs::write(dir_template.join("01-base.json"), DIR_BASE_TEMPLATE).expect("write base");
    fs::write(dir_template.join("02-override.json"), DIR_OVERRIDE_FRAGMENT)
        .expect("write override");

    // Legacy single-file template
    fs::write(template_dir.join("legacy.json"), LEGACY_TEMPLATE).expect("write legacy");
}

#[test]
fn create_with_directory_template() {
    let config_dir = TempDir::new().expect("config dir");
    write_dir_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("create")
        .arg("dirtest")
        .arg("github")
        .assert()
        .success();

    let output_path = work_dir.path().join("opencode.json");
    let data = fs::read_to_string(&output_path).expect("read output");
    let value: Value = serde_json::from_str(&data).expect("parse json");

    // The override fragment sets variant to "large" (literal), not the placeholder.
    // Because the palette's build agent has no variant, alias resolution does not
    // overwrite it.
    assert_eq!(value["agent"]["build"]["model"], "openrouter/openai/gpt-4o");
    assert_eq!(value["agent"]["build"]["variant"], "large");
    assert_eq!(
        value["agent"]["review"]["model"],
        "openrouter/openai/gpt-4o"
    );
}

#[test]
fn switch_with_directory_template() {
    let config_dir = TempDir::new().expect("config dir");
    write_dir_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    // First create with legacy template
    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("create")
        .arg("legacy")
        .arg("github")
        .assert()
        .success();

    // Switch to directory template
    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("switch")
        .arg("dirtest")
        .arg("github")
        .assert()
        .success();

    let output_path = work_dir.path().join("opencode.json");
    let data = fs::read_to_string(&output_path).expect("read output");
    let value: Value = serde_json::from_str(&data).expect("parse json");

    assert_eq!(value["agent"]["build"]["variant"], "large");
}

#[test]
fn ambiguous_file_and_directory_error() {
    let config_dir = TempDir::new().expect("config dir");
    write_dir_config(config_dir.path());

    // Create ambiguity: both legacy.json AND legacy.d/ exist
    let template_dir = config_dir.path().join("template.d");
    let ambiguous_dir = template_dir.join("legacy.d");
    fs::create_dir_all(&ambiguous_dir).expect("create legacy.d");
    fs::write(ambiguous_dir.join("base.json"), r#"{"agent":{}}"#).expect("write fragment");

    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("create")
        .arg("legacy")
        .arg("github")
        .assert()
        .failure()
        .stderr(predicate::str::contains("ambiguous"));

    // Ensure no output file was created
    assert!(!work_dir.path().join("opencode.json").exists());
}

#[test]
fn empty_directory_error() {
    let config_dir = TempDir::new().expect("config dir");
    fs::write(config_dir.path().join("model-configs.yaml"), SAMPLE_YAML).expect("write yaml");
    let template_dir = config_dir.path().join("template.d");
    fs::create_dir_all(&template_dir).expect("create template dir");

    // Create empty template directory
    let empty_dir = template_dir.join("empty.d");
    fs::create_dir_all(&empty_dir).expect("create empty.d");

    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("create")
        .arg("empty")
        .arg("github")
        .assert()
        .failure()
        .stderr(predicate::str::contains("empty"));
}

#[test]
fn file_only_backwards_compat() {
    let config_dir = TempDir::new().expect("config dir");
    write_dir_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("create")
        .arg("legacy")
        .arg("github")
        .assert()
        .success();

    let output_path = work_dir.path().join("opencode.json");
    let data = fs::read_to_string(&output_path).expect("read output");
    let value: Value = serde_json::from_str(&data).expect("parse json");

    assert_eq!(value["agent"]["build"]["model"], "openrouter/openai/gpt-4o");
}

#[test]
fn list_templates_includes_directories() {
    let config_dir = TempDir::new().expect("config dir");
    write_dir_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("list-templates")
        .assert()
        .success()
        .stdout(predicate::str::contains("dirtest"))
        .stdout(predicate::str::contains("legacy"));
}
