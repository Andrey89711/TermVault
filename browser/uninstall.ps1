[CmdletBinding()]
param([ValidateSet('Yandex', 'Chrome', 'Edge')][string]$Browser = 'Yandex')
$ErrorActionPreference = 'Stop'
$vendor = @{ Yandex = 'Chromium'; Chrome = 'Google\Chrome'; Edge = 'Microsoft\Edge' }[$Browser]
$key = "HKCU:\Software\$vendor\NativeMessagingHosts\com.termvault.browser"
if (Test-Path -LiteralPath $key) { Remove-Item -LiteralPath $key }
# Keep files/config: another supported browser might still be using this bridge.
Write-Host "Unregistered TermVault bridge for $Browser. No vaults or backups were touched."
Write-Host 'Remove the extension manually on the browser extensions page.'
