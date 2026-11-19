[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repo = Split-Path $PSScriptRoot -Parent
$base = Join-Path $env:LOCALAPPDATA 'Temp\opencode'
$work = Join-Path $base ('termvault-launcher-test-' + [guid]::NewGuid().ToString('N'))
$previousPolicy = $env:PSExecutionPolicyPreference
try {
    New-Item -ItemType Directory -Path $work -Force | Out-Null
    # Dummy scripts only: no actual setup, vault, PATH, registration or user config.
    $probe = @'
param([string]$ProbePath, [int]$ExitCode = 0)
$ErrorActionPreference = 'Stop'
if ($env:PSExecutionPolicyPreference -ne 'Bypass') { throw 'Expected process-only Bypass.' }
[IO.File]::WriteAllText($ProbePath, 'launch-ok')
exit $ExitCode
'@
    $env:PSExecutionPolicyPreference = 'AllSigned'
    foreach ($name in @('setup', 'uninstall')) {
        Copy-Item -LiteralPath (Join-Path $repo "$name.cmd") -Destination $work
        $script = Join-Path $work "$name.ps1"
        [IO.File]::WriteAllText($script, $probe)
        [IO.File]::WriteAllText(($script + ':Zone.Identifier'), "[ZoneTransfer]`r`nZoneId=3`r`n")
        foreach ($expected in @(0, 7)) {
            $marker = Join-Path $work "$name-$expected.txt"
            $command = 'call "' + (Join-Path $work "$name.cmd") + '" -ProbePath "' + $marker + '" -ExitCode ' + $expected + ' <NUL'
            $output = @(& $env:ComSpec /d /c $command 2>&1)
            $actual = $LASTEXITCODE
            if ($expected -eq 0 -and $actual -ne 0) { throw "$name.cmd failed to run its unsigned downloaded script." }
            if ($expected -ne 0 -and $actual -eq 0) { throw "$name.cmd suppressed a failing script exit status." }
            if (!(Test-Path -LiteralPath $marker) -or [IO.File]::ReadAllText($marker) -cne 'launch-ok') {
                throw "$name.cmd failed to forward a path containing spaces."
            }
            if ($env:PSExecutionPolicyPreference -ne 'AllSigned') { throw 'Parent process policy changed.' }
        }
    }
    Write-Host 'PASS: 4 unsigned downloaded launcher cases under process AllSigned; arguments and failure status preserved; parent policy unchanged.'
} finally {
    $env:PSExecutionPolicyPreference = $previousPolicy
    if (Test-Path -LiteralPath $work) { Remove-Item -LiteralPath $work -Recurse -Force }
}
exit 0
