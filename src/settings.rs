//! Non-secret per-user defaults written by setup. Explicit CLI paths take precedence.
use crate::i18n::Language;
use anyhow::{bail, Context, Result};
use serde_json::{Map, Value};
use std::{
    fs::File,
    io::{ErrorKind, Read, Write},
    path::{Path, PathBuf},
};

const MAX_SETTINGS: u64 = 8192;
type SettingsDocument = (Vec<u8>, Map<String, Value>);

/// Runtime-only location discovery. Tests inject an absolute disposable root;
/// settings are stored below it at TermVault/settings.json, never beside a vault.
pub fn local_data() -> Result<PathBuf> {
    if let Some(value) = std::env::var_os("TERMVAULT_LOCAL_DATA_DIR") {
        let path = PathBuf::from(value);
        if !path.is_absolute() {
            bail!("TERMVAULT_LOCAL_DATA_DIR must be an absolute directory path");
        }
        return Ok(path);
    }
    dirs::data_local_dir().context("Cannot locate the local-data directory")
}

pub fn settings_file() -> Result<PathBuf> {
    Ok(local_data()?.join("TermVault/settings.json"))
}

fn read(path: &Path) -> Result<Option<SettingsDocument>> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.file_type().is_file() => bail!("Settings must be a regular file"),
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let file = File::open(path).context("Cannot read TermVault settings")?;
    let mut bytes = Vec::new();
    file.take(MAX_SETTINGS + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_SETTINGS {
        bail!("TermVault settings exceed the size limit");
    }
    let value: Value = serde_json::from_slice(&bytes).context("Invalid TermVault settings")?;
    let object = value
        .as_object()
        .context("Invalid TermVault settings")?
        .clone();
    Ok(Some((bytes, object)))
}

fn selected_vault(object: &Map<String, Value>, settings_file: &Path) -> Result<PathBuf> {
    let Some(value) = object.get("vault") else {
        if object.contains_key("language") {
            return Ok(settings_file
                .parent()
                .context("Settings have no parent directory")?
                .join("vault.kdbx"));
        }
        bail!("Missing TermVault default vault");
    };
    let vault = PathBuf::from(value.as_str().context("Invalid TermVault default vault")?);
    if !vault.is_absolute()
        || !vault
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("kdbx"))
    {
        bail!("TermVault default vault must be an absolute .kdbx path; rerun setup");
    }
    Ok(vault)
}

/// Missing, old, malformed and unrecognized language fields always mean English,
/// independently of the OS locale. This never opens the selected vault.
#[cfg(test)]
pub fn language(local_data: &Path) -> Language {
    language_file(&local_data.join("TermVault/settings.json"))
}

pub fn language_file(path: &Path) -> Language {
    read(path)
        .ok()
        .flatten()
        .and_then(|(_, object)| object.get("language").cloned())
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

/// Merge only the non-secret language field. Refuse malformed/oversized settings
/// instead of replacing them; use a flushed temporary file and atomic rename.
#[cfg(test)]
pub fn save_language(local_data: &Path, language: Language) -> Result<()> {
    save_language_file(&local_data.join("TermVault/settings.json"), language)
}

pub fn save_language_file(path: &Path, language: Language) -> Result<()> {
    if !path.is_absolute() {
        bail!("Settings file must be an absolute path");
    }
    let previous = read(path)?;
    let mut object = previous
        .as_ref()
        .map(|(_, value)| value.clone())
        .unwrap_or_default();
    if previous.is_some() {
        selected_vault(&object, path)?;
    }
    object.insert("language".into(), serde_json::to_value(language)?);
    let bytes = serde_json::to_vec_pretty(&object)?;
    if bytes.len() as u64 > MAX_SETTINGS {
        bail!("TermVault settings exceed the size limit");
    }
    let folder = path.parent().context("Settings have no parent directory")?;
    std::fs::create_dir_all(folder)?;
    let mut temporary = tempfile::NamedTempFile::new_in(folder)?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    // Do not knowingly overwrite another writer's changes.
    if read(path)?.as_ref().map(|(bytes, _)| bytes) != previous.as_ref().map(|(bytes, _)| bytes) {
        bail!("TermVault settings changed while saving");
    }
    if previous.is_none() {
        temporary.persist_noclobber(path)?;
    } else {
        temporary.persist(path)?;
    }
    Ok(())
}

#[cfg(test)]
pub fn default_vault(local_data: &Path) -> Result<PathBuf> {
    default_vault_file(&local_data.join("TermVault/settings.json"))
}

pub fn default_vault_file(path: &Path) -> Result<PathBuf> {
    match read(path)? {
        Some((_, object)) => selected_vault(&object, path),
        None => Ok(path
            .parent()
            .context("Settings have no parent directory")?
            .join("vault.kdbx")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_file_injection_reads_and_writes_only_the_supplied_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("custom-name.json");
        assert_eq!(language_file(&path), Language::English);
        save_language_file(&path, Language::Russian).unwrap();
        assert_eq!(language_file(&path), Language::Russian);
        assert_eq!(
            default_vault_file(&path).unwrap(),
            root.path().join("vault.kdbx")
        );
        assert!(!root.path().join("TermVault").exists());
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
        assert!(save_language_file(Path::new("relative.json"), Language::English).is_err());
    }
    #[test]
    fn language_defaults_and_atomic_merge_preserve_vault_and_other_settings() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(language(root.path()), Language::English);
        save_language(root.path(), Language::Russian).unwrap();
        assert_eq!(language(root.path()), Language::Russian);
        assert_eq!(
            default_vault(root.path()).unwrap(),
            root.path().join("TermVault/vault.kdbx")
        );
        let file = root.path().join("TermVault/settings.json");
        let vault = root.path().join("chosen.kdbx");
        std::fs::write(
            &file,
            serde_json::to_vec(&serde_json::json!({"vault":vault,"other":42})).unwrap(),
        )
        .unwrap();
        assert_eq!(language(root.path()), Language::English);
        save_language(root.path(), Language::Russian).unwrap();
        let settings: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(settings["other"], 42);
        assert_eq!(default_vault(root.path()).unwrap(), vault);
        assert!(!vault.exists());
        save_language(root.path(), Language::English).unwrap();
        assert_eq!(language(root.path()), Language::English);
    }
    #[test]
    fn invalid_language_falls_back_and_bad_settings_are_never_overwritten() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("TermVault");
        std::fs::create_dir(&folder).unwrap();
        let file = folder.join("settings.json");
        let vault = root.path().join("chosen.kdbx");
        for value in [
            Value::Null,
            Value::Bool(true),
            Value::from("unknown"),
            Value::from(12),
        ] {
            std::fs::write(
                &file,
                serde_json::to_vec(&serde_json::json!({"vault":vault,"language":value})).unwrap(),
            )
            .unwrap();
            assert_eq!(language(root.path()), Language::English);
            assert_eq!(default_vault(root.path()).unwrap(), vault);
            save_language(root.path(), Language::Russian).unwrap();
        }
        for invalid in [
            "not json".into(),
            "[]".into(),
            r#"{"vault":false}"#.into(),
            r#"{"vault":"relative.kdbx"}"#.into(),
            " ".repeat(8193),
        ] {
            std::fs::write(&file, &invalid).unwrap();
            assert_eq!(language(root.path()), Language::English);
            assert!(save_language(root.path(), Language::Russian).is_err());
            assert_eq!(std::fs::read(&file).unwrap(), invalid.as_bytes());
        }
    }
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
