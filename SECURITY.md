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
No plaintext exports, password CLI flags/environment/piped inputs, browser integration,
external commands, or automatic navigation. Clipboard operations and reveal require explicit
actions. Memory cleanup and clipboard clearing are best effort, not secure-erasure claims.
