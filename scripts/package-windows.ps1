[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$BinaryPath,
    [string]$OutputDirectory = (Join-Path (Split-Path $PSScriptRoot -Parent) 'dist')
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'setup-common.ps1')
$repo = Split-Path $PSScriptRoot -Parent
$binary = (Resolve-Path -LiteralPath $BinaryPath).ProviderPath
$cargo = Get-Content -Raw -LiteralPath (Join-Path $repo 'Cargo.toml')
$versionMatch = [regex]::Match($cargo, '(?m)^version\s*=\s*"(\d+\.\d+\.\d+)"\s*$')
if (!$versionMatch.Success) { throw 'Cargo.toml must specify a stable X.Y.Z version.' }
$version = $versionMatch.Groups[1].Value
$manifest = Get-Content -Raw -LiteralPath (Join-Path $repo 'browser/extension/manifest.json') | ConvertFrom-Json
if ($manifest.version -ne $version) { throw 'Rust and extension versions must match.' }
$binaryVersion = & $binary --version
if ($LASTEXITCODE -ne 0 -or $binaryVersion -ne "tvault $version") { throw 'Binary version does not match Cargo.toml.' }
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$output = (Resolve-Path -LiteralPath $OutputDirectory).ProviderPath
$name = "termvault-$version-windows-x86_64"
$archive = Join-Path $output "$name.zip"
$checksums = Join-Path $output 'SHA256SUMS.txt'
if ((Test-Path -LiteralPath $archive) -or (Test-Path -LiteralPath $checksums)) {
    throw 'Release output already exists. Choose a fresh output directory; existing artifacts are never overwritten.'
}
$stage = Join-Path $output ('.stage-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $stage | Out-Null
try {
    Copy-Item -LiteralPath $binary -Destination (Join-Path $stage 'tvault.exe')
    # Explicit allowlist: never include vaults, backups, keys, local bridge config or build folders.
    $files = @(Get-TermVaultPayloadFiles)
    foreach ($relative in $files) {
        $destination = Join-Path $stage $relative
        New-Item -ItemType Directory -Path (Split-Path $destination -Parent) -Force | Out-Null
        Copy-Item -LiteralPath (Join-Path $repo $relative) -Destination $destination
    }
    Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $archive
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [IO.Compression.ZipFile]::OpenRead($archive)
    try {
        $actual = @($zip.Entries | Where-Object { $_.Name } | ForEach-Object { $_.FullName.Replace('\', '/') } | Sort-Object)
        $expected = @(@('tvault.exe') + $files | Sort-Object)
        if (Compare-Object $expected $actual) { throw 'ZIP contents do not match the release allowlist.' }
    } finally { $zip.Dispose() }
    $hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText($checksums, "$hash  $name.zip`n", (New-Object Text.UTF8Encoding($false)))
    Write-Host "Created and verified $archive"
} finally {
    # Only remove this invocation's generated staging directory.
    Remove-Item -LiteralPath $stage -Recurse -Force
}
