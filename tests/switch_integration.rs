use std::fs;
use std::path::Path;

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::{PredicateBooleanExt, predicate};
use tempfile::TempDir;

const SAMPLE_YAML: &str = r#"
palettes:
  github:
    agents:
      build:
        model: openrouter/openai/gpt-4o
        variant: mini
"#;

const DEFAULT_TEMPLATE: &str = r#"{
  "agent": { "build": { "model": "{{build}}" } }
}"#;

fn write_config(config_dir: &Path) {
    fs::write(config_dir.join("model-configs.yaml"), SAMPLE_YAML)
        .expect("write model-configs.yaml");

    let template_dir = config_dir.join("template.d");
    fs::create_dir_all(&template_dir).expect("create template dir");
    fs::write(template_dir.join("default.json"), DEFAULT_TEMPLATE).expect("write template");
}

#[test]
fn switch_overwrites_existing_output() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    // First create with create command
    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("create")
        .arg("default")
        .arg("github")
        .assert()
        .success();

    let output_path = work_dir.path().join("opencode.json");
    let original = fs::read_to_string(&output_path).expect("read output");
    assert!(original.contains("openrouter/openai/gpt-4o"));

    // Run switch which should overwrite (force=true)
    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("switch")
        .arg("default")
        .arg("github")
        .assert()
        .success();

    let after = fs::read_to_string(&output_path).expect("read output");
    // content should have been updated (still contains because same inputs), but ensure file was written
    assert!(!after.is_empty());
}

#[test]
fn switch_supports_out_and_strict() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    let out_path = work_dir.path().join("custom.json");

    // smoke test: ensure -o and --strict flags are accepted
    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("switch")
        .arg("default")
        .arg("github")
        .arg("-o")
        .arg(&out_path)
        .arg("--strict")
        .assert()
        .success();

    assert!(out_path.exists());
}

#[test]
fn switch_strict_fails_on_missing_placeholder() {
    let config_dir = TempDir::new().expect("config dir");
    // write base config
    write_config(config_dir.path());

    // add a template that references a missing placeholder
    let template_dir = config_dir.path().join("template.d");
    fs::write(
        template_dir.join("strict.json"),
        r#"{ "agent": { "build": { "model": "{{missing}}" } } }"#,
    )
    .expect("write strict template");

    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("switch")
        .arg("strict")
        .arg("github")
        .arg("--strict")
        .assert()
        .failure()
        .stderr(predicate::str::contains("missing placeholder"));
}

#[test]
fn switch_rejects_template_with_path() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("switch")
        .arg("../default")
        .arg("github")
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid template name"));
}

// ---------------------------------------------------------------------------
// Dry-run integration tests
// ---------------------------------------------------------------------------

#[test]
fn test_switch_dry_run_shows_diff() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");
    let out_path = work_dir.path().join("opencode.json");

    // Write an existing file with different content
    fs::write(
        &out_path,
        r#"{
  "agent": {
    "build": {
      "model": "old-model"
    }
  }
}"#,
    )
    .expect("write existing");
    let before = fs::read_to_string(&out_path).expect("read before");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("switch")
        .arg("default")
        .arg("github")
        .arg("--dry-run")
        .arg("-o")
        .arg(&out_path)
        .assert()
        .code(1)
        .stdout(
            predicate::str::contains("---")
                .and(predicate::str::contains("+++"))
                .and(predicate::str::contains("openrouter/openai/gpt-4o")),
        );

    // Existing file must be unchanged
    let after = fs::read_to_string(&out_path).expect("read after");
    assert_eq!(before, after, "dry-run must not modify the existing file");
}

#[test]
fn test_switch_dry_run_no_write() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");
    let out_path = work_dir.path().join("opencode.json");

    // No existing file — switch --dry-run should print diff from /dev/null
    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("switch")
        .arg("default")
        .arg("github")
        .arg("--dry-run")
        .arg("-o")
        .arg(&out_path)
        .assert()
        .code(1)
        .stdout(predicate::str::contains("--- /dev/null").and(predicate::str::contains("+++ b/")));

    // Output file must NOT exist
    assert!(
        !out_path.exists(),
        "dry-run must not create the output file"
    );
}

#[test]
fn test_switch_dry_run_no_changes() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");
    let out_path = work_dir.path().join("opencode.json");

    // First, create the file normally so it matches the rendered output
    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("create")
        .arg("default")
        .arg("github")
        .arg("-o")
        .arg(&out_path)
        .assert()
        .success();

    let before = fs::read_to_string(&out_path).expect("read before");

    // Now switch --dry-run with the same template/palette — should report no changes
    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("switch")
        .arg("default")
        .arg("github")
        .arg("--dry-run")
        .arg("-o")
        .arg(&out_path)
        .assert()
        .success() // exit 0: no changes
        .stdout(predicate::str::contains("No changes"));

    // File must be byte-identical
    let after = fs::read_to_string(&out_path).expect("read after");
    assert_eq!(before, after, "dry-run must not modify the file");
}

#[test]
fn switch_with_directory_template() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());

    // Add a directory template
    let template_dir = config_dir.path().join("template.d");
    let dir_template = template_dir.join("dirswitch.d");
    fs::create_dir_all(&dir_template).expect("create dirswitch.d");
    fs::write(
        dir_template.join("01-base.json"),
        r#"{"agent": {"build": {"model": "{{build}}", "extra": "from-dir"}}}"#,
    )
    .expect("write base");

    let work_dir = TempDir::new().expect("work dir");

    // Create first with regular template
    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("create")
        .arg("default")
        .arg("github")
        .assert()
        .success();

    // Switch to directory template
    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("switch")
        .arg("dirswitch")
        .arg("github")
        .assert()
        .success();

    let output_path = work_dir.path().join("opencode.json");
    let data = fs::read_to_string(&output_path).expect("read output");
    let value: serde_json::Value = serde_json::from_str(&data).expect("parse json");

    assert_eq!(value["agent"]["build"]["model"], "openrouter/openai/gpt-4o");
    assert_eq!(value["agent"]["build"]["extra"], "from-dir");
}
