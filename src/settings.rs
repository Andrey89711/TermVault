//! Non-secret per-user defaults written by setup. Explicit CLI paths take precedence.
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::{
    fs::File,
    io::{ErrorKind, Read},
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    vault: PathBuf,
}

pub fn default_vault(local_data: &Path) -> Result<PathBuf> {
    let folder = local_data.join("TermVault");
    let file = match File::open(folder.join("settings.json")) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(folder.join("vault.kdbx")),
        Err(_) => {
            bail!("Cannot read TermVault settings; rerun setup or pass a vault path explicitly")
        }
    };
    let mut bytes = Vec::new();
    file.take(8193)
        .read_to_end(&mut bytes)
        .context("Cannot read TermVault settings")?;
    if bytes.len() > 8192 {
        bail!("TermVault settings exceed the size limit");
    }
    let settings: Settings = serde_json::from_slice(&bytes)
        .context("Invalid TermVault settings; rerun setup or pass a vault path explicitly")?;
    if !settings.vault.is_absolute()
        || !settings
            .vault
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("kdbx"))
    {
        bail!("TermVault default vault must be an absolute .kdbx path; rerun setup");
    }
    Ok(settings.vault)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_settings_preserve_the_original_default_without_writing() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(
            default_vault(root.path()).unwrap(),
            root.path().join("TermVault/vault.kdbx")
        );
        assert!(!root.path().join("TermVault").exists());
    }
    #[test]
    fn reads_selected_path_without_opening_the_vault() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("TermVault");
        std::fs::create_dir(&folder).unwrap();
        let selected = root.path().join("different vault.kdbx");
        std::fs::write(
            folder.join("settings.json"),
            serde_json::to_vec(&serde_json::json!({"vault": selected})).unwrap(),
        )
        .unwrap();
        assert_eq!(default_vault(root.path()).unwrap(), selected);
        assert!(!selected.exists());
    }
    #[test]
    fn rejects_bad_or_oversized_configuration() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("TermVault");
        std::fs::create_dir(&folder).unwrap();
        for invalid in [
            r#"{"vault":"relative.kdbx"}"#.to_string(),
            r#"{"password":"no"}"#.to_string(),
            " ".repeat(8193),
        ] {
            std::fs::write(folder.join("settings.json"), invalid).unwrap();
            assert!(default_vault(root.path()).is_err());
        }
    }
}
