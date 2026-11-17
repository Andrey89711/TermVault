# Private TermVault extension (Windows)

Experimental click-to-fill integration, usable without a store listing.
**Test with fictional credentials first.**

## Install in Yandex Browser

For a prebuilt ZIP without Rust, see the [quickstart](QUICKSTART.ru.md).
The `setup.ps1` wizard installs the extension in a permanent folder and registers
the selected browser. Open the extension's options and select **Check connection**
for diagnostics: `ping` checks the bridge without opening the vault. Browser labels
may appear in Russian depending on the installed extension/browser version.
The following instructions are for a source build.

1. From the project folder, run:

   ```powershell
   cargo build --release --locked --target-dir target/browser-build
   ```

   A separate build directory lets an ordinary TermVault session remain running.
2. Open `browser://extensions` (or `chrome://extensions` if needed), enable
   developer mode and choose **Load unpacked**. Select the checkout's folder:

   ```text
   C:\Projects\TermVault\browser\extension
   ```

3. Copy the extension ID (32 letters) and run from the project folder:

   ```powershell
   .\browser\install.ps1 -ExtensionId 'YOUR_EXTENSION_ID'
   ```

   For a vault outside the default location:

   ```powershell
   .\browser\install.ps1 -ExtensionId 'YOUR_EXTENSION_ID' -Vault 'C:\Vaults\personal.kdbx'
   ```

4. Pin the extension button to the browser toolbar.

Administrator privileges are not required. The script copies the application into
`%LOCALAPPDATA%\TermVault\browser`, writes configuration **without passwords**,
permits one extension ID and registers the Native Messaging host under
`HKCU\Software\Chromium\NativeMessagingHosts\com.termvault.browser`.
Yandex Browser 26.10 uses the Chromium registry path, not a separate Yandex branch.
The bridge directory receives an ACL for the current user and SYSTEM. The vault
is not modified. Use `-Browser Chrome` or `-Browser Edge` for those browsers.
If PowerShell blocks scripts, do not weaken global security policy: review the
script and use an execution method permitted by your system's policy.

## Use

1. In ordinary TermVault, set an entry's **URL**, for example
   `https://example.com/login`, and save with Ctrl+S. HTTPS scheme, hostname and
   port must exactly match the website. `example.com`, `www.example.com` and
   `login.example.com` are different destinations.
2. Open an HTTPS login page and click the extension button.
3. Check the origin in the separate TermVault window and enter the master passphrase.
   Select an entry with the arrows, press **Enter**, then **y** to approve.
   **Esc** cancels.
4. The username and password are filled. Submit the login form yourself.

The `…` badge means waiting, `✓` means filled and `!` means denied/error. Hover
over the button for details. Requests expire after three minutes; the local window
locks after 120 seconds of inactivity. An existing terminal session is not reused:
each filling request requires a separate unlock.

Simple forms with one password and an unambiguous username are supported, as is a
password-only login step. If several forms exist, first select the password field.
The MVP does not support iframes, Shadow DOM, fields outside `<form>`, new-password
forms, ambiguous fields or submission to a different origin.

## Security and limitations

- Permissions: `activeTab`, `scripting`, `nativeMessaging`; no permanent all-sites
  permission, `storage`, filesystem access, telemetry or network server.
- The browser and app check the registered ID. Entries match an exact HTTPS origin,
  not a substring or automatically included subdomain. Approval is read-only:
  the bridge does not edit entries or encrypted folder structure.
- A request is bound to the original document and form fields. Navigation, replaced
  fields, an unsafe submission origin or expiration cancels filling.
- The master passphrase stays in the local application. Only the selected username
  and password reach the browser, not notes, other entry titles or the whole database.
- Communication uses Native Messaging and an anonymous child-process pipe, not a
  TCP listener, local HTTP API or temporary files containing decrypted secrets.

Local installation **does not make the code uncopyable**. Security does not rely
on source secrecy. Page scripts can read a filled password. Same-user malware,
administrators, memory dumps and compromised browsers are outside the threat model.
JavaScript strings and internal IPC copies cannot be guaranteed to be wiped.
This is an unaudited MVP, not a security guarantee.

## Update and uninstall

After building a new version, rerun `install.ps1` and reload the extension on its
extensions page. If moving its folder changes the ID, reinstall the bridge with
the new ID. One configuration binds one ID to one vault.

```powershell
.\browser\uninstall.ps1
```

This removes only the selected browser's registration; remove the extension
manually. Bridge files remain in case another browser uses them. Vaults and backups
are not touched.

## Testing and diagnostics

```powershell
cargo test --all-targets --locked
node --test browser/tests/*.test.mjs
```

Manual browser testing remains necessary: a disposable HTTPS form, unlock,
Enter/y approval and filling; then Esc cancellation, a wrong password, no matching
entries and navigation during approval. DOM tests do not replace this check.

If the bridge is missing, check the ID and registration, reinstall after building,
and verify Manifest V3/Native Messaging support in the installed browser version.
Enterprise policy may block per-user Native Messaging hosts. Do not disable browser
security protections to make the extension work.
