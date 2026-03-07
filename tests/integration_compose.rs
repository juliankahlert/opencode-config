use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::{PredicateBooleanExt, predicate};
use serde_json::Value;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Path to the non-conflicting fixture fragments shipped with the repo.
fn fixture_fragments_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("fragments")
}

/// Path to the conflicting fixture fragments shipped with the repo.
fn fixture_conflicting_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("conflicting-fragments")
}

/// Copy every file from `src_dir` into `dst_dir` (flat copy, no recursion).
fn copy_fragments(src_dir: &Path, dst_dir: &Path) {
    fs::create_dir_all(dst_dir).expect("create fragment dir");
    for entry in fs::read_dir(src_dir).expect("read fixture dir") {
        let entry = entry.expect("dir entry");
        if entry.path().is_file() {
            fs::copy(entry.path(), dst_dir.join(entry.file_name())).expect("copy fragment");
        }
    }
}

/// Build a command pre-configured with a temp working dir and isolated
/// XDG config so it never touches real user config.
fn compose_cmd(work_dir: &Path, xdg: &Path) -> assert_cmd::Command {
    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir)
        .env("XDG_CONFIG_HOME", xdg)
        .arg("compose");
    cmd
}

// ---------------------------------------------------------------------------
// 1. Compose fragments merge correctly
// ---------------------------------------------------------------------------

#[test]
fn compose_merges_fragments_correctly() {
    let work_dir = TempDir::new().expect("work dir");
    let xdg = TempDir::new().expect("xdg dir");
    let frag_dir = work_dir.path().join("frags");
    copy_fragments(&fixture_fragments_dir(), &frag_dir);

    let out = work_dir.path().join("opencode.json");

    let mut cmd = compose_cmd(work_dir.path(), xdg.path());
    cmd.arg(&frag_dir).arg("-o").arg(&out).assert().success();

    let data = fs::read_to_string(&out).expect("read output");
    let value: Value = serde_json::from_str(&data).expect("parse json");

    // Keys from 01-base.json
    assert_eq!(value["$schema"], "https://opencode.ai/config.json");
    assert_eq!(value["default_agent"], "interactive");
    assert_eq!(value["permission"]["todowrite"], "allow");

    // Keys from 02-agents.json
    assert_eq!(value["agent"]["interactive"]["description"], "Orchestrator");
    assert_eq!(value["agent"]["explore"]["description"], "Explorer");
    assert_eq!(
        value["agent"]["coder"]["description"],
        "Implementation specialist"
    );

    // Keys from 03-tools.json (deep-merged into agents)
    assert_eq!(value["agent"]["interactive"]["tools"]["task"], true);
    assert_eq!(value["agent"]["explore"]["tools"]["grep"], true);
    assert_eq!(value["agent"]["coder"]["tools"]["bash"], true);
}

// ---------------------------------------------------------------------------
// 2. Dry-run behaviour
// ---------------------------------------------------------------------------

#[test]
fn compose_dry_run_shows_diff_when_output_missing() {
    let work_dir = TempDir::new().expect("work dir");
    let xdg = TempDir::new().expect("xdg dir");
    let frag_dir = work_dir.path().join("frags");
    copy_fragments(&fixture_fragments_dir(), &frag_dir);

    let out = work_dir.path().join("opencode.json");

    let mut cmd = compose_cmd(work_dir.path(), xdg.path());
    cmd.arg(&frag_dir)
        .arg("-o")
        .arg(&out)
        .arg("--dry-run")
        .assert()
        .code(1)
        .stdout(
            predicate::str::contains("--- /dev/null")
                .and(predicate::str::contains("+++ b/"))
                .and(predicate::str::contains("opencode.ai")),
        );

    assert!(!out.exists(), "dry-run must not create the output file");
}

#[test]
fn compose_dry_run_exit_zero_when_unchanged() {
    let work_dir = TempDir::new().expect("work dir");
    let xdg = TempDir::new().expect("xdg dir");
    let frag_dir = work_dir.path().join("frags");
    copy_fragments(&fixture_fragments_dir(), &frag_dir);

    let out = work_dir.path().join("opencode.json");

    // First, write the file normally
    let mut cmd = compose_cmd(work_dir.path(), xdg.path());
    cmd.arg(&frag_dir).arg("-o").arg(&out).assert().success();

    let before = fs::read_to_string(&out).expect("read before");

    // Now dry-run against the identical file
    let mut cmd = compose_cmd(work_dir.path(), xdg.path());
    cmd.arg(&frag_dir)
        .arg("-o")
        .arg(&out)
        .arg("--dry-run")
        .assert()
        .success()
        .stdout(predicate::str::contains("No changes"));

    let after = fs::read_to_string(&out).expect("read after");
    assert_eq!(before, after, "dry-run must not modify the file");
}

// ---------------------------------------------------------------------------
// 3. Backup behaviour
// ---------------------------------------------------------------------------

#[test]
fn compose_backup_creates_bak_file() {
    let work_dir = TempDir::new().expect("work dir");
    let xdg = TempDir::new().expect("xdg dir");
    let frag_dir = work_dir.path().join("frags");
    copy_fragments(&fixture_fragments_dir(), &frag_dir);

    let out = work_dir.path().join("opencode.json");

    // Write an existing file to be backed up
    let old_content = r#"{"old": "content"}"#;
    fs::write(&out, old_content).expect("write existing");

    let mut cmd = compose_cmd(work_dir.path(), xdg.path());
    cmd.arg(&frag_dir)
        .arg("-o")
        .arg(&out)
        .arg("--force")
        .arg("--backup")
        .assert()
        .success();

    // Find backup file
    let backup_entries: Vec<_> = fs::read_dir(work_dir.path())
        .expect("read work dir")
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("opencode.json.bak."))
        })
        .collect();
    assert_eq!(
        backup_entries.len(),
        1,
        "exactly one backup file should exist"
    );

    // Backup should contain old content
    let backup_content = fs::read_to_string(backup_entries[0].path()).expect("read backup");
    assert_eq!(backup_content, old_content);

    // New output should be different from old
    let new_content = fs::read_to_string(&out).expect("read output");
    let parsed: Value = serde_json::from_str(&new_content).expect("parse new output");
    assert!(
        parsed.get("$schema").is_some(),
        "new output should have merged fragment content"
    );
}

// ---------------------------------------------------------------------------
// 4. Verify behaviour
// ---------------------------------------------------------------------------

#[test]
fn compose_verify_succeeds_on_normal_write() {
    let work_dir = TempDir::new().expect("work dir");
    let xdg = TempDir::new().expect("xdg dir");
    let frag_dir = work_dir.path().join("frags");
    copy_fragments(&fixture_fragments_dir(), &frag_dir);

    let out = work_dir.path().join("opencode.json");

    let mut cmd = compose_cmd(work_dir.path(), xdg.path());
    cmd.arg(&frag_dir)
        .arg("-o")
        .arg(&out)
        .arg("--verify")
        .assert()
        .success();

    assert!(out.exists(), "output file should exist after verify");
    let data = fs::read_to_string(&out).expect("read output");
    let _: Value = serde_json::from_str(&data).expect("output should be valid JSON");
}

// ---------------------------------------------------------------------------
// 5. Force overwrite behaviour
// ---------------------------------------------------------------------------

#[test]
fn compose_fails_when_output_exists_without_force() {
    let work_dir = TempDir::new().expect("work dir");
    let xdg = TempDir::new().expect("xdg dir");
    let frag_dir = work_dir.path().join("frags");
    copy_fragments(&fixture_fragments_dir(), &frag_dir);

    let out = work_dir.path().join("opencode.json");
    fs::write(&out, "existing").expect("write existing");

    let mut cmd = compose_cmd(work_dir.path(), xdg.path());
    cmd.arg(&frag_dir)
        .arg("-o")
        .arg(&out)
        .assert()
        .failure()
        .stderr(predicate::str::contains("output already exists"));
}

#[test]
fn compose_succeeds_with_force_overwrite() {
    let work_dir = TempDir::new().expect("work dir");
    let xdg = TempDir::new().expect("xdg dir");
    let frag_dir = work_dir.path().join("frags");
    copy_fragments(&fixture_fragments_dir(), &frag_dir);

    let out = work_dir.path().join("opencode.json");
    fs::write(&out, "existing").expect("write existing");

    let mut cmd = compose_cmd(work_dir.path(), xdg.path());
    cmd.arg(&frag_dir)
        .arg("-o")
        .arg(&out)
        .arg("--force")
        .assert()
        .success();

    let data = fs::read_to_string(&out).expect("read output");
    assert!(data.contains("opencode.ai"), "output should be overwritten");
}

// ---------------------------------------------------------------------------
// 6. Conflict handling
// ---------------------------------------------------------------------------

#[test]
fn compose_conflict_error_fails_and_mentions_path() {
    let work_dir = TempDir::new().expect("work dir");
    let xdg = TempDir::new().expect("xdg dir");
    let frag_dir = work_dir.path().join("frags");
    copy_fragments(&fixture_conflicting_dir(), &frag_dir);

    let out = work_dir.path().join("opencode.json");

    let mut cmd = compose_cmd(work_dir.path(), xdg.path());
    cmd.arg(&frag_dir)
        .arg("-o")
        .arg(&out)
        .arg("--conflict")
        .arg("error")
        .assert()
        .failure()
        .stderr(predicate::str::contains("conflict at path"));
}

#[test]
fn compose_conflict_last_wins_succeeds() {
    let work_dir = TempDir::new().expect("work dir");
    let xdg = TempDir::new().expect("xdg dir");
    let frag_dir = work_dir.path().join("frags");
    copy_fragments(&fixture_conflicting_dir(), &frag_dir);

    let out = work_dir.path().join("opencode.json");

    let mut cmd = compose_cmd(work_dir.path(), xdg.path());
    cmd.arg(&frag_dir)
        .arg("-o")
        .arg(&out)
        .arg("--conflict")
        .arg("last-wins")
        .assert()
        .success();

    let data = fs::read_to_string(&out).expect("read output");
    let value: Value = serde_json::from_str(&data).expect("parse json");

    // 02-conflict.json overrides: build.tools = "none", metadata.version = "two",
    // metadata.tags = "experimental"
    assert_eq!(value["agent"]["build"]["tools"], "none");
    assert_eq!(value["metadata"]["version"], "two");
    assert_eq!(value["metadata"]["tags"], "experimental");
}

// ---------------------------------------------------------------------------
// 7. Missing fragments behaviour
// ---------------------------------------------------------------------------

#[test]
fn compose_missing_input_dir_fails() {
    let work_dir = TempDir::new().expect("work dir");
    let xdg = TempDir::new().expect("xdg dir");

    let mut cmd = compose_cmd(work_dir.path(), xdg.path());
    cmd.arg(work_dir.path().join("nonexistent"))
        .assert()
        .failure()
        .stderr(predicate::str::contains("input directory not found"));
}

#[test]
fn compose_empty_dir_fails() {
    let work_dir = TempDir::new().expect("work dir");
    let xdg = TempDir::new().expect("xdg dir");
    let empty_dir = work_dir.path().join("empty");
    fs::create_dir_all(&empty_dir).expect("create empty dir");

    let mut cmd = compose_cmd(work_dir.path(), xdg.path());
    cmd.arg(&empty_dir)
        .assert()
        .failure()
        .stderr(predicate::str::contains("no template fragments found"));
}

// ---------------------------------------------------------------------------
// 8. TTY conflict fallback behaviour
// ---------------------------------------------------------------------------

#[test]
fn compose_interactive_fallback_to_last_wins_non_tty() {
    let work_dir = TempDir::new().expect("work dir");
    let xdg = TempDir::new().expect("xdg dir");
    let frag_dir = work_dir.path().join("frags");
    copy_fragments(&fixture_conflicting_dir(), &frag_dir);

    let out = work_dir.path().join("opencode.json");

    // Enable verbose logging so the fallback warning is emitted to stderr
    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .env("RUST_LOG", "warn")
        .arg("--verbose")
        .arg("compose")
        .arg(&frag_dir)
        .arg("-o")
        .arg(&out)
        .arg("--conflict")
        .arg("interactive")
        .assert()
        .success()
        .stderr(predicate::str::contains("non-interactive terminal"));

    // Should have resolved like last-wins
    let data = fs::read_to_string(&out).expect("read output");
    let value: Value = serde_json::from_str(&data).expect("parse json");
    assert_eq!(value["agent"]["build"]["tools"], "none");
}

// ---------------------------------------------------------------------------
// 9. Pretty / minify output behaviour
// ---------------------------------------------------------------------------

#[test]
fn compose_pretty_output_has_indentation() {
    let work_dir = TempDir::new().expect("work dir");
    let xdg = TempDir::new().expect("xdg dir");
    let frag_dir = work_dir.path().join("frags");
    copy_fragments(&fixture_fragments_dir(), &frag_dir);

    let out = work_dir.path().join("opencode.json");

    let mut cmd = compose_cmd(work_dir.path(), xdg.path());
    cmd.arg(&frag_dir)
        .arg("-o")
        .arg(&out)
        .arg("--pretty")
        .assert()
        .success();

    let data = fs::read_to_string(&out).expect("read output");
    assert!(data.contains('\n'), "pretty output should contain newlines");
    assert!(
        data.contains("  "),
        "pretty output should contain indentation"
    );
    assert!(
        data.ends_with('\n'),
        "pretty output should end with trailing newline"
    );
}

#[test]
fn compose_minify_output_is_compact() {
    let work_dir = TempDir::new().expect("work dir");
    let xdg = TempDir::new().expect("xdg dir");
    let frag_dir = work_dir.path().join("frags");
    copy_fragments(&fixture_fragments_dir(), &frag_dir);

    let out = work_dir.path().join("opencode.json");

    let mut cmd = compose_cmd(work_dir.path(), xdg.path());
    cmd.arg(&frag_dir)
        .arg("-o")
        .arg(&out)
        .arg("--minify")
        .assert()
        .success();

    let data = fs::read_to_string(&out).expect("read output");
    assert!(
        !data.contains('\n'),
        "minified output must not contain newlines"
    );
    assert!(
        !data.contains("  "),
        "minified output must not contain indentation"
    );
}

// ---------------------------------------------------------------------------
// 10. Template-name resolution for compose
// ---------------------------------------------------------------------------

/// Build a command that uses `--config` to point at a custom config directory.
fn config_compose_cmd(work_dir: &Path, config_dir: &Path) -> assert_cmd::Command {
    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir)
        .arg("--config")
        .arg(config_dir)
        .arg("compose");
    cmd
}

#[test]
fn compose_resolves_template_name() {
    let work_dir = TempDir::new().expect("work dir");
    let config_dir = TempDir::new().expect("config dir");

    // Set up template.d/<name>.d/ with fixture fragments
    let template_d = config_dir.path().join("template.d").join("mytemplate.d");
    copy_fragments(&fixture_fragments_dir(), &template_d);

    let out = work_dir.path().join("opencode.json");

    let mut cmd = config_compose_cmd(work_dir.path(), config_dir.path());
    cmd.arg("mytemplate").arg("-o").arg(&out).assert().success();

    let data = fs::read_to_string(&out).expect("read output");
    let value: Value = serde_json::from_str(&data).expect("parse json");

    // Verify content came from the fixture fragments
    assert_eq!(value["$schema"], "https://opencode.ai/config.json");
    assert_eq!(value["agent"]["interactive"]["description"], "Orchestrator");
}

#[test]
fn compose_rejects_file_template() {
    let work_dir = TempDir::new().expect("work dir");
    let config_dir = TempDir::new().expect("config dir");

    // Set up template.d/<name>.json (file template, not a directory)
    let template_d = config_dir.path().join("template.d");
    fs::create_dir_all(&template_d).expect("create template.d");
    fs::write(
        template_d.join("fileonly.json"),
        r#"{"agent": {"build": {"model": "gpt-4"}}}"#,
    )
    .expect("write file template");

    let out = work_dir.path().join("opencode.json");

    let mut cmd = config_compose_cmd(work_dir.path(), config_dir.path());
    cmd.arg("fileonly")
        .arg("-o")
        .arg(&out)
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "resolved to a file, not a fragment directory",
        ));
}

#[test]
fn compose_literal_dir_still_works() {
    let work_dir = TempDir::new().expect("work dir");
    let xdg = TempDir::new().expect("xdg dir");
    let frag_dir = work_dir.path().join("my-fragments");
    copy_fragments(&fixture_fragments_dir(), &frag_dir);

    let out = work_dir.path().join("opencode.json");

    let mut cmd = compose_cmd(work_dir.path(), xdg.path());
    cmd.arg(&frag_dir).arg("-o").arg(&out).assert().success();

    let data = fs::read_to_string(&out).expect("read output");
    let value: Value = serde_json::from_str(&data).expect("parse json");

    assert_eq!(value["$schema"], "https://opencode.ai/config.json");
    assert_eq!(
        value["agent"]["coder"]["description"],
        "Implementation specialist"
    );
}

// ---------------------------------------------------------------------------
// 11. Round-trip decompose → compose tests and derived output path tests
// ---------------------------------------------------------------------------

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

const SAMPLE_YAML: &str = r#"
palettes:
  github:
    agents:
      build:
        model: openrouter/openai/gpt-4o
      review:
        model: openrouter/openai/gpt-4o
"#;

/// Write model-configs.yaml plus two file templates into a config directory.
fn write_roundtrip_config(config_dir: &Path) {
    fs::write(config_dir.join("model-configs.yaml"), SAMPLE_YAML)
        .expect("write model-configs.yaml");

    let template_dir = config_dir.join("template.d");
    fs::create_dir_all(&template_dir).expect("create template dir");
    fs::write(template_dir.join("default.json"), DEFAULT_TEMPLATE).expect("write default template");
    fs::write(template_dir.join("custom.json"), CUSTOM_TEMPLATE).expect("write custom template");
}

/// Build a decompose command pointing at a custom config directory.
fn decompose_cmd(work_dir: &Path, config_dir: &Path) -> assert_cmd::Command {
    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir)
        .arg("--config")
        .arg(config_dir)
        .arg("decompose");
    cmd
}

#[test]
fn roundtrip_decompose_compose_default() {
    let config_dir = TempDir::new().expect("config dir");
    let work_dir = TempDir::new().expect("work dir");
    write_roundtrip_config(config_dir.path());

    let original: Value = serde_json::from_str(DEFAULT_TEMPLATE).expect("parse original");

    // Decompose: default.json → default.d/
    decompose_cmd(work_dir.path(), config_dir.path())
        .arg("default")
        .assert()
        .success();

    // Compose: default.d/ → default.json (derived output)
    config_compose_cmd(work_dir.path(), config_dir.path())
        .arg("default")
        .assert()
        .success();

    let out_path = config_dir.path().join("template.d").join("default.json");
    assert!(out_path.exists(), "derived output should exist");

    let data = fs::read_to_string(&out_path).expect("read composed");
    let composed: Value = serde_json::from_str(&data).expect("parse composed");
    assert_eq!(original, composed, "round-trip must preserve JSON value");
}

#[test]
fn roundtrip_decompose_compose_custom() {
    let config_dir = TempDir::new().expect("config dir");
    let work_dir = TempDir::new().expect("work dir");
    write_roundtrip_config(config_dir.path());

    let original: Value = serde_json::from_str(CUSTOM_TEMPLATE).expect("parse original");

    // Decompose
    decompose_cmd(work_dir.path(), config_dir.path())
        .arg("custom")
        .assert()
        .success();

    // Compose back
    config_compose_cmd(work_dir.path(), config_dir.path())
        .arg("custom")
        .assert()
        .success();

    let out_path = config_dir.path().join("template.d").join("custom.json");
    assert!(out_path.exists(), "derived output should exist");

    let data = fs::read_to_string(&out_path).expect("read composed");
    let composed: Value = serde_json::from_str(&data).expect("parse composed");
    assert_eq!(original, composed, "round-trip must preserve JSON value");
}

#[test]
fn compose_template_name_derives_output() {
    let config_dir = TempDir::new().expect("config dir");
    let work_dir = TempDir::new().expect("work dir");

    // Set up template.d/myname.d/ with fixture fragments
    let template_d = config_dir.path().join("template.d").join("myname.d");
    copy_fragments(&fixture_fragments_dir(), &template_d);

    // Compose with template name only — no -o flag
    config_compose_cmd(work_dir.path(), config_dir.path())
        .arg("myname")
        .assert()
        .success();

    // Output should land at config_dir/template.d/myname.json
    let derived = config_dir.path().join("template.d").join("myname.json");
    assert!(derived.exists(), "derived output path should be created");

    // CWD should NOT get an opencode.json
    assert!(
        !work_dir.path().join("opencode.json").exists(),
        "CWD must not receive opencode.json in template-name mode"
    );

    let data = fs::read_to_string(&derived).expect("read derived output");
    let value: Value = serde_json::from_str(&data).expect("parse json");
    assert_eq!(value["$schema"], "https://opencode.ai/config.json");
}

#[test]
fn compose_explicit_out_overrides_derived() {
    let config_dir = TempDir::new().expect("config dir");
    let work_dir = TempDir::new().expect("work dir");

    let template_d = config_dir.path().join("template.d").join("myname.d");
    copy_fragments(&fixture_fragments_dir(), &template_d);

    let explicit_out = work_dir.path().join("explicit.json");

    // Compose with template name AND explicit -o
    config_compose_cmd(work_dir.path(), config_dir.path())
        .arg("myname")
        .arg("-o")
        .arg(&explicit_out)
        .assert()
        .success();

    // Explicit path should exist
    assert!(explicit_out.exists(), "explicit -o path should be written");

    // Derived path should NOT exist
    let derived = config_dir.path().join("template.d").join("myname.json");
    assert!(
        !derived.exists(),
        "derived path must not be written when -o is given"
    );
}

#[test]
fn compose_literal_dir_keeps_default_out() {
    let work_dir = TempDir::new().expect("work dir");
    let xdg = TempDir::new().expect("xdg dir");
    let frag_dir = work_dir.path().join("my-fragments");
    copy_fragments(&fixture_fragments_dir(), &frag_dir);

    // Compose with literal directory — no -o flag
    compose_cmd(work_dir.path(), xdg.path())
        .arg(&frag_dir)
        .assert()
        .success();

    // Default output should be opencode.json in CWD
    let default_out = work_dir.path().join("opencode.json");
    assert!(
        default_out.exists(),
        "literal-dir compose without -o should write opencode.json in CWD"
    );

    let data = fs::read_to_string(&default_out).expect("read output");
    let value: Value = serde_json::from_str(&data).expect("parse json");
    assert_eq!(value["$schema"], "https://opencode.ai/config.json");
}

#[test]
fn compose_force_overwrites_template_file() {
    let config_dir = TempDir::new().expect("config dir");
    let work_dir = TempDir::new().expect("work dir");

    let template_d = config_dir.path().join("template.d").join("myname.d");
    copy_fragments(&fixture_fragments_dir(), &template_d);

    let derived = config_dir.path().join("template.d").join("myname.json");
    // Pre-create a file at the derived path
    fs::write(&derived, r#"{"old": true}"#).expect("write existing");

    // Without --force, compose should fail
    config_compose_cmd(work_dir.path(), config_dir.path())
        .arg("myname")
        .assert()
        .failure()
        .stderr(predicate::str::contains("output already exists"));

    // With --force, it should overwrite
    config_compose_cmd(work_dir.path(), config_dir.path())
        .arg("myname")
        .arg("--force")
        .assert()
        .success();

    let data = fs::read_to_string(&derived).expect("read output");
    let value: Value = serde_json::from_str(&data).expect("parse json");
    assert_eq!(
        value["$schema"], "https://opencode.ai/config.json",
        "derived file should be overwritten with composed content"
    );
}

#[test]
fn compose_no_force_rejects_existing() {
    let config_dir = TempDir::new().expect("config dir");
    let work_dir = TempDir::new().expect("work dir");

    let template_d = config_dir.path().join("template.d").join("myname.d");
    copy_fragments(&fixture_fragments_dir(), &template_d);

    let derived = config_dir.path().join("template.d").join("myname.json");
    fs::write(&derived, r#"{"old": true}"#).expect("write existing");

    // Without --force, compose should fail
    config_compose_cmd(work_dir.path(), config_dir.path())
        .arg("myname")
        .assert()
        .failure()
        .stderr(predicate::str::contains("output already exists"));

    // Existing file should be untouched
    let data = fs::read_to_string(&derived).expect("read existing");
    assert_eq!(
        data, r#"{"old": true}"#,
        "existing file must not be modified"
    );
}

#[test]
fn roundtrip_dry_run_no_side_effects() {
    let config_dir = TempDir::new().expect("config dir");
    let work_dir = TempDir::new().expect("work dir");
    write_roundtrip_config(config_dir.path());

    // Decompose default to create fragments
    decompose_cmd(work_dir.path(), config_dir.path())
        .arg("default")
        .assert()
        .success();

    let frag_dir = config_dir.path().join("template.d").join("default.d");
    assert!(frag_dir.exists(), "fragments should exist after decompose");

    // The derived output path
    let derived = config_dir.path().join("template.d").join("default.json");
    // decompose removes the original file, so derived should not exist
    assert!(
        !derived.exists(),
        "decompose should have removed the original file"
    );

    // Compose with --dry-run: exit code 1 means diff was shown (new file)
    config_compose_cmd(work_dir.path(), config_dir.path())
        .arg("default")
        .arg("--dry-run")
        .assert()
        .code(1)
        .stdout(predicate::str::contains("--- /dev/null"));

    // Derived output must NOT exist after dry-run
    assert!(
        !derived.exists(),
        "dry-run must not create the derived output file"
    );

    // CWD must not have any new files
    assert!(
        !work_dir.path().join("opencode.json").exists(),
        "dry-run must not create opencode.json in CWD"
    );
}

// ---------------------------------------------------------------------------
// 12. Template-name resolution takes priority over local directory
// ---------------------------------------------------------------------------

#[test]
fn compose_template_name_preferred_over_local_dir() {
    let config_dir = TempDir::new().expect("config dir");
    let work_dir = TempDir::new().expect("work dir");

    // Set up config-dir template: template.d/default.d/ with fixture fragments
    let template_d = config_dir.path().join("template.d").join("default.d");
    copy_fragments(&fixture_fragments_dir(), &template_d);

    // Set up a LOCAL directory in the working dir with the same name but
    // different content — a single fragment producing {"local": true}.
    let local_dir = work_dir.path().join("default");
    fs::create_dir_all(&local_dir).expect("create local dir");
    fs::write(local_dir.join("only.json"), r#"{"local": true}"#).expect("write local fragment");

    // Run `compose default` (no -o) from work_dir
    config_compose_cmd(work_dir.path(), config_dir.path())
        .arg("default")
        .assert()
        .success();

    // Template-name resolution should win: output at config_dir/template.d/default.json
    let derived = config_dir.path().join("template.d").join("default.json");
    assert!(
        derived.exists(),
        "template-name resolution should produce derived output"
    );

    let data = fs::read_to_string(&derived).expect("read derived output");
    let value: Value = serde_json::from_str(&data).expect("parse json");

    // Content must come from the config-dir template fragments, not the local dir
    assert_eq!(
        value["$schema"], "https://opencode.ai/config.json",
        "output must come from config-dir template, not local dir"
    );
    assert!(
        value.get("local").is_none(),
        "output must NOT contain content from local dir"
    );

    // CWD must NOT have opencode.json (which would indicate literal-dir fallback)
    assert!(
        !work_dir.path().join("opencode.json").exists(),
        "CWD must not receive opencode.json when template-name wins"
    );
}
