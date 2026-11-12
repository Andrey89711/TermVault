use std::{
    fs,
    process::{Command, Stdio},
};

#[test]
fn help_version_and_no_password_flags() {
    for arg in ["--help", "--version"] {
        let output = Command::new(env!("CARGO_BIN_EXE_tvault"))
            .arg(arg)
            .output()
            .unwrap();
        assert!(output.status.success());
    }
    let output = Command::new(env!("CARGO_BIN_EXE_tvault"))
        .args(["--password", "dummy"])
        .output()
        .unwrap();
    assert!(!output.status.success());
}
#[test]
fn nonterminal_create_does_not_create_files_or_directories() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("absent/test.kdbx");
    let output = Command::new(env!("CARGO_BIN_EXE_tvault"))
        .arg("--create")
        .arg(&path)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!root.path().join("absent").exists());
    assert!(String::from_utf8_lossy(&output.stderr).contains("interactive terminal"));
}
#[test]
fn rejected_commands_never_overwrite_existing_file() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("keep.kdbx");
    fs::write(&path, "unchanged").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_tvault"))
        .arg("--create")
        .arg(&path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(fs::read_to_string(path).unwrap(), "unchanged");
}
