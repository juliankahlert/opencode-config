use std::fs;
use std::path::Path;

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::predicate;
use serde_json::Value;
use tempfile::TempDir;

const SAMPLE_YAML: &str = r#"
palettes:
  alpha:
    agents:
      build:
        model: openrouter/openai/gpt-4o
        variant: mini
        reasoning: true
"#;

fn write_config(config_dir: &Path) {
    fs::write(config_dir.join("model-configs.yaml"), SAMPLE_YAML)
        .expect("write model-configs.yaml");

    let template_dir = config_dir.join("template.d");
    fs::create_dir_all(&template_dir).expect("create template dir");
    fs::write(template_dir.join("default.json"), "{}".as_bytes()).expect("write template");
}

#[test]
fn palette_export_writes_json_output() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");
    let output_path = work_dir.path().join("alpha.json");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("palette")
        .arg("export")
        .arg("--name")
        .arg("alpha")
        .arg("--format")
        .arg("json")
        .arg("--out")
        .arg(&output_path)
        .assert()
        .success();

    let data = fs::read_to_string(&output_path).expect("read export");
    let value: Value = serde_json::from_str(&data).expect("parse json");
    assert_eq!(
        value["agents"]["build"]["model"],
        "openrouter/openai/gpt-4o"
    );
    assert_eq!(value["agents"]["build"]["variant"], "mini");
}

#[test]
fn palette_import_requires_force_to_persist() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    let import_path = work_dir.path().join("beta.yaml");
    fs::write(
        &import_path,
        r#"agents:
  build:
    model: openrouter/openai/gpt-4.1
"#,
    )
    .expect("write import");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("palette")
        .arg("import")
        .arg("--from")
        .arg(&import_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("[NO-WRITE]"));

    let data =
        fs::read_to_string(config_dir.path().join("model-configs.yaml")).expect("read configs");
    assert!(data.contains("alpha"));
    assert!(!data.contains("beta"));
}

#[test]
fn palette_import_abort_exits_nonzero() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    let import_path = work_dir.path().join("alpha.yaml");
    fs::write(
        &import_path,
        r#"agents:
  build:
    model: openrouter/openai/gpt-4.1
"#,
    )
    .expect("write import");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("palette")
        .arg("import")
        .arg("--from")
        .arg(&import_path)
        .arg("--merge")
        .arg("abort")
        .arg("--force")
        .assert()
        .failure()
        .stderr(predicate::str::contains("palette import aborted"));
}
