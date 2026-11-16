[CmdletBinding()]
param([Parameter(Mandatory = $true)][string]$BinaryPath)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'setup-common.ps1')
. (Join-Path $PSScriptRoot 'uninstall-common.ps1')
$repo = Split-Path $PSScriptRoot -Parent
$tempParent = if (Test-Path -LiteralPath (Join-Path $env:TEMP 'opencode')) { Join-Path $env:TEMP 'opencode' } else { $env:TEMP }
$id = [guid]::NewGuid().ToString('N')
$root = Join-Path $tempParent ('termvault-uninstall-test-' + $id)
$testKeyPath = 'Software\TermVaultTests\' + $id
$registry = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($testKeyPath)
New-Item -ItemType Directory -Path $root | Out-Null
try {
    $install = Join-Path $root 'installed'
    $binary = Install-TermVaultPayload -SourceRoot $repo -BinaryPath $BinaryPath -Destination $install
    $install = Split-Path $binary -Parent
    $path = 'C:\Other;' + $install + ';%SystemRoot%\System32'
    if ((Get-TermVaultUpdatedPath -CurrentPath $path -InstallDirectory $install -Remove) -cne 'C:\Other;%SystemRoot%\System32') { throw 'PATH removal changed unrelated entries.' }
    if ((Get-TermVaultUpdatedPath -CurrentPath 'C:\Other' -InstallDirectory $install -Remove) -cne 'C:\Other') { throw 'Absent PATH entry was not a no-op.' }
    $vault = Join-Path $install 'keep.kdbx'
    $backup = Join-Path $install 'nested/keep.kdbx.backup-test.kdbx'
    New-Item -ItemType Directory -Path (Split-Path $backup -Parent) | Out-Null
    [IO.File]::WriteAllText($vault, 'DUMMY VAULT')
    [IO.File]::WriteAllText($backup, 'DUMMY BACKUP')
    $unknown = Join-Path $root 'not-an-install'
    New-Item -ItemType Directory -Path $unknown | Out-Null
    [IO.File]::WriteAllText((Join-Path $unknown 'keep.txt'), 'KEEP')
    $rejected = $false
    try { $null = Get-TermVaultRemovalPlan -InstallDirectory $unknown } catch { $rejected = $true }
    if (!$rejected -or !(Test-Path -LiteralPath (Join-Path $unknown 'keep.txt'))) { throw 'Unknown directory was not protected.' }
    $lock = [IO.File]::Open($binary, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        $rejected = $false
        try { $null = Get-TermVaultRemovalPlan -InstallDirectory $install } catch { $rejected = $true }
        if (!$rejected) { throw 'Running/locked executable was not protected.' }
    } finally { $lock.Dispose() }
    & (Join-Path $install 'uninstall.ps1') -InstallDirectory $install -ConfirmRemoval -SkipUserIntegration -WhatIf
    if (!(Test-Path -LiteralPath $binary)) { throw 'WhatIf removed program files.' }

    # Fake bridge data and a dedicated test registry subtree; never use real application registrations.
    $data = Join-Path $root 'data'
    $bridge = Join-Path $data 'browser'
    New-Item -ItemType Directory -Path $bridge -Force | Out-Null
    [IO.File]::WriteAllText((Join-Path $data 'vault.kdbx'), 'DATA VAULT')
    [IO.File]::WriteAllText((Join-Path $data 'settings.json'), 'SETTINGS TO KEEP')
    $manifest = Join-Path $bridge 'com.termvault.browser.json'
    foreach ($file in @('tvault-browser-host.exe', 'browser-config.json', 'com.termvault.browser.json')) { [IO.File]::WriteAllText((Join-Path $bridge $file), 'DUMMY BRIDGE') }
    $owner = Join-Path $bridge '.termvault-owner.json'
    [IO.File]::WriteAllText($owner, (@{ install_directory = (Join-Path $root 'other-install') } | ConvertTo-Json))
    $hostKey = 'Software\Chromium\NativeMessagingHosts\com.termvault.browser'
    $otherKey = 'Software\Google\Chrome\NativeMessagingHosts\com.termvault.browser'
    $key = $registry.CreateSubKey($hostKey); $key.SetValue('', $manifest); $key.Dispose()
    $key = $registry.CreateSubKey($otherKey); $key.SetValue('', 'C:\Other\manifest.json'); $key.Dispose()
    $uninstallKey = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\TermVault'
    $key = $registry.CreateSubKey($uninstallKey); $key.SetValue('InstallLocation', $install); $key.Dispose()
    Remove-TermVaultIntegrations -InstallDirectory $install -RegistryRoot $registry -DataDirectory $data -SkipPathUpdate
    if ($null -ne $registry.OpenSubKey($uninstallKey)) { throw 'Owned Windows Apps entry was not removed.' }
    if (!(Test-Path -LiteralPath $manifest)) { throw 'Another installation bridge was removed.' }
    $key = $registry.OpenSubKey($hostKey)
    if ($null -eq $key) { throw 'Another installation registration was removed.' }; $key.Dispose()
    [IO.File]::WriteAllText($owner, (@{ install_directory = $install } | ConvertTo-Json))
    $key = $registry.CreateSubKey($uninstallKey); $key.SetValue('InstallLocation', (Join-Path $root 'other-install')); $key.Dispose()
    Remove-TermVaultIntegrations -InstallDirectory $install -RegistryRoot $registry -DataDirectory $data -SkipPathUpdate
    $key = $registry.OpenSubKey($uninstallKey)
    if ($null -eq $key) { throw 'Another installation Windows Apps entry was removed.' }; $key.Dispose()
    if ($null -ne $registry.OpenSubKey($hostKey)) { throw 'Owned registration was not removed.' }
    $key = $registry.OpenSubKey($otherKey)
    if ($null -eq $key) { throw 'Unrelated registration was removed.' }; $key.Dispose()
    if ((Test-Path -LiteralPath $bridge) -or (Get-Content -Raw (Join-Path $data 'vault.kdbx')) -ne 'DATA VAULT' -or (Get-Content -Raw (Join-Path $data 'settings.json')) -ne 'SETTINGS TO KEEP') { throw 'Bridge cleanup failed or touched retained data.' }
    & (Join-Path $install 'uninstall.ps1') -InstallDirectory $install -ConfirmRemoval -SkipUserIntegration
    if ((Test-Path -LiteralPath $binary) -or (Test-Path -LiteralPath (Join-Path $install '.termvault-install.json'))) { throw 'Program files remain after uninstall.' }
    if ((Get-Content -Raw $vault) -ne 'DUMMY VAULT' -or (Get-Content -Raw $backup) -ne 'DUMMY BACKUP') { throw 'Uninstall changed a vault or backup.' }
    $clean = Join-Path $root 'clean install'
    $null = Install-TermVaultPayload -SourceRoot $repo -BinaryPath $BinaryPath -Destination $clean
    & (Join-Path $clean 'uninstall.ps1') -ConfirmRemoval -SkipUserIntegration
    if (Test-Path -LiteralPath $clean) { throw 'Empty installation directory was not removed.' }
    $cmdInstall = Join-Path $root 'cmd install'
    $null = Install-TermVaultPayload -SourceRoot $repo -BinaryPath $BinaryPath -Destination $cmdInstall
    $command = 'call "' + (Join-Path $cmdInstall 'uninstall.cmd') + '" -ConfirmRemoval -SkipUserIntegration <NUL'
    & $env:ComSpec /d /c $command
    if ($LASTEXITCODE -ne 0) { throw 'Uninstall launcher reported a failure.' }
    if (Test-Path -LiteralPath $cmdInstall) { throw 'Double-click launcher did not remove the clean test installation.' }
    Write-Host 'Uninstall tests passed: file allowlist, data preservation, PATH filtering, WhatIf, locked EXE, bridge ownership and isolated registry cleanup.'
} finally {
    $registry.Dispose()
    [Microsoft.Win32.Registry]::CurrentUser.DeleteSubKeyTree($testKeyPath, $false)
    # Only this invocation's generated test folder and dummy data.
    Remove-Item -LiteralPath $root -Recurse -Force
}
