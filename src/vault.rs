use anyhow::{bail, Context, Result};
use keepass::{
    config::{CompressionConfig, DatabaseConfig, KdfConfig},
    db::{fields, EntryId, GroupId},
    Database, DatabaseKey,
};
use std::{
    collections::HashMap,
    fs::{self, File, Metadata},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
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
    pub(crate) fn validate(&self) -> Result<()> {
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

/// Reauthentication is tied to one path/revision and expires before final confirmation.
pub struct DeleteAuthorization {
    path: PathBuf,
    revision: Vec<u8>, // Ciphertext only; no master passphrase retained.
    expires: Instant,
}

fn validate_master(password: &str) -> Result<()> {
    if password.chars().count() < 12
        || password.len() > 1024
        || !crate::input::valid(password, false)
    {
        bail!("Use a master passphrase of at least 12 characters (maximum 1024 bytes)");
    }
    Ok(())
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
        validate_master(password)?;
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
    /// Distinguish an absent title from user text equal to a UI placeholder.
    pub fn entry_has_title(&self, id: EntryId) -> bool {
        self.db
            .entry(id)
            .is_some_and(|entry| entry.get_title().is_some())
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
    /// Search only these four fields of the current entry, not passwords,
    /// custom fields, or entry history. The caller folds the query once.
    pub fn matches_search(&self, id: EntryId, lowercase_query: &str) -> bool {
        self.db.entry(id).is_some_and(|entry| {
            [
                entry.get_title(),
                entry.get_username(),
                entry.get_url(),
                entry.get(fields::NOTES),
            ]
            .into_iter()
            .flatten()
            .any(|field| Zeroizing::new(field.to_lowercase()).contains(lowercase_query))
        })
    }
    pub fn root_folder(&self) -> GroupId {
        self.db.root().id()
    }
    pub fn has_folders(&self) -> bool {
        self.db.num_groups() > 1
    }
    pub fn folder_parent(&self, id: GroupId) -> Option<GroupId> {
        self.db.group(id)?.parent().map(|parent| parent.id())
    }
    pub fn folder_children(&self, parent: GroupId) -> Vec<(GroupId, String)> {
        let Some(group) = self.db.group(parent) else {
            return Vec::new();
        };
        let mut children: Vec<_> = group
            .groups()
            .map(|child| (child.id(), crate::input::display(&child.name)))
            .collect();
        children.sort_by(|a, b| {
            a.1.to_lowercase()
                .cmp(&b.1.to_lowercase())
                .then(a.0.to_string().cmp(&b.0.to_string()))
        });
        children
    }
    pub fn entry_folder(&self, id: EntryId) -> Option<GroupId> {
        self.db.entry(id).map(|entry| entry.parent().id())
    }
    pub fn folders(&self) -> Vec<(GroupId, String)> {
        let mut folders: Vec<_> = self
            .db
            .iter_all_groups()
            .map(|group| {
                let id = group.id();
                let mut names = Vec::new();
                let mut current = Some(id);
                while let Some(current_id) = current {
                    let group = self.db.group(current_id).expect("Known folder ID");
                    if group.parent().is_some() {
                        names.push(crate::input::display(&group.name));
                    }
                    current = group.parent().map(|parent| parent.id());
                }
                names.reverse();
                (id, format!("/{}", names.join(" / ")))
            })
            .collect();
        folders.sort_by(|a, b| {
            a.1.to_lowercase()
                .cmp(&b.1.to_lowercase())
                .then(a.0.to_string().cmp(&b.0.to_string()))
        });
        folders
    }
    pub fn create_folder(&mut self, parent: Option<GroupId>, name: &str) -> Result<GroupId> {
        if self.readonly {
            bail!("Foreign vault is read-only");
        }
        let name = name.trim();
        if name.is_empty()
            || name.len() > 128
            || !crate::input::valid(name, false)
            || name.contains(['/', '\\'])
            || matches!(name, "." | "..")
        {
            bail!("Folder name must contain 1–128 bytes without slashes or control characters");
        }
        if self.db.num_groups() >= 100 {
            bail!("Vault limit: 100 folders including root");
        }
        let parent = parent.unwrap_or_else(|| self.root_folder());
        let group = self
            .db
            .group(parent)
            .context("Parent folder no longer exists")?;
        if group
            .groups()
            .any(|group| group.name.to_lowercase() == name.to_lowercase())
        {
            bail!("A folder with this name already exists here");
        }
        let mut candidate = self.db.clone();
        let id = {
            let mut parent = candidate
                .group_mut(parent)
                .context("Parent folder no longer exists")?;
            let mut group = parent.add_group();
            group.name = name.into();
            group.id()
        };
        self.commit(candidate)?;
        Ok(id)
    }
    pub fn folder_contents(&self, id: GroupId) -> Option<(usize, usize)> {
        let group = self.db.group(id)?;
        let mut entries = group.entries().count();
        let mut folders = 0;
        for child in group.groups() {
            let (child_entries, child_folders) = self.folder_contents(child.id())?;
            entries += child_entries;
            folders += child_folders + 1;
        }
        Some((entries, folders))
    }
    pub fn delete_folder(&mut self, id: GroupId) -> Result<GroupId> {
        if self.readonly {
            bail!("Foreign vault is read-only");
        }
        let parent = self
            .db
            .group(id)
            .context("Folder no longer exists")?
            .parent()
            .context("The root folder cannot be deleted")?
            .id();
        let mut candidate = self.db.clone();
        candidate
            .group_mut(id)
            .context("Folder no longer exists")?
            .remove();
        self.commit(candidate)?;
        Ok(parent)
    }
    pub fn move_entry(&mut self, id: EntryId, folder: GroupId) -> Result<()> {
        if self.readonly {
            bail!("Foreign vault is read-only");
        }
        if self.db.group(folder).is_none() {
            bail!("Destination folder no longer exists");
        }
        let mut candidate = self.db.clone();
        candidate
            .entry_mut(id)
            .context("Entry no longer exists")?
            .move_to(folder)?;
        self.commit(candidate)
    }
    pub fn update(&mut self, id: Option<EntryId>, data: &EntryData) -> Result<EntryId> {
        self.update_in_folder(id, data, None)
    }
    pub fn update_in_folder(
        &mut self,
        id: Option<EntryId>,
        data: &EntryData,
        folder: Option<GroupId>,
    ) -> Result<EntryId> {
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
            None => candidate
                .group_mut(folder.unwrap_or_else(|| self.root_folder()))
                .context("Folder no longer exists")?
                .add_entry()
                .id(),
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
    /// Append only; a failed validation/save leaves the live database unchanged.
    pub fn import_csv(
        &mut self,
        preview: &crate::csv_transfer::ImportPreview,
        folder: Option<GroupId>,
    ) -> Result<Vec<EntryId>> {
        if self.readonly {
            bail!("Foreign vault is read-only");
        }
        crate::csv_transfer::check_ancestors(&self.path)?;
        if self.db.num_entries().saturating_add(preview.len()) > 2000 {
            bail!("Vault limit: 2000 entries");
        }
        let folder = folder.unwrap_or_else(|| self.root_folder());
        if self.db.group(folder).is_none() {
            bail!("Folder no longer exists");
        }
        for (index, data) in preview.entries.iter().enumerate() {
            data.validate()
                .map_err(|_| anyhow::anyhow!("Invalid CSV fields at row {}", index + 2))?;
        }
        if preview.is_empty() {
            if read(&self.path)? != self.revision {
                bail!("Vault changed on disk; lock and reopen it");
            }
            return Ok(Vec::new());
        }
        let mut candidate = self.db.clone();
        let mut ids = Vec::with_capacity(preview.len());
        for data in &preview.entries {
            let id = candidate
                .group_mut(folder)
                .context("Folder no longer exists")?
                .add_entry()
                .id();
            let mut entry = candidate.entry_mut(id).context("Entry no longer exists")?;
            for (field, value) in [
                (fields::TITLE, &*data.title),
                (fields::USERNAME, &*data.username),
                (fields::PASSWORD, &*data.password),
                (fields::URL, &*data.url),
                (fields::NOTES, &*data.notes),
            ] {
                entry.set_protected(field, value.to_owned());
            }
            ids.push(id);
        }
        self.commit(candidate)?;
        Ok(ids)
    }
    /// Plaintext export, deliberately requiring the current master even when unlocked.
    pub fn export_csv(&self, path: &Path, master: &str) -> Result<usize> {
        if self.readonly {
            bail!("Foreign vault is read-only");
        }
        crate::csv_transfer::check_ancestors(&self.path)?;
        self.verify_master(master).map_err(|_| {
            anyhow::anyhow!("Cannot export: current master is incorrect or vault changed")
        })?;
        let mut ids: Vec<_> = self.db.iter_all_entries().map(|entry| entry.id()).collect();
        ids.sort_by_key(|id| id.to_string());
        let bytes = crate::csv_transfer::encode(ids.iter().filter_map(|id| self.entry(*id)))?;
        // Recheck after encoding and before creating any plaintext file.
        if read(&self.path).map_err(|_| anyhow::anyhow!("Cannot verify current vault"))?
            != self.revision
        {
            bail!("Vault changed on disk; export cancelled");
        }
        crate::csv_transfer::persist_export(path, &bytes)?;
        Ok(ids.len())
    }
    fn verify_master(&self, password: &str) -> Result<()> {
        if password.len() > 1024 || !crate::input::valid(password, false) {
            bail!("Invalid current master passphrase");
        }
        if read(&self.path)? != self.revision {
            bail!("Vault changed on disk; lock and reopen it before retrying");
        }
        let key = DatabaseKey::new().with_password(password);
        Database::open(&mut self.revision.as_slice(), key)
            .map_err(|_| anyhow::anyhow!("Current master passphrase is incorrect"))?;
        Ok(())
    }
    pub fn change_master(&mut self, current: &str, new: &str) -> Result<()> {
        if self.readonly {
            bail!("Foreign vault is read-only");
        }
        validate_master(new)?;
        self.verify_master(current)?;
        if current == new {
            bail!("The new master passphrase must be different");
        }
        // Keep the old in-memory key until the new encrypted file has been verified and committed.
        self.commit_with_key(self.db.clone(), DatabaseKey::new().with_password(new))
    }
    pub fn authorize_delete(&self, current: &str, filename: &str) -> Result<DeleteAuthorization> {
        if self.readonly {
            bail!("Foreign vault is read-only");
        }
        let expected = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .context("Vault filename cannot be confirmed in this interface")?;
        if filename != expected {
            bail!("The typed filename does not match the vault filename");
        }
        self.verify_master(current)?;
        Ok(DeleteAuthorization {
            path: self.path.clone(),
            revision: self.revision.clone(),
            expires: Instant::now() + Duration::from_secs(60),
        })
    }
    pub fn remove_vault(&self, authorization: &DeleteAuthorization) -> Result<PathBuf> {
        if self.readonly {
            bail!("Foreign vault is read-only");
        }
        if authorization.path != self.path
            || authorization.revision != self.revision
            || Instant::now() >= authorization.expires
        {
            bail!("Delete confirmation expired or no longer matches; authenticate again");
        }
        if read(&self.path)? != self.revision {
            bail!("Vault changed on disk; deletion cancelled");
        }
        let parent = self.path.parent().context("Missing vault parent")?;
        for _ in 0..32 {
            let mut random = [0u8; 12];
            getrandom::fill(&mut random)
                .map_err(|_| anyhow::anyhow!("System randomness unavailable"))?;
            let suffix: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
            let backup = parent.join(format!(
                "{}.backup-{suffix}.kdbx",
                self.path.file_name().unwrap().to_string_lossy()
            ));
            match fs::symlink_metadata(&backup) {
                Ok(_) => continue,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => bail!("Cannot check encrypted backup destination; deletion cancelled"),
            }
            if read(&self.path)? != self.revision {
                bail!("Vault changed on disk; deletion cancelled");
            }
            // Remove the active filename by moving the actual ciphertext to a retained backup.
            // Even a concurrent source replacement is preserved, not destroyed. Trusted parent
            // storage and no concurrent writers remain required, as with normal vault saves.
            fs::rename(&self.path, &backup)
                .context("Cannot move vault to encrypted backup; deletion cancelled")?;
            return Ok(backup);
        }
        bail!("Cannot allocate an encrypted backup name; deletion cancelled")
    }
    fn commit(&mut self, candidate: Database) -> Result<()> {
        self.commit_with_key(candidate, self.key.clone())
    }
    fn commit_with_key(&mut self, candidate: Database, key: DatabaseKey) -> Result<()> {
        if read(&self.path)? != self.revision {
            bail!("Vault changed on disk; lock and reopen it. Your draft is still in memory");
        }
        let encrypted = encrypt(&candidate, &key)?;
        let parent = self.path.parent().context("Missing vault parent")?;
        let temporary = encrypted_temp(parent, &encrypted)?;
        // Verify the encrypted candidate before replacing anything.
        let verification = Database::open(&mut encrypted.as_slice(), key.clone())
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
        self.key = key;
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
    fn nested_folders_and_entry_moves_survive_reopen_without_losing_fields() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("folders.kdbx");
        let mut vault = Session::create(&path, MASTER).unwrap();
        let work = vault.create_folder(None, "Работа").unwrap();
        let mail = vault.create_folder(Some(work), "Почта").unwrap();
        let id = vault.update_in_folder(None, &data(), Some(mail)).unwrap();
        assert_eq!(vault.entry_folder(id), Some(mail));
        vault.update(Some(id), &data()).unwrap();
        assert_eq!(vault.entry_folder(id), Some(mail));
        let reopened = Session::unlock(&path, MASTER).unwrap();
        assert!(reopened
            .folders()
            .contains(&(mail, "/Работа / Почта".into())));
        assert_eq!(reopened.entry_folder(id), Some(mail));
        assert_eq!(
            &*reopened.entry(id).unwrap().password,
            "UNIQUE_PASSWORD_NOT_PLAINTEXT"
        );
        let before_move = read(&path).unwrap();
        vault.move_entry(id, vault.root_folder()).unwrap();
        let reopened = Session::unlock(&path, MASTER).unwrap();
        assert_eq!(reopened.entry_folder(id), Some(reopened.root_folder()));
        assert_eq!(reopened.entries().len(), 1);
        let backup = fs::read_dir(root.path())
            .unwrap()
            .map(|item| item.unwrap().path())
            .find(|candidate| candidate != &path && fs::read(candidate).unwrap() == before_move)
            .unwrap();
        assert_eq!(
            Session::unlock(&backup, MASTER).unwrap().entry_folder(id),
            Some(mail)
        );
        for secret in ["Работа", "Почта", "UNIQUE_PASSWORD_NOT_PLAINTEXT"] {
            assert!(!read(&path)
                .unwrap()
                .windows(secret.len())
                .any(|window| window == secret.as_bytes()));
        }
    }
    #[test]
    fn folder_deletion_removes_only_its_subtree_and_retains_recoverable_backup() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("folders.kdbx");
        let mut vault = Session::create(&path, MASTER).unwrap();
        let folder = vault.create_folder(None, "Delete me").unwrap();
        let nested = vault.create_folder(Some(folder), "Nested").unwrap();
        let direct = vault.update_in_folder(None, &data(), Some(folder)).unwrap();
        let child = vault.update_in_folder(None, &data(), Some(nested)).unwrap();
        let retained = vault.update(None, &data()).unwrap();
        assert_eq!(vault.folder_contents(folder), Some((2, 1)));
        let before = read(&path).unwrap();
        assert_eq!(vault.delete_folder(folder).unwrap(), vault.root_folder());
        let reopened = Session::unlock(&path, MASTER).unwrap();
        assert!(reopened.entry(direct).is_none());
        assert!(reopened.entry(child).is_none());
        assert!(reopened.folder_contents(folder).is_none());
        assert!(reopened.folder_contents(nested).is_none());
        assert!(reopened.entry(retained).is_some());
        let backup = fs::read_dir(root.path())
            .unwrap()
            .map(|item| item.unwrap().path())
            .find(|candidate| candidate != &path && fs::read(candidate).unwrap() == before)
            .unwrap();
        let restored = Session::unlock(&backup, MASTER).unwrap();
        assert_eq!(restored.folder_contents(folder), Some((2, 1)));
        assert!(restored.entry(direct).is_some());
        assert!(restored.entry(child).is_some());
    }

    #[test]
    fn folder_deletion_rejects_root_readonly_and_external_conflicts() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("folders.kdbx");
        let mut vault = Session::create(&path, MASTER).unwrap();
        let folder = vault.create_folder(None, "Empty").unwrap();
        let before = read(&path).unwrap();
        assert!(vault.delete_folder(vault.root_folder()).is_err());
        vault.readonly = true;
        assert!(vault.delete_folder(folder).is_err());
        assert_eq!(read(&path).unwrap(), before);
        vault.readonly = false;
        let mut other = Session::unlock(&path, MASTER).unwrap();
        other.update_in_folder(None, &data(), Some(folder)).unwrap();
        let changed = read(&path).unwrap();
        assert!(vault.delete_folder(folder).is_err());
        assert_eq!(read(&path).unwrap(), changed);
        assert_eq!(vault.folder_contents(folder), Some((0, 0)));
        let mut reopened = Session::unlock(&path, MASTER).unwrap();
        let empty = reopened.create_folder(None, "Truly empty").unwrap();
        reopened.delete_folder(empty).unwrap();
        assert!(reopened.folder_contents(empty).is_none());
    }

    #[test]
    fn folder_validation_readonly_and_external_conflicts_do_not_modify_the_vault() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("folders.kdbx");
        let mut vault = Session::create(&path, MASTER).unwrap();
        let folder = vault.create_folder(None, "Work").unwrap();
        let id = vault.update(None, &data()).unwrap();
        let before = read(&path).unwrap();
        for name in ["", "  ", ".", "..", "a/b", "a\\b", "a\nb", "WORK"] {
            assert!(vault.create_folder(None, name).is_err());
            assert_eq!(read(&path).unwrap(), before);
        }
        assert!(vault.create_folder(None, &"x".repeat(129)).is_err());
        vault.readonly = true;
        assert!(vault.create_folder(None, "Private").is_err());
        assert!(vault.move_entry(id, folder).is_err());
        assert_eq!(read(&path).unwrap(), before);
        vault.readonly = false;
        let mut other = Session::unlock(&path, MASTER).unwrap();
        other.create_folder(None, "Other").unwrap();
        let changed = read(&path).unwrap();
        assert!(vault.create_folder(None, "Stale").is_err());
        assert!(vault.move_entry(id, folder).is_err());
        assert_eq!(vault.entry_folder(id), Some(vault.root_folder()));
        assert_eq!(read(&path).unwrap(), changed);
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
    fn title_presence_distinguishes_missing_metadata_from_literal_placeholder_text() {
        let root = tempfile::tempdir().unwrap();
        let mut vault = Session::create(&root.path().join("titles.kdbx"), MASTER).unwrap();
        let missing = vault.db.root_mut().add_entry().id();
        let literal = vault.db.root_mut().add_entry().id();
        vault
            .db
            .entry_mut(literal)
            .unwrap()
            .set_protected(fields::TITLE, "Untitled");
        assert!(!vault.entry_has_title(missing));
        assert!(vault.entry_has_title(literal));
        // Storage never translates either entry; only UI-generated fallback text
        // uses the presence bit, not a lookup on the user's authored title.
        assert_eq!(
            vault
                .entries()
                .iter()
                .filter(|(_, title)| title == "Untitled")
                .count(),
            2
        );
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
    #[test]
    fn master_change_reopens_with_new_key_preserves_records_and_keeps_old_encrypted_backup() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("rotate.kdbx");
        let mut vault = Session::create(&path, MASTER).unwrap();
        let id = vault.update(None, &data()).unwrap();
        let before = read(&path).unwrap();
        let new = "different master passphrase";
        vault.change_master(MASTER, new).unwrap();
        assert!(Session::unlock(&path, MASTER).is_err());
        let reopened = Session::unlock(&path, new).unwrap();
        assert_eq!(
            &*reopened.entry(id).unwrap().password,
            "UNIQUE_PASSWORD_NOT_PLAINTEXT"
        );
        let backup = fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|candidate| candidate != &path && fs::read(candidate).unwrap() == before)
            .unwrap();
        assert!(Session::unlock(&backup, MASTER).is_ok());
        assert!(Session::unlock(&backup, new).is_err());
        vault.update(Some(id), &data()).unwrap(); // The live session must also use the NEW key.
        assert!(Session::unlock(&path, new).is_ok());
        assert!(Session::unlock(&path, MASTER).is_err());
        let bytes = read(&path).unwrap();
        for secret in [MASTER, new, "UNIQUE_PASSWORD_NOT_PLAINTEXT"] {
            assert!(!bytes
                .windows(secret.len())
                .any(|window| window == secret.as_bytes()));
        }
    }
    #[test]
    fn failed_master_changes_and_readonly_actions_never_modify_the_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("test.kdbx");
        let mut vault = Session::create(&path, MASTER).unwrap();
        let before = read(&path).unwrap();
        for (current, new) in [
            ("incorrect", "different master passphrase"),
            (MASTER, "short"),
            (MASTER, MASTER),
        ] {
            assert!(vault.change_master(current, new).is_err());
            assert_eq!(read(&path).unwrap(), before);
            assert_eq!(vault.revision, before);
        }
        vault.readonly = true;
        assert!(vault
            .change_master(MASTER, "different master passphrase")
            .is_err());
        assert!(vault.authorize_delete(MASTER, "test.kdbx").is_err());
        assert_eq!(read(&path).unwrap(), before);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
    #[test]
    fn master_change_refuses_external_modifications() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("test.kdbx");
        let mut stale = Session::create(&path, MASTER).unwrap();
        let mut other = Session::unlock(&path, MASTER).unwrap();
        other.update(None, &data()).unwrap();
        let before = read(&path).unwrap();
        assert!(stale
            .change_master(MASTER, "different master passphrase")
            .is_err());
        assert_eq!(read(&path).unwrap(), before);
        assert!(Session::unlock(&path, MASTER).is_ok());
    }
    #[test]
    fn deletion_requires_current_master_exact_filename_and_a_fresh_bound_authorization() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("test.kdbx");
        let vault = Session::create(&path, MASTER).unwrap();
        let other_path = root.path().join("other.kdbx");
        let other = Session::create(&other_path, MASTER).unwrap();
        let before = read(&path).unwrap();
        assert!(vault.authorize_delete("wrong", "test.kdbx").is_err());
        assert!(vault.authorize_delete(MASTER, "TEST.kdbx").is_err());
        let mut authorization = vault.authorize_delete(MASTER, "test.kdbx").unwrap();
        assert!(other.remove_vault(&authorization).is_err());
        authorization.expires = Instant::now() - Duration::from_secs(1);
        assert!(vault.remove_vault(&authorization).is_err());
        assert_eq!(read(&path).unwrap(), before);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
    }
    #[test]
    fn deletion_preserves_exact_ciphertext_and_unrelated_files_and_cannot_be_replayed() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("test.kdbx");
        let mut vault = Session::create(&path, MASTER).unwrap();
        vault.update(None, &data()).unwrap();
        let before = read(&path).unwrap();
        let unrelated = root.path().join("do-not-touch.txt");
        fs::write(&unrelated, "KEEP").unwrap();
        let count = fs::read_dir(root.path()).unwrap().count();
        let authorization = vault.authorize_delete(MASTER, "test.kdbx").unwrap();
        let backup = vault.remove_vault(&authorization).unwrap();
        assert!(!path.exists());
        assert_eq!(read(&backup).unwrap(), before);
        assert_eq!(Session::unlock(&backup, MASTER).unwrap().entries().len(), 1);
        assert_eq!(fs::read_to_string(&unrelated).unwrap(), "KEEP");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), count);
        assert!(vault.remove_vault(&authorization).is_err());
    }
    #[test]
    fn deletion_refuses_a_file_changed_after_reauthentication() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("test.kdbx");
        let vault = Session::create(&path, MASTER).unwrap();
        let authorization = vault.authorize_delete(MASTER, "test.kdbx").unwrap();
        let mut other = Session::unlock(&path, MASTER).unwrap();
        other.update(None, &data()).unwrap();
        let before = read(&path).unwrap();
        assert!(vault.remove_vault(&authorization).is_err());
        assert_eq!(read(&path).unwrap(), before);
    }
}
