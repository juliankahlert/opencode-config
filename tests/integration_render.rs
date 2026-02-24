use std::fs;
use std::path::Path;

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;
use tempfile::TempDir;

const SAMPLE_YAML: &str = r#"
palettes:
  default:
    agents:
      build:
        model: openrouter/openai/gpt-4o
        variant: mini
        reasoning: true
"#;

const DEFAULT_TEMPLATE: &str = r#"{
  "agent": {
    "build": {
      "model": "{{build}}",
      "variant": "{{build-variant}}"
    }
  },
  "description": "Build uses {{build}}"
}
"#;

fn write_config(config_dir: &Path) {
    fs::write(config_dir.join("model-configs.yaml"), SAMPLE_YAML)
        .expect("write model-configs.yaml");

    let template_dir = config_dir.join("template.d");
    fs::create_dir_all(&template_dir).expect("create template dir");
    fs::write(template_dir.join("default.json"), DEFAULT_TEMPLATE).expect("write template");
}

#[test]
fn render_outputs_json_to_stdout() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    let assert = cmd
        .current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("render")
        .arg("--template")
        .arg("default")
        .arg("--palette")
        .arg("default")
        .arg("--format")
        .arg("json")
        .arg("--out")
        .arg("-")
        .assert()
        .success();

    let output = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout utf8");
    let value: Value = serde_json::from_str(output.trim()).expect("parse json");
    assert_eq!(value["agent"]["build"]["model"], "openrouter/openai/gpt-4o");
    assert_eq!(value["agent"]["build"]["variant"], "mini");
    assert_eq!(value["agent"]["build"]["reasoningEffort"], "high");
    assert_eq!(value["description"], "Build uses openrouter/openai/gpt-4o");
}

#[test]
fn render_outputs_yaml_to_file() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");
    let out_path = work_dir.path().join("render.yaml");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("render")
        .arg("--template")
        .arg("default")
        .arg("--palette")
        .arg("default")
        .arg("--format")
        .arg("yaml")
        .arg("--out")
        .arg(&out_path)
        .assert()
        .success();

    let output = fs::read_to_string(&out_path).expect("read yaml");
    let yaml_value: serde_yaml::Value = serde_yaml::from_str(&output).expect("parse yaml");
    let json_value = serde_json::to_value(yaml_value).expect("convert yaml");
    assert_eq!(
        json_value["agent"]["build"]["model"],
        "openrouter/openai/gpt-4o"
    );
    assert_eq!(json_value["agent"]["build"]["variant"], "mini");
    assert_eq!(
        json_value["description"],
        "Build uses openrouter/openai/gpt-4o"
    );
}
