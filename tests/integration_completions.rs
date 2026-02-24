use assert_cmd::cargo::cargo_bin_cmd;
use tempfile::TempDir;

#[test]
fn completions_creates_file() {
    let out_dir = TempDir::new().expect("out dir");
    let work_dir = TempDir::new().expect("work dir");

    let mut cmd = cargo_bin_cmd!("opencode-config");
    cmd.current_dir(work_dir.path())
        .arg("completions")
        .arg("bash")
        .arg("--out-dir")
        .arg(out_dir.path())
        .assert()
        .success();

    let entries: Vec<_> = std::fs::read_dir(out_dir.path())
        .expect("read dir")
        .collect();
    assert!(!entries.is_empty(), "expected completion files");
}
