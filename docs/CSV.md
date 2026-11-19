# CSV password transfer

CSV is a plaintext interchange format, **not an encrypted vault backup**.
Use independent encrypted KDBX backups for normal retention. Start with fictional
credentials when testing the experimental importer/exporter.

## Format

- UTF-8, optionally with a UTF-8 BOM, comma-separated.
- One header row containing exactly `title`, `username`, `password`, `url`, `notes`.
  Header order may vary; names are case-insensitive.
- Quote fields containing commas, quotes or line breaks. Inside a quoted field,
  encode a quote as two quotes (`""`).
- Values are not trimmed: spaces in passwords and usernames matter.
- Notes may contain quoted line breaks; CRLF note line endings are normalized to LF.
- This five-column format does not preserve folders, history or other KDBX metadata.
  It is not a universal browser/password-manager CSV format.

Fictional example:

```csv
title,username,password,url,notes
Demo,user@example.test,DEMO_PASSWORD_NOT_REAL,https://example.test,"Fictional entry, not real credentials"
```

A header-only file contains no records. Duplicate/missing/unknown headers, invalid
UTF-8, malformed quoting, wrong column counts or invalid field values are rejected.
Do not change a real password merely to make a malformed CSV importable.

## Limits

The bounded importer accepts at most 8 MiB and 2,000 records, subject to the vault's
overall 2,000-entry limit. Existing field limits still apply:

| Column | Maximum UTF-8 bytes | Notes |
| --- | ---: | --- |
| `title` | 256 | Required, not whitespace-only |
| `username` | 1,024 | May be empty |
| `password` | 1,024 | May be empty |
| `url` | 2,048 | May be empty |
| `notes` | 8,192 | May contain line breaks |

Terminal control characters and misleading bidirectional controls are not accepted.

## Import

Unlock a writable TermVault vault, finish any draft, then choose CSV import in
Ctrl+P. Enter the absolute `.csv` path, review the count and destination, and explicitly
confirm appending the records. The entire file is validated before one encrypted
commit and backup. Invalid rows do not cause a partial import.

Imports create new entries, even when titles or usernames already exist.
Repeating an import duplicates those records; it does not merge or replace them.
Read-only foreign vaults and browser approval sessions cannot import.

## Export

Choose CSV export in Ctrl+P, select a **new `.csv` file**, enter the current master
passphrase in the masked prompt, and read the plaintext warning before confirming.
Export includes the five fields for entries across the vault, not just the current
filtered list. An existing file is never overwritten.

Anyone with access to the CSV can read its passwords without the master passphrase.
Use a private local directory; avoid shared folders, cloud sync, email and support
tickets. Locking TermVault does not delete the CSV. Deleting it later is not a
secure-erasure guarantee.

**Spreadsheet warning:** values starting with `=`, `+`, `-` or `@` can be treated as
formulas by spreadsheet software. Exact values are preserved for round-tripping;
do not open password CSV in a spreadsheet with automatic formula execution.
