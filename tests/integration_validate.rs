use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::predicate;
use serde_json::Value;
use tempfile::TempDir;

const SAMPLE_YAML: &str = r#"
palettes:
  default:
    agents:
      build:
        model: openrouter/openai/gpt-4o
        variant: mini
"#;

const TEMPLATE_OK: &str = r#"{
  "agent": {
    "build": {
      "model": "{{build}}",
      "variant": "{{build-variant}}"
    }
  }
}
"#;

const TEMPLATE_BAD: &str = r#"{
  "agent": {
    "build": {
      "model": "{{missing}}"
    }
  }
}
"#;

/// Template where `model` is a number instead of a string — violates the
/// JSON Schema which requires `model` to be `"type": "string"`.
const TEMPLATE_SCHEMA_BAD_TYPE: &str = r#"{
  "agent": {
    "build": {
      "model": 42
    }
  }
}
"#;

/// Multi-palette YAML: "default" defines agent "build", "alt" defines agent
/// "deploy".  Used to verify that schema violations are attributed to the
/// correct palette.
const MULTI_PALETTE_YAML: &str = r#"
palettes:
  default:
    agents:
      build:
        model: openrouter/openai/gpt-4o
        variant: mini
  alt:
    agents:
      deploy:
        model: openrouter/anthropic/claude-3.5-sonnet
"#;

fn config_dir_from_home(config_home: &TempDir) -> PathBuf {
    config_home.path().join("opencode-config.d")
}

fn write_config(config_dir: &Path) {
    fs::create_dir_all(config_dir).expect("create config dir");
    fs::write(config_dir.join("model-configs.yaml"), SAMPLE_YAML)
        .expect("write model-configs.yaml");
    let template_dir = config_dir.join("template.d");
    fs::create_dir_all(&template_dir).expect("create template dir");
    fs::write(template_dir.join("ok.json"), TEMPLATE_OK).expect("write ok template");
    fs::write(template_dir.join("bad.json"), TEMPLATE_BAD).expect("write bad template");
}

/// Write config with the standard palette plus a bad-type template for
/// schema validation tests.
fn write_schema_config(config_dir: &Path) {
    write_config(config_dir);
    let template_dir = config_dir.join("template.d");
    fs::write(template_dir.join("bad-type.json"), TEMPLATE_SCHEMA_BAD_TYPE)
        .expect("write bad-type template");
}

#[test]
fn integration_validate_success() {
    let config_home = TempDir::new().expect("config home");
    let config_dir = config_dir_from_home(&config_home);
    write_config(&config_dir);
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .arg("validate")
        .arg("--templates")
        .arg("template.d/ok.json")
        .assert()
        .success();
}

#[test]
fn integration_validate_reports_missing_placeholders() {
    let config_home = TempDir::new().expect("config home");
    let config_dir = config_dir_from_home(&config_home);
    write_config(&config_dir);
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    let assert = cmd
        .current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .arg("validate")
        .arg("--format")
        .arg("json")
        .arg("--templates")
        .arg("template.d/bad.json")
        .assert()
        .failure();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout utf8");
    let report: Value = serde_json::from_str(stdout.trim()).expect("parse json");
    assert!(report["errors"].as_array().is_some());
    assert!(!report["errors"].as_array().unwrap().is_empty());
    assert!(
        report["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["kind"] == "unknown-placeholder")
    );
}

#[test]
fn integration_validate_text_output() {
    let config_home = TempDir::new().expect("config home");
    let config_dir = config_dir_from_home(&config_home);
    write_config(&config_dir);
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .arg("validate")
        .arg("--templates")
        .arg("template.d/bad.json")
        .assert()
        .failure()
        .stdout(predicate::str::contains("unknown placeholder"));
}

// -- schema validation CLI tests ------------------------------------------

/// E1: `--schema` with a valid template produces exit 0 and the JSON report
/// contains no `schema-violation` entries in either `errors` or `warnings`.
#[test]
fn integration_validate_schema_pass() {
    let config_home = TempDir::new().expect("config home");
    let config_dir = config_dir_from_home(&config_home);
    write_schema_config(&config_dir);
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    let assert = cmd
        .current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .arg("validate")
        .arg("--schema")
        .arg("--templates")
        .arg("template.d/ok.json")
        .arg("--format")
        .arg("json")
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout utf8");
    let report: Value = serde_json::from_str(stdout.trim()).expect("parse json");

    let errors = report["errors"].as_array().expect("errors array");
    let warnings = report["warnings"].as_array().expect("warnings array");

    assert!(
        !errors
            .iter()
            .any(|e| e["kind"] == "schema-violation" || e["kind"] == "schema-not-implemented"),
        "valid template should produce no schema errors, got: {stdout}"
    );
    assert!(
        !warnings
            .iter()
            .any(|w| w["kind"] == "schema-violation" || w["kind"] == "schema-not-implemented"),
        "valid template should produce no schema warnings, got: {stdout}"
    );
}

/// E2: `--schema` with invalid data (wrong type) plus `--strict` causes exit
/// non-zero and the JSON `errors` array contains a `schema-violation` entry.
#[test]
fn integration_validate_schema_violation_json() {
    let config_home = TempDir::new().expect("config home");
    let config_dir = config_dir_from_home(&config_home);
    write_schema_config(&config_dir);
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    let assert = cmd
        .current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .arg("validate")
        .arg("-S") // strict: elevate schema violations to errors
        .arg("--schema")
        .arg("--templates")
        .arg("template.d/bad-type.json")
        .arg("--format")
        .arg("json")
        .assert()
        .failure();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout utf8");
    let report: Value = serde_json::from_str(stdout.trim()).expect("parse json");

    let errors = report["errors"].as_array().expect("errors array");
    assert!(
        errors.iter().any(|e| e["kind"] == "schema-violation"),
        "expected schema-violation in errors array, got: {stdout}"
    );

    // Each schema-violation finding must have the standard structure.
    let violation = errors
        .iter()
        .find(|e| e["kind"] == "schema-violation")
        .unwrap();
    assert!(violation.get("file").is_some(), "finding missing 'file'");
    assert!(violation.get("path").is_some(), "finding missing 'path'");
    assert!(
        violation.get("message").is_some(),
        "finding missing 'message'"
    );
}

/// E3: Text output for a schema failure includes schema context (palette
/// name and type-mismatch description).
#[test]
fn integration_validate_schema_text_output() {
    let config_home = TempDir::new().expect("config home");
    let config_dir = config_dir_from_home(&config_home);
    write_schema_config(&config_dir);
    let work_dir = TempDir::new().expect("work dir");

    // Non-strict: schema violations become warnings → exit 0.
    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .arg("validate")
        .arg("--schema")
        .arg("--templates")
        .arg("template.d/bad-type.json")
        .assert()
        .success()
        // Text output includes the palette context prefix from the schema
        // violation message: "[palette: default] …".
        .stdout(predicate::str::contains("[palette:"))
        // The jsonschema error for a type mismatch mentions "string".
        .stdout(predicate::str::contains("string"));
}

/// Without `--schema`, templates with type mismatches do NOT produce any
/// schema-related findings — schema validation is opt-in.
#[test]
fn integration_validate_schema_opt_out() {
    let config_home = TempDir::new().expect("config home");
    let config_dir = config_dir_from_home(&config_home);
    write_schema_config(&config_dir);
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    let assert = cmd
        .current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .arg("validate")
        .arg("--templates")
        .arg("template.d/bad-type.json")
        .arg("--format")
        .arg("json")
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout utf8");
    let report: Value = serde_json::from_str(stdout.trim()).expect("parse json");

    // Collect all findings from both arrays.
    let empty_vec = Vec::new();
    let all_findings: Vec<&Value> = report["errors"]
        .as_array()
        .unwrap_or(&empty_vec)
        .iter()
        .chain(report["warnings"].as_array().unwrap_or(&empty_vec).iter())
        .collect();

    assert!(
        !all_findings
            .iter()
            .any(|f| f["kind"].as_str().unwrap_or("").starts_with("schema-")),
        "without --schema flag, no schema-related findings should appear, got: {stdout}"
    );
}

/// Multi-palette interaction: schema violations reference the correct palette.
/// The "default" palette defines agent "build" (model must be string), so
/// `model: 42` triggers a violation.  The "alt" palette defines "deploy"
/// (not "build"), so no violation is expected for "alt" because "build" is
/// an additional property allowed by `additionalProperties: true`.
#[test]
fn integration_validate_schema_multi_palette() {
    let config_home = TempDir::new().expect("config home");
    let config_dir = config_dir_from_home(&config_home);
    fs::create_dir_all(&config_dir).expect("create config dir");
    fs::write(config_dir.join("model-configs.yaml"), MULTI_PALETTE_YAML)
        .expect("write model-configs.yaml");
    let template_dir = config_dir.join("template.d");
    fs::create_dir_all(&template_dir).expect("create template dir");
    fs::write(template_dir.join("bad-type.json"), TEMPLATE_SCHEMA_BAD_TYPE)
        .expect("write bad-type template");
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    let assert = cmd
        .current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .arg("validate")
        .arg("-S") // strict
        .arg("--schema")
        .arg("--templates")
        .arg("template.d/bad-type.json")
        .arg("--format")
        .arg("json")
        .assert()
        .failure();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout utf8");
    let report: Value = serde_json::from_str(stdout.trim()).expect("parse json");

    let errors = report["errors"].as_array().expect("errors array");
    let violations: Vec<&Value> = errors
        .iter()
        .filter(|e| e["kind"] == "schema-violation")
        .collect();

    assert!(
        !violations.is_empty(),
        "expected at least one schema-violation in multi-palette config, got: {stdout}"
    );

    // Violations should mention the "default" palette which defines "build".
    assert!(
        violations
            .iter()
            .any(|v| v["message"].as_str().unwrap_or("").contains("default")),
        "expected a schema-violation mentioning palette 'default', got: {:?}",
        violations
            .iter()
            .map(|v| v["message"].as_str().unwrap_or(""))
            .collect::<Vec<_>>()
    );

    // No violations should be attributed to the "alt" palette (which does
    // not define "build" — additionalProperties: true allows it).
    assert!(
        violations.iter().all(|v| !v["message"]
            .as_str()
            .unwrap_or("")
            .contains("[palette: alt]")),
        "palette 'alt' should not trigger a schema-violation for agent 'build'"
    );
}

#[test]
fn validate_ambiguous_template_finding() {
    let config_home = TempDir::new().expect("config home");
    let config_dir = config_dir_from_home(&config_home);
    write_config(&config_dir);

    // Create ambiguity: ok.json already exists, add ok.d/
    let template_dir = config_dir.join("template.d");
    let ambiguous_dir = template_dir.join("ok.d");
    fs::create_dir_all(&ambiguous_dir).expect("create ok.d");
    fs::write(ambiguous_dir.join("base.json"), r#"{"agent":{}}"#).expect("write fragment");

    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .arg("validate")
        .arg("--format")
        .arg("json")
        .assert()
        .failure();

    // Re-run and capture output to check for ambiguous finding
    let mut cmd = cargo_bin_cmd!("opencode-config");
    let output = cmd
        .current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .arg("validate")
        .arg("--format")
        .arg("json")
        .output()
        .expect("run validate");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("ambiguous-template"),
        "expected ambiguous-template finding in output: {stdout}"
    );
}

#[test]
fn validate_empty_template_dir_finding() {
    let config_home = TempDir::new().expect("config home");
    let config_dir = config_dir_from_home(&config_home);
    write_config(&config_dir);

    // Create empty template directory
    let template_dir = config_dir.join("template.d");
    let empty_dir = template_dir.join("empty.d");
    fs::create_dir_all(&empty_dir).expect("create empty.d");

    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    let output = cmd
        .current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .arg("validate")
        .arg("--format")
        .arg("json")
        .output()
        .expect("run validate");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("empty-template-dir"),
        "expected empty-template-dir finding in output: {stdout}"
    );
}

// -- ambiguous-fragment tests ---------------------------------------------

/// Non-strict mode: a template directory containing fragments with the same
/// stem but different extensions (e.g. `base.json` + `base.yaml`) should
/// produce an `ambiguous-fragment` **warning** (exit 0) in JSON output.
#[test]
fn validate_ambiguous_fragment_warning_json() {
    let config_home = TempDir::new().expect("config home");
    let config_dir = config_dir_from_home(&config_home);
    fs::create_dir_all(&config_dir).expect("create config dir");
    fs::write(config_dir.join("model-configs.yaml"), SAMPLE_YAML).expect("write yaml");

    let template_dir = config_dir.join("template.d");
    let frag_dir = template_dir.join("ambig.d");
    fs::create_dir_all(&frag_dir).expect("create ambig.d");
    // Same stem "base", different extensions — triggers ambiguous-fragment
    fs::write(
        frag_dir.join("base.json"),
        r#"{"agent": {"build": {"model": "{{build}}"}}}"#,
    )
    .expect("write base.json");
    fs::write(
        frag_dir.join("base.yaml"),
        "agent:\n  build:\n    model: \"{{build}}\"",
    )
    .expect("write base.yaml");

    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    let assert = cmd
        .current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .arg("validate")
        .arg("--format")
        .arg("json")
        .assert()
        .success(); // warnings only → exit 0

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout utf8");
    let report: Value = serde_json::from_str(stdout.trim()).expect("parse json");

    let warnings = report["warnings"].as_array().expect("warnings array");
    assert!(
        warnings.iter().any(|w| w["kind"] == "ambiguous-fragment"),
        "expected ambiguous-fragment warning in output: {stdout}"
    );

    // The finding should mention the stem name
    let finding = warnings
        .iter()
        .find(|w| w["kind"] == "ambiguous-fragment")
        .unwrap();
    assert!(
        finding["message"].as_str().unwrap_or("").contains("base"),
        "ambiguous-fragment message should mention stem 'base': {:?}",
        finding["message"]
    );
}

/// Text output for `ambiguous-fragment` should reference the finding kind
/// and the conflicting stem.
#[test]
fn validate_ambiguous_fragment_warning_text() {
    let config_home = TempDir::new().expect("config home");
    let config_dir = config_dir_from_home(&config_home);
    fs::create_dir_all(&config_dir).expect("create config dir");
    fs::write(config_dir.join("model-configs.yaml"), SAMPLE_YAML).expect("write yaml");

    let template_dir = config_dir.join("template.d");
    let frag_dir = template_dir.join("ambig.d");
    fs::create_dir_all(&frag_dir).expect("create ambig.d");
    fs::write(
        frag_dir.join("base.json"),
        r#"{"agent": {"build": {"model": "{{build}}"}}}"#,
    )
    .expect("write base.json");
    fs::write(
        frag_dir.join("base.yaml"),
        "agent:\n  build:\n    model: \"{{build}}\"",
    )
    .expect("write base.yaml");

    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .arg("validate")
        .assert()
        .success() // non-strict → warnings only → exit 0
        .stdout(predicate::str::contains("base"))
        .stdout(predicate::str::contains("WARNING"));

    // Verify finding kind string via JSON format
    let mut cmd = cargo_bin_cmd!("opencode-config");
    let output = cmd
        .current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .arg("validate")
        .arg("--format")
        .arg("json")
        .output()
        .expect("run validate json");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("ambiguous-fragment"),
        "expected finding kind 'ambiguous-fragment' in output: {stdout}"
    );
}

// -- fragment-merge-conflict tests ----------------------------------------

/// Non-strict mode: ordered fragments where a later fragment overwrites an
/// existing scalar with a different value should produce a
/// `fragment-merge-conflict` **warning** (exit 0) in JSON output.
#[test]
fn validate_fragment_merge_conflict_warning_json() {
    let config_home = TempDir::new().expect("config home");
    let config_dir = config_dir_from_home(&config_home);
    fs::create_dir_all(&config_dir).expect("create config dir");
    fs::write(config_dir.join("model-configs.yaml"), SAMPLE_YAML).expect("write yaml");

    let template_dir = config_dir.join("template.d");
    let frag_dir = template_dir.join("conflict.d");
    fs::create_dir_all(&frag_dir).expect("create conflict.d");
    // 01-base sets agent.build.model to one value
    fs::write(
        frag_dir.join("01-base.json"),
        r#"{"agent": {"build": {"model": "gpt-4"}}}"#,
    )
    .expect("write 01-base.json");
    // 02-overlay overwrites agent.build.model with a *different* value
    fs::write(
        frag_dir.join("02-overlay.json"),
        r#"{"agent": {"build": {"model": "gpt-5"}}}"#,
    )
    .expect("write 02-overlay.json");

    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    let assert = cmd
        .current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .arg("validate")
        .arg("--format")
        .arg("json")
        .assert()
        .success(); // warnings only → exit 0

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout utf8");
    let report: Value = serde_json::from_str(stdout.trim()).expect("parse json");

    let warnings = report["warnings"].as_array().expect("warnings array");
    assert!(
        warnings
            .iter()
            .any(|w| w["kind"] == "fragment-merge-conflict"),
        "expected fragment-merge-conflict warning in output: {stdout}"
    );

    // The finding should reference the conflicting fragment name and path
    let finding = warnings
        .iter()
        .find(|w| w["kind"] == "fragment-merge-conflict")
        .unwrap();
    assert!(
        finding["message"]
            .as_str()
            .unwrap_or("")
            .contains("02-overlay.json"),
        "fragment-merge-conflict message should mention the overlay fragment: {:?}",
        finding["message"]
    );
    assert!(
        finding["path"].as_str().unwrap_or("").contains("model"),
        "fragment-merge-conflict path should reference the conflicting key: {:?}",
        finding["path"]
    );
}

/// Text output for `fragment-merge-conflict` should reference the finding
/// and the overwriting fragment.
#[test]
fn validate_fragment_merge_conflict_warning_text() {
    let config_home = TempDir::new().expect("config home");
    let config_dir = config_dir_from_home(&config_home);
    fs::create_dir_all(&config_dir).expect("create config dir");
    fs::write(config_dir.join("model-configs.yaml"), SAMPLE_YAML).expect("write yaml");

    let template_dir = config_dir.join("template.d");
    let frag_dir = template_dir.join("conflict.d");
    fs::create_dir_all(&frag_dir).expect("create conflict.d");
    fs::write(
        frag_dir.join("01-base.json"),
        r#"{"agent": {"build": {"model": "gpt-4"}}}"#,
    )
    .expect("write 01-base.json");
    fs::write(
        frag_dir.join("02-overlay.json"),
        r#"{"agent": {"build": {"model": "gpt-5"}}}"#,
    )
    .expect("write 02-overlay.json");

    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .arg("validate")
        .assert()
        .success() // non-strict → warnings only → exit 0
        .stdout(predicate::str::contains("overwrites"))
        .stdout(predicate::str::contains("02-overlay.json"))
        .stdout(predicate::str::contains("WARNING"));

    // Verify finding kind string via JSON format
    let mut cmd = cargo_bin_cmd!("opencode-config");
    let output = cmd
        .current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .arg("validate")
        .arg("--format")
        .arg("json")
        .output()
        .expect("run validate json");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("fragment-merge-conflict"),
        "expected finding kind 'fragment-merge-conflict' in output: {stdout}"
    );
}

#[test]
fn validate_directory_template_succeeds() {
    let config_home = TempDir::new().expect("config home");
    let config_dir = config_dir_from_home(&config_home);
    fs::create_dir_all(&config_dir).expect("create config dir");
    fs::write(config_dir.join("model-configs.yaml"), SAMPLE_YAML).expect("write yaml");

    let template_dir = config_dir.join("template.d");
    fs::create_dir_all(&template_dir).expect("create template dir");

    // Only directory template, no file templates
    let dir_template = template_dir.join("mydir.d");
    fs::create_dir_all(&dir_template).expect("create mydir.d");
    fs::write(
        dir_template.join("01-base.json"),
        r#"{"agent": {"build": {"model": "{{build}}"}}}"#,
    )
    .expect("write base");

    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .arg("validate")
        .assert()
        .success();
}
