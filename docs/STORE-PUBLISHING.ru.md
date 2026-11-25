# First Chrome Web Store upload

This guide is in English; its historical filename is retained for existing links.

Yandex Browser supports compatible Chrome Web Store extensions. A store listing
does not install the Windows application: that app is still required to unlock
the local vault. Check current requirements before requesting review and provide
publisher information yourself. Do not claim an independent security audit.

## Upload a draft first; do not submit for review yet

1. Open https://chrome.google.com/webstore/devconsole and sign in to your account.
   Register as a developer if needed and complete any verification/payment yourself.
   Do not share passwords, verification codes or payment details.
2. Prepare the ZIP from the repository root:

   ```powershell
   python scripts/package-extension.py
   ```

3. Choose **Add new item** and upload `dist/termvault-extension-0.1.4.zip`.
   The filename depends on the version. **Never upload the Windows ZIP, EXE or a
   vault folder.** The correct ZIP has `manifest.json` directly at its root.
4. Save the store-assigned extension ID (32 letters a–p). A developer must add it
   to the installer once; an unpacked extension ID is not a substitute. Keep this
   listing and upload future versions to the same item.
5. Leave the item as a draft for now. Review requires an actual screenshot, a
   published privacy policy and an accessible Windows release.

Do not add a random manifest `key` or generate new IDs for updates. If local
development needs the same ID, use the public key issued for this exact store
listing. Never commit private keys.

## Listing information

The name and short description come from the manifest. The existing extension's
listing language is Russian; match the submitted package and provide any translated
listing deliberately. Select the current category appropriate for password management.

### Full description

TermVault fills a username and password on an HTTPS page when you click its button,
and only after approval in the separately installed local TermVault Windows app.

Click the extension, verify the origin in the TermVault window, unlock your local
vault, select a record and approve its transfer. Submit the website's login form
yourself. The extension options include a connection check that does not open the
vault or require a master passphrase.

The master passphrase is not sent to the extension. The extension does not upload
your database to a cloud, include advertising or analytics, or request permanent
access to all websites. Simple login forms with exact HTTPS-origin matching are
supported. Iframes, complex custom forms and signup forms may not work.

Important: the extension cannot fill passwords without the separate Windows app.
TermVault is experimental and unaudited. Start with test data and keep independent
encrypted backups. Website scripts can read a password after filling. This does
not protect against malware on your computer.

Source and instructions: https://github.com/Andrey89711/TermVault

### Privacy practices

Single purpose: fill selected local credentials into the current HTTPS website's
form after explicit user approval in TermVault.

Permission justifications:

- `activeTab`: temporary access to the current tab only after the user clicks.
- `scripting`: inspect the form and fill fields in the original top-level document;
  permanent access to all websites is not needed.
- `nativeMessaging`: communicate with local TermVault to check the connection and
  transfer one approved record. The master passphrase stays in the native app.

Remote code: none. JavaScript ships inside the package; the EXE is installed
separately and its code is not downloaded for execution inside the extension.

Policy: https://github.com/Andrey89711/TermVault/blob/main/PRIVACY.md
**Make sure this file has been committed and published before submitting its URL.**

Do not automatically declare that no user data is processed merely because there
is no server. The extension processes usernames/passwords, the current site's
origin and form structure. Dashboard declarations must match current store
definitions and `PRIVACY.md`; the absence of analytics does not remove that
processing. Review current data categories, including authentication information
and URLs/site content, and confirm limited use only after your own review.

### Images

- 128×128 icon: `browser/extension/icons/icon-128.png`.
- At least **one actual 1280×800 screenshot** of the extension/interface in use is
  required. Use fictional data only. Do not expose real passwords, accounts, tabs,
  personal paths or keys. Never present a mockup as a screenshot of a working feature.
- Promotional materials may be optional; check the current dashboard requirements.

### Distribution

- **Unlisted**: accessible by a shareable link, not shown in search.
- **Private**: available to trusted testers with Google accounts. Test installation
  in Yandex Browser separately; do not assume identical authentication behavior.
- **Public**: a publicly discoverable listing.

Visibility does not bypass review. Private may suit an initial test; Unlisted may
suit ordinary installation by link. The final choice belongs to the publisher.

## Reviewer instructions and publication

Provide a direct link to a published Windows x64 release and short installation
steps. Do not submit while the application link is unavailable. Reviewers can
create their own test KDBX vault with `tvault.exe --create`; there is no service
account. Explain how to register the store-assigned ID through `setup.ps1`, open
extension options, check the connection and test a simple HTTPS form with fictional
credentials.

After receiving the ID, the developer can update the installer to connect without
manual ID entry and open the store page. Then test the actual store package with
the local bridge and prepare the corresponding Windows release. Review every
declaration before submission; disable automatic publication after review if you
want to approve publication manually. These instructions do not publish anything.

Official sources:

- https://browser.yandex.ru/help/personalization/extension
- https://developer.chrome.com/docs/webstore/publish
- https://developer.chrome.com/docs/webstore/cws-dashboard-privacy
- https://developer.chrome.com/docs/webstore/cws-dashboard-distribution
