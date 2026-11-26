# Run a prebuilt Windows ZIP

This guide is in English; its historical filename is retained for existing links.

1. Download the Windows x64 ZIP and `SHA256SUMS.txt` from GitHub Releases.
   Compare the archive's SHA-256 with the checksum file:

   ```powershell
   Get-FileHash .\termvault-0.1.6-windows-x86_64.zip -Algorithm SHA256
   ```

   The filename depends on the version. Do not run the program if hashes differ.
   The EXE is not signed: a checksum does not replace a digital signature or audit.

2. Extract the ZIP and run `setup.cmd`, or run this in PowerShell:

   ```powershell
   .\setup.ps1
   ```

   The wizard installs the app and extension in `%LOCALAPPDATA%\Programs\TermVault`.
   Select an existing vault or confirm creation of a new one. Enter the master
   passphrase only in the TermVault UI. Select a browser, load the extension
   **from the permanent folder shown by the wizard**, and paste its ID. On the
   extension details page, open **Extension options → Check connection**. Labels
   may be localized. This check requires no password and does not open the vault.

   The wizard adds the app to the **user PATH** and remembers the selected vault.
   After installation, open a new terminal:

   ```powershell
   tvault
   ```

   If a new tab still cannot find the command, close the entire terminal application
   and reopen it. Existing processes retain their old PATH. An explicit
   `tvault C:\Vaults\other.kdbx` opens another vault without changing the default.
   Settings contain only non-secret preferences, never the master passphrase.
   The UI defaults to English; **Ctrl+P → Language** saves English or Russian
   without changing the configured vault path.

   Rerunning the wizard from a newer ZIP updates program files only. Close TermVault
   first. Vaults and backups are retained; installing over an unrelated nonempty
   folder and automatic downgrades are refused. Program-file updates are not atomic:
   if interrupted, rerun the wizard from the same or a newer complete ZIP. Updating
   does not edit the vault. `setup.cmd` does not bypass PowerShell policy. If scripts
   are blocked, use a permitted execution method rather than weakening global policy.

## Uninstall

Open **Windows Settings → Apps → Installed apps → TermVault → Uninstall**, or run
`uninstall.cmd` from the installation folder. Confirm with `y`. Close TermVault
and any browser-filling windows waiting for approval first.

Only known program files, the installation's user PATH entry and its owned local
bridge are removed. Vaults, backups, saved vault preferences and unrelated files
remain, even inside the installation folder. System PATH and browser profiles are
not changed. Remove the extension separately on the browser's extensions page.
An older installation without bridge ownership metadata may retain the bridge for
safety; reinstalling with browser integration creates that metadata for later removal.

## Manual installation without the wizard

Extract the ZIP into a permanent folder, for example
`%LOCALAPPDATA%\Programs\TermVault`. Do not load the extension from a temporary
folder: the browser needs continued access to its files.

1. Open PowerShell in that folder and create a **test** vault:

   ```powershell
   .\tvault.exe --create
   ```

   If the vault exists, run `.\tvault.exe` without `--create`. Enter the master
   passphrase only in the TermVault window. Rust is not required.
2. Enable developer mode on the browser's extensions page and load
   `browser\extension` from the extracted ZIP. Copy the extension ID.
3. For Yandex Browser, run:

   ```powershell
   .\browser\install.ps1 -ExtensionId 'YOUR_EXTENSION_ID'
   ```

   Add `-Browser Chrome` for Chrome or `-Browser Edge` for Edge. For a non-default
   vault path, add `-Vault 'C:\Vaults\personal.kdbx'`. Administrator privileges are
   not required. Do not disable browser security or weaken global PowerShell policy.
4. Give a test entry the website's HTTPS URL and save with Ctrl+S. Open the login
   form, click the extension, unlock TermVault and approve with Enter → y.

See the [browser README](README.md) for details and limitations. This is an
experimental, unaudited project. Never keep your only copy of real passwords here.
