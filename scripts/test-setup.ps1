[CmdletBinding()]
param([Parameter(Mandatory = $true)][string]$BinaryPath)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'setup-common.ps1')
$repo = Split-Path $PSScriptRoot -Parent
$tempParent = if (Test-Path -LiteralPath (Join-Path $env:TEMP 'opencode')) { Join-Path $env:TEMP 'opencode' } else { $env:TEMP }
$root = Join-Path $tempParent ('termvault-setup-test-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root | Out-Null
try {
    $destination = Join-Path $root 'installed'
    $binary = Install-TermVaultPayload -SourceRoot $repo -BinaryPath $BinaryPath -Destination $destination
    $destination = Split-Path $binary -Parent
    if (!(Test-Path -LiteralPath $binary)) { throw 'Missing installed executable.' }
    $ownership = Get-Content -Raw -LiteralPath (Join-Path $destination '.termvault-install.json') | ConvertFrom-Json
    if ($ownership.payload_layout -ne 2) { throw 'Installer must record the current owned payload layout for in-app updates.' }
    if (!(Test-Path -LiteralPath (Join-Path $destination 'browser/extension/options.html'))) { throw 'Missing extension diagnostics.' }
    $dummyVault = Join-Path $destination 'preserve.kdbx'
    [IO.File]::WriteAllText($dummyVault, 'DUMMY CIPHERTEXT - never delete during update')
    $before = (Get-FileHash -LiteralPath $dummyVault).Hash
    $existingPath = 'C:\Other;%SystemRoot%\System32;C:\Tools;'
    $updatedPath = Get-TermVaultUpdatedPath -CurrentPath $existingPath -InstallDirectory $destination
    if ($updatedPath -cne "$destination;$existingPath") { throw "PATH update did not preserve raw entries: expected [$destination;$existingPath], got [$updatedPath]." }
    $duplicates = "$existingPath;`"$($destination.ToUpperInvariant())\`";$destination"
    if ((Get-TermVaultUpdatedPath -CurrentPath $duplicates -InstallDirectory $destination) -cne "$destination;$existingPath") { throw 'PATH deduplication changed unrelated entries.' }
    if ((Get-TermVaultUpdatedPath -CurrentPath $updatedPath -InstallDirectory $destination) -cne $updatedPath) { throw 'PATH update is not idempotent.' }
    if ((Get-TermVaultUpdatedPath -CurrentPath $null -InstallDirectory $destination) -cne $destination) { throw 'Empty PATH handling failed.' }
    $settingsPath = Join-Path $root 'settings/settings.json'
    Set-TermVaultDefaultVault -Vault $dummyVault -SettingsPath $settingsPath
    if ((Get-TermVaultDefaultVault -SettingsPath $settingsPath) -cne $dummyVault) { throw 'Default vault was not saved.' }
    $differentVault = Join-Path $root 'other vault.kdbx'
    Set-TermVaultDefaultVault -Vault $differentVault -SettingsPath $settingsPath
    if ((Get-TermVaultDefaultVault -SettingsPath $settingsPath) -cne $differentVault) { throw 'Default vault update failed.' }
    if (Test-Path -LiteralPath $differentVault) { throw 'Saving a default must not create/open its vault.' }
    $withLanguage = Get-Content -Raw -LiteralPath $settingsPath | ConvertFrom-Json
    $withLanguage | Add-Member -NotePropertyName language -NotePropertyValue 'ru'
    $withLanguage | Add-Member -NotePropertyName future_setting -NotePropertyValue ([pscustomobject]@{ nested = 42 })
    [IO.File]::WriteAllText($settingsPath, ($withLanguage | ConvertTo-Json -Depth 10))
    if ((Get-TermVaultDefaultVault -SettingsPath $settingsPath) -cne $differentVault) { throw 'Language preference broke setup settings loading.' }
    Set-TermVaultDefaultVault -Vault $dummyVault -SettingsPath $settingsPath
    $preserved = Get-Content -Raw -LiteralPath $settingsPath | ConvertFrom-Json
    if ($preserved.language -cne 'ru' -or $preserved.future_setting.nested -ne 42) { throw 'Changing the default vault discarded language or other settings.' }
    [IO.File]::WriteAllText($settingsPath, '{"language":"en"}')
    if ((Get-TermVaultDefaultVault -SettingsPath $settingsPath) -cne (Join-Path (Split-Path $settingsPath -Parent) 'vault.kdbx')) { throw 'Language-only preferences broke default vault resolution.' }
    Set-TermVaultDefaultVault -Vault $dummyVault -SettingsPath $settingsPath
    if ((Get-Content -Raw -LiteralPath $settingsPath | ConvertFrom-Json).language -cne 'en') { throw 'Language-only preference was lost.' }
    foreach ($invalid in @('{"vault":false}', '{"language":"de"}', '[]')) {
        [IO.File]::WriteAllText($settingsPath, $invalid)
        $rejected = $false
        try { Set-TermVaultDefaultVault -Vault $dummyVault -SettingsPath $settingsPath } catch { $rejected = $true }
        if (!$rejected -or (Get-Content -Raw -LiteralPath $settingsPath) -cne $invalid) { throw 'Invalid settings were overwritten.' }
    }
    $null = Install-TermVaultPayload -SourceRoot $repo -BinaryPath $BinaryPath -Destination $destination
    $null = Install-TermVaultPayload -SourceRoot $destination -BinaryPath $binary -Destination $destination
    if ((Get-FileHash -LiteralPath $dummyVault).Hash -ne $before) { throw 'Update changed an unrelated file.' }
    # Exercise the actual packaged wizard, without browser registration or reading any secrets.
    $wizardDestination = Join-Path $root 'wizard-install'
    & (Join-Path $destination 'setup.ps1') -NonInteractive -SkipBrowser -SkipShellIntegration -Vault $dummyVault -InstallDirectory $wizardDestination
    if (!(Test-Path -LiteralPath (Join-Path $wizardDestination 'tvault.exe'))) { throw 'Packaged wizard did not install.' }
    $missingVault = Join-Path $root 'must-not-create.kdbx'
    $rejected = $false
    try { & (Join-Path $destination 'setup.ps1') -NonInteractive -SkipBrowser -SkipShellIntegration -CreateVault -Vault $missingVault -InstallDirectory (Join-Path $root 'must-not-install') }
    catch { $rejected = $true }
    if (!$rejected -or (Test-Path -LiteralPath $missingVault) -or (Test-Path -LiteralPath (Join-Path $root 'must-not-install'))) { throw 'Noninteractive creation was not rejected before installation.' }
    $unknown = Join-Path $root 'user-files'
    New-Item -ItemType Directory -Path $unknown | Out-Null
    [IO.File]::WriteAllText((Join-Path $unknown 'keep.txt'), 'KEEP')
    $rejected = $false
    try { $null = Install-TermVaultPayload -SourceRoot $repo -BinaryPath $BinaryPath -Destination $unknown }
    catch { $rejected = $true }
    if (!$rejected -or (Get-Content -Raw (Join-Path $unknown 'keep.txt')) -ne 'KEEP') { throw 'Non-TermVault directory was not protected.' }
    $marker = Join-Path $destination '.termvault-install.json'
    [IO.File]::WriteAllText($marker, '{"product":"TermVault","schema":1,"version":"999.0.0"}')
    $rejected = $false
    try { $null = Install-TermVaultPayload -SourceRoot $repo -BinaryPath $BinaryPath -Destination $destination }
    catch { $rejected = $true }
    if (!$rejected) { throw 'Downgrade was not rejected.' }
    Write-Host 'Setup tests passed: install, repeat/update, repair, unrelated-file preservation, unknown-directory and downgrade protection.'
    Write-Host 'No real vaults, browser registrations or user installations were touched.'
} finally {
    # Only this test invocation's generated temporary directory (including dummy data).
    Remove-Item -LiteralPath $root -Recurse -Force
}
