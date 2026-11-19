# Security policy

TermVault 0.1 is an **experimental, unaudited MVP**, not a production security guarantee.
Do not store the only copy of real credentials in it. Its KDBX writer dependency is experimental.

For private reporting, use GitHub's private vulnerability reporting if enabled on this
repository. Otherwise contact the maintainer to arrange a private channel before sharing
sensitive details. Never attach real vaults, passwords, keys, or decrypted crash dumps to issues.

## Threat model

The goal is encrypted local storage when locked/on disk and user-initiated secret access.
We do not defend against malware, administrators, compromised terminals, other same-user
processes, keyloggers, screen capture, or OS clipboard/history/swap/crash dumps.

No proprietary cryptography: KDBX4 via `keepass`. New files use AES-256, Argon2id, and
protected fields. Read/write compatibility is intentionally narrowed; see README.
Atomic replacement, encrypted backups, and optimistic conflict detection reduce accidental
loss but do not eliminate filesystem races, power-loss risks, or dependency bugs.

Manual dirty exits require confirmation. Auto-lock intentionally discards unsaved drafts.
No password CLI flags/environment/piped inputs, arbitrary external commands, or
automatic navigation. CSV export is an explicit, reauthenticated plaintext
disclosure, not an encrypted backup. Clipboard operations and reveal require explicit
actions. Memory cleanup and clipboard clearing are best effort, not secure-erasure claims.

## Manual maintenance commands

Master-passphrase changes reauthenticate against the current encrypted revision and
reuse verified atomic-save/backup handling. The live key is changed only after the
new ciphertext has been committed successfully. Previous backups remain protected
by the old passphrase. No passphrase recovery or secure-erasure claim is made.

Vault removal requires reauthentication, exact filename entry and a visible final
confirmation. Its short-lived authorization is bound to one path and ciphertext
revision. Removal moves the active file to an encrypted backup in the same directory;
it does not remove backups or unrelated files. Trusted parent storage/no concurrent
writers are required, as for normal saves. Same-user filesystem attacks are not covered.

Update checking and package downloading require explicit consent and bounded HTTPS
requests to the official repository and permitted release-asset hosts. No vault/session
data is supplied to the worker. Downloaded archives must match the published checksum
and strict payload allowlist before staging. Installation requires a separate visible
confirmation; program-file replacement must not include vaults, backups or settings.
Packages and their checksums are hosted together and are unsigned: they cannot defend
against a compromised publisher/repository. This is not cryptographically signed update
authorization. Updates run with the current user's permissions, not administrator rights.

CSV import validates the complete bounded file before a single encrypted transaction.
It appends records and retains an encrypted pre-import backup; it never silently merges
or overwrites records. CSV export requires the current master passphrase and creates
a new plaintext file without overwriting existing files. Treat both imported and
exported CSV as exposed credentials. Do not use shared/cloud-sync directories.
Formula-like values are preserved for password round-tripping and may execute when
opened in spreadsheet software. Filesystem permissions, zeroizing buffers and deletion
cannot guarantee secure erasure of plaintext, paging, library copies or storage remnants.

## Experimental browser bridge

The optional Windows extension uses Native Messaging and per-request local unlock/approval.
It releases only one selected login/password for an exact HTTPS origin; the vault is not
modified. No local listening port or decrypted temporary files are used. The installer
registers a single extension ID in HKCU. This is not authentication against malware running
as the same user. Filled credentials are visible to page scripts; JavaScript/browser IPC
copies are not guaranteed to be wiped. No audit or production-security claim is implied.
See [setup and limitations](browser/README.md). Never test with your only real vault.
