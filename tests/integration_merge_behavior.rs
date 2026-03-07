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
    mapping:
      build-variant: mini
"#;

const BASE_TEMPLATE: &str = r#"{
  "agent": {
    "build": {
      "model": "{{build}}",
      "variant": "{{build-variant}}"
    }
  },
  "extra": "base-value"
}"#;

const OVERRIDE_FRAGMENT: &str = r#"{
  "agent": {
    "build": {
      "variant": "large"
    }
  }
}"#;

const INVALID_FRAGMENT: &str = r#"{ this is not valid json "#;

fn write_merge_config(config_dir: &Path) {
    fs::write(config_dir.join("model-configs.yaml"), SAMPLE_YAML).expect("write yaml");
    let template_dir = config_dir.join("template.d");
    fs::create_dir_all(&template_dir).expect("create template dir");

    let dir_template = template_dir.join("merge.d");
    fs::create_dir_all(&dir_template).expect("create merge.d");
    fs::write(dir_template.join("01-base.json"), BASE_TEMPLATE).expect("write base");
    fs::write(dir_template.join("02-override.json"), OVERRIDE_FRAGMENT).expect("write override");
}

#[test]
fn fragment_override_semantics() {
    let config_dir = TempDir::new().expect("config dir");
    write_merge_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("create")
        .arg("merge")
        .arg("github")
        .assert()
        .success();

    let output_path = work_dir.path().join("opencode.json");
    let data = fs::read_to_string(&output_path).expect("read output");
    let value: Value = serde_json::from_str(&data).expect("parse json");

    // Fragment overrides variant to "large" literal (not placeholder).
    // Because the palette's build agent has no variant, alias resolution
    // does not overwrite it.
    assert_eq!(value["agent"]["build"]["variant"], "large");
    // Model still comes from base and is resolved
    assert_eq!(value["agent"]["build"]["model"], "openrouter/openai/gpt-4o");
    // Extra key from base is preserved
    assert_eq!(value["extra"], "base-value");
}

#[test]
fn invalid_fragment_json_error() {
    let config_dir = TempDir::new().expect("config dir");
    fs::write(config_dir.path().join("model-configs.yaml"), SAMPLE_YAML).expect("write yaml");
    let template_dir = config_dir.path().join("template.d");
    fs::create_dir_all(&template_dir).expect("create template dir");

    let dir_template = template_dir.join("broken.d");
    fs::create_dir_all(&dir_template).expect("create broken.d");
    fs::write(dir_template.join("01-base.json"), r#"{"agent":{}}"#).expect("write base");
    fs::write(dir_template.join("02-bad.json"), INVALID_FRAGMENT).expect("write invalid");

    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("create")
        .arg("broken")
        .arg("github")
        .assert()
        .failure()
        .stderr(predicate::str::contains("parse"));
}

#[test]
fn mixed_format_directory_template() {
    let config_dir = TempDir::new().expect("config dir");
    fs::write(config_dir.path().join("model-configs.yaml"), SAMPLE_YAML).expect("write yaml");
    let template_dir = config_dir.path().join("template.d");
    fs::create_dir_all(&template_dir).expect("create template dir");

    let dir_template = template_dir.join("mixed.d");
    fs::create_dir_all(&dir_template).expect("create mixed.d");
    fs::write(
        dir_template.join("01-base.json"),
        r#"{"agent": {"build": {"model": "{{build}}"}}}"#,
    )
    .expect("write json base");
    fs::write(
        dir_template.join("02-overlay.yaml"),
        "agent:\n  build:\n    variant: yaml-value",
    )
    .expect("write yaml overlay");

    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("create")
        .arg("mixed")
        .arg("github")
        .assert()
        .success();

    let output_path = work_dir.path().join("opencode.json");
    let data = fs::read_to_string(&output_path).expect("read output");
    let value: Value = serde_json::from_str(&data).expect("parse json");

    assert_eq!(value["agent"]["build"]["model"], "openrouter/openai/gpt-4o");
    assert_eq!(value["agent"]["build"]["variant"], "yaml-value");
}
