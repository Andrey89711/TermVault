[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[a-p]{32}$')]
    [string]$ExtensionId,
    [string]$Vault = (Join-Path $env:LOCALAPPDATA 'TermVault\vault.kdbx'),
    [ValidateSet('Yandex', 'Chrome', 'Edge')]
    [string]$Browser = 'Yandex'
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
$binary = Join-Path $repo 'target\browser-build\release\tvault.exe'
if (Test-Path -LiteralPath (Join-Path $repo 'tvault.exe') -PathType Leaf) {
    $binary = Join-Path $repo 'tvault.exe' # Prebuilt release ZIP; no Rust toolchain required.
}
if (!(Test-Path -LiteralPath $binary -PathType Leaf)) { throw 'Run cargo build --release --locked --target-dir target/browser-build first.' }
$vaultPath = (Resolve-Path -LiteralPath $Vault).ProviderPath
if ([IO.Path]::GetExtension($vaultPath) -ine '.kdbx') { throw 'Choose an existing .kdbx vault.' }
$directory = Join-Path $env:LOCALAPPDATA 'TermVault\browser'
New-Item -ItemType Directory -Path $directory -Force | Out-Null
# Restrict bridge code/config to this user and SYSTEM (not protection from same-user malware).
$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
# Build a DACL-only update: replaying Get-Acl can request privileged SACL changes
# on an existing installation, despite only access permissions being intended.
$acl = New-Object Security.AccessControl.DirectorySecurity
$acl.SetSecurityDescriptorSddlForm("D:P(A;OICI;FA;;;${sid})(A;OICI;FA;;;SY)", [Security.AccessControl.AccessControlSections]::Access)
# Persist only modified access rules, without PowerShell Set-Acl's broader sections.
$directoryInfo = New-Object IO.DirectoryInfo($directory)
if ($PSVersionTable.PSEdition -eq 'Core') {
    [IO.FileSystemAclExtensions]::SetAccessControl($directoryInfo, $acl)
} else {
    $directoryInfo.SetAccessControl($acl)
}
$hostPath = Join-Path $directory 'tvault-browser-host.exe'
Copy-Item -LiteralPath $binary -Destination $hostPath -Force
$utf8 = New-Object Text.UTF8Encoding($false)
$config = @{ extension_id = $ExtensionId; vault = $vaultPath } | ConvertTo-Json
[IO.File]::WriteAllText((Join-Path $directory 'browser-config.json'), $config, $utf8)
$manifestPath = Join-Path $directory 'com.termvault.browser.json'
$manifest = @{
    name = 'com.termvault.browser'
    description = 'TermVault local read-only browser bridge'
    path = $hostPath
    type = 'stdio'
    allowed_origins = @("chrome-extension://$ExtensionId/")
} | ConvertTo-Json
[IO.File]::WriteAllText($manifestPath, $manifest, $utf8)
# Yandex 26.10 uses Chromium's Native Messaging lookup, not a Yandex-specific key.
$vendor = @{ Yandex = 'Chromium'; Chrome = 'Google\Chrome'; Edge = 'Microsoft\Edge' }[$Browser]
$key = "HKCU:\Software\$vendor\NativeMessagingHosts\com.termvault.browser"
New-Item -Path $key -Force | Out-Null
Set-Item -Path $key -Value $manifestPath
if ($Browser -eq 'Yandex') {
    $legacyKey = 'HKCU:\Software\Yandex\YandexBrowser\NativeMessagingHosts\com.termvault.browser'
    if ((Test-Path -LiteralPath $legacyKey) -and ((Get-Item -LiteralPath $legacyKey).GetValue('') -eq $manifestPath)) {
        Remove-Item -LiteralPath $legacyKey
    }
}
Write-Host "Installed for $Browser (current user only). Vault: $vaultPath"
Write-Host 'Pin the extension, open an HTTPS login page and click TermVault.'
Write-Host 'Re-run this script after rebuilding or changing the extension ID/vault path.'
