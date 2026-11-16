Set-StrictMode -Version Latest

function Get-TermVaultRemovalPlan {
    param([Parameter(Mandatory = $true)][string]$InstallDirectory)
    $root = [IO.Path]::GetFullPath($InstallDirectory).TrimEnd('\', '/')
    if ($root.Length -le [IO.Path]::GetPathRoot($root).TrimEnd('\', '/').Length) { throw 'Refusing a drive root.' }
    $rootInfo = Get-Item -LiteralPath $root -Force
    if (!$rootInfo.PSIsContainer -or ($rootInfo.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Install directory must not be a link.' }
    $markerPath = Join-Path $root '.termvault-install.json'
    $markerInfo = Get-Item -LiteralPath $markerPath -Force -ErrorAction Stop
    if ($markerInfo.PSIsContainer -or $markerInfo.Length -gt 4096 -or ($markerInfo.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Invalid installation marker; no files were deleted.' }
    $marker = Get-Content -Raw -LiteralPath $markerPath | ConvertFrom-Json
    if ($marker.product -ne 'TermVault' -or $marker.schema -ne 1) { throw 'This is not a recognized TermVault installation.' }
    $files = @()
    foreach ($relative in (@('tvault.exe') + @(Get-TermVaultPayloadFiles))) {
        $file = Join-Path $root $relative
        $parent = Split-Path $file -Parent
        while ($parent.Length -gt $root.Length) {
            if ((Test-Path -LiteralPath $parent) -and ((Get-Item -LiteralPath $parent -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Refusing linked install subdirectories.' }
            $parent = Split-Path $parent -Parent
        }
        if (Test-Path -LiteralPath $file) {
            $info = Get-Item -LiteralPath $file -Force
            if ($info.PSIsContainer -or ($info.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Refusing to delete unexpected directories or links.' }
            $files += $file
        }
    }
    $exe = Join-Path $root 'tvault.exe'
    if (Test-Path -LiteralPath $exe) {
        try { $lock = [IO.File]::Open($exe, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None); $lock.Dispose() }
        catch { throw 'Close TermVault first (Ctrl+Q) and retry uninstall. No files were deleted.' }
    }
    [pscustomobject]@{ Root = $root; Files = $files; Marker = $markerPath }
}

function Remove-TermVaultPayload {
    param([Parameter(Mandatory = $true)][string]$InstallDirectory, [switch]$KeepLauncher)
    $plan = Get-TermVaultRemovalPlan -InstallDirectory $InstallDirectory
    $directories = @($plan.Root)
    foreach ($file in $plan.Files) {
        if ($KeepLauncher -and $file -ieq (Join-Path $plan.Root 'uninstall.cmd')) { continue }
        $parent = Split-Path $file -Parent
        while ($parent.Length -ge $plan.Root.Length) {
            $directories += $parent
            if ($parent -ieq $plan.Root) { break }
            $parent = Split-Path $parent -Parent
        }
        Remove-Item -LiteralPath $file -Force -ErrorAction Stop
    }
    # Keep the marker on any failure, so uninstall can be retried. Never delete user data.
    Remove-Item -LiteralPath $plan.Marker -Force
    foreach ($directory in ($directories | Select-Object -Unique | Sort-Object Length -Descending)) {
        if ((Test-Path -LiteralPath $directory) -and @(Get-ChildItem -LiteralPath $directory -Force).Count -eq 0) {
            [IO.Directory]::Delete($directory, $false)
        }
    }
}

function Register-TermVaultUninstaller {
    param([Parameter(Mandatory = $true)][string]$InstallDirectory)
    $root = [IO.Path]::GetFullPath($InstallDirectory).TrimEnd('\', '/')
    $version = Get-TermVaultBinaryVersion -BinaryPath (Join-Path $root 'tvault.exe')
    $key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Software\Microsoft\Windows\CurrentVersion\Uninstall\TermVault')
    try {
        $key.SetValue('DisplayName', 'TermVault')
        $key.SetValue('DisplayVersion', $version)
        $key.SetValue('Publisher', 'TermVault project')
        $key.SetValue('InstallLocation', $root)
        $powershell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
        $command = '"' + $powershell + '" -NoProfile -File "' + (Join-Path $root 'uninstall.ps1') + '" -InstallDirectory "' + $root + '"'
        $key.SetValue('UninstallString', $command)
        $key.SetValue('DisplayIcon', (Join-Path $root 'tvault.exe'))
        $key.SetValue('NoModify', 1, [Microsoft.Win32.RegistryValueKind]::DWord)
        $key.SetValue('NoRepair', 1, [Microsoft.Win32.RegistryValueKind]::DWord)
    } finally { $key.Dispose() }
}

function Get-TermVaultBridgeRemovalPlan {
    param([Parameter(Mandatory = $true)][string]$InstallDirectory, [string]$DataDirectory = (Join-Path $env:LOCALAPPDATA 'TermVault'))
    $root = [IO.Path]::GetFullPath($InstallDirectory).TrimEnd('\', '/')
    $bridge = Join-Path $DataDirectory 'browser'
    $manifestPath = Join-Path $bridge 'com.termvault.browser.json'
    $ownerPath = Join-Path $bridge '.termvault-owner.json'
    # Unknown/legacy bridge ownership is left intact, rather than breaking another installation.
    if (!(Test-Path -LiteralPath $ownerPath -PathType Leaf)) {
        if (Test-Path -LiteralPath $manifestPath) { Write-Warning 'Legacy bridge ownership is unknown; its files/registration were preserved. Reinstall with browser setup before uninstalling to migrate ownership.' }
        return
    }
    $ownerInfo = Get-Item -LiteralPath $ownerPath -Force
    if ($ownerInfo.Length -gt 4096 -or ($ownerInfo.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Invalid bridge ownership record.' }
    $owner = Get-Content -Raw -LiteralPath $ownerPath | ConvertFrom-Json
    if ($owner.install_directory -ine $root) { return }
    if ((Get-Item -LiteralPath $bridge -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Bridge directory must not be a link.' }
    $bridgeFiles = @('tvault-browser-host.exe', 'browser-config.json', 'com.termvault.browser.json', '.termvault-owner.json')
    foreach ($relative in $bridgeFiles) {
        $file = Join-Path $bridge $relative
        if (Test-Path -LiteralPath $file) {
            $info = Get-Item -LiteralPath $file -Force
            if ($info.PSIsContainer -or ($info.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Unexpected bridge file; stopping removal.' }
        }
    }
    $hostPath = Join-Path $bridge 'tvault-browser-host.exe'
    if (Test-Path -LiteralPath $hostPath) {
        try { $lock = [IO.File]::Open($hostPath, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None); $lock.Dispose() }
        catch { throw 'Close pending browser requests and retry uninstall; the bridge is still running.' }
    }
    [pscustomobject]@{ Directory = $bridge; Manifest = $manifestPath; Files = @($bridgeFiles | ForEach-Object { Join-Path $bridge $_ }) }
}

function Remove-TermVaultIntegrations {
    param(
        [Parameter(Mandatory = $true)][string]$InstallDirectory,
        [Microsoft.Win32.RegistryKey]$RegistryRoot = [Microsoft.Win32.Registry]::CurrentUser,
        [string]$DataDirectory = (Join-Path $env:LOCALAPPDATA 'TermVault'),
        [switch]$SkipPathUpdate
    )
    $root = [IO.Path]::GetFullPath($InstallDirectory).TrimEnd('\', '/')
    # Validate bridge ownership, links and running processes before changing any integration.
    $plan = Get-TermVaultBridgeRemovalPlan -InstallDirectory $root -DataDirectory $DataDirectory
    if (!$SkipPathUpdate) { Set-TermVaultUserPath -InstallDirectory $root -Remove }
    if ($null -ne $plan) {
      foreach ($vendor in @('Chromium', 'Google\Chrome', 'Microsoft\Edge', 'Yandex\YandexBrowser')) {
        $keyPath = "Software\$vendor\NativeMessagingHosts\com.termvault.browser"
        $key = $RegistryRoot.OpenSubKey($keyPath)
        $owned = $false
        if ($null -ne $key) {
            try { $owned = $key.GetValue('', '') -ieq $plan.Manifest } finally { $key.Dispose() }
        }
        if ($owned) { $RegistryRoot.DeleteSubKey($keyPath, $false) }
      }
      foreach ($file in $plan.Files) {
        if (Test-Path -LiteralPath $file) { Remove-Item -LiteralPath $file -Force }
      }
      if (@(Get-ChildItem -LiteralPath $plan.Directory -Force).Count -eq 0) { [IO.Directory]::Delete($plan.Directory, $false) }
    }
    $uninstallPath = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\TermVault'
    $key = $RegistryRoot.OpenSubKey($uninstallPath)
    $owned = $false
    if ($null -ne $key) {
        try { $owned = $key.GetValue('InstallLocation', '') -ieq $root } finally { $key.Dispose() }
    }
    if ($owned) { $RegistryRoot.DeleteSubKey($uninstallPath, $false) }
}
