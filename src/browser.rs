//! Read-only, one-shot browser bridge. No listening socket or decrypted files.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
#[cfg(any(windows, test))]
use std::io::Read;
use std::io::Write;
use url::Url;
use zeroize::{Zeroize, Zeroizing};

const MAX_MESSAGE: usize = 16 * 1024;

/// Exact HTTPS origin matching, not suffix or substring matching.
pub fn origin(value: &str) -> Result<String> {
    if value.len() > 4096 || value.chars().any(|c| c.is_control() || c.is_whitespace()) {
        bail!("Invalid HTTPS address");
    }
    let url = Url::parse(value).context("Invalid HTTPS address")?;
    if url.scheme() != "https"
        || url.host_str().is_none_or(|host| host.len() > 253)
        || !url.username().is_empty()
        || url.password().is_some()
    {
        bail!("Only HTTPS addresses without embedded credentials are allowed");
    }
    Ok(url.origin().ascii_serialization())
}

pub fn matches(entry_url: &str, target: &str) -> bool {
    origin(entry_url).is_ok_and(|value| value == target)
}

#[cfg(any(windows, test))]
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "lowercase", deny_unknown_fields)]
enum Request {
    Fill { url: String },
    Ping {},
}

#[derive(Serialize, Deserialize, Zeroize)]
#[serde(deny_unknown_fields)]
pub struct Credentials {
    pub origin: String,
    pub username: String,
    pub password: String,
}

#[cfg(any(windows, test))]
fn read_message(reader: &mut impl Read) -> Result<Zeroizing<Vec<u8>>> {
    let mut size = [0; 4];
    reader.read_exact(&mut size)?;
    let size = u32::from_le_bytes(size) as usize;
    if size == 0 || size > MAX_MESSAGE {
        bail!("Invalid browser message size");
    }
    let mut bytes = Zeroizing::new(vec![0; size]);
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn write_message(writer: &mut impl Write, bytes: &[u8]) -> Result<()> {
    if bytes.is_empty() || bytes.len() > MAX_MESSAGE {
        bail!("Invalid browser response size");
    }
    writer.write_all(&(bytes.len() as u32).to_le_bytes())?;
    writer.write_all(bytes)?;
    writer.flush()?;
    Ok(())
}

#[cfg(windows)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    extension_id: String,
    vault: std::path::PathBuf,
}

#[cfg(windows)]
fn native_request() -> Result<Zeroizing<Vec<u8>>> {
    use std::{
        os::windows::process::CommandExt,
        process::{Command, Stdio},
        time::Duration,
    };
    let executable = std::env::current_exe()?;
    let config_path = executable.with_file_name("browser-config.json");
    let config_bytes = std::fs::read(config_path).context("Browser bridge is not installed")?;
    if config_bytes.len() > MAX_MESSAGE {
        bail!("Invalid bridge configuration");
    }
    let config: Config = serde_json::from_slice(&config_bytes)?;
    if config.extension_id.len() != 32
        || !config
            .extension_id
            .bytes()
            .all(|c| (b'a'..=b'p').contains(&c))
        || !config.vault.is_absolute()
    {
        bail!("Invalid bridge configuration");
    }
    let mut arguments = std::env::args().skip(1);
    let first = arguments.next().context("Missing extension origin")?;
    let caller = if first == "--native-host" {
        arguments.next().context("Missing extension origin")?
    } else {
        first
    };
    if caller != format!("chrome-extension://{}/", config.extension_id) {
        bail!("Extension not allowed");
    }
    let bytes = read_message(&mut std::io::stdin().lock())?;
    let request: Request = serde_json::from_slice(&bytes)?;
    let target = match request {
        Request::Ping {} => {
            // Authenticate the caller first, but never inspect or unlock the vault for a ping.
            return Ok(Zeroizing::new(serde_json::to_vec(&serde_json::json!({
                "ok": true, "kind": "pong", "protocol": 1, "version": env!("CARGO_PKG_VERSION")
            }))?));
        }
        Request::Fill { url } => origin(&url)?,
    };
    crate::vault::inspect(&config.vault)?;
    // A fresh console handles master-password entry. The anonymous stdout pipe
    // carries only the explicitly approved record; never a password argument.
    let mut child = Command::new(executable)
        .arg("--browser-approve")
        .arg(&target)
        .arg(&config.vault)
        .creation_flags(0x00000010) // CREATE_NEW_CONSOLE
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut output = child.stdout.take().context("Missing approval pipe")?;
    let (sender, receiver) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut bytes = Zeroizing::new(Vec::new());
        let result = output
            .by_ref()
            .take((MAX_MESSAGE + 1) as u64)
            .read_to_end(&mut bytes);
        let _ = sender.send(result.map(|_| bytes));
    });
    let received = receiver.recv_timeout(Duration::from_secs(180));
    if received.is_err() {
        let _ = child.kill();
    }
    let status = child.wait()?;
    let _ = reader.join();
    let bytes = received.context("Approval timed out")??;
    if !status.success() || bytes.is_empty() || bytes.len() > MAX_MESSAGE {
        bail!("Approval cancelled");
    }
    let credentials = Zeroizing::new(serde_json::from_slice::<Credentials>(&bytes)?);
    if credentials.origin != target
        || credentials.username.len() > 1024
        || credentials.password.len() > 1024
    {
        bail!("Invalid approval response");
    }
    // Wrap the already validated, zeroizing serialization without copying secrets to Value.
    let mut response = Zeroizing::new(b"{\"ok\":true,\"credentials\":".to_vec());
    response.extend_from_slice(&bytes);
    response.push(b'}');
    Ok(response)
}

pub fn native_host() -> Result<()> {
    // Do not print errors/panic payloads into the native messaging stream.
    std::panic::set_hook(Box::new(|_| {}));
    #[cfg(windows)]
    let response = native_request().unwrap_or_else(|_| {
        Zeroizing::new(br#"{"ok":false,"error":"Request denied, cancelled, timed out, or bridge unavailable. Check local setup."}"#.to_vec())
    });
    #[cfg(not(windows))]
    let response = Zeroizing::new(
        br#"{"ok":false,"error":"Browser integration currently requires Windows."}"#.to_vec(),
    );
    write_message(&mut std::io::stdout().lock(), &response)
}

#[cfg(windows)]
pub fn approve(path: std::path::PathBuf, target: &str) -> Result<()> {
    use std::{
        fs::OpenOptions,
        os::windows::io::{AsRawHandle, FromRawHandle},
    };
    use windows_sys::Win32::Storage::FileSystem::{GetFileType, FILE_TYPE_PIPE};
    use windows_sys::Win32::System::Console::{
        GetStdHandle, SetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };
    let target = origin(target)?;
    // Capture the inherited anonymous pipe before rebinding the UI to its own console.
    let pipe_handle = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
    if pipe_handle.is_null()
        || pipe_handle as isize == -1
        || unsafe { GetFileType(pipe_handle) } != FILE_TYPE_PIPE
    {
        bail!("Missing browser approval pipe");
    }
    let input = OpenOptions::new().read(true).write(true).open("CONIN$")?;
    let output = OpenOptions::new().read(true).write(true).open("CONOUT$")?;
    unsafe {
        if SetStdHandle(STD_INPUT_HANDLE, input.as_raw_handle()) == 0
            || SetStdHandle(STD_OUTPUT_HANDLE, output.as_raw_handle()) == 0
            || SetStdHandle(STD_ERROR_HANDLE, output.as_raw_handle()) == 0
        {
            bail!("Cannot initialize approval console");
        }
    }
    // The old stdout handle is no longer used by stdio after SetStdHandle.
    let mut pipe = unsafe { std::fs::File::from_raw_handle(pipe_handle) };
    let credentials = crate::app::run_browser(path, target)?;
    if let Some(credentials) = credentials {
        let bytes = Zeroizing::new(serde_json::to_vec(&*credentials)?);
        pipe.write_all(&bytes)?;
        pipe.flush()?;
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn approve(_path: std::path::PathBuf, _target: &str) -> Result<()> {
    bail!("Browser integration currently requires Windows")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_origins() {
        let target = origin("https://Example.com/login?q=1").unwrap();
        assert_eq!(target, "https://example.com");
        assert!(matches("https://example.com:443/account", &target));
        for url in [
            "http://example.com",
            "https://example.com.evil.org",
            "https://evil.example.com",
            "https://example.com:444",
            "https://user@example.com",
            "https://example.com\n",
            "javascript:alert(1)",
            "example.com",
        ] {
            assert!(!matches(url, &target), "{url}");
        }
        assert!(!matches("https://exаmple.com", &target)); // Cyrillic a
    }
    #[test]
    fn bounded_framing() {
        let mut data = Vec::new();
        write_message(&mut data, b"{} ").unwrap();
        assert_eq!(&*read_message(&mut data.as_slice()).unwrap(), b"{} ");
        assert!(read_message(&mut 0u32.to_le_bytes().as_slice()).is_err());
        assert!(read_message(&mut ((MAX_MESSAGE + 1) as u32).to_le_bytes().as_slice()).is_err());
        assert!(read_message(&mut [5, 0, 0, 0, 1].as_slice()).is_err());
        assert!(write_message(&mut Vec::new(), &vec![0; MAX_MESSAGE + 1]).is_err());
    }
    #[test]
    fn rejects_extra_commands() {
        let request: Request =
            serde_json::from_str(r#"{"action":"fill","url":"https://example.com"}"#).unwrap();
        assert!(matches!(request, Request::Fill { url } if url == "https://example.com"));
        assert!(matches!(
            serde_json::from_str::<Request>(r#"{"action":"ping"}"#).unwrap(),
            Request::Ping {}
        ));
        for invalid in [
            r#"{"action":"ping","url":"https://example.com"}"#,
            r#"{"action":"export"}"#,
            r#"{"action":"fill"}"#,
        ] {
            assert!(serde_json::from_str::<Request>(invalid).is_err());
        }
        assert!(serde_json::from_str::<Request>(
            r#"{"action":"fill","url":"https://example.com","password":"no"}"#
        )
        .is_err());
    }
}
