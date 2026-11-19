//! Manual-only update checks and bounded, verified Windows payload staging.
//! Checksums authenticate bytes, NOT their publisher: releases are unsigned.
use crate::i18n::{localized, Language};
use anyhow::{bail, Context, Result};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    time::Duration,
};

const ENDPOINT: &str = "https://api.github.com/repos/Andrey89711/TermVault/releases/latest";
const MAX_RESPONSE: u64 = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateStatus {
    UpToDate,
    Available(String),
    NoRelease,
}

impl UpdateStatus {
    pub fn description(&self, language: Language) -> String {
        match self {
            Self::UpToDate => localized!(language, "No updates. Installed version: {}.", "Новых версий нет. Установлена версия {}.", env!("CARGO_PKG_VERSION")),
            Self::Available(version) => localized!(language, "Version {version} is available. Installed: {}.\n\nRelease page:\nhttps://github.com/Andrey89711/TermVault/releases/tag/v{version}\n\nWindows: Ctrl+P → Download and install update.\nSeparate download and installation consent required.\nOther platforms: install manually from the release page.", "Доступна версия {version}. Установлена {}.\n\nСтраница релиза:\nhttps://github.com/Andrey89711/TermVault/releases/tag/v{version}\n\nWindows: Ctrl+P → Скачать и установить обновление.\nСкачивание и установка требуют отдельных подтверждений.\nДругие платформы: установите вручную со страницы релиза.", env!("CARGO_PKG_VERSION")),
            Self::NoRelease => language.text("No published stable release found yet.", "Опубликованный стабильный релиз пока не найден.").into(),
        }
    }
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
}

fn parse_release(bytes: &[u8], current: &str) -> Result<UpdateStatus> {
    if bytes.len() as u64 > MAX_RESPONSE {
        bail!("GitHub response exceeds the size limit");
    }
    let release: Release = serde_json::from_slice(bytes).context("Invalid GitHub response")?;
    if release.draft || release.prerelease {
        bail!("Expected a published stable release");
    }
    let version = Version::parse(
        release
            .tag_name
            .strip_prefix('v')
            .context("Invalid release tag")?,
    )
    .context("Invalid release version")?;
    if !version.pre.is_empty() || !version.build.is_empty() {
        bail!("Expected a stable vX.Y.Z tag");
    }
    let current = Version::parse(current).context("Invalid application version")?;
    Ok(if version > current {
        UpdateStatus::Available(version.to_string())
    } else {
        UpdateStatus::UpToDate
    })
}

fn check() -> Result<UpdateStatus> {
    let client = reqwest::blocking::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .user_agent(concat!("TermVault/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| anyhow::anyhow!("Cannot prepare HTTPS update check"))?;
    let response = client
        .get(ENDPOINT)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .map_err(|_| anyhow::anyhow!("Cannot reach GitHub. Check your network and retry."))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(UpdateStatus::NoRelease);
    }
    if !response.status().is_success() {
        bail!("GitHub rejected the check. Try again later.");
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_RESPONSE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow::anyhow!("Cannot read GitHub response"))?;
    parse_release(&bytes, env!("CARGO_PKG_VERSION"))
}

pub type CheckResult = std::result::Result<UpdateStatus, String>;

pub fn start_check() -> Result<Receiver<CheckResult>> {
    let (sender, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name("termvault-update-check".into())
        .spawn(move || {
            // Fixed/context-only errors, never request objects or HTTP payloads.
            let _ = sender.send(check().map_err(|error| error.to_string()));
        })
        .context("Cannot start update check")?;
    Ok(receiver)
}

// Keep in sync with Get-TermVaultPayloadFiles. Entire archives are validated,
// not merely the executable. Unknown members are rejected, never ignored.
pub const PAYLOAD: &[&str] = &[
    "tvault.exe",
    "LICENSE",
    "README.md",
    "SECURITY.md",
    "PRIVACY.md",
    "CHANGELOG.md",
    "docs/RELEASING.md",
    "docs/STORE-PUBLISHING.ru.md",
    "docs/CSV.md",
    "setup.ps1",
    "setup.cmd",
    "uninstall.ps1",
    "uninstall.cmd",
    "scripts/setup-common.ps1",
    "scripts/uninstall-common.ps1",
    "browser/README.md",
    "browser/QUICKSTART.ru.md",
    "browser/install.ps1",
    "browser/uninstall.ps1",
    "browser/extension/manifest.json",
    "browser/extension/background.js",
    "browser/extension/content.js",
    "browser/extension/package.json",
    "browser/extension/options.html",
    "browser/extension/options.js",
    "browser/extension/icons/icon-16.png",
    "browser/extension/icons/icon-32.png",
    "browser/extension/icons/icon-48.png",
    "browser/extension/icons/icon-128.png",
];
// Historical ownership is deliberately independent of the incoming allowlist.
// Marker-less-layout v0.1.2 never owned docs/CSV.md. Future layout changes must
// explicitly extend this mapping; missing arbitrary PAYLOAD members are errors.
const LEGACY_012_PAYLOAD: &[&str] = &[
    "tvault.exe",
    "LICENSE",
    "README.md",
    "SECURITY.md",
    "PRIVACY.md",
    "CHANGELOG.md",
    "docs/RELEASING.md",
    "docs/STORE-PUBLISHING.ru.md",
    "setup.ps1",
    "setup.cmd",
    "uninstall.ps1",
    "uninstall.cmd",
    "scripts/setup-common.ps1",
    "scripts/uninstall-common.ps1",
    "browser/README.md",
    "browser/QUICKSTART.ru.md",
    "browser/install.ps1",
    "browser/uninstall.ps1",
    "browser/extension/manifest.json",
    "browser/extension/background.js",
    "browser/extension/content.js",
    "browser/extension/package.json",
    "browser/extension/options.html",
    "browser/extension/options.js",
    "browser/extension/icons/icon-16.png",
    "browser/extension/icons/icon-32.png",
    "browser/extension/icons/icon-48.png",
    "browser/extension/icons/icon-128.png",
];
const LAYOUT_2_ADDITIONS: &[&str] = &["docs/CSV.md"];

fn original_hash(root: &Path, name: &str, marker: &serde_json::Value) -> Result<Option<String>> {
    let legacy = match marker.get("payload_layout") {
        None if marker["version"] == "0.1.2" => true,
        Some(layout) if layout == 2 => false,
        _ => bail!("Unsupported installation payload layout"),
    };
    let path = root.join(name);
    if LEGACY_012_PAYLOAD.contains(&name) || (!legacy && LAYOUT_2_ADDITIONS.contains(&name)) {
        return Ok(Some(hash(&bounded_file(&path, MAX_FILE)?)));
    }
    if legacy && LAYOUT_2_ADDITIONS.contains(&name) {
        regular_path(path.parent().context("Missing update parent")?, true)?;
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            _ => bail!("New update file collides with an unowned path"),
        }
    }
    bail!("No ownership mapping for update member")
}
const MAX_ARCHIVE: u64 = 64 * 1024 * 1024;
const MAX_EXPANDED: u64 = 128 * 1024 * 1024;
const MAX_FILE: u64 = 64 * 1024 * 1024;
const APPLY_HELPER: &str = include_str!("../scripts/update-apply.ps1");

#[derive(Debug)]
pub struct StagedUpdate {
    directory: tempfile::TempDir,
    verified_hashes: HashMap<&'static str, String>,
    pub version: String,
    pub destination: PathBuf,
}
pub type DownloadResult = std::result::Result<StagedUpdate, String>;

fn stable_new_version(version: &str) -> Result<Version> {
    let parsed = Version::parse(version).context("Invalid update version")?;
    if !parsed.pre.is_empty()
        || !parsed.build.is_empty()
        || parsed.to_string() != version
        || parsed <= Version::parse(env!("CARGO_PKG_VERSION"))?
    {
        bail!("Update must be a newer stable version");
    }
    Ok(parsed)
}

fn allowed_url(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && matches!(
            url.host_str(),
            Some(
                "github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
            )
        )
}

fn download_client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 4 || !allowed_url(attempt.url()) {
                attempt.error("Update redirect is not an allowed HTTPS GitHub asset URL")
            } else {
                attempt.follow()
            }
        }))
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(90))
        .user_agent(concat!("TermVault/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("Cannot prepare update HTTPS client")
}

fn fetch_asset(client: &reqwest::blocking::Client, url: &str, maximum: u64) -> Result<Vec<u8>> {
    let url = reqwest::Url::parse(url)?;
    if !allowed_url(&url) {
        bail!("Invalid update asset URL");
    }
    let response = client
        .get(url)
        .send()
        .context("Cannot download official GitHub asset")?;
    if !response.status().is_success() || response.content_length().is_some_and(|n| n > maximum) {
        bail!("Missing or oversized official update asset");
    }
    let mut bytes = Vec::new();
    response
        .take(maximum + 1)
        .read_to_end(&mut bytes)
        .context("Cannot read update asset")?;
    if bytes.len() as u64 > maximum {
        bail!("Update asset exceeds size limit");
    }
    Ok(bytes)
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn verify_checksum(sums: &[u8], filename: &str, bytes: &[u8]) -> Result<()> {
    if sums.len() as u64 > MAX_RESPONSE {
        bail!("Oversized checksum file");
    }
    let text = std::str::from_utf8(sums).context("Invalid checksum text")?;
    let mut expected = None;
    for line in text.lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() == 2 && fields[1].trim_start_matches('*') == filename {
            if expected.is_some()
                || fields[0].len() != 64
                || !fields[0].bytes().all(|b| b.is_ascii_hexdigit())
            {
                bail!("Invalid or duplicate SHA256 entry");
            }
            expected = Some(fields[0].to_ascii_lowercase());
        }
    }
    if expected.as_deref() != Some(hash(bytes).as_str()) {
        bail!("Update SHA256 verification failed");
    }
    Ok(())
}

/// Reject reparse points on Windows, symlinks elsewhere, including ancestors.
fn regular_path(path: &Path, directory: bool) -> Result<()> {
    let metadata = fs::symlink_metadata(path).context("Missing installation payload")?;
    if metadata.file_type().is_symlink()
        || (directory && !metadata.is_dir())
        || (!directory && !metadata.is_file())
    {
        bail!("Installation contains non-regular files");
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            bail!("Installation contains a reparse point");
        }
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| *parent != path && !parent.as_os_str().is_empty())
    {
        regular_path(parent, true)?;
    }
    Ok(())
}

fn bounded_file(path: &Path, max: u64) -> Result<Vec<u8>> {
    regular_path(path, false)?;
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(max + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        bail!("Installation file exceeds size limit");
    }
    Ok(bytes)
}

fn installed_root(executable: &Path) -> Result<PathBuf> {
    if executable.file_name().and_then(|n| n.to_str()) != Some("tvault.exe") {
        bail!("Update only a setup-installed tvault.exe");
    }
    regular_path(executable, false)?;
    let root = executable.parent().context("Cannot locate installation")?;
    let marker: serde_json::Value =
        serde_json::from_slice(&bounded_file(&root.join(".termvault-install.json"), 4096)?)?;
    if marker["product"] != "TermVault"
        || marker["schema"] != 1
        || marker["version"] != env!("CARGO_PKG_VERSION")
    {
        bail!("Installation ownership/version marker does not match this application");
    }
    for file in PAYLOAD {
        original_hash(root, file, &marker)?;
    }
    Ok(root.to_owned())
}

fn extract_payload(
    bytes: &[u8],
    version: &str,
    destination: &Path,
) -> Result<HashMap<&'static str, String>> {
    if bytes.len() as u64 > MAX_ARCHIVE {
        bail!("Oversized update ZIP");
    }
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).context("Invalid update ZIP")?;
    // Directory entries from Compress-Archive are allowed only when exact parents.
    let parents: HashSet<String> = PAYLOAD
        .iter()
        .flat_map(|name| {
            let mut result = Vec::new();
            let mut parent = Path::new(name).parent();
            while let Some(path) = parent.filter(|p| !p.as_os_str().is_empty()) {
                result.push(format!("{}/", path.to_string_lossy().replace('\\', "/")));
                parent = path.parent();
            }
            result
        })
        .collect();
    if archive.len() > PAYLOAD.len() + parents.len() {
        bail!("Too many ZIP entries");
    }
    let mut seen = HashSet::new();
    let mut total = 0u64;
    // Pass one validates the WHOLE archive before creating output.
    for index in 0..archive.len() {
        let file = archive.by_index(index).context("Invalid ZIP member")?;
        let name = file.name();
        if name.contains('\\')
            || name.contains(':')
            || name.starts_with('/')
            || name.split('/').any(|p| p == "." || p == "..")
            || !seen.insert(name.to_owned())
            || file.enclosed_name().is_none()
        {
            bail!("Unsafe or duplicate ZIP path");
        }
        if let Some(mode) = file.unix_mode() {
            let kind = mode & 0o170000;
            if kind != 0 && kind != if file.is_dir() { 0o040000 } else { 0o100000 } {
                bail!("Non-regular ZIP entry");
            }
        }
        if file.is_dir() {
            if !parents.contains(name) || file.size() != 0 {
                bail!("Unexpected ZIP directory");
            }
            continue;
        }
        if !PAYLOAD.contains(&name) || file.size() > MAX_FILE {
            bail!("Unexpected or oversized ZIP file");
        }
        total = total
            .checked_add(file.size())
            .context("ZIP size overflow")?;
        if total > MAX_EXPANDED
            || file.size()
                > file
                    .compressed_size()
                    .saturating_mul(500)
                    .saturating_add(1024 * 1024)
        {
            bail!("ZIP expansion limit exceeded");
        }
    }
    if PAYLOAD.iter().any(|name| !seen.contains(*name)) {
        bail!("Incomplete update payload");
    }
    fs::create_dir(destination)?;
    let mut hashes = HashMap::new();
    for name in PAYLOAD {
        let mut member = archive.by_name(name)?;
        let mut content = Vec::new();
        (&mut member)
            .take(MAX_FILE + 1)
            .read_to_end(&mut content)
            .context("Cannot decompress ZIP member")?;
        if content.len() as u64 != member.size() || content.len() as u64 > MAX_FILE {
            bail!("ZIP member size mismatch");
        }
        let target = destination.join(name);
        fs::create_dir_all(target.parent().unwrap())?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target)?;
        std::io::Write::write_all(&mut output, &content)?;
        hashes.insert(*name, hash(&content));
    }
    let manifest: serde_json::Value = serde_json::from_slice(&bounded_file(
        &destination.join("browser/extension/manifest.json"),
        MAX_RESPONSE,
    )?)?;
    if manifest["version"] != version {
        bail!("Extension version does not match release");
    }
    let executable = bounded_file(&destination.join("tvault.exe"), MAX_FILE)?;
    if !executable.starts_with(b"MZ") {
        bail!("Update executable is not a Windows PE file");
    }
    Ok(hashes)
}

pub fn start_download(version: String) -> Result<Receiver<DownloadResult>> {
    if !cfg!(all(windows, target_arch = "x86_64")) {
        bail!("In-app installation currently supports Windows x86_64 only. Use the official release page.");
    }
    stable_new_version(&version)?;
    let destination = installed_root(&std::env::current_exe()?)?;
    let (sender, receiver) = mpsc::channel();
    std::thread::Builder::new().name("termvault-update-stage".into()).spawn(move || {
        let result = (|| -> Result<StagedUpdate> {
            let client = download_client()?;
            let filename = format!("termvault-{version}-windows-x86_64.zip");
            let base = format!("https://github.com/Andrey89711/TermVault/releases/download/v{version}");
            let sums = fetch_asset(&client, &format!("{base}/SHA256SUMS.txt"), MAX_RESPONSE)?;
            let bytes = fetch_asset(&client, &format!("{base}/{filename}"), MAX_ARCHIVE)?;
            verify_checksum(&sums, &filename, &bytes)?;
            let directory = tempfile::Builder::new().prefix("termvault-update-").tempdir()?;
            let verified_hashes = extract_payload(&bytes, &version, &directory.path().join("payload"))?;
            Ok(StagedUpdate { directory, verified_hashes, version, destination })
        })();
        // HTTP requests and response bodies never enter notices.
        let _ = sender.send(result.map_err(|_| "Update download/verification failed. No installation files changed. Retry from the update command.".into()));
    }).context("Cannot start update download")?;
    Ok(receiver)
}

#[derive(serde::Serialize)]
struct ApplyFile {
    relative: String,
    before: Option<String>,
    after: String,
}

fn upgraded_marker(mut marker: serde_json::Value, version: &str) -> Result<Vec<u8>> {
    marker["version"] = serde_json::Value::String(version.into());
    marker["payload_layout"] = serde_json::json!(2);
    // Retain unknown fields for forward-compatible setup/ownership metadata.
    let bytes = serde_json::to_vec(&marker)?;
    if bytes.len() > 4096 {
        bail!("Upgraded installation marker exceeds size limit");
    }
    Ok(bytes)
}

impl StagedUpdate {
    fn verified_hash(&self, name: &str) -> Result<String> {
        let expected = self
            .verified_hashes
            .get(name)
            .context("Missing verified update hash")?;
        if hash(&bounded_file(
            &self.directory.path().join("payload").join(name),
            MAX_FILE,
        )?) != *expected
        {
            bail!("Staged package changed after verification");
        }
        Ok(expected.clone())
    }

    #[cfg(test)]
    pub(crate) fn for_test(directory: tempfile::TempDir) -> Self {
        Self {
            destination: directory.path().join("isolated-test-destination"),
            directory,
            verified_hashes: HashMap::new(),
            version: "99.0.0".into(),
        }
    }

    /// Call only AFTER vault/clipboard teardown and final visible consent.
    pub fn apply(self) -> Result<()> {
        if !cfg!(windows) {
            bail!("In-app installation supports Windows only");
        }
        let current = std::env::current_exe()?;
        if installed_root(&current)? != self.destination {
            bail!("Installation changed during update");
        }
        let payload = self.directory.path().join("payload");
        let marker_name = ".termvault-install.json";
        let old_marker_bytes = bounded_file(&self.destination.join(marker_name), 4096)?;
        let marker: serde_json::Value = serde_json::from_slice(&old_marker_bytes)?;
        let mut files = Vec::new();
        for name in PAYLOAD {
            files.push(ApplyFile {
                relative: (*name).into(),
                before: original_hash(&self.destination, name, &marker)?,
                after: self.verified_hash(name)?,
            });
        }
        let marker = upgraded_marker(marker, &self.version)?;
        fs::write(payload.join(marker_name), &marker)?;
        files.push(ApplyFile {
            relative: marker_name.into(),
            before: Some(hash(&old_marker_bytes)),
            after: hash(&marker),
        });
        let plan = serde_json::json!({
            "schema": 1, "root": self.destination, "version": self.version,
            "files": files, "parent": std::process::id(),
        });
        let plan_path = self.directory.path().join("plan.json");
        fs::write(&plan_path, serde_json::to_vec(&plan)?)?;
        let helper_path = self.directory.path().join("apply.ps1");
        fs::write(&helper_path, APPLY_HELPER)?;
        let powershell = PathBuf::from(
            std::env::var_os("SystemRoot").context("Cannot locate Windows PowerShell")?,
        )
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        regular_path(&powershell, false)?;
        let mut command = std::process::Command::new(powershell);
        command
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&helper_path)
            .arg("-PlanPath")
            .arg(&plan_path);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW; no elevation/relaunch.
        }
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .context("Cannot launch trusted update helper")?;
        // Only the helper owns cleanup after a successful spawn.
        let _ = self.directory.keep();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn fixture_zip(extra: Option<(&str, u32)>, omit: Option<&str>, version: &str) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o644);
        for name in PAYLOAD {
            if Some(*name) == omit {
                continue;
            }
            writer.start_file(*name, options).unwrap();
            let content = match *name {
                "tvault.exe" => b"MZ isolated test PE placeholder".to_vec(),
                "browser/extension/manifest.json" => {
                    serde_json::to_vec(&serde_json::json!({"version":version})).unwrap()
                }
                _ => b"isolated payload fixture; never execute".to_vec(),
            };
            writer.write_all(&content).unwrap();
        }
        if let Some((name, mode)) = extra {
            writer
                .start_file(name, options.unix_permissions(mode))
                .unwrap();
            writer.write_all(b"forbidden").unwrap();
        }
        writer.finish().unwrap().into_inner()
    }
    #[test]
    fn download_urls_are_exact_https_github_asset_hosts() {
        for url in [
            "http://github.com/Andrey89711/TermVault",
            "https://github.com.evil.test/a",
            "https://evil.test",
            "https://user@github.com/a",
            "https://github.com:444/a",
            "file:///C:/payload.zip",
            "https://api.github.com/anything",
        ] {
            assert!(!allowed_url(&reqwest::Url::parse(url).unwrap()), "{url}");
        }
        for host in [
            "github.com",
            "release-assets.githubusercontent.com",
            "objects.githubusercontent.com",
        ] {
            assert!(allowed_url(
                &reqwest::Url::parse(&format!("https://{host}/asset")).unwrap()
            ));
        }
        for version in [
            "0.1.2",
            "0.0.1",
            "99.0.0-rc.1",
            "99.0.0+build",
            "../99",
            "099.0.0",
        ] {
            assert!(stable_new_version(version).is_err());
        }
        assert!(stable_new_version("99.0.0").is_ok());
    }
    #[test]
    fn checksum_matching_requires_unique_exact_asset_and_detects_tampering() {
        let bytes = b"fixture";
        let name = "termvault-99.0.0-windows-x86_64.zip";
        let valid = format!("{}  {name}\n", hash(bytes));
        assert!(verify_checksum(valid.as_bytes(), name, bytes).is_ok());
        assert!(verify_checksum(valid.as_bytes(), name, b"tampered").is_err());
        assert!(verify_checksum(valid.repeat(2).as_bytes(), name, bytes).is_err());
        assert!(
            verify_checksum(valid.replace(name, "another.zip").as_bytes(), name, bytes).is_err()
        );
    }
    #[test]
    fn whole_zip_allowlist_rejects_missing_unknown_duplicate_traversal_and_version_mismatch() {
        let root = tempfile::tempdir().unwrap();
        for (extra, omit, version) in [
            (Some(("../escape", 0o644)), None, "99.0.0"),
            (Some(("browser\\extension\\evil.js", 0o644)), None, "99.0.0"),
            (Some(("config.json", 0o644)), None, "99.0.0"),
            (
                Some(("browser/extension/unexpected.js", 0o644)),
                None,
                "99.0.0",
            ),
            (None, Some("SECURITY.md"), "99.0.0"),
            (None, None, "98.0.0"),
        ] {
            let dir = root.path().join(format!("reject-{}", rand_suffix()));
            assert!(extract_payload(&fixture_zip(extra, omit, version), "99.0.0", &dir).is_err());
            assert!(!root.path().parent().unwrap().join("escape").exists());
        }
        let valid = root.path().join("valid");
        extract_payload(&fixture_zip(None, None, "99.0.0"), "99.0.0", &valid).unwrap();
        for name in PAYLOAD {
            assert!(valid.join(name).is_file());
        }
        assert!(!valid.join("vault.kdbx").exists());
    }
    fn rand_suffix() -> String {
        // Existing dependency; no RNG/network/global configuration involved.
        let temporary = tempfile::tempdir().unwrap();
        temporary
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }
    #[test]
    fn zip_bombs_and_nonregular_members_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let mut bytes = fixture_zip(None, None, "99.0.0");
        // ZIP central directory Unix mode bits identify symlinks even if the
        // name itself is allowlisted. Mutate a static fixture, not filesystem links.
        let position = bytes.windows(4).position(|x| x == b"PK\x01\x02").unwrap();
        bytes[position + 5] = 3; // Unix creator
        bytes[position + 38..position + 42].copy_from_slice(&(0o120777u32 << 16).to_le_bytes());
        assert!(extract_payload(&bytes, "99.0.0", &root.path().join("link")).is_err());
        let mut bytes = fixture_zip(None, None, "99.0.0");
        let position = bytes.windows(4).position(|x| x == b"PK\x01\x02").unwrap();
        bytes[position + 24..position + 28].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(extract_payload(&bytes, "99.0.0", &root.path().join("bomb")).is_err());
    }
    #[test]
    fn staging_is_deleted_when_worker_result_is_cancelled_or_dropped() {
        let directory = tempfile::Builder::new()
            .prefix("termvault-update-")
            .tempdir()
            .unwrap();
        let path = directory.path().to_owned();
        let staged = StagedUpdate {
            directory,
            verified_hashes: HashMap::new(),
            version: "99.0.0".into(),
            destination: PathBuf::from("not-used"),
        };
        let (sender, receiver) = mpsc::channel::<DownloadResult>();
        sender.send(Ok(staged)).unwrap();
        drop(receiver);
        assert!(!path.exists());
    }
    #[test]
    fn apply_hashes_remain_bound_to_verified_archive_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let payload = directory.path().join("payload");
        let verified_hashes =
            extract_payload(&fixture_zip(None, None, "99.0.0"), "99.0.0", &payload).unwrap();
        let staged = StagedUpdate {
            directory,
            verified_hashes,
            version: "99.0.0".into(),
            destination: PathBuf::from("not-used"),
        };
        assert!(staged.verified_hash("tvault.exe").is_ok());
        fs::write(payload.join("tvault.exe"), b"changed since download").unwrap();
        assert!(staged.verified_hash("tvault.exe").is_err());
    }
    #[test]
    fn installation_marker_is_required_and_payload_list_matches_packaging_and_helper() {
        let root = tempfile::tempdir().unwrap();
        for name in PAYLOAD {
            let path = root.path().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"fixture").unwrap();
            assert!(
                include_str!("../scripts/setup-common.ps1").contains(&format!("'{name}'"))
                    || *name == "tvault.exe"
            );
            assert!(APPLY_HELPER.contains(&format!("'{name}'")));
        }
        let exe = root.path().join("tvault.exe");
        assert!(installed_root(&exe).is_err());
        fs::write(root.path().join(".termvault-install.json"), serde_json::to_vec(&serde_json::json!({"product":"TermVault","schema":1,"version":env!("CARGO_PKG_VERSION"),"payload_layout":2})).unwrap()).unwrap();
        assert_eq!(installed_root(&exe).unwrap(), root.path());
        fs::write(root.path().join(".termvault-install.json"), b"{}").unwrap();
        assert!(installed_root(&exe).is_err());
    }
    #[test]
    fn legacy_layout_adds_only_explicit_new_member_and_rejects_all_collisions() {
        let root = tempfile::tempdir().unwrap();
        let marker = serde_json::json!({"product":"TermVault","schema":1,"version":"0.1.2"});
        fs::create_dir(root.path().join("docs")).unwrap();
        assert_eq!(
            original_hash(root.path(), "docs/CSV.md", &marker).unwrap(),
            None
        );
        assert!(original_hash(root.path(), "README.md", &marker).is_err()); // missing owned file
        assert!(original_hash(root.path(), "arbitrary-new.txt", &marker).is_err());
        fs::write(
            root.path().join("docs/CSV.md"),
            b"even matching bytes are unowned",
        )
        .unwrap();
        assert!(original_hash(root.path(), "docs/CSV.md", &marker).is_err());
        let modern = serde_json::json!({"product":"TermVault","schema":1,"version":"0.1.2","payload_layout":2});
        assert!(original_hash(root.path(), "docs/CSV.md", &modern)
            .unwrap()
            .is_some());
        for unsupported in [
            serde_json::json!({"version":"0.1.3"}),
            serde_json::json!({"version":"0.1.2","payload_layout":3}),
            serde_json::json!({"version":"0.1.2","payload_layout":true}),
        ] {
            assert!(original_hash(root.path(), "docs/CSV.md", &unsupported).is_err());
        }
        // Every member has a deliberate ownership rule; future payload additions
        // must update the historical/schema mapping rather than silently clobber.
        for name in PAYLOAD {
            assert!(LEGACY_012_PAYLOAD.contains(name) || LAYOUT_2_ADDITIONS.contains(name));
            assert!(APPLY_HELPER.contains(&format!("'{name}'")));
            assert!(
                include_str!("../scripts/setup-common.ps1").contains(&format!("'{name}'"))
                    || *name == "tvault.exe"
            );
        }
    }
    #[test]
    fn marker_upgrade_retains_unknown_fields_and_records_explicit_ownership_layout() {
        let old = serde_json::json!({"product":"TermVault","schema":1,"version":"0.1.2","future":{"nested":[1,true,"keep"]}});
        let bytes = upgraded_marker(old.clone(), "99.0.0").unwrap();
        let new: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(new["future"], old["future"]);
        assert_eq!(new["version"], "99.0.0");
        assert_eq!(new["payload_layout"], 2);
        assert_eq!(new["product"], old["product"]);
    }
    #[test]
    fn compares_stable_versions_and_never_offers_a_downgrade() {
        for (remote, expected) in [
            ("v0.1.0", UpdateStatus::UpToDate),
            ("v0.0.9", UpdateStatus::UpToDate),
            ("v0.10.0", UpdateStatus::Available("0.10.0".into())),
        ] {
            let json = serde_json::json!({"tag_name":remote,"draft":false,"prerelease":false,"html_url":"https://untrusted.example"});
            let status = parse_release(&serde_json::to_vec(&json).unwrap(), "0.1.0").unwrap();
            assert_eq!(status, expected);
            assert!(!status
                .description(Language::English)
                .contains("untrusted.example"));
        }
    }
    #[test]
    fn rejects_invalid_metadata_and_bounds_response_size() {
        for value in [
            serde_json::json!({"tag_name":"v1.0.0","draft":true,"prerelease":false}),
            serde_json::json!({"tag_name":"v1.0.0-beta","draft":false,"prerelease":false}),
            serde_json::json!({"tag_name":"v1.0.0+build","draft":false,"prerelease":false}),
            serde_json::json!({"tag_name":"https://evil.example","draft":false,"prerelease":false}),
        ] {
            assert!(parse_release(&serde_json::to_vec(&value).unwrap(), "0.1.0").is_err());
        }
        assert!(parse_release(&vec![b' '; MAX_RESPONSE as usize + 1], "0.1.0").is_err());
    }
}
