# TermVault

**Experimental local password vault in your terminal.** Rust, Ratatui, black background,
white text, ordinary KeePass KDBX4 storage. Command: **`tvault`**.

> **Not audited. Do not use this MVP as the only copy of real credentials.**
> The `keepass` Rust library describes KDBX writing as experimental. Keep independent,
> tested backups. This is not a claim of production security or full KeePass compatibility.

## Features

- Create a vault, unlock with a masked master passphrase, and lock manually or on inactivity.
- Entry list, title filtering, details, creation, editing, and confirmed deletion.
- Title, username, password, URL, and multiline notes. Passwords hidden by default.
- Cryptographically random 24-character passwords from the OS random source.
- Explicit password/username copy with best-effort conditional clipboard clearing.
- Encrypted backups before every mutation, atomic file replacement, external-change checks.
- No server, account, cloud, browser extension, automatic URL opening, or command execution.

TermVault is independent of OpenTerminal and TerminalNote. It can run in a compatible
terminal such as Windows Terminal or OpenTerminal. It does not modify shell profiles.

## Build and run

Requires stable Rust **1.89 or newer**. On Windows use the MSVC toolchain, Visual Studio
C++ Build Tools, and the Windows SDK. No administrator permissions are needed.

```powershell
git clone https://github.com/Andrey89711/TermVault.git
cd TermVault
cargo build --release --locked

# Explicitly create a new vault, then choose and confirm a master passphrase in the UI
.\target\release\tvault.exe --create

# Reopen it
.\target\release\tvault.exe

# Or use an explicit path (relative paths are relative to the current directory)
.\target\release\tvault.exe --create C:\Vaults\demo.kdbx
.\target\release\tvault.exe C:\Vaults\demo.kdbx
```

`--create` never overwrites an existing file. Parent folders are created only after
you submit a valid, matching new master passphrase. Without `--create`, missing files
are errors. Default location: the OS local-data directory, under `TermVault/vault.kdbx`
(usually `%LOCALAPPDATA%\TermVault\vault.kdbx` on Windows).

Install the command into your Cargo bin directory, if desired:

```powershell
cargo install --path . --locked
tvault
```

The TUI requires a terminal on stdin/stdout. Master passwords are not accepted via
arguments, environment variables, pipes, or files. There is no plaintext export command.

## Keys

| Key | Action |
| --- | --- |
| ↑ / ↓ | Select entry |
| `/` | Filter entry titles; Enter applies, Esc clears |
| `n` | New entry |
| `g` | New entry with a generated 24-character password |
| Enter | Edit entry |
| `p` | Reveal selected password for 10 seconds |
| `c` / `u` | Copy password / username |
| `d` | Ask to delete; another `d` confirms, Esc cancels |
| Ctrl+L | Lock vault |
| `q` / Ctrl+Q | Quit (`q` only in the entry browser) |
| Tab / Shift+Tab | Next / previous form field |
| Ctrl+S | Save entry immediately |
| Ctrl+G | Generate a new password in the entry form |
| Ctrl+P | Reveal the form password for 10 seconds |
| Ctrl+U | Clear the current input |
| ← / →, Home / End, Backspace / Delete | Edit input |
| Enter in Notes | Insert a newline |
| Esc in form | Return, confirming if unsaved |

Paste using your terminal's paste action (often Ctrl+Shift+V). Bracketed paste is
supported; multiline paste is accepted only in Notes. Ctrl+S/L/Q/G also recognize
the corresponding standard Russian keyboard characters. Single-letter commands
and confirmations currently use EN. The form does not have undo/redo.

## Locking and unsaved changes

Inactivity auto-lock defaults to **120 seconds**. Override with `--lock-after 15..3600`.
Manual lock/quit/back asks to save, discard, or cancel when an entry draft has changes.
**Inactivity auto-lock discards unsaved drafts without waiting for a confirmation**,
so save important edits before leaving the terminal. There is no draft recovery.

There is no master-password recovery or backdoor. Master-password changes, keyfiles,
TOTP, passkeys, groups management, attachments, and browser autofill are not implemented.

## Storage and compatibility

New vaults use the standard KDBX4 format via [`keepass` 0.17.0](https://github.com/sseemayer/keepass-rs):

- AES-256 outer encryption, the KDBX format's integrity protection, ChaCha20 protected fields.
- Argon2id v1.3: **64 MiB, 3 iterations, parallelism 1** in production builds.
- No compression. All five entry fields use protected KDBX values, not only the password.
- Maximum encrypted file: 8 MiB; maximum 2,000 entries and 100 groups when opening.
- Bounded preflight validation rejects excessive/malformed KDF parameters before decryption.

This MVP deliberately opens only the uncompressed AES-256 / Argon2id KDBX4 subset,
with KDF memory 8–128 MiB, iterations 1–10, and parallelism 1–8. Other KeePass files
may be rejected; public custom header data is also unsupported. Files whose generator
is not `TermVault/0.1` open **read-only**;
editing foreign databases is restricted to avoid damaging unsupported features.
Another application may change that generator, making the vault read-only here.
Do not import your main password database merely to try this MVP.

Candidate saves are encrypted and reopened before replacement. If the source file
changed since unlock, saving fails and the draft stays in memory. Do not use two
editors concurrently: the final comparison/rename still has an unavoidable race.

Each successful mutation retains the previous encrypted file as
`<name>.kdbx.backup-<random>.kdbx` in the same directory. Backups are **not automatically
deleted** and retain old passwords/deleted records. You can open a backup by its path.
If replacement fails after the backup was written, that valid backup may remain too.
Temporary files contain ciphertext only. Keep the entire folder on trusted storage;
Windows access inherits the parent directory's ACLs. There is no additional DPAPI binding.

## Security limitations

- This app and its dependencies have not been independently audited as a product.
- Explicit reveal exposes a password to terminal buffers, screen capture, and shoulder surfing.
- Decrypted data exists in memory while unlocked. App-owned inputs use zeroizing buffers
  and the library protects secret values, but parser/renderer/OS copies, swap, crash dumps,
  and terminal buffers are **not guaranteed to be wiped**. There is no memory locking.
- Auto-lock drops the unlocked session; it is not protection against malware, an administrator,
  or another process running with the same user's privileges.
- Clipboard clearing is best effort and occurs after 20 seconds, on lock, or on clean exit,
  only if the current text still equals what TermVault copied. Clipboard races, history,
  synchronization, and other software can retain copies. Windows history/cloud exclusion
  hints are requested, not guaranteed. Force termination prevents scheduled clearing.
- No credentials are intentionally logged. Panic messages omit payloads. Debuggers and OS
  dumps are outside that guarantee. Do not pass secrets in shell arguments or filenames.
- Open only trusted vault files. Size/KDF bounds reduce risks but are not a parser sandbox
  or a hard wall-clock timeout. A strong master passphrase remains essential.

## Development

```powershell
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
cargo build --release --locked
```

Unit tests use temporary vaults and dummy passwords. Test-only KDF settings are reduced
to 8 MiB/1 iteration for speed; the regular binary, including debug, uses production settings.
Do not commit vaults, backups, keyfiles, or real credentials. They are ignored by default.

The real-PTY test launches the regular binary (Windows ConPTY on Windows), creates a
temporary vault, edits/saves a dummy record, locks, rejects a wrong password and reopens
the vault. It uses a terminal screen parser for rendering assertions and checks that
masked dummy passwords were never emitted. It does not read or alter your clipboard.

## License

[MIT](LICENSE), copyright (c) 2026 Andrey89711. Third-party dependency licenses remain applicable.
