# TermVault browser extension privacy policy

Last updated: 2026-11-19

This policy covers the TermVault browser extension and its communication with the
separately installed local TermVault application. TermVault is an experimental,
unaudited password manager. It is not a production-security guarantee.

## Data handled and why

When you click the extension on a login page, it checks the current HTTPS origin
(scheme, hostname and port), visible login fields and the form submission origin.
The origin is sent to the local TermVault application through browser Native Messaging;
the full page URL, path and query string are not sent to that application.
No page scan is initiated in the background on every site visit.

After you unlock the local vault and explicitly approve one matching record,
its username and password pass through the extension into the selected page's form.
The extension does not receive the master passphrase, vault decryption key,
complete password database, entry notes or list of other records.
It does not submit the form automatically. A connection-check request exchanges
only protocol/version information; it does not open the vault.

The local bridge stores the permitted extension ID and vault file path in a
configuration file in your Windows user profile. Passwords remain in the encrypted
local vault and are decrypted by the local application when unlocked.

## Storage, transmission and disclosure

The extension does not persist credentials in browser storage. It does not include
analytics, advertising, remote executable code or a network API for uploading your data
to the developer or another server. No user data is sold, used for advertising,
creditworthiness or unrelated purposes. Data is used only for the described
user-requested password filling and connection diagnostics.

Filling a website's form intentionally discloses the selected credentials to that
page. Its scripts may read them before submission, and the website's handling of
those credentials is governed by its own policies. Only fill a site you trust.
The extension cannot protect against a compromised website, browser, administrator
or malware running with your account's privileges.

Credentials are held temporarily in application/browser memory. JavaScript strings,
IPC copies, OS paging, crash dumps and browser internals cannot be guaranteed to be
erased. Removing the extension does not delete the vault, backups or local bridge
configuration. Those files remain under your control on your computer.

## Local application maintenance

The standalone terminal application offers user-initiated update checking,
downloading and installation. After consent, it requests public release metadata
from `api.github.com`; download consent also permits requests to GitHub and its
approved release-asset host. GitHub receives your IP address and a
`TermVault/<version>` User-Agent; no vault contents, credentials, vault paths or
current website are sent. There are no startup/scheduled checks or unattended
downloads/installations. Installation requires separate explicit approval.
The extension and browser bridge do not initiate these requests. GitHub handles
request metadata under its own privacy policy.

CSV import/export is local and does not send the file or its contents to GitHub
or the developer. Export deliberately writes all exported fields, including
passwords, to an unencrypted file you select after reauthentication and confirmation.
That file may be accessible to other local software or uploaded by third-party
sync/backup software monitoring its directory. It is not automatically removed
when the vault locks or the application is uninstalled. Manage it as exposed secrets.

Changing the master passphrase does not re-encrypt existing backups: those still
require the old passphrase. The terminal's vault-removal command moves the active
encrypted file to a retained backup rather than securely erasing it.

## Your choices

You can cancel a filling request in the local application, lock the vault, restrict
the extension's site access, disable/remove the extension, or unregister the bridge
using the supplied browser/uninstall.ps1 script. Vault files and backups can be
managed separately. Do not upload them to a support ticket.

## Contact and changes

For non-sensitive questions, use https://github.com/Andrey89711/TermVault/issues.
For suspected vulnerabilities, follow SECURITY.md in the same repository. Never
send real passwords, vault files, keys or decrypted logs. Future changes to this
policy will be published in this repository with an updated date.
