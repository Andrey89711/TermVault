# CI/CD and releases

## Continuous integration

`.github/workflows/ci.yml` runs on pushes to `main`, pull requests, manual dispatch
and calls from the release workflow:

- Rust formatting, Clippy, tests and release compilation on Windows and Linux.
- Browser tests on Node.js 24, without npm dependencies.
- Windows ZIP packaging smoke test with an explicit file allowlist.
- Isolated Windows setup/update/repair tests without changing real browser registrations.
- On `main`, a `windows-preview` Actions artifact is retained for seven days.
  Preview artifacts are not published releases.

Pull requests have read-only repository permissions. No deployment secrets are
needed. Configure required status checks for `main` in GitHub repository rules
after the first successful run; workflows alone do not enforce branch protection.

## Release a stable Windows x64 version

1. Update `version` in both `Cargo.toml` and `browser/extension/manifest.json`.
   Regenerate `Cargo.lock` with Cargo if the package version changed, and update
   `CHANGELOG.md`. Stable tags only: `vX.Y.Z` (no prerelease suffix).
2. Commit, push to `main` and wait for CI to pass.
3. Tag the intended commit, then push the tag, for example:

   ```powershell
   git tag -a v0.1.1 -m "TermVault 0.1.1"
   git push origin v0.1.1
   ```

   Do not reuse or force-move an existing release tag.
4. `.github/workflows/release.yml` verifies the tag matches both versions, reruns
   all checks and builds `x86_64-pc-windows-msvc` with `--locked`.
5. The workflow verifies the ZIP file allowlist, uploads the ZIP and SHA-256,
   verifies the checksums again on the publishing runner, and creates a **draft**
   GitHub Release. Only this final job has `contents: write`.

## Contents and installation checks

The shared allowlist in `scripts/setup-common.ps1` controls both packaging and
installation. It includes `setup.cmd`, `setup.ps1`, the CLI and extension diagnostics.
Vaults, backups, local bridge configuration and installation markers are never packaged.
Run `scripts/test-setup.ps1 -BinaryPath <built tvault.exe>` on Windows to test
install/update/repair in a generated temporary directory. No real user vault or
browser registration is modified. The extension's ping verifies connectivity only:
manual browser testing with a dummy HTTPS login is still required before publishing.
6. Download and check the draft ZIP on Windows: run `tvault.exe --version`, create
   a disposable test vault and check extension connection/filling. Review release
   notes, then click **Publish release** in GitHub.

No tag, commit, push or public release is created merely by editing these workflow
files. Existing releases are not overwritten automatically. No code-signing
certificate is configured; hashes detect mismatches, not a compromised publisher.

## Package locally

```powershell
cargo build --release --locked --target-dir target/browser-build
.\scripts\package-windows.ps1 -BinaryPath target/browser-build/release/tvault.exe
```

Output: `dist/termvault-X.Y.Z-windows-x86_64.zip` and `dist/SHA256SUMS.txt`.
The output directory is ignored by Git. Use a fresh `-OutputDirectory` if artifacts
already exist: the packaging script refuses to overwrite them.

The ZIP contains only the executable, documentation, license, extension and bridge
setup scripts. It never packages vaults, backups, keys or machine-specific bridge
configuration. `browser/install.ps1` detects the bundled `tvault.exe`; end users
do not need Rust. Start with `browser/QUICKSTART.ru.md` inside the ZIP.
