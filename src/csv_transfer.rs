//! Five-field plaintext CSV transfer. Formula-looking values are NOT escaped:
//! callers must warn users not to open exports in spreadsheet applications.
use crate::vault::EntryData;
use anyhow::{bail, Result};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

const MAX_BYTES: usize = 8 * 1024 * 1024;
const HEADERS: [&str; 5] = ["title", "username", "password", "url", "notes"];

pub struct ImportPreview {
    pub(crate) entries: Vec<EntryData>,
}

impl ImportPreview {
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn is_link(meta: &fs::Metadata) -> bool {
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

/// Best-effort path checks; trusted parent directories and no concurrent path
/// replacements are required (portable std APIs cannot lock every ancestor).
pub(crate) fn check_ancestors(path: &Path) -> Result<()> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|_| anyhow::anyhow!("Cannot check CSV path"))?
            .join(path)
    };
    for ancestor in absolute.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(meta) if is_link(&meta) => bail!("Symlink/reparse paths are not allowed"),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && ancestor == absolute => {}
            Err(_) => bail!("Cannot check CSV path"),
        }
    }
    Ok(())
}

fn csv_path(path: &Path) -> Result<()> {
    // Requiring .csv also refuses raw vaults, backups, ADS suffixes, and accidental
    // exports to configuration files. No extension guessing.
    if !path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("csv"))
    {
        bail!("Choose a .csv filename");
    }
    #[cfg(windows)]
    if path
        .file_name()
        .is_some_and(|name| name.to_string_lossy().contains(':'))
    {
        bail!("Alternate data streams are not allowed");
    }
    check_ancestors(path)
}

pub fn read_import(path: &Path) -> Result<ImportPreview> {
    csv_path(path)?;
    let meta = fs::symlink_metadata(path).map_err(|_| anyhow::anyhow!("Cannot read CSV file"))?;
    if !meta.is_file() || is_link(&meta) {
        bail!("CSV must be a regular file, not a symlink/reparse point");
    }
    if meta.len() > MAX_BYTES as u64 {
        bail!("CSV exceeds the 8 MiB limit");
    }
    let file = File::open(path).map_err(|_| anyhow::anyhow!("Cannot read CSV file"))?;
    let opened = file
        .metadata()
        .map_err(|_| anyhow::anyhow!("Cannot read CSV file"))?;
    if !opened.is_file() || is_link(&opened) {
        bail!("CSV must be a regular file");
    }
    let mut bytes = Zeroizing::new(Vec::with_capacity(MAX_BYTES + 1));
    file.take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow::anyhow!("Cannot read CSV file"))?;
    if bytes.len() > MAX_BYTES {
        bail!("CSV exceeds the 8 MiB limit");
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| anyhow::anyhow!("CSV must be UTF-8"))?;
    parse(text.strip_prefix('\u{feff}').unwrap_or(text))
}

// csv's reader intentionally accepts malformed quoting. This strict parser
// instead rejects quotes in unquoted fields and text after a closing quote.
// All field and source buffers are zeroizing; errors never include user data.
fn parse(text: &str) -> Result<ImportPreview> {
    let bytes = text.as_bytes();
    let mut offset = 0;
    let mut row = 0;
    let mut mapping = [0usize; 5];
    let mut entries = Vec::new();
    while offset < bytes.len() {
        row += 1;
        if row > 2001 {
            bail!("CSV exceeds the 2000-row limit");
        }
        let mut values = Vec::with_capacity(5);
        loop {
            if values.len() == 5 {
                bail!("Invalid CSV width at row {row}");
            }
            let mut value = Zeroizing::new(Vec::new());
            if bytes.get(offset) == Some(&b'"') {
                offset += 1;
                loop {
                    match bytes.get(offset) {
                        None => bail!("Malformed CSV quotes at row {row}"),
                        Some(b'"') if bytes.get(offset + 1) == Some(&b'"') => {
                            value.push(b'"');
                            offset += 2;
                        }
                        Some(b'"') => {
                            offset += 1;
                            break;
                        }
                        Some(&byte) => {
                            value.push(byte);
                            offset += 1;
                        }
                    }
                }
            } else {
                while let Some(&byte) = bytes.get(offset) {
                    if matches!(byte, b',' | b'\r' | b'\n') {
                        break;
                    }
                    if byte == b'"' {
                        bail!("Malformed CSV quotes at row {row}");
                    }
                    value.push(byte);
                    offset += 1;
                }
            }
            // UTF-8 is already checked and quote/delimiter splitting is ASCII.
            let value = Zeroizing::new(
                String::from_utf8(value.to_vec())
                    .map_err(|_| anyhow::anyhow!("CSV must be UTF-8"))?,
            );
            values.push(value);
            match bytes.get(offset) {
                Some(b',') => {
                    offset += 1;
                }
                Some(b'\n') => {
                    offset += 1;
                    break;
                }
                Some(b'\r') if bytes.get(offset + 1) == Some(&b'\n') => {
                    offset += 2;
                    break;
                }
                None => break,
                _ => bail!("Malformed CSV quoting or line ending at row {row}"),
            }
        }
        if values.len() != 5 {
            bail!("Invalid CSV width at row {row}");
        }
        if row == 1 {
            let mut seen = [false; 5];
            for (index, value) in values.iter().enumerate() {
                let Some(field) = HEADERS
                    .iter()
                    .position(|header| value.eq_ignore_ascii_case(header))
                else {
                    bail!("CSV headers must be title, username, password, url, notes");
                };
                if seen[field] {
                    bail!("Duplicate CSV header");
                }
                seen[field] = true;
                mapping[field] = index;
            }
        } else {
            let mut data = EntryData::default();
            for (field, destination) in [
                &mut data.title,
                &mut data.username,
                &mut data.password,
                &mut data.url,
                &mut data.notes,
            ]
            .into_iter()
            .enumerate()
            {
                std::mem::swap(destination, &mut values[mapping[field]]);
            }
            data.notes = Zeroizing::new(data.notes.replace("\r\n", "\n"));
            data.validate()
                .map_err(|_| anyhow::anyhow!("Invalid CSV fields at row {row}"))?;
            entries.push(data);
        }
    }
    if row == 0 {
        bail!("CSV header is missing");
    }
    Ok(ImportPreview { entries })
}

struct Output(Zeroizing<Vec<u8>>);
impl Write for Output {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > MAX_BYTES {
            return Err(std::io::Error::other("CSV output exceeds limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(crate) fn encode(entries: impl Iterator<Item = EntryData>) -> Result<Zeroizing<Vec<u8>>> {
    // Reserve the bounded output once, avoiding plaintext left in old allocations.
    let mut output = Output(Zeroizing::new(Vec::with_capacity(MAX_BYTES)));
    write_record(&mut output, HEADERS).map_err(|_| anyhow::anyhow!("Cannot encode CSV"))?;
    for (index, data) in entries.enumerate() {
        if index >= 2000 {
            bail!("CSV exceeds the 2000-row limit");
        }
        write_record(
            &mut output,
            [
                &*data.title,
                &*data.username,
                &*data.password,
                &*data.url,
                &*data.notes,
            ],
        )
        .map_err(|_| anyhow::anyhow!("Cannot encode CSV within the 8 MiB limit"))?;
    }
    Ok(output.0)
}

/// Five borrowed fields, comma delimiter, doubled quotes, and LF record endings.
/// No buffered writer or intermediate plaintext allocation: only borrowed input
/// slices and static punctuation are copied directly into the zeroizing output.
fn write_record<S: AsRef<str>>(output: &mut Output, fields: [S; 5]) -> std::io::Result<()> {
    for (index, field) in fields.iter().enumerate() {
        if index != 0 {
            output.write_all(b",")?;
        }
        let bytes = field.as_ref().as_bytes();
        if bytes
            .iter()
            .any(|byte| matches!(byte, b',' | b'"' | b'\r' | b'\n'))
        {
            output.write_all(b"\"")?;
            let mut start = 0;
            for (offset, byte) in bytes.iter().enumerate() {
                if *byte == b'"' {
                    output.write_all(&bytes[start..offset])?;
                    output.write_all(b"\"\"")?;
                    start = offset + 1;
                }
            }
            output.write_all(&bytes[start..])?;
            output.write_all(b"\"")?;
        } else {
            output.write_all(bytes)?;
        }
    }
    output.write_all(b"\n")
}

pub(crate) fn persist_export(path: &Path, bytes: &[u8]) -> Result<()> {
    csv_path(path)?;
    if bytes.len() > MAX_BYTES {
        bail!("CSV exceeds the 8 MiB limit");
    }
    match fs::symlink_metadata(path) {
        Ok(_) => bail!("CSV destination already exists; refusing to overwrite"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => bail!("Cannot check CSV destination"),
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = parent
        .canonicalize()
        .map_err(|_| anyhow::anyhow!("Cannot open CSV destination directory"))?;
    let destination: PathBuf = parent.join(
        path.file_name()
            .ok_or_else(|| anyhow::anyhow!("Missing CSV filename"))?,
    );
    let mut temporary = tempfile::NamedTempFile::new_in(&parent)
        .map_err(|_| anyhow::anyhow!("Cannot create private CSV temporary file"))?;
    private_file(&temporary)?;
    temporary
        .write_all(bytes)
        .map_err(|_| anyhow::anyhow!("Cannot write CSV export"))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|_| anyhow::anyhow!("Cannot flush CSV export"))?;
    check_ancestors(path)?;
    temporary
        .persist_noclobber(destination)
        .map_err(|_| anyhow::anyhow!("Cannot create CSV export; destination may already exist"))?;
    Ok(())
}

#[cfg(unix)]
fn private_file(file: &tempfile::NamedTempFile) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|_| anyhow::anyhow!("Cannot restrict CSV file permissions"))
}

#[cfg(windows)]
fn private_file(file: &tempfile::NamedTempFile) -> Result<()> {
    use std::{os::windows::ffi::OsStrExt, ptr};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, LocalFree},
        Security::{
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            },
            GetTokenInformation, SetFileSecurityW, TokenUser, DACL_SECURITY_INFORMATION,
            PROTECTED_DACL_SECURITY_INFORMATION, TOKEN_QUERY, TOKEN_USER,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    // Restrict the empty temporary file BEFORE writing any plaintext.
    unsafe {
        let mut token = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            bail!("Cannot restrict CSV file permissions");
        }
        let mut needed = 0;
        GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut needed);
        let mut storage = vec![0usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
        let ok = GetTokenInformation(
            token,
            TokenUser,
            storage.as_mut_ptr().cast(),
            needed,
            &mut needed,
        );
        CloseHandle(token);
        if ok == 0 {
            bail!("Cannot restrict CSV file permissions");
        }
        let user = &*storage.as_ptr().cast::<TOKEN_USER>();
        let mut sid = ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut sid) == 0 {
            bail!("Cannot restrict CSV file permissions");
        }
        let mut len = 0;
        while *sid.add(len) != 0 {
            len += 1;
        }
        let sid_text = String::from_utf16_lossy(std::slice::from_raw_parts(sid, len));
        LocalFree(sid.cast());
        let sddl: Vec<u16> = format!("D:P(A;;FA;;;{sid_text})")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut descriptor = ptr::null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            ptr::null_mut(),
        ) == 0
        {
            bail!("Cannot restrict CSV file permissions");
        }
        let path: Vec<u16> = file
            .path()
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        let ok = SetFileSecurityW(
            path.as_ptr(),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            descriptor,
        );
        LocalFree(descriptor);
        if ok == 0 {
            bail!("Cannot restrict CSV file permissions");
        }
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn private_file(_: &tempfile::NamedTempFile) -> Result<()> {
    bail!("Private CSV export is unsupported on this platform")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::Session;
    const MASTER: &str = "isolated test master passphrase";
    const HEADER: &str = "title,username,password,url,notes\n";

    #[test]
    fn direct_serializer_matches_csv_writer_and_roundtrips() {
        let fixtures = [
            [
                "Заголовок 🦀",
                " user ",
                "=@\"secret, ",
                "https://example.test",
                "line1\r\nстрока2\n",
            ],
            ["\"quoted\", title", "", "+formula", "", ""],
            ["plain", "@user", "-password", "=url", "\"\"\n,comma\r\n"],
            [
                " spaces ",
                "日本語",
                " =password",
                "https://example.test/a,b",
                "last line",
            ],
        ];
        // Ordinary buffers are intentionally confined to this synthetic-data oracle.
        let mut oracle = csv::Writer::from_writer(Vec::new());
        oracle.write_record(HEADERS).unwrap();
        for fields in &fixtures {
            oracle.write_record(fields).unwrap();
        }
        let expected = oracle.into_inner().unwrap();
        let entries = fixtures.iter().map(|fields| EntryData {
            title: Zeroizing::new(fields[0].into()),
            username: Zeroizing::new(fields[1].into()),
            password: Zeroizing::new(fields[2].into()),
            url: Zeroizing::new(fields[3].into()),
            notes: Zeroizing::new(fields[4].into()),
        });
        let encoded = encode(entries).unwrap();
        assert_eq!(&*encoded, &expected);
        let preview = parse(std::str::from_utf8(&encoded).unwrap()).unwrap();
        assert_eq!(preview.len(), fixtures.len());
        for (entry, fields) in preview.entries.iter().zip(fixtures) {
            assert_eq!(&*entry.title, fields[0]);
            assert_eq!(&*entry.username, fields[1]);
            assert_eq!(&*entry.password, fields[2]);
            assert_eq!(&*entry.url, fields[3]);
            assert_eq!(&*entry.notes, &fields[4].replace("\r\n", "\n"));
        }
    }

    #[test]
    fn direct_serializer_writes_in_place_and_bounds_escaped_output() {
        // Production write_record accepts only borrowed fields and the zeroizing
        // destination. Pointer stability verifies no destination reallocations;
        // absence of staging buffers is a structural property of that function.
        let mut output = Output(Zeroizing::new(Vec::with_capacity(MAX_BYTES)));
        let allocation = output.0.as_ptr();
        let capacity = output.0.capacity();
        write_record(&mut output, ["title", "юзер", "=@\",secret", "", "a\nb"]).unwrap();
        assert_eq!(output.0.as_ptr(), allocation);
        assert_eq!(output.0.capacity(), capacity);
        assert_eq!(
            &*output.0,
            "title,юзер,\"=@\"\",secret\",,\"a\nb\"\n".as_bytes()
        );
        output.0.resize(MAX_BYTES - 5, b'x');
        write_record(&mut output, [""; 5]).unwrap();
        assert_eq!(output.0.len(), MAX_BYTES);
        assert!(output.write_all(b"x").is_err());
        assert_eq!(output.0.len(), MAX_BYTES);
        assert_eq!(output.0.as_ptr(), allocation);
        output.0.truncate(MAX_BYTES - 4);
        assert!(write_record(&mut output, ["\"", "", "", "", ""]).is_err());
        assert_eq!(output.0.len(), MAX_BYTES);
        assert_eq!(output.0.as_ptr(), allocation);
        assert_eq!(output.0.capacity(), capacity);

        let error = encode((0..1100).map(|_| EntryData {
            title: Zeroizing::new("title".into()),
            notes: Zeroizing::new("SENSITIVE".repeat(910)),
            ..Default::default()
        }))
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "Cannot encode CSV within the 8 MiB limit"
        );
    }

    #[test]
    fn bom_reordered_headers_and_exact_values() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("input.csv");
        fs::write(
            &path,
            "\u{feff}NOTES,Password,URL,USERNAME,Title\r\n\"line1\r\nстрока2\", \"bad\",,,x",
        )
        .unwrap();
        assert!(read_import(&path).is_err()); // spaces before quoted field are not RFC quoting
        fs::write(&path, "\u{feff}NOTES,Password,URL,USERNAME,Title\r\n\"line1\r\nстрока2\",\" =@\"\"secret, \",https://example.test, user ,Заголовок\r\n").unwrap();
        let preview = read_import(&path).unwrap();
        assert_eq!(preview.len(), 1);
        let data = &preview.entries[0];
        assert_eq!(&*data.password, " =@\"secret, ");
        assert_eq!(&*data.username, " user ");
        assert_eq!(&*data.notes, "line1\nстрока2");
        assert_eq!(&*data.title, "Заголовок");
    }

    #[test]
    fn malformed_csv_is_rejected_without_echoing_values() {
        for body in [
            "",
            "title,username,password,url\n",
            "title,title,password,url,notes\n",
            "title,username,password,url,other\n",
            "title,username,password,url,notes,extra\n",
            "title,username,password,url,notes\nx,y,\"SENSITIVE",
            "title,username,password,url,notes\nx,y,\"SENSITIVE\"bad,z,q",
            "title,username,password,url,notes\nx,y,SENSITIVE\"bad,z,q",
            "title,username,password,url,notes\nx,y,SENSITIVE,z",
            "title,username,password,url,notes\nx,y,SENSITIVE,z,q,extra",
            "title,username,password,url,notes\nx,y,SENSITIVE,z,q\r",
        ] {
            let error = parse(body).err().expect("must reject malformed input");
            assert!(!format!("{error:#}").contains("SENSITIVE"));
        }
        assert!(parse(HEADER).unwrap().is_empty());
        let too_many = format!("{HEADER}{}", "x,,,,\n".repeat(2001));
        assert!(parse(&too_many).is_err());
        assert!(parse(&format!("{HEADER}x,,{},,\n", "S".repeat(1025))).is_err());
    }

    #[test]
    fn isolated_roundtrip_import_backup_append_and_export_permissions() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("fake.kdbx");
        let mut session = Session::create(&path, MASTER).unwrap();
        let folder = session.create_folder(None, "Folder not exported").unwrap();
        let preview = parse(&format!("{HEADER}\"Юникод, \"\"title\"\"\", user ,\"=@\"\"secret, \",https://example.test,\"first\r\nsecond\"\r\n")).unwrap();
        let before = fs::read(&path).unwrap();
        let ids = session.import_csv(&preview, Some(folder)).unwrap();
        assert_eq!(ids.len(), 1);
        assert_eq!(session.entry_folder(ids[0]), Some(folder));
        let backup = fs::read_dir(root.path())
            .unwrap()
            .map(|item| item.unwrap().path())
            .find(|candidate| candidate != &path && fs::read(candidate).unwrap() == before)
            .unwrap();
        let restored = Session::unlock(&backup, MASTER).unwrap();
        assert!(restored.entries().is_empty());
        let output = root.path().join("new.csv");
        let count_before = fs::read_dir(root.path()).unwrap().count();
        assert!(session.export_csv(&output, "wrong").is_err());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), count_before);
        assert!(!output.exists());
        assert_eq!(session.export_csv(&output, MASTER).unwrap(), 1);
        let exported = fs::read(&output).unwrap();
        assert!(session.export_csv(&output, MASTER).is_err());
        assert_eq!(fs::read(&output).unwrap(), exported);
        let reread = read_import(&output).unwrap();
        assert_eq!(&*reread.entries[0].password, "=@\"secret, ");
        assert_eq!(&*reread.entries[0].notes, "first\nsecond");
        let appended = session.import_csv(&reread, None).unwrap();
        assert_ne!(appended[0], ids[0]);
        assert_eq!(
            session.entry_folder(appended[0]),
            Some(session.root_folder())
        );
        assert_eq!(Session::unlock(&path, MASTER).unwrap().entries().len(), 2);
        assert_eq!(fs::read(&output).unwrap(), exported);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&output).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn import_rollback_readonly_limit_and_conflict_and_stale_export() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("fake.kdbx");
        let mut session = Session::create(&path, MASTER).unwrap();
        let preview = parse(&format!("{HEADER}Good,,,,\n")).unwrap();
        let before = fs::read(&path).unwrap();
        let bad = ImportPreview {
            entries: vec![
                EntryData {
                    title: Zeroizing::new("Good".into()),
                    ..Default::default()
                },
                EntryData::default(),
            ],
        };
        assert!(session.import_csv(&bad, None).is_err());
        assert!(session.entries().is_empty());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
        session.readonly = true;
        assert!(session.import_csv(&preview, None).is_err());
        assert!(session
            .export_csv(&root.path().join("blocked.csv"), MASTER)
            .is_err());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
        session.readonly = false;
        let maximum = parse(&format!("{HEADER}{}", "x,,,,\n".repeat(2000))).unwrap();
        session.import_csv(&maximum, None).unwrap();
        let full = fs::read(&path).unwrap();
        assert!(session.import_csv(&preview, None).is_err());
        assert_eq!(fs::read(&path).unwrap(), full);
        let mut stale = Session::unlock(&path, MASTER).unwrap();
        session.delete(session.entries()[0].0).unwrap();
        let changed = fs::read(&path).unwrap();
        assert!(stale.import_csv(&preview, None).is_err());
        assert!(stale
            .export_csv(&root.path().join("stale.csv"), MASTER)
            .is_err());
        assert!(!root.path().join("stale.csv").exists());
        assert_eq!(fs::read(&path).unwrap(), changed);
        assert_eq!(stale.entries().len(), 2000);
    }

    #[test]
    fn paths_limits_utf8_and_no_clobber() {
        let root = tempfile::tempdir().unwrap();
        assert!(read_import(root.path()).is_err());
        let raw = root.path().join("raw.kdbx");
        fs::write(&raw, HEADER).unwrap();
        assert!(read_import(&raw).is_err());
        assert!(persist_export(&raw, HEADER.as_bytes()).is_err());
        let bad = root.path().join("bad.csv");
        fs::write(&bad, [0xff]).unwrap();
        assert!(read_import(&bad).is_err());
        let large = File::create(&bad).unwrap();
        large.set_len((MAX_BYTES + 1) as u64).unwrap();
        drop(large);
        assert!(read_import(&bad).is_err());
        let destination = root.path().join("keep.csv");
        fs::write(&destination, "KEEP").unwrap();
        assert!(persist_export(&destination, HEADER.as_bytes()).is_err());
        assert_eq!(fs::read_to_string(destination).unwrap(), "KEEP");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_files_and_ancestors_are_rejected() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real.csv");
        fs::write(&real, HEADER).unwrap();
        let link = root.path().join("link.csv");
        symlink(&real, &link).unwrap();
        assert!(read_import(&link).is_err());
        assert!(persist_export(&link, HEADER.as_bytes()).is_err());
        let directory = root.path().join("directory");
        fs::create_dir(&directory).unwrap();
        let linked_dir = root.path().join("linked-directory");
        symlink(&directory, &linked_dir).unwrap();
        assert!(persist_export(&linked_dir.join("new.csv"), HEADER.as_bytes()).is_err());
        assert!(!directory.join("new.csv").exists());
    }
}
