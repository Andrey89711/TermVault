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

#[cfg(windows)]
#[test]
fn browser_ping_authenticates_without_opening_or_creating_a_vault() {
    use std::io::Write;
    let root = tempfile::tempdir().unwrap();
    let host = root.path().join("tvault-browser-host.exe");
    fs::copy(env!("CARGO_BIN_EXE_tvault"), &host).unwrap();
    let vault = root.path().join("does-not-exist.kdbx");
    fs::write(
        root.path().join("browser-config.json"),
        serde_json::to_vec(&serde_json::json!({
            "extension_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "vault": vault
        }))
        .unwrap(),
    )
    .unwrap();
    for (caller, allowed) in [
        ("chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/", true),
        (
            "chrome-extension://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/",
            false,
        ),
    ] {
        let mut child = Command::new(&host)
            .arg(caller)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let request = br#"{"action":"ping"}"#;
        let mut input = child.stdin.take().unwrap();
        let _ = input.write_all(&(request.len() as u32).to_le_bytes());
        let _ = input.write_all(request);
        drop(input);
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        let size = u32::from_le_bytes(output.stdout[..4].try_into().unwrap()) as usize;
        assert_eq!(size, output.stdout.len() - 4);
        let response: serde_json::Value = serde_json::from_slice(&output.stdout[4..]).unwrap();
        assert_eq!(response["ok"], allowed);
        assert!(response.get("credentials").is_none());
        assert!(response.get("vault").is_none());
        if allowed {
            assert_eq!(response["kind"], "pong");
            assert_eq!(response["protocol"], 1);
        }
    }
    assert!(!vault.exists());
}

#[cfg(windows)]
#[test]
fn native_host_denies_unregistered_callers_and_unsupported_actions_without_secret_output() {
    use std::io::Write;
    let root = tempfile::tempdir().unwrap();
    let host = root.path().join("tvault-browser-host.exe");
    fs::copy(env!("CARGO_BIN_EXE_tvault"), &host).unwrap();
    let id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let vault = root.path().join("dummy.kdbx");
    fs::write(&vault, "not a real vault").unwrap();
    fs::write(
        root.path().join("browser-config.json"),
        serde_json::to_vec(&serde_json::json!({
            "extension_id": id, "vault": vault
        }))
        .unwrap(),
    )
    .unwrap();
    for (caller, request) in [
        (
            "chrome-extension://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/",
            r#"{"action":"fill","url":"https://example.com"}"#,
        ),
        (
            "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/",
            r#"{"action":"export","url":"https://example.com","password":"NEVER_LOG_THIS"}"#,
        ),
        (
            "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/",
            r#"{"action":"export","url":"https://example.com"}"#,
        ),
    ] {
        let mut child = Command::new(&host)
            .arg(caller)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut input = child.stdin.take().unwrap();
        // A rejected origin may close stdin before this write; either outcome must be a denial.
        let _ = input.write_all(&(request.len() as u32).to_le_bytes());
        let _ = input.write_all(request.as_bytes());
        drop(input);
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        assert!(output.stdout.len() > 4);
        let size = u32::from_le_bytes(output.stdout[..4].try_into().unwrap()) as usize;
        assert_eq!(size, output.stdout.len() - 4);
        let response: serde_json::Value = serde_json::from_slice(&output.stdout[4..]).unwrap();
        assert_eq!(response["ok"], false);
        assert!(response.get("credentials").is_none());
        assert!(!String::from_utf8_lossy(&output.stdout).contains("NEVER_LOG_THIS"));
    }
    // The renamed host must still dispatch the internal approval flag to the UI branch,
    // not recursively launch native messaging. Invalid HTTP fails before opening any UI.
    let output = Command::new(&host)
        .args(["--browser-approve", "http://example.com"])
        .arg(&vault)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}
