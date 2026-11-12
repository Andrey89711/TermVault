# Changelog

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
