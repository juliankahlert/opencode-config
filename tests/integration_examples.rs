use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::predicate;
use serde_json::Value;
use tempfile::TempDir;

fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples")
}

fn write_examples_to(config_dir: &Path) {
    let source_dir = examples_dir();
    let model_configs = fs::read_to_string(source_dir.join("model-configs.yaml"))
        .expect("read examples model-configs.yaml");
    fs::write(config_dir.join("model-configs.yaml"), model_configs)
        .expect("write model-configs.yaml");

    let template_dir = config_dir.join("template.d");
    fs::create_dir_all(&template_dir).expect("create template dir");
    let template = fs::read_to_string(source_dir.join("template.d").join("default.json"))
        .expect("read examples template");
    fs::write(template_dir.join("default.json"), template).expect("write template");
}

#[test]
fn examples_create_generates_opencode_json() {
    let config_dir = TempDir::new().expect("config dir");
    write_examples_to(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("create")
        .arg("default")
        .arg("default")
        .assert()
        .success();

    let output_path = work_dir.path().join("opencode.json");
    let data = fs::read_to_string(&output_path).expect("read output");
    let value: Value = serde_json::from_str(&data).expect("parse json");

    assert_eq!(value["agent"]["build"]["model"], "openrouter/openai/gpt-4o");
    assert_eq!(value["agent"]["build"]["variant"], "mini");
    assert_eq!(value["agent"]["build"]["reasoningEffort"], "high");
    assert!(value["agent"]["build"].get("textVerbosity").is_none());
    assert_eq!(
        value["agent"]["review"]["model"],
        "openrouter/openai/gpt-4o"
    );
    assert!(value["agent"]["review"].get("variant").is_none());
    assert_eq!(value["agent"]["review"]["reasoningEffort"], "medium");
    assert_eq!(value["agent"]["review"]["textVerbosity"], "low");
    assert_eq!(value["description"], "Build uses openrouter/openai/gpt-4o");
}

#[test]
fn examples_list_commands_use_xdg_config_home() {
    let config_home = TempDir::new().expect("config home");
    let config_dir = config_home.path().join("opencode-config.d");
    fs::create_dir_all(&config_dir).expect("create config dir");
    write_examples_to(&config_dir);
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .arg("list-templates")
        .assert()
        .success()
        .stdout(predicate::str::contains("default"));

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .arg("list-palettes")
        .assert()
        .success()
        .stdout(predicate::str::contains("default"));
}

#[test]
fn switch_overwrites_existing_output() {
    let config_dir = TempDir::new().expect("config dir");
    write_examples_to(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    // Create an initial output file
    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("create")
        .arg("default")
        .arg("default")
        .assert()
        .success();

    let output_path = work_dir.path().join("opencode.json");
    let before = fs::read_to_string(&output_path).expect("read output before");

    // Update model-configs to include an alternate palette so switch will change output
    let alt = r#"palettes:
  default:
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
  alt:
    agents:
      build:
        model: openai/alt-model
        variant: mega
        reasoning: true
      review:
        model: openai/alt-model
        reasoning:
          effort: low
          text_verbosity: high
"#;
    fs::write(config_dir.path().join("model-configs.yaml"), alt).expect("write alt configs");

    // Run switch to apply the alternate palette; switch should overwrite output
    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("switch")
        .arg("default")
        .arg("alt")
        .assert()
        .success();

    let after = fs::read_to_string(&output_path).expect("read output after");
    assert_ne!(before, after);
}

#[test]
fn switch_supports_out_and_strict() {
    let config_dir = TempDir::new().expect("config dir");
    write_examples_to(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    // Use -o to write to a custom file; ensure the -o path is honored
    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("switch")
        .arg("default")
        .arg("default")
        .arg("-o")
        .arg("custom.json")
        .assert()
        .success();

    let output_path = work_dir.path().join("custom.json");
    let data = fs::read_to_string(&output_path).expect("read custom output");
    let _: Value = serde_json::from_str(&data).expect("parse json");

    // Running with --strict should fail when placeholders are missing
    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("switch")
        .arg("default")
        .arg("default")
        .arg("--strict")
        .assert()
        .failure()
        .stderr(predicate::str::contains("missing placeholder"));
}
