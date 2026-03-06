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

const ENV_TEMPLATE: &str = r#"{
  "agent": {
    "build": {
      "model": "{{build}}",
      "variant": "{{build-variant}}",
      "apiKey": "{{env:OCFG_INTEG_RENDER_KEY}}"
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

fn write_env_config(config_dir: &Path) {
    fs::write(config_dir.join("model-configs.yaml"), SAMPLE_YAML)
        .expect("write model-configs.yaml");

    let template_dir = config_dir.join("template.d");
    fs::create_dir_all(&template_dir).expect("create template dir");
    fs::write(template_dir.join("env-test.json"), ENV_TEMPLATE).expect("write env template");
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

#[test]
fn render_dry_run_diff_new_file() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");
    let out_path = work_dir.path().join("opencode.json");

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
        .arg("--dry-run")
        .arg("--out")
        .arg(&out_path)
        .assert();

    // Exit 1 because diff is non-empty (new file)
    let assert = assert.code(1);

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout utf8");

    // Should show unified diff for a new file
    assert!(
        stdout.contains("--- /dev/null"),
        "new file diff should have /dev/null old label, got: {stdout}"
    );
    assert!(
        stdout.contains("+++ b/"),
        "new file diff should have b/ new label, got: {stdout}"
    );
    assert!(
        stdout.contains('+'),
        "new file diff should contain addition lines, got: {stdout}"
    );

    // File must NOT be created
    assert!(
        !out_path.exists(),
        "dry-run must not create the output file"
    );
}

#[test]
fn render_dry_run_diff_existing_changed() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");
    let out_path = work_dir.path().join("opencode.json");

    // Write a stale file that differs from what render would produce
    let stale_content = r#"{
  "agent": {
    "build": {
      "model": "old-model",
      "variant": "old-variant"
    }
  },
  "description": "stale"
}
"#;
    fs::write(&out_path, stale_content).expect("write stale file");
    let stale_before = fs::read_to_string(&out_path).expect("read stale before");

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
        .arg("--dry-run")
        .arg("--out")
        .arg(&out_path)
        .assert();

    // Exit 1 because diff is non-empty (file changed)
    let assert = assert.code(1);

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout utf8");

    // Should show unified diff with both removals and additions
    assert!(
        stdout.contains("---"),
        "diff should contain --- header, got: {stdout}"
    );
    assert!(
        stdout.contains("+++"),
        "diff should contain +++ header, got: {stdout}"
    );
    assert!(
        stdout.contains("-      \"model\": \"old-model\"")
            || stdout.contains("-  \"description\": \"stale\""),
        "diff should contain removal lines, got: {stdout}"
    );
    assert!(
        stdout.contains('+'),
        "diff should contain addition lines, got: {stdout}"
    );

    // Original file must NOT be modified
    let stale_after = fs::read_to_string(&out_path).expect("read stale after");
    assert_eq!(
        stale_before, stale_after,
        "dry-run must not modify the existing file"
    );
}

#[test]
fn render_dry_run_no_changes() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");
    let out_path = work_dir.path().join("opencode.json");

    // First, render without --dry-run to produce the actual output file
    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("render")
        .arg("--template")
        .arg("default")
        .arg("--palette")
        .arg("default")
        .arg("--out")
        .arg(&out_path)
        .assert()
        .success();

    assert!(out_path.exists(), "render should have created the file");

    // Now dry-run against the same file — should report no changes
    let mut cmd2 = cargo_bin_cmd!("opencode-config");
    let assert = cmd2
        .current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("render")
        .arg("--template")
        .arg("default")
        .arg("--palette")
        .arg("default")
        .arg("--dry-run")
        .arg("--out")
        .arg(&out_path)
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout utf8");
    assert!(
        stdout.contains("No changes"),
        "should report no changes when file matches, got: {stdout}"
    );
}

#[test]
fn render_dry_run_stdout_unchanged() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    // Dry-run with --out - should print raw rendered data (no diff)
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
        .arg("--dry-run")
        .arg("--out")
        .arg("-")
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout utf8");

    // Should be valid JSON (raw rendered output, not a diff)
    let value: Value = serde_json::from_str(stdout.trim()).expect("stdout should be valid JSON");
    assert_eq!(value["agent"]["build"]["model"], "openrouter/openai/gpt-4o");

    // Should NOT contain diff markers
    assert!(
        !stdout.contains("--- "),
        "stdout mode should not contain diff headers, got: {stdout}"
    );
    assert!(
        !stdout.contains("+++ "),
        "stdout mode should not contain diff headers, got: {stdout}"
    );
}

// ------------------------------------------------------------------
// Environment placeholder integration tests
// ------------------------------------------------------------------

#[test]
fn render_env_allow_resolves_env_placeholder() {
    let config_dir = TempDir::new().expect("config dir");
    write_env_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    let assert = cmd
        .current_dir(work_dir.path())
        .env("OCFG_INTEG_RENDER_KEY", "sk-integration-secret")
        .arg("--config")
        .arg(config_dir.path())
        .arg("render")
        .arg("--template")
        .arg("env-test")
        .arg("--palette")
        .arg("default")
        .arg("--format")
        .arg("json")
        .arg("--out")
        .arg("-")
        .arg("--env-allow")
        .assert()
        .success();

    let output = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout utf8");
    let value: Value = serde_json::from_str(output.trim()).expect("parse json");
    assert_eq!(
        value["agent"]["build"]["apiKey"], "sk-integration-secret",
        "env placeholder should resolve when --env-allow is set"
    );
    assert_eq!(value["agent"]["build"]["model"], "openrouter/openai/gpt-4o");
}

#[test]
fn render_without_env_allow_leaves_placeholder() {
    let config_dir = TempDir::new().expect("config dir");
    write_env_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    let assert = cmd
        .current_dir(work_dir.path())
        .env("OCFG_INTEG_RENDER_KEY", "should-not-appear")
        .arg("--config")
        .arg(config_dir.path())
        .arg("render")
        .arg("--template")
        .arg("env-test")
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
    assert_eq!(
        value["agent"]["build"]["apiKey"], "{{env:OCFG_INTEG_RENDER_KEY}}",
        "env placeholder should remain when --env-allow is not set"
    );
}

#[test]
fn render_env_allow_strict_missing_var_fails() {
    let config_dir = TempDir::new().expect("config dir");
    write_env_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    let assert = cmd
        .current_dir(work_dir.path())
        .env_remove("OCFG_INTEG_RENDER_KEY")
        .arg("--config")
        .arg(config_dir.path())
        .arg("render")
        .arg("--template")
        .arg("env-test")
        .arg("--palette")
        .arg("default")
        .arg("--format")
        .arg("json")
        .arg("--out")
        .arg("-")
        .arg("--env-allow")
        .arg("--strict")
        .assert()
        .failure();

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("stderr utf8");
    assert!(
        stderr.contains("OCFG_INTEG_RENDER_KEY") || stderr.contains("env:"),
        "stderr should mention the missing variable or env: prefix, got: {stderr}"
    );
}

#[test]
fn render_env_mask_logs_does_not_alter_output() {
    let config_dir = TempDir::new().expect("config dir");
    write_env_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    let secret = "sk-mask-test-value";

    let mut cmd = cargo_bin_cmd!("opencode-config");
    let assert = cmd
        .current_dir(work_dir.path())
        .env("OCFG_INTEG_RENDER_KEY", secret)
        .arg("--config")
        .arg(config_dir.path())
        .arg("render")
        .arg("--template")
        .arg("env-test")
        .arg("--palette")
        .arg("default")
        .arg("--format")
        .arg("json")
        .arg("--out")
        .arg("-")
        .arg("--env-allow")
        .arg("--env-mask-logs")
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout utf8");
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("stderr utf8");

    // Output must contain the resolved secret — masking only applies to logs.
    let value: Value = serde_json::from_str(stdout.trim()).expect("parse json");
    assert_eq!(
        value["agent"]["build"]["apiKey"], secret,
        "env_mask_logs must not affect rendered output content"
    );

    // The raw secret value must NOT appear in stderr / log output.
    assert!(
        !stderr.contains(secret),
        "raw secret must not leak to stderr when --env-mask-logs is active, got: {stderr}"
    );
}
