# Changelog

## Unreleased

- Accept AltGr text such as `@` in login and other input fields without treating
  Ctrl+Alt as control shortcuts; preserve field byte limits and password masking.

## 0.1.1 — 2026-11-17

- Shared capability checks for keyboard and command-palette actions, including
  read-only editing restrictions and unsaved-change confirmation for lock/quit.
- Consolidated runtime teardown on lock and per-frame render feedback to revoke
  stale mouse targets and confirmations after resizing or changing dialogs.
- Audit regression tests for command availability/order, shortcut parity, complete
  lock cleanup, visible browser approval and full vault-deletion warnings.

- English-default TUI with persistent English/Russian selection through Ctrl+P →
  Language, preserving the existing `vault` setting in `settings.json`.
- English documentation and actual-render screenshots for browsing, unlock and commands.
- Expandable Explorer-style folder tree with nested records, gray selection,
  keyboard navigation, folder clicks and mouse-wheel navigation.
- Click-safe command palette action mapping and folder-specific deletion targets;
  Delete on a folder always opens deletion confirmation, never folder creation.
- Delete the current folder through Ctrl+P with visible path/subtree counts and
  explicit confirmation, retaining an encrypted backup; root deletion is blocked.
- Native KDBX folders and nested folders: create with Shift+F, browse with `f`,
  move a selected entry with `m`; also available in Ctrl+P. New entries use the
  current folder; All entries and browser matching continue to span the vault.
- Centered welcome/unlock screen with TermVault wordmark and masked prompt-style inputs.
- More compact notification cards with gray edges and a slim status accent, without headings.
- Shared dark theme across vault screens, focused inputs and confirmation dialogs;
  peach selection, purple headings, muted secondary text and dimmed modal backdrops.
- Borderless top-right notification cards with green, amber or pink status accents.
- Ctrl+P opens a searchable, context-aware command palette for existing vault actions.
- Centered, borderless command palette with a dimmed backdrop, grouped actions,
  purple section headings, peach selection and right-aligned keyboard shortcuts.
- Password reveal moved from Ctrl+P to Ctrl+O; opening the palette hides revealed passwords.

- Top-right, auto-expiring terminal notifications for unlock, save, copy and errors.
- Removed the duplicated bottom status line; keyboard shortcuts remain in the footer.
- Masked master-passphrase change with reauthentication and verified replacement;
  retained encrypted backups continue to use their previous passphrase.
- Whole-vault removal with password, exact filename and visible final confirmation;
  moves the active file to an encrypted backup, not secure erasure.
- Explicitly consented, asynchronous GitHub release checks from the command palette;
  no scheduled checks, automatic downloads or installation.

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
