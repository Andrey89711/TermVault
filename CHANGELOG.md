# Changelog

## Unreleased

- Guided per-user Windows setup from a release ZIP or source checkout, with permanent extension files.
- Repeat installation/repair, protection of unrelated files and rejection of automatic downgrades.
- Extension connection diagnostics via an authenticated ping that never opens the vault.
- Tag-based draft releases with allowlisted Windows ZIPs, SHA-256 and isolated setup tests in CI.

## 0.1.0

Experimental, unaudited terminal MVP:

- Standalone `tvault` command and monochrome keyboard-driven TUI.
- Masked master-password create/unlock flow, manual lock and inactivity auto-lock.
- Create, view, edit, title-filter and delete records; 10-second explicit reveal.
- OS-random password generation and best-effort conditional clipboard clearing.
- Restricted KDBX4 AES-256/Argon2id storage through `keepass` (experimental writer).
- Encrypted backups, verified candidate saves, atomic replacement and conflict detection.
- Unit, CLI and real terminal/ConPTY tests using dummy secrets only.
- MIT license and Windows/Linux CI. No browser integration or plaintext exports.
