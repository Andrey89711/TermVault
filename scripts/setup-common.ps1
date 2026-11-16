Set-StrictMode -Version Latest

function Get-TermVaultPayloadFiles {
    @(
        'LICENSE', 'README.md', 'SECURITY.md', 'CHANGELOG.md', 'docs/RELEASING.md',
        'setup.ps1', 'setup.cmd', 'scripts/setup-common.ps1',
        'browser/README.md', 'browser/QUICKSTART.ru.md', 'browser/install.ps1', 'browser/uninstall.ps1',
        'browser/extension/manifest.json', 'browser/extension/background.js',
        'browser/extension/content.js', 'browser/extension/package.json',
        'browser/extension/options.html', 'browser/extension/options.js'
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
