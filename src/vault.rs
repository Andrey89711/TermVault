use anyhow::{bail, Context, Result};
use keepass::{
    config::{CompressionConfig, DatabaseConfig, KdfConfig},
    db::{fields, EntryId},
    Database, DatabaseKey,
};
use std::{
    collections::HashMap,
    fs::{self, File, Metadata},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use tempfile::NamedTempFile;
use zeroize::Zeroizing;

const MAX_FILE: usize = 8 * 1024 * 1024;
const GENERATOR: &str = "TermVault/0.1";
const SIGNATURE: &[u8] = &[0x03, 0xd9, 0xa2, 0x9a, 0x67, 0xfb, 0x4b, 0xb5];
const ARGON2ID: &[u8] = &[
    0x9e, 0x29, 0x8b, 0x19, 0x56, 0xdb, 0x47, 0x73, 0xb2, 0x3d, 0xfc, 0x3e, 0xc6, 0xf0, 0xa1, 0xe6,
];

#[derive(Default)]
pub struct EntryData {
    pub title: Zeroizing<String>,
    pub username: Zeroizing<String>,
    pub password: Zeroizing<String>,
    pub url: Zeroizing<String>,
    pub notes: Zeroizing<String>,
}

impl EntryData {
    fn validate(&self) -> Result<()> {
        if self.title.trim().is_empty() || self.title.len() > 256 {
            bail!("Title is required (maximum 256 bytes)");
        }
        for (value, limit, multiline) in [
            (&*self.title, 256, false),
            (&*self.username, 1024, false),
            (&*self.password, 1024, false),
            (&*self.url, 2048, false),
            (&*self.notes, 8192, true),
        ] {
            if value.len() > limit || !crate::input::valid(value, multiline) {
                bail!("A field exceeds its size limit or contains unsupported control characters");
            }
        }
        Ok(())
    }
}

pub struct Session {
    path: PathBuf,
    db: Database,
    key: DatabaseKey,
    revision: Vec<u8>, // Ciphertext only.
    pub readonly: bool,
}

pub fn inspect(path: &Path) -> Result<()> {
    if !path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("kdbx"))
    {
        bail!("Choose a .kdbx file");
    }
    let metadata = fs::symlink_metadata(path)
        .context("Vault not found; use --create to explicitly create it")?;
    if is_link(&metadata) || !metadata.is_file() {
        bail!("Vault must be a regular file, not a symlink/reparse point");
    }
    if metadata.len() > MAX_FILE as u64 {
        bail!("Vault exceeds the 8 MiB MVP limit");
    }
    Ok(())
}

fn is_link(meta: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_type().is_symlink() || meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}

fn read(path: &Path) -> Result<Vec<u8>> {
    inspect(path)?;
    let mut bytes = Vec::new();
    File::open(path)?
        .take((MAX_FILE + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_FILE {
        bail!("Vault exceeds the 8 MiB MVP limit");
    }
    Ok(bytes)
}

fn config() -> DatabaseConfig {
    let mut config = DatabaseConfig::default();
    config.compression_config = CompressionConfig::None;
    config.kdf_config = KdfConfig::Argon2id {
        iterations: if cfg!(test) { 1 } else { 3 },
        memory: if cfg!(test) {
            8 * 1024 * 1024
        } else {
            64 * 1024 * 1024
        },
        parallelism: 1,
        version: argon2::Version::Version13,
    };
    config
}

impl Session {
    pub fn create(path: &Path, password: &str) -> Result<Self> {
        if password.chars().count() < 12
            || password.len() > 1024
            || !crate::input::valid(password, false)
        {
            bail!("Use a master passphrase of at least 12 characters (maximum 1024 bytes)");
        }
        if !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("kdbx"))
        {
            bail!("Choose a .kdbx filename");
        }
        if fs::symlink_metadata(path).is_ok() {
            bail!("Destination already exists; refusing to overwrite it");
        }
        let parent = path.parent().context("Vault has no parent directory")?;
        fs::create_dir_all(parent).context("Cannot create the vault directory")?;
        let path = parent
            .canonicalize()?
            .join(path.file_name().context("Missing vault filename")?);
        let mut db = Database::with_config(config());
        db.meta.generator = Some(GENERATOR.into());
        db.root_mut().name = "TermVault".into();
        let key = DatabaseKey::new().with_password(password);
        let bytes = encrypt(&db, &key)?;
        let temporary = encrypted_temp(path.parent().unwrap(), &bytes)?;
        temporary
            .persist_noclobber(&path)
            .map_err(|_| anyhow::anyhow!("Cannot create vault; destination may already exist"))?;
        Ok(Self {
            path,
            db,
            key,
            revision: bytes,
            readonly: false,
        })
    }

    pub fn unlock(path: &Path, password: &str) -> Result<Self> {
        if password.len() > 1024 {
            bail!("Master password exceeds the input limit");
        }
        let revision = read(path)?;
        preflight(&revision)?;
        let key = DatabaseKey::new().with_password(password);
        let db = Database::open(&mut revision.as_slice(), key.clone()).map_err(|_| {
            anyhow::anyhow!("Cannot unlock: wrong master password or damaged/unsupported vault")
        })?;
        if db.num_entries() > 2000 || db.num_groups() > 100 {
            bail!("Vault exceeds MVP entry/group limits");
        }
        let readonly = db.meta.generator.as_deref() != Some(GENERATOR);
        let parent = path
            .parent()
            .context("Missing vault directory")?
            .canonicalize()?;
        let path = parent.join(path.file_name().context("Missing vault filename")?);
        Ok(Self {
            path,
            db,
            key,
            revision,
            readonly,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn entries(&self) -> Vec<(EntryId, String)> {
        let mut entries: Vec<_> = self
            .db
            .iter_all_entries()
            .map(|entry| {
                (
                    entry.id(),
                    crate::input::display(entry.get_title().unwrap_or("Untitled")),
                )
            })
            .collect();
        entries.sort_by(|a, b| {
            a.1.to_lowercase()
                .cmp(&b.1.to_lowercase())
                .then(a.0.to_string().cmp(&b.0.to_string()))
        });
        entries
    }
    pub fn entry(&self, id: EntryId) -> Option<EntryData> {
        let entry = self.db.entry(id)?;
        Some(EntryData {
            title: Zeroizing::new(entry.get_title().unwrap_or("").into()),
            username: Zeroizing::new(entry.get_username().unwrap_or("").into()),
            password: Zeroizing::new(entry.get_password().unwrap_or("").into()),
            url: Zeroizing::new(entry.get_url().unwrap_or("").into()),
            notes: Zeroizing::new(entry.get(fields::NOTES).unwrap_or("").into()),
        })
    }
    pub fn update(&mut self, id: Option<EntryId>, data: &EntryData) -> Result<EntryId> {
        if self.readonly {
            bail!("Foreign vault is read-only in this MVP");
        }
        data.validate()?;
        if id.is_none() && self.db.num_entries() >= 2000 {
            bail!("MVP limit: 2000 entries");
        }
        let mut candidate = self.db.clone();
        let entry_id = match id {
            Some(id) => id,
            None => candidate.root_mut().add_entry().id(),
        };
        {
            let mut entry = candidate
                .entry_mut(entry_id)
                .context("Entry no longer exists")?;
            // Protect every user field, not only the password, in KDBX's inner stream.
            for (field, value) in [
                (fields::TITLE, &*data.title),
                (fields::USERNAME, &*data.username),
                (fields::PASSWORD, &*data.password),
                (fields::URL, &*data.url),
                (fields::NOTES, &*data.notes),
            ] {
                entry.set_protected(field, value.to_owned());
            }
        }
        self.commit(candidate)?;
        Ok(entry_id)
    }
    pub fn delete(&mut self, id: EntryId) -> Result<()> {
        if self.readonly {
            bail!("Foreign vault is read-only in this MVP");
        }
        let mut candidate = self.db.clone();
        candidate
            .entry_mut(id)
            .context("Entry no longer exists")?
            .remove();
        self.commit(candidate)
    }
    fn commit(&mut self, candidate: Database) -> Result<()> {
        if read(&self.path)? != self.revision {
            bail!("Vault changed on disk; lock and reopen it. Your draft is still in memory");
        }
        let encrypted = encrypt(&candidate, &self.key)?;
        let parent = self.path.parent().context("Missing vault parent")?;
        let temporary = encrypted_temp(parent, &encrypted)?;
        // Verify the encrypted candidate before replacing anything.
        let verification = Database::open(&mut encrypted.as_slice(), self.key.clone())
            .map_err(|_| anyhow::anyhow!("Encrypted save verification failed"))?;
        if verification != candidate {
            bail!("Encrypted save verification failed");
        }
        drop(verification);
        if read(&self.path)? != self.revision {
            bail!("Vault changed on disk; no changes were written");
        }
        let mut random = [0u8; 12];
        getrandom::fill(&mut random)
            .map_err(|_| anyhow::anyhow!("System randomness unavailable"))?;
        let suffix: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let backup = parent.join(format!(
            "{}.backup-{suffix}.kdbx",
            self.path.file_name().unwrap().to_string_lossy()
        ));
        encrypted_temp(parent, &self.revision)?
            .persist_noclobber(backup)
            .map_err(|_| anyhow::anyhow!("Cannot create encrypted backup; save cancelled"))?;
        if read(&self.path)? != self.revision {
            bail!("Vault changed on disk; no changes were written");
        }
        temporary
            .persist(&self.path)
            .map_err(|_| anyhow::anyhow!("Cannot atomically replace vault; draft retained"))?;
        self.db = candidate;
        self.revision = encrypted;
        Ok(())
    }
}

fn encrypt(db: &Database, key: &DatabaseKey) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    db.save(&mut bytes, key.clone())
        .map_err(|_| anyhow::anyhow!("KDBX encryption failed; no changes written"))?;
    if bytes.len() > MAX_FILE {
        bail!("Encrypted vault exceeds the 8 MiB MVP limit");
    }
    Ok(bytes)
}

fn encrypted_temp(parent: &Path, bytes: &[u8]) -> Result<NamedTempFile> {
    let mut file =
        NamedTempFile::new_in(parent).context("Cannot create encrypted temporary file")?;
    file.write_all(bytes)
        .context("Cannot write encrypted file")?;
    file.as_file()
        .sync_all()
        .context("Cannot flush encrypted file")?;
    Ok(file)
}

// Validate unencrypted format metadata before invoking the KDBX parser/KDF.
// This is not cryptography. It narrows the MVP to uncompressed AES/Argon2id KDBX4,
// bounds KDF resources, and rejects malformed typed dictionaries before parsing.
fn take<'a>(bytes: &mut &'a [u8], count: usize) -> Result<&'a [u8]> {
    if bytes.len() < count {
        bail!("Truncated KDBX header");
    }
    let (part, rest) = bytes.split_at(count);
    *bytes = rest;
    Ok(part)
}
fn number(bytes: &[u8]) -> Result<u64> {
    match bytes.len() {
        4 => Ok(u32::from_le_bytes(bytes.try_into()?) as u64),
        8 => Ok(u64::from_le_bytes(bytes.try_into()?)),
        _ => bail!("Invalid KDBX integer"),
    }
}
fn preflight(bytes: &[u8]) -> Result<()> {
    let mut data = bytes;
    if take(&mut data, 8)? != SIGNATURE || number(take(&mut data, 4)?)? >> 16 != 4 {
        bail!("Only KDBX4 vaults are supported");
    }
    let mut header = HashMap::new();
    loop {
        if bytes.len() - data.len() > 64 * 1024 {
            bail!("KDBX header too large");
        }
        let kind = take(&mut data, 1)?[0];
        let size = number(take(&mut data, 4)?)? as usize;
        if size > 16 * 1024 {
            bail!("KDBX header field too large");
        }
        let value = take(&mut data, size)?;
        if kind == 0 {
            break;
        }
        if header.insert(kind, value).is_some() {
            bail!("Duplicate KDBX header field");
        }
    }
    if header.contains_key(&12) {
        bail!("Public custom header data is unsupported in this MVP");
    }
    if header.get(&3).copied() != Some(&[0, 0, 0, 0][..]) {
        bail!("MVP supports uncompressed vaults only; do not import your main password database");
    }
    let aes = [
        0x31, 0xc1, 0xf2, 0xe6, 0xbf, 0x71, 0x43, 0x50, 0xbe, 0x58, 0x05, 0x21, 0x6a, 0xfc, 0x5a,
        0xff,
    ];
    if header.get(&2).copied() != Some(&aes[..]) {
        bail!("MVP supports AES-256 KDBX4 vaults only");
    }
    let mut dictionary = *header.get(&11).context("Missing KDF parameters")?;
    if take(&mut dictionary, 2)? != [0, 1] {
        bail!("Unsupported KDF dictionary version");
    }
    let mut fields = HashMap::new();
    loop {
        let kind = take(&mut dictionary, 1)?[0];
        if kind == 0 {
            if !dictionary.is_empty() {
                bail!("Invalid KDF dictionary terminator");
            }
            break;
        }
        let key_len = number(take(&mut dictionary, 4)?)? as usize;
        if key_len > 128 {
            bail!("KDF key too long");
        }
        let key = take(&mut dictionary, key_len)?;
        let value_len = number(take(&mut dictionary, 4)?)? as usize;
        let value = take(&mut dictionary, value_len)?;
        let valid = match kind {
            4 | 12 => value.len() == 4,
            5 | 13 => value.len() == 8,
            8 => value.len() == 1,
            0x18 | 0x42 => true,
            _ => false,
        };
        if !valid || fields.insert(key, value).is_some() {
            bail!("Invalid KDF dictionary");
        }
    }
    if fields.get(&b"$UUID"[..]).copied() != Some(ARGON2ID) {
        bail!("Only Argon2id vaults are supported in the MVP");
    }
    let value =
        |key: &[u8]| -> Result<u64> { number(fields.get(key).context("Missing KDF parameter")?) };
    if !(8 * 1024 * 1024..=128 * 1024 * 1024).contains(&value(b"M")?)
        || !(1..=10).contains(&value(b"I")?)
        || !(1..=8).contains(&value(b"P")?)
        || value(b"V")? != 0x13
    {
        bail!("KDF parameters exceed the MVP resource policy");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const MASTER: &str = "test master passphrase";
    fn data() -> EntryData {
        EntryData {
            title: Zeroizing::new("GitHub".into()),
            username: Zeroizing::new("demo@example.test".into()),
            password: Zeroizing::new("UNIQUE_PASSWORD_NOT_PLAINTEXT".into()),
            notes: Zeroizing::new("секретная заметка".into()),
            ..Default::default()
        }
    }
    #[test]
    fn encrypted_create_update_roundtrip_wrong_password_and_backup() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("test.kdbx");
        let mut vault = Session::create(&path, MASTER).unwrap();
        preflight(&read(&path).unwrap()).unwrap();
        let id = vault.update(None, &data()).unwrap();
        let bytes = read(&path).unwrap();
        let marker = b"UNIQUE_PASSWORD_NOT_PLAINTEXT";
        assert!(!bytes.windows(marker.len()).any(|w| w == marker));
        assert!(Session::unlock(&path, "wrong").is_err());
        let reopened = Session::unlock(&path, MASTER).unwrap();
        assert_eq!(
            &*reopened.entry(id).unwrap().password,
            "UNIQUE_PASSWORD_NOT_PLAINTEXT"
        );
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
        assert!(Session::create(&path, MASTER).is_err());
        vault.delete(id).unwrap();
        assert!(Session::unlock(&path, MASTER).unwrap().entries().is_empty());
    }
    #[test]
    fn external_conflict_is_not_overwritten_and_session_is_unchanged() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("test.kdbx");
        let mut a = Session::create(&path, MASTER).unwrap();
        let mut b = Session::unlock(&path, MASTER).unwrap();
        a.update(None, &data()).unwrap();
        let bytes = read(&path).unwrap();
        assert!(b.update(None, &data()).is_err());
        assert!(b.entries().is_empty());
        assert_eq!(read(&path).unwrap(), bytes);
    }
    #[test]
    fn master_and_entry_validation_and_malformed_headers() {
        let root = tempfile::tempdir().unwrap();
        assert!(Session::create(&root.path().join("a.kdbx"), "short").is_err());
        assert!(data().validate().is_ok());
        assert!(EntryData::default().validate().is_err());
        for bytes in [vec![], vec![0; 12], SIGNATURE.to_vec()] {
            assert!(preflight(&bytes).is_err());
        }
    }
    #[test]
    fn foreign_generator_is_readonly() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("foreign.kdbx");
        let mut db = Database::with_config(config());
        db.meta.generator = Some("Another application".into());
        fs::write(
            &path,
            encrypt(&db, &DatabaseKey::new().with_password(MASTER)).unwrap(),
        )
        .unwrap();
        let mut vault = Session::unlock(&path, MASTER).unwrap();
        assert!(vault.readonly);
        assert!(vault.update(None, &data()).is_err());
    }

    #[test]
    fn resource_policy_rejects_tampered_kdf_before_decryption() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("test.kdbx");
        Session::create(&path, MASTER).unwrap();
        let mut bytes = read(&path).unwrap();
        let prefix = [5, 1, 0, 0, 0, b'M', 8, 0, 0, 0];
        let offset = bytes
            .windows(prefix.len())
            .position(|w| w == prefix)
            .unwrap()
            + prefix.len();
        bytes[offset..offset + 8].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(preflight(&bytes)
            .unwrap_err()
            .to_string()
            .contains("resource policy"));
        fs::write(&path, &bytes).unwrap();
        assert!(Session::unlock(&path, MASTER).is_err());
    }

    #[test]
    fn backup_reopens_old_revision_and_ciphertext_tampering_fails() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("test.kdbx");
        let mut vault = Session::create(&path, MASTER).unwrap();
        vault.update(None, &data()).unwrap();
        let backup = fs::read_dir(root.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p != &path)
            .unwrap();
        assert!(Session::unlock(&backup, MASTER)
            .unwrap()
            .entries()
            .is_empty());
        let mut bytes = read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0x80;
        fs::write(&path, &bytes).unwrap();
        assert!(Session::unlock(&path, MASTER).is_err());
    }
}
