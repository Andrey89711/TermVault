[CmdletBinding()]
param(
    [ValidateSet('Yandex', 'Chrome', 'Edge')][string]$Browser,
    [string]$Vault,
    [string]$InstallDirectory = (Join-Path $env:LOCALAPPDATA 'Programs\TermVault'),
    [ValidatePattern('^[a-p]{32}$')][string]$ExtensionId,
    [switch]$Build,
    [switch]$CreateVault,
    [switch]$SkipBrowser,
    [switch]$SkipShellIntegration,
    [switch]$NonInteractive
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:OS -ne 'Windows_NT') { throw 'This setup currently supports Windows only.' }
. (Join-Path $PSScriptRoot 'scripts/setup-common.ps1')
. (Join-Path $PSScriptRoot 'scripts/uninstall-common.ps1')
Write-Host 'TermVault setup - experimental, unaudited. Keep independent encrypted backups.'
$binary = Join-Path $PSScriptRoot 'tvault.exe'
if ($Build -or !(Test-Path -LiteralPath $binary -PathType Leaf)) {
    $binary = Join-Path $PSScriptRoot 'target\browser-build\release\tvault.exe'
    if ($Build -or !(Test-Path -LiteralPath $binary -PathType Leaf)) {
        if (!(Get-Command cargo -ErrorAction SilentlyContinue)) {
            Write-Host 'Rust is not installed. Download a Windows ZIP from https://github.com/Andrey89711/TermVault/releases'
            if (!$NonInteractive -and (Read-Host 'Open releases in your browser? [y/N]') -eq 'y') {
                Start-Process 'https://github.com/Andrey89711/TermVault/releases'
            }
            throw 'No binary found. Extract a complete release ZIP and run setup.ps1 there. No tools were installed.'
        }
        if (!$Build -and ($NonInteractive -or (Read-Host 'Build from source using the installed Rust toolchain? [y/N]') -ne 'y')) {
            throw 'Build cancelled. Use a prebuilt release ZIP instead.'
        }
        Push-Location $PSScriptRoot
        try {
            & cargo build --release --locked --target-dir target/browser-build
            if ($LASTEXITCODE -ne 0) { throw 'Build failed. Windows C++ Build Tools may be required; a release ZIP avoids this requirement.' }
        } finally { Pop-Location }
    }
}
if (!$Vault) {
    $Vault = Get-TermVaultDefaultVault
    if (!$NonInteractive) {
        $entered = Read-Host "Vault path (Enter for $Vault)"
        if ($entered) { $Vault = $entered }
    }
}
$Vault = [IO.Path]::GetFullPath($Vault)
if ([IO.Path]::GetExtension($Vault) -ine '.kdbx') { throw 'Choose a .kdbx vault.' }
if (Test-Path -LiteralPath $Vault) {
    if ($CreateVault) { throw '--CreateVault never overwrites an existing vault.' }
    $item = Get-Item -LiteralPath $Vault
    if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Vault must be a regular file.' }
} elseif (!$CreateVault) {
    if ($NonInteractive -or (Read-Host 'Vault does not exist. Create a NEW vault in the local TermVault UI? [y/N]') -ne 'y') { throw 'No vault selected. Nothing was overwritten.' }
    $CreateVault = $true
}
if ($NonInteractive -and $CreateVault) { throw 'Vault creation requires an interactive terminal; no passwords are accepted by setup.' }
if ($NonInteractive -and !$SkipBrowser -and !$ExtensionId) { throw 'Specify -ExtensionId or -SkipBrowser for noninteractive setup.' }
if (!$SkipBrowser -and !$Browser) {
    if ($NonInteractive) { throw 'Specify -Browser or -SkipBrowser for noninteractive setup.' }
    $choice = Read-Host 'Browser: 1 Yandex (default), 2 Chrome, 3 Edge'
    $Browser = switch ($choice) { '2' { 'Chrome' }; '3' { 'Edge' }; '' { 'Yandex' }; '1' { 'Yandex' }; default { throw 'Unknown browser choice.' } }
}
if (!$SkipShellIntegration) { Write-Host 'Setup will update your user PATH and remember the selected default vault (no administrator rights needed).' }
if (!$NonInteractive -and (Read-Host "Install/update program files in $InstallDirectory ? [y/N]") -ne 'y') { throw 'Setup cancelled.' }
if (!$SkipShellIntegration) { $null = Get-TermVaultUpdatedPath -CurrentPath '' -InstallDirectory $InstallDirectory }
$installed = Install-TermVaultPayload -SourceRoot $PSScriptRoot -BinaryPath $binary -Destination $InstallDirectory
$installedRoot = Split-Path $installed -Parent
if ($CreateVault) {
    if ($NonInteractive) { throw 'Vault creation requires an interactive terminal. Run the installed tvault.exe --create manually.' }
    Write-Host 'Enter the master passphrase ONLY in TermVault. The setup script does not read it.'
    & $installed --create $Vault
    if ($LASTEXITCODE -ne 0 -or !(Test-Path -LiteralPath $Vault -PathType Leaf)) { throw 'Vault creation cancelled or failed. Program files remain installed; run setup again to continue.' }
}
if (!$SkipShellIntegration) {
    Set-TermVaultDefaultVault -Vault $Vault
    Set-TermVaultUserPath -InstallDirectory $installedRoot
    Register-TermVaultUninstaller -InstallDirectory $installedRoot
    Write-Host 'Added TermVault to user PATH and saved the default vault. Existing PATH entries were preserved.'
}
if (!$SkipBrowser) {
    $extensionDirectory = Join-Path $installedRoot 'browser\extension'
    $extensionsPage = @{ Yandex = 'browser://extensions'; Chrome = 'chrome://extensions'; Edge = 'edge://extensions' }[$Browser]
    Write-Host "In $Browser, open $extensionsPage, enable developer mode and load:"
    Write-Host $extensionDirectory
    Write-Host 'Use this permanent folder, NOT the cloned repository or downloaded ZIP folder.'
    Write-Host 'For an update, reload the already installed extension instead of adding a duplicate.'
    if (!$ExtensionId) {
        if ($NonInteractive) { throw 'Specify -ExtensionId or -SkipBrowser for noninteractive setup.' }
        $ExtensionId = Read-Host 'Paste the extension ID (32 lowercase letters a-p); Enter to finish without browser setup'
    }
    if ($ExtensionId) {
        if ($ExtensionId -cnotmatch '^[a-p]{32}$') { throw 'Invalid extension ID. Re-run setup to repair the browser connection.' }
        & (Join-Path $installedRoot 'browser\install.ps1') -ExtensionId $ExtensionId -Browser $Browser -Vault $Vault
        Write-Host 'Open extension Details -> Extension options -> Check connection (no master password needed).'
    } else { Write-Host 'Browser registration skipped. Re-run setup when you have the extension ID.' }
}
if ($SkipShellIntegration) { Write-Host "Ready. Run: & '$installed' '$Vault'" }
else { Write-Host 'Ready. Open a NEW terminal and run: tvault' }
Write-Host 'To uninstall: Windows Settings -> Apps -> TermVault, or uninstall.cmd in the installed folder. Vaults and backups are kept.'
Write-Host 'Updating/removing program files must not delete vaults or encrypted backups.'
