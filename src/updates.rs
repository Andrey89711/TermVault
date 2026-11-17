//! Explicit, read-only update checks. No downloads, execution, credentials or vault data.
use crate::i18n::{localized, Language};
use anyhow::{bail, Context, Result};
use semver::Version;
use serde::Deserialize;
use std::{
    io::Read,
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
            Self::Available(version) => localized!(language, "Version {version} is available. Installed: {}.\n\nRelease page:\nhttps://github.com/Andrey89711/TermVault/releases/tag/v{version}\n\nNo automatic download or installation.", "Доступна версия {version}. Установлена {}.\n\nСтраница релиза:\nhttps://github.com/Andrey89711/TermVault/releases/tag/v{version}\n\nСкачивание и установка автоматически не выполняются.", env!("CARGO_PKG_VERSION")),
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

#[cfg(test)]
mod tests {
    use super::*;
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
