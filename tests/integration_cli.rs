use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::{PredicateBooleanExt, predicate};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Subcommand existence (--help)
// ---------------------------------------------------------------------------

#[test]
fn decompose_help_succeeds() {
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("decompose")
        .arg("--help")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("--dry-run")
                .and(predicate::str::contains("--verify"))
                .and(predicate::str::contains("--force"))
                .and(predicate::str::contains("<TEMPLATE>")),
        );
}

#[test]
fn compose_help_succeeds() {
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("compose")
        .arg("--help")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("--dry-run")
                .and(predicate::str::contains("--pretty"))
                .and(predicate::str::contains("--minify"))
                .and(predicate::str::contains("--conflict"))
                .and(predicate::str::contains("--force"))
                .and(predicate::str::contains("--verify")),
        );
}

// ---------------------------------------------------------------------------
// --conflict value parsing (compose only)
// ---------------------------------------------------------------------------

#[test]
fn compose_conflict_accepts_error() {
    let work_dir = TempDir::new().expect("work dir");
    let xdg = TempDir::new().expect("xdg dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .arg("compose")
        .arg("--conflict")
        .arg("error")
        .assert()
        .failure()
        // Clap parsed successfully; compose runs but finds no fragments.
        .stderr(predicate::str::contains("no template fragments found"));
}

#[test]
fn compose_conflict_accepts_last_wins() {
    let work_dir = TempDir::new().expect("work dir");
    let xdg = TempDir::new().expect("xdg dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .arg("compose")
        .arg("--conflict")
        .arg("last-wins")
        .assert()
        .failure()
        .stderr(predicate::str::contains("no template fragments found"));
}

#[test]
fn compose_conflict_accepts_interactive() {
    let work_dir = TempDir::new().expect("work dir");
    let xdg = TempDir::new().expect("xdg dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .arg("compose")
        .arg("--conflict")
        .arg("interactive")
        .assert()
        .failure()
        .stderr(predicate::str::contains("no template fragments found"));
}

#[test]
fn compose_conflict_rejects_invalid_value() {
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("compose")
        .arg("--conflict")
        .arg("bogus")
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid value 'bogus'"));
}

// ---------------------------------------------------------------------------
// --pretty / --minify mutual exclusion
//
// The clap definition uses `overrides_with = "minify"` on `--pretty` and
// `conflicts_with = "pretty"` on `--minify`.  Because `overrides_with`
// suppresses `--minify` *before* the conflict check fires, clap actually
// accepts the combination — `--pretty` silently wins.  The tests below
// verify that `--minify` alone is accepted and that `--pretty --minify`
// passes clap parsing (i.e. the command reaches the todo!() body, not a
// clap error).  A future tightening of the clap definition could make the
// combination a hard error; update these tests accordingly.
// ---------------------------------------------------------------------------

#[test]
fn decompose_dry_run_prints_plan() {
    let work_dir = TempDir::new().expect("work dir");
    let xdg = TempDir::new().expect("xdg dir");

    // Set up a template that decompose can read
    let template_dir = xdg.path().join("opencode-config.d").join("template.d");
    std::fs::create_dir_all(&template_dir).expect("create template.d");
    std::fs::write(
        template_dir.join("test.json"),
        r#"{"agent": {"build": {"model": "gpt-4"}}}"#,
    )
    .expect("write template");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .arg("decompose")
        .arg("--dry-run")
        .arg("test")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("[DRY-RUN]")
                .and(predicate::str::contains("_test.json"))
                .and(predicate::str::contains("build.json")),
        );
}

#[test]
fn decompose_requires_template_arg() {
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("decompose")
        .assert()
        .failure()
        .stderr(predicate::str::contains("<TEMPLATE>"));
}

#[test]
fn compose_minify_alone_accepted() {
    let work_dir = TempDir::new().expect("work dir");
    let xdg = TempDir::new().expect("xdg dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .arg("compose")
        .arg("--minify")
        .assert()
        .failure()
        .stderr(predicate::str::contains("no template fragments found"));
}

// (decompose_pretty_and_minify_conflicts removed — flags no longer exist)

#[test]
fn compose_pretty_and_minify_conflicts() {
    let work_dir = TempDir::new().expect("work dir");
    let xdg = TempDir::new().expect("xdg dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .arg("compose")
        .arg("--pretty")
        .arg("--minify")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "the argument '--pretty' cannot be used with '--minify'",
        ));
}

// ---------------------------------------------------------------------------
// --dry-run selects the preview path (run_preview)
//
// main.rs dispatches dry-run to decompose::run_preview / compose::run_preview.
// compose::run_preview is now fully implemented: it loads fragments and prints
// a preview.  When run in an empty directory it reports "no template fragments
// found".
//
// All tests run in a temp working directory with XDG_CONFIG_HOME pointing
// to a temp path to avoid reading or writing user config.
// ---------------------------------------------------------------------------

// (decompose_dry_run_hits_todo replaced by decompose_dry_run_prints_plan above)

#[test]
fn compose_dry_run_no_fragments() {
    let work_dir = TempDir::new().expect("work dir");
    let xdg = TempDir::new().expect("xdg dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .arg("compose")
        .arg("--dry-run")
        .assert()
        .failure()
        .stderr(predicate::str::contains("no template fragments found"));
}
