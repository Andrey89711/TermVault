[CmdletBinding(SupportsShouldProcess = $true)]
param(
    [string]$InstallDirectory,
    [switch]$ConfirmRemoval,
    [switch]$KeepLauncher,
    [switch]$SkipUserIntegration
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:OS -ne 'Windows_NT') { throw 'This uninstaller supports Windows only.' }
. (Join-Path $PSScriptRoot 'scripts/setup-common.ps1')
. (Join-Path $PSScriptRoot 'scripts/uninstall-common.ps1')
if (!$InstallDirectory) {
    $InstallDirectory = if (Test-Path -LiteralPath (Join-Path $PSScriptRoot '.termvault-install.json')) { $PSScriptRoot } else { Join-Path $env:LOCALAPPDATA 'Programs\TermVault' }
}
$plan = Get-TermVaultRemovalPlan -InstallDirectory $InstallDirectory
Write-Host "Remove TermVault program files from: $($plan.Root)"
Write-Host 'Vaults, encrypted backups, settings and unrelated files will be KEPT.'
Write-Host 'The browser extension must be removed in the browser; this utility never edits browser profiles.'
if (!$PSCmdlet.ShouldProcess($plan.Root, 'Uninstall TermVault (keep all vaults and backups)')) { return }
if (!$ConfirmRemoval -and (Read-Host 'Uninstall TermVault? [y/N]') -ne 'y') { Write-Host 'Cancelled. Nothing was removed.'; return }
if (!$SkipUserIntegration) { Remove-TermVaultIntegrations -InstallDirectory $plan.Root }
Remove-TermVaultPayload -InstallDirectory $plan.Root -KeepLauncher:$KeepLauncher
Write-Host 'TermVault uninstalled. Your vaults, backups and default-vault settings were preserved.'
Write-Host 'Remove the extension in your browser. Restart your terminal to refresh PATH.'
if (Test-Path -LiteralPath $plan.Root) {
    $remaining = @(Get-ChildItem -LiteralPath $plan.Root -Force | Where-Object { !$KeepLauncher -or $_.Name -ine 'uninstall.cmd' })
    if ($remaining.Count -gt 0) { Write-Host 'The install folder contains other files; those were left untouched.' }
}
