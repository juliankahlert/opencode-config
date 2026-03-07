use std::fs;
use std::path::Path;

use assert_cmd::cargo::cargo_bin_cmd;
use tempfile::TempDir;

const SAMPLE_YAML: &str = r#"
palettes:
  github:
    agents:
      build:
        model: openrouter/openai/gpt-4o
"#;

fn write_config(config_dir: &Path) {
    fs::write(config_dir.join("model-configs.yaml"), SAMPLE_YAML).expect("write yaml");
    let template_dir = config_dir.join("template.d");
    fs::create_dir_all(&template_dir).expect("create template dir");

    // Directory template with fragments
    let dir_template = template_dir.join("traced.d");
    fs::create_dir_all(&dir_template).expect("create traced.d");
    fs::write(
        dir_template.join("01-base.json"),
        r#"{"agent": {"build": {"model": "{{build}}"}}}"#,
    )
    .expect("write base");
    fs::write(dir_template.join("02-extra.json"), r#"{"extra": "value"}"#).expect("write extra");
}

#[test]
fn verbose_produces_debug_output() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    let output = cmd
        .current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("--verbose")
        .arg("create")
        .arg("traced")
        .arg("github")
        .output()
        .expect("run command");

    assert!(output.status.success(), "command should succeed");
    let stderr = String::from_utf8_lossy(&output.stderr);
    // With --verbose, we expect debug-level output containing our trace messages
    assert!(
        stderr.contains("template source resolved")
            || stderr.contains("loading template directory"),
        "expected tracing output in stderr with --verbose, got: {stderr}"
    );
}

#[test]
fn no_verbose_suppresses_debug_output() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    let output = cmd
        .current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("create")
        .arg("traced")
        .arg("github")
        .output()
        .expect("run command");

    assert!(output.status.success(), "command should succeed");
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Without --verbose, debug output should not appear
    assert!(
        !stderr.contains("template source resolved"),
        "debug output should not appear without --verbose: {stderr}"
    );
}
