use std::fs;
use std::path::Path;

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::{PredicateBooleanExt, predicate};
use serde_json::Value;
use tempfile::TempDir;

const ENV_TEMPLATE: &str = r#"{
  "agent": {
    "build": {
      "model": "{{build}}",
      "apiKey": "{{env:OCFG_INTEG_CREATE_KEY}}"
    }
  }
}
"#;

const SAMPLE_YAML: &str = r#"
palettes:
  github:
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
    mapping:
      build-variant: mini
      review-variant: mini
  docs:
    agents:
      build:
        model: openrouter/openai/gpt-4.1
"#;

const DEFAULT_TEMPLATE: &str = r#"{
  "agent": {
    "build": {
      "model": "{{build}}",
      "variant": "{{build-variant}}"
    },
    "review": {
      "model": "{{review}}",
      "variant": "{{review-variant}}"
    }
  },
  "meta": {
    "variant": "{{review-variant}}",
    "other": "{{missing-variant}}"
  },
  "description": "Build uses {{build}}"
}
"#;

const STRICT_TEMPLATE: &str = r#"{
  "agent": {
    "build": {
      "model": "{{missing}}"
    }
  }
}
"#;

fn write_config(config_dir: &Path) {
    fs::write(config_dir.join("model-configs.yaml"), SAMPLE_YAML)
        .expect("write model-configs.yaml");

    let template_dir = config_dir.join("template.d");
    fs::create_dir_all(&template_dir).expect("create template dir");
    fs::write(template_dir.join("default.json"), DEFAULT_TEMPLATE).expect("write template");
    fs::write(template_dir.join("strict.json"), STRICT_TEMPLATE).expect("write template");
    fs::write(template_dir.join("env.json"), ENV_TEMPLATE).expect("write env template");
}

#[test]
fn create_writes_opencode_json() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

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
    assert_eq!(value["agent"]["review"]["variant"], "mini");
    assert_eq!(value["agent"]["review"]["reasoningEffort"], "medium");
    assert_eq!(value["agent"]["review"]["textVerbosity"], "low");
    assert_eq!(value["meta"]["variant"], "mini");
    assert_eq!(value["meta"]["other"], "{{missing-variant}}");
    assert_eq!(value["description"], "Build uses openrouter/openai/gpt-4o");
}

#[test]
fn create_respects_force_flag() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");
    let output_path = work_dir.path().join("opencode.json");
    fs::write(&output_path, "existing").expect("write existing");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("create")
        .arg("default")
        .arg("github")
        .assert()
        .failure();

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("create")
        .arg("default")
        .arg("github")
        .arg("--force")
        .assert()
        .success();

    let data = fs::read_to_string(&output_path).expect("read output");
    assert!(data.contains("openrouter/openai/gpt-4o"));
}

#[test]
fn create_strict_errors_on_missing_placeholder() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());

    let non_strict_dir = TempDir::new().expect("non-strict work dir");
    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(non_strict_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("create")
        .arg("strict")
        .arg("github")
        .assert()
        .success();

    let output_path = non_strict_dir.path().join("opencode.json");
    let data = fs::read_to_string(&output_path).expect("read output");
    let value: Value = serde_json::from_str(&data).expect("parse json");
    assert_eq!(value["agent"]["build"]["model"], "{{missing}}");

    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("create")
        .arg("strict")
        .arg("github")
        .arg("--strict")
        .assert()
        .failure();
}

#[test]
fn create_rejects_template_with_extension() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("create")
        .arg("default.json")
        .arg("github")
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid template name"));
}

// ---------------------------------------------------------------------------
// Environment placeholder integration tests
// ---------------------------------------------------------------------------

#[test]
fn create_resolves_env_placeholder_with_env_allow() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .env("OCFG_INTEG_CREATE_KEY", "sk-secret-42")
        .arg("--config")
        .arg(config_dir.path())
        .arg("create")
        .arg("env")
        .arg("github")
        .arg("--env-allow")
        .assert()
        .success();

    let output_path = work_dir.path().join("opencode.json");
    let data = fs::read_to_string(&output_path).expect("read output");
    let value: Value = serde_json::from_str(&data).expect("parse json");

    assert_eq!(value["agent"]["build"]["model"], "openrouter/openai/gpt-4o");
    assert_eq!(value["agent"]["build"]["apiKey"], "sk-secret-42");
}

#[test]
fn create_leaves_env_placeholder_without_env_allow() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .env("OCFG_INTEG_CREATE_KEY", "should-not-appear")
        .arg("--config")
        .arg(config_dir.path())
        // no --env-allow flag
        .arg("create")
        .arg("env")
        .arg("github")
        .assert()
        .success();

    let output_path = work_dir.path().join("opencode.json");
    let data = fs::read_to_string(&output_path).expect("read output");
    let value: Value = serde_json::from_str(&data).expect("parse json");

    // Without --env-allow, the placeholder should remain unresolved
    assert_eq!(
        value["agent"]["build"]["apiKey"],
        "{{env:OCFG_INTEG_CREATE_KEY}}"
    );
}

#[test]
fn create_strict_errors_on_missing_env_var() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .env_remove("OCFG_INTEG_CREATE_KEY")
        .arg("--config")
        .arg(config_dir.path())
        .arg("create")
        .arg("env")
        .arg("github")
        .arg("--env-allow")
        .arg("--strict")
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("env").and(predicate::str::contains("OCFG_INTEG_CREATE_KEY")),
        );
}

#[test]
fn create_env_mask_logs_does_not_affect_output() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .env("OCFG_INTEG_CREATE_KEY", "real-secret-value")
        .arg("--config")
        .arg(config_dir.path())
        .arg("create")
        .arg("env")
        .arg("github")
        .arg("--env-allow")
        .arg("--env-mask-logs")
        .assert()
        .success();

    let output_path = work_dir.path().join("opencode.json");
    let data = fs::read_to_string(&output_path).expect("read output");
    let value: Value = serde_json::from_str(&data).expect("parse json");

    // env-mask-logs should not affect the actual output file content
    assert_eq!(value["agent"]["build"]["apiKey"], "real-secret-value");
}

// ---------------------------------------------------------------------------
// Dry-run integration tests
// ---------------------------------------------------------------------------

#[test]
fn test_create_dry_run_no_existing() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");
    let out_path = work_dir.path().join("opencode.json");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("create")
        .arg("default")
        .arg("github")
        .arg("--dry-run")
        .arg("-o")
        .arg(&out_path)
        .assert()
        // exit 1: changes detected (new file)
        .code(1)
        .stdout(
            predicate::str::contains("--- /dev/null")
                .and(predicate::str::contains("+++ b/"))
                .and(predicate::str::contains("openrouter/openai/gpt-4o")),
        );

    // Output file must NOT exist
    assert!(
        !out_path.exists(),
        "dry-run must not create the output file"
    );
}

#[test]
fn test_create_dry_run_with_existing() {
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
      "model": "old-model",
      "variant": "old-variant"
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
        .arg("create")
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
                .and(predicate::str::contains("-"))
                .and(predicate::str::contains("+")),
        );

    // Existing file must be unchanged
    let after = fs::read_to_string(&out_path).expect("read after");
    assert_eq!(before, after, "dry-run must not modify the existing file");
}

#[test]
fn test_create_dry_run_skips_force_check() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");
    let out_path = work_dir.path().join("opencode.json");

    // Write an existing file — without --force, normal create would fail
    fs::write(&out_path, "existing content").expect("write existing");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("create")
        .arg("default")
        .arg("github")
        .arg("--dry-run")
        .arg("-o")
        .arg(&out_path)
        .assert()
        // Should succeed (exit 1 = diff detected, not an error)
        .code(1);

    // File should still contain original content
    let after = fs::read_to_string(&out_path).expect("read after");
    assert_eq!(
        after, "existing content",
        "dry-run must not modify the existing file"
    );
}

#[test]
fn test_create_dry_run_no_changes() {
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

    // Now dry-run against the identical file — should report no changes
    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("create")
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
fn create_with_directory_template() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());

    // Add a directory template alongside existing file templates.
    // Use "extra" rather than "variant" to test merge, because alias
    // resolution overwrites variant from the palette agent config.
    let template_dir = config_dir.path().join("template.d");
    let dir_template = template_dir.join("dircreate.d");
    fs::create_dir_all(&dir_template).expect("create dircreate.d");
    fs::write(
        dir_template.join("01-base.json"),
        r#"{"agent": {"build": {"model": "{{build}}", "extra": "base-value"}}}"#,
    )
    .expect("write base");
    fs::write(
        dir_template.join("02-override.json"),
        r#"{"agent": {"build": {"extra": "overridden"}}}"#,
    )
    .expect("write override");

    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("create")
        .arg("dircreate")
        .arg("github")
        .assert()
        .success();

    let output_path = work_dir.path().join("opencode.json");
    let data = fs::read_to_string(&output_path).expect("read output");
    let value: Value = serde_json::from_str(&data).expect("parse json");

    assert_eq!(value["agent"]["build"]["model"], "openrouter/openai/gpt-4o");
    assert_eq!(value["agent"]["build"]["extra"], "overridden");
}
