use std::fs;
use std::path::Path;

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::predicate;
use tempfile::TempDir;

const SAMPLE_YAML: &str = r#"
palettes:
  alpha:
    agents:
      build:
        model: openrouter/openai/gpt-4o
  beta:
    agents:
      build:
        model: openrouter/openai/gpt-4.1
"#;

fn write_config(config_dir: &Path) {
    fs::write(config_dir.join("model-configs.yaml"), SAMPLE_YAML)
        .expect("write model-configs.yaml");

    let template_dir = config_dir.join("template.d");
    fs::create_dir_all(&template_dir).expect("create template dir");
    fs::write(template_dir.join("default.json"), "{}").expect("write template");
    fs::write(template_dir.join("minimal.json"), "{}").expect("write template");
}

#[test]
fn list_templates_outputs_names() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("list-templates")
        .assert()
        .success()
        .stdout(predicate::str::contains("default"))
        .stdout(predicate::str::contains("minimal"));
}

#[test]
fn list_palettes_outputs_names() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("list-palettes")
        .assert()
        .success()
        .stdout(predicate::str::contains("alpha"))
        .stdout(predicate::str::contains("beta"));
}

#[test]
fn list_templates_includes_directory_templates() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());

    // Add a directory template
    let template_dir = config_dir.path().join("template.d");
    let dir_template = template_dir.join("dironly.d");
    fs::create_dir_all(&dir_template).expect("create dir template");
    fs::write(dir_template.join("base.json"), "{}").expect("write base");

    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("list-templates")
        .assert()
        .success()
        .stdout(predicate::str::contains("default"))
        .stdout(predicate::str::contains("minimal"))
        .stdout(predicate::str::contains("dironly"));
}
