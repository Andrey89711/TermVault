Set-StrictMode -Version Latest

function Get-TermVaultUpdatedPath {
    param([AllowNull()][AllowEmptyString()][string]$CurrentPath, [Parameter(Mandatory = $true)][string]$InstallDirectory, [switch]$Remove)
    $directory = [IO.Path]::GetFullPath($InstallDirectory).TrimEnd('\', '/')
    if ($directory.IndexOfAny([char[]]';"%') -ge 0) { throw 'Install path cannot contain semicolons, quotes or percent signs when adding to PATH.' }
    $remaining = @()
    if ($CurrentPath) {
        foreach ($entry in ($CurrentPath -split ';')) {
            $normalized = [Environment]::ExpandEnvironmentVariables($entry.Trim().Trim('"')).TrimEnd('\', '/')
            if ($normalized -ine $directory) { $remaining += $entry }
        }
    }
    # Only move/deduplicate our own directory. Preserve all other raw entries and their order.
    if ($Remove) { return ($remaining -join ';') }
    (@($directory) + $remaining) -join ';'
}

function Set-TermVaultUserPath {
    param([Parameter(Mandatory = $true)][string]$InstallDirectory, [switch]$Remove)
    $key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Environment')
    $changed = $false
    try {
        $raw = $key.GetValue('Path', $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
        $updated = Get-TermVaultUpdatedPath -CurrentPath $raw -InstallDirectory $InstallDirectory -Remove:$Remove
        if (($null -ne $raw -or !$Remove) -and $raw -cne $updated) {
            $kind = if ($null -eq $raw) { [Microsoft.Win32.RegistryValueKind]::ExpandString } else { $key.GetValueKind('Path') }
            if ($kind -notin @([Microsoft.Win32.RegistryValueKind]::String, [Microsoft.Win32.RegistryValueKind]::ExpandString)) { throw 'User PATH has an unsupported registry type; it was not modified.' }
            $key.SetValue('Path', $updated, $kind)
            $changed = $true
        }
    } finally { $key.Dispose() }
    $env:Path = Get-TermVaultUpdatedPath -CurrentPath $env:Path -InstallDirectory $InstallDirectory -Remove:$Remove
    if ($changed) {
        if (!('TermVault.InstallerEnvironment' -as [type])) {
            Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
namespace TermVault {
    public static class InstallerEnvironment {
        [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        public static extern IntPtr SendMessageTimeout(IntPtr window, uint message, UIntPtr wparam,
            string lparam, uint flags, uint timeout, out UIntPtr result);
    }
}
'@
        }
        $result = [UIntPtr]::Zero
        [void][TermVault.InstallerEnvironment]::SendMessageTimeout([IntPtr]0xffff, 0x1a, [UIntPtr]::Zero, 'Environment', 2, 3000, [ref]$result)
    }
}

function Get-TermVaultDefaultVault {
    param([string]$SettingsPath = (Join-Path $env:LOCALAPPDATA 'TermVault\settings.json'))
    if (!(Test-Path -LiteralPath $SettingsPath)) { return (Join-Path $env:LOCALAPPDATA 'TermVault\vault.kdbx') }
    $info = Get-Item -LiteralPath $SettingsPath
    if ($info.PSIsContainer -or $info.Length -gt 8192 -or ($info.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Invalid TermVault settings file.' }
    $settings = Get-Content -Raw -Encoding UTF8 -LiteralPath $SettingsPath | ConvertFrom-Json
    if (@($settings.PSObject.Properties.Name).Count -ne 1 -or !$settings.vault -or ![IO.Path]::IsPathRooted($settings.vault) -or [IO.Path]::GetExtension($settings.vault) -ine '.kdbx') { throw 'Invalid TermVault default vault setting.' }
    [string]$settings.vault
}

function Set-TermVaultDefaultVault {
    param([Parameter(Mandatory = $true)][string]$Vault, [string]$SettingsPath = (Join-Path $env:LOCALAPPDATA 'TermVault\settings.json'))
    if (![IO.Path]::IsPathRooted($Vault) -or [IO.Path]::GetExtension($Vault) -ine '.kdbx') { throw 'Default vault must be an absolute .kdbx path.' }
    if (Test-Path -LiteralPath $SettingsPath) { $null = Get-TermVaultDefaultVault -SettingsPath $SettingsPath }
    $parent = Split-Path $SettingsPath -Parent
    New-Item -ItemType Directory -Path $parent -Force | Out-Null
    $temporary = Join-Path $parent ('.settings-' + [guid]::NewGuid().ToString('N') + '.tmp')
    try {
        [IO.File]::WriteAllText($temporary, (@{ vault = $Vault } | ConvertTo-Json), (New-Object Text.UTF8Encoding($false)))
        if (Test-Path -LiteralPath $SettingsPath) { [IO.File]::Replace($temporary, $SettingsPath, [NullString]::Value) }
        else { [IO.File]::Move($temporary, $SettingsPath) }
    } finally {
        if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary }
    }
}

function Get-TermVaultPayloadFiles {
    @(
        'LICENSE', 'README.md', 'SECURITY.md', 'PRIVACY.md', 'CHANGELOG.md', 'docs/RELEASING.md', 'docs/STORE-PUBLISHING.ru.md',
        'setup.ps1', 'setup.cmd', 'uninstall.ps1', 'uninstall.cmd', 'scripts/setup-common.ps1', 'scripts/uninstall-common.ps1',
        'browser/README.md', 'browser/QUICKSTART.ru.md', 'browser/install.ps1', 'browser/uninstall.ps1',
        'browser/extension/manifest.json', 'browser/extension/background.js',
        'browser/extension/content.js', 'browser/extension/package.json',
        'browser/extension/options.html', 'browser/extension/options.js',
        'browser/extension/icons/icon-16.png', 'browser/extension/icons/icon-32.png',
        'browser/extension/icons/icon-48.png', 'browser/extension/icons/icon-128.png'
    )
}

function Get-TermVaultBinaryVersion {
    param([Parameter(Mandatory = $true)][string]$BinaryPath)
    $output = & $BinaryPath --version
    if ($LASTEXITCODE -ne 0 -or $output -notmatch '^tvault (\d+\.\d+\.\d+)$') {
        throw 'Cannot read a compatible TermVault binary version.'
    }
    $Matches[1]
}

function Install-TermVaultPayload {
    param(
        [Parameter(Mandatory = $true)][string]$SourceRoot,
        [Parameter(Mandatory = $true)][string]$BinaryPath,
        [Parameter(Mandatory = $true)][string]$Destination
    )
    $source = (Resolve-Path -LiteralPath $SourceRoot).ProviderPath
    $binary = (Resolve-Path -LiteralPath $BinaryPath).ProviderPath
    $destinationPath = [IO.Path]::GetFullPath($Destination).TrimEnd('\', '/')
    if ($destinationPath.Length -le [IO.Path]::GetPathRoot($destinationPath).TrimEnd('\', '/').Length) { throw 'Do not install into a drive root.' }
    $version = Get-TermVaultBinaryVersion -BinaryPath $binary
    $manifest = Get-Content -Raw -LiteralPath (Join-Path $source 'browser/extension/manifest.json') | ConvertFrom-Json
    if ($manifest.version -ne $version) { throw 'Binary and extension versions differ. Rebuild or use a complete release ZIP.' }
    $files = @(Get-TermVaultPayloadFiles)
    foreach ($relative in $files) {
        if (!(Test-Path -LiteralPath (Join-Path $source $relative) -PathType Leaf)) { throw "Incomplete package: $relative" }
    }
    $markerPath = Join-Path $destinationPath '.termvault-install.json'
    if (Test-Path -LiteralPath $destinationPath) {
        $info = Get-Item -LiteralPath $destinationPath
        if (!$info.PSIsContainer -or ($info.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw 'Install destination must be a regular directory, not a link.'
        }
        if (@(Get-ChildItem -Force -LiteralPath $destinationPath).Count -gt 0) {
            if (!(Test-Path -LiteralPath $markerPath -PathType Leaf)) { throw 'Destination is not an existing TermVault installation or an empty directory.' }
            $markerInfo = Get-Item -LiteralPath $markerPath -Force
            if ($markerInfo.Length -gt 4096 -or ($markerInfo.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Invalid installation marker.' }
            $marker = Get-Content -Raw -LiteralPath $markerPath | ConvertFrom-Json
            if ($marker.product -ne 'TermVault' -or $marker.schema -ne 1) { throw 'Unrecognized installation marker.' }
            if ([version]$marker.version -gt [version]$version) { throw 'Refusing an automatic downgrade. Use the current or a newer release.' }
        }
    }
    if ($source.TrimEnd('\', '/') -ieq $destinationPath) {
        # Repair from an installed copy: do not overwrite the running script or extension files.
        return (Join-Path $destinationPath 'tvault.exe')
    }
    New-Item -ItemType Directory -Path $destinationPath -Force | Out-Null
    $stage = Join-Path $destinationPath ('.setup-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $stage | Out-Null
    try {
        Copy-Item -LiteralPath $binary -Destination (Join-Path $stage 'tvault.exe')
        foreach ($relative in $files) {
            $target = Join-Path $stage $relative
            New-Item -ItemType Directory -Path (Split-Path $target -Parent) -Force | Out-Null
            Copy-Item -LiteralPath (Join-Path $source $relative) -Destination $target
        }
        # Binary first: a running/locked EXE fails before modifying extension files.
        foreach ($relative in (@('tvault.exe') + $files)) {
            $target = Join-Path $destinationPath $relative
            if (Test-Path -LiteralPath $target) {
                if ((Get-Item -LiteralPath $target).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Refusing to replace a link: $relative" }
            }
            $parent = Split-Path $target -Parent
            while ($parent.Length -gt $destinationPath.Length) {
                if ((Test-Path -LiteralPath $parent) -and ((Get-Item -LiteralPath $parent).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Install subdirectories must not be links.' }
                $parent = Split-Path $parent -Parent
            }
            New-Item -ItemType Directory -Path (Split-Path $target -Parent) -Force | Out-Null
            Copy-Item -LiteralPath (Join-Path $stage $relative) -Destination $target -Force
        }
        $marker = @{ product = 'TermVault'; schema = 1; version = $version } | ConvertTo-Json
        [IO.File]::WriteAllText($markerPath, $marker, (New-Object Text.UTF8Encoding($false)))
    } finally {
        Remove-Item -LiteralPath $stage -Recurse -Force
    }
    Join-Path $destinationPath 'tvault.exe'
}
