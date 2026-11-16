# Changelog

## 0.1.0 — 2026-11-16

First published Windows package. Experimental and unaudited; use test credentials
first and keep independent encrypted backups.

- Per-user Windows Apps uninstall entry and double-click uninstaller; remove only known program files and owned integrations while preserving vaults, backups and settings.

- Setup adds TermVault to the user PATH without replacing existing entries and saves the selected default vault for plain `tvault` launch.

- Guided per-user Windows setup from a release ZIP or source checkout, with permanent extension files.
- Repeat installation/repair, protection of unrelated files and rejection of automatic downgrades.
- Extension connection diagnostics via an authenticated ping that never opens the vault.
- Tag-based draft releases with allowlisted Windows ZIPs, SHA-256 and isolated setup tests in CI.
- Windows release binaries link the C runtime statically; CI rejects external VC++ runtime DLL dependencies.

Terminal vault:

- Standalone `tvault` command and monochrome keyboard-driven TUI.
- Masked master-password create/unlock flow, manual lock and inactivity auto-lock.
- Create, view, edit, title-filter and delete records; 10-second explicit reveal.
- OS-random password generation and best-effort conditional clipboard clearing.
- Restricted KDBX4 AES-256/Argon2id storage through `keepass` (experimental writer).
- Encrypted backups, verified candidate saves, atomic replacement and conflict detection.
- Unit, CLI and real terminal/ConPTY tests using dummy secrets only.
- Optional read-only browser filling with local unlock and explicit approval; no automatic form submission.
- MIT license and Windows/Linux CI. No cloud accounts or general plaintext exports.
