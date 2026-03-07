use std::fs;
use std::path::Path;

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::{PredicateBooleanExt, predicate};
use serde_json::Value;
use tempfile::TempDir;

const SAMPLE_YAML: &str = r#"
palettes:
  github:
    agents:
      build:
        model: openrouter/openai/gpt-4o
      review:
        model: openrouter/openai/gpt-4o
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
    "version": "1.0"
  },
  "description": "Default template"
}"#;

const CUSTOM_TEMPLATE: &str = r#"{
  "agent": {
    "build": {
      "model": "{{build}}"
    }
  },
  "description": "Custom template"
}"#;

fn write_config(config_dir: &Path) {
    fs::write(config_dir.join("model-configs.yaml"), SAMPLE_YAML)
        .expect("write model-configs.yaml");

    let template_dir = config_dir.join("template.d");
    fs::create_dir_all(&template_dir).expect("create template dir");
    fs::write(template_dir.join("default.json"), DEFAULT_TEMPLATE).expect("write default template");
    fs::write(template_dir.join("custom.json"), CUSTOM_TEMPLATE).expect("write custom template");
}

#[test]
fn decompose_default_mapping() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("decompose")
        .arg("default")
        .assert()
        .success();

    let template_dir = config_dir.path().join("template.d");
    let frag_dir = template_dir.join("default.d");
    assert!(frag_dir.exists(), "fragment directory should exist");

    // Global fragment: _default.json
    let global_path = frag_dir.join("_default.json");
    assert!(global_path.exists(), "global fragment should exist");
    let global: Value =
        serde_json::from_str(&fs::read_to_string(&global_path).expect("read global"))
            .expect("parse global");
    assert_eq!(global["agent"], serde_json::json!({}));
    assert_eq!(global["meta"]["version"], "1.0");
    assert_eq!(global["description"], "Default template");

    // Agent fragments
    let build_path = frag_dir.join("build.json");
    assert!(build_path.exists(), "build fragment should exist");
    let build: Value = serde_json::from_str(&fs::read_to_string(&build_path).expect("read build"))
        .expect("parse build");
    assert_eq!(build["agent"]["build"]["model"], "{{build}}");
    assert_eq!(build["agent"]["build"]["variant"], "{{build-variant}}");
    assert_eq!(
        build.as_object().unwrap().len(),
        1,
        "agent fragment should only have 'agent' key"
    );

    let review_path = frag_dir.join("review.json");
    assert!(review_path.exists(), "review fragment should exist");
    let review: Value =
        serde_json::from_str(&fs::read_to_string(&review_path).expect("read review"))
            .expect("parse review");
    assert_eq!(review["agent"]["review"]["model"], "{{review}}");
    assert_eq!(review["agent"]["review"]["variant"], "{{review-variant}}");

    // Original file should be removed
    assert!(
        !template_dir.join("default.json").exists(),
        "original template should be removed"
    );
}

#[test]
fn decompose_dry_run() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("decompose")
        .arg("default")
        .arg("--dry-run")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("[DRY-RUN]")
                .and(predicate::str::contains("_default.json"))
                .and(predicate::str::contains("build.json"))
                .and(predicate::str::contains("review.json"))
                .and(predicate::str::contains("1 global + 2 agents")),
        );

    let template_dir = config_dir.path().join("template.d");

    // No side-effects: original still exists, no fragment dir
    assert!(
        template_dir.join("default.json").exists(),
        "original must still exist after dry-run"
    );
    assert!(
        !template_dir.join("default.d").exists(),
        "fragment dir must not exist after dry-run"
    );
}

#[test]
fn decompose_backup() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("decompose")
        .arg("default")
        .assert()
        .success();

    let template_dir = config_dir.path().join("template.d");

    // Original should be removed
    assert!(
        !template_dir.join("default.json").exists(),
        "original should be removed after decompose"
    );

    // Backup should exist with pattern default.json.bak.<timestamp>
    let backup_entries: Vec<_> = fs::read_dir(&template_dir)
        .expect("read template dir")
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("default.json.bak."))
        })
        .collect();
    assert_eq!(
        backup_entries.len(),
        1,
        "exactly one backup file should exist"
    );

    // Backup content should match original template
    let backup_path = backup_entries[0].path();
    let backup_content = fs::read_to_string(&backup_path).expect("read backup");
    let backup_value: Value = serde_json::from_str(&backup_content).expect("parse backup");
    let original_value: Value = serde_json::from_str(DEFAULT_TEMPLATE).expect("parse original");
    assert_eq!(backup_value, original_value, "backup should match original");
}

#[test]
fn decompose_verify() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("decompose")
        .arg("default")
        .arg("--verify")
        .assert()
        .success();

    // Fragments should exist and be valid
    let frag_dir = config_dir.path().join("template.d").join("default.d");
    assert!(frag_dir.join("_default.json").exists());
    assert!(frag_dir.join("build.json").exists());
    assert!(frag_dir.join("review.json").exists());
}

#[test]
fn decompose_force() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    let template_dir = config_dir.path().join("template.d");
    let target_dir = template_dir.join("default.d");
    fs::create_dir_all(&target_dir).expect("create target dir");
    fs::write(target_dir.join("old.json"), "{}").expect("write old fragment");

    // Without --force, should fail
    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("decompose")
        .arg("default")
        .assert()
        .failure()
        .stderr(predicate::str::contains("target directory already exists"));

    // With --force, should succeed
    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("decompose")
        .arg("default")
        .arg("--force")
        .assert()
        .success();

    // New fragments should exist
    assert!(target_dir.join("_default.json").exists());
    assert!(target_dir.join("build.json").exists());
    assert!(target_dir.join("review.json").exists());
    // Old fragment should be gone
    assert!(
        !target_dir.join("old.json").exists(),
        "old fragment should be removed after force"
    );
}

#[test]
fn decompose_custom_mapping() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("decompose")
        .arg("custom")
        .assert()
        .success();

    let template_dir = config_dir.path().join("template.d");
    let frag_dir = template_dir.join("custom.d");
    assert!(frag_dir.exists(), "custom fragment directory should exist");

    // Global fragment: _custom.json
    let global_path = frag_dir.join("_custom.json");
    assert!(global_path.exists(), "custom global fragment should exist");
    let global: Value =
        serde_json::from_str(&fs::read_to_string(&global_path).expect("read global"))
            .expect("parse global");
    assert_eq!(global["agent"], serde_json::json!({}));
    assert_eq!(global["description"], "Custom template");

    // Agent fragment: build.json only (custom has one agent)
    let build_path = frag_dir.join("build.json");
    assert!(build_path.exists(), "build fragment should exist");
    let build: Value = serde_json::from_str(&fs::read_to_string(&build_path).expect("read build"))
        .expect("parse build");
    assert_eq!(build["agent"]["build"]["model"], "{{build}}");

    // No review fragment for custom template
    assert!(
        !frag_dir.join("review.json").exists(),
        "custom template should not have review fragment"
    );

    // Original should be removed
    assert!(
        !template_dir.join("custom.json").exists(),
        "original custom template should be removed"
    );
}

#[test]
fn decompose_error_missing_template() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("decompose")
        .arg("nonexistent")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "template error: failed to read template at",
        ));
}

#[test]
fn decompose_error_already_decomposed() {
    let config_dir = TempDir::new().expect("config dir");
    write_config(config_dir.path());
    let work_dir = TempDir::new().expect("work dir");

    // Remove the file and create a directory template instead
    let template_dir = config_dir.path().join("template.d");
    fs::remove_file(template_dir.join("default.json")).expect("remove default.json");
    let frag_dir = template_dir.join("default.d");
    fs::create_dir_all(&frag_dir).expect("create fragment dir");
    fs::write(frag_dir.join("_default.json"), r#"{"agent": {}}"#).expect("write global fragment");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(config_dir.path())
        .arg("decompose")
        .arg("default")
        .assert()
        .failure()
        .stderr(predicate::str::contains("template is not a single file"));
}

#[test]
fn decompose_error_missing_config() {
    let work_dir = TempDir::new().expect("work dir");
    let nonexistent = work_dir.path().join("no-such-config");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("--config")
        .arg(&nonexistent)
        .arg("decompose")
        .arg("default")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "template error: failed to read template at",
        ));
}
