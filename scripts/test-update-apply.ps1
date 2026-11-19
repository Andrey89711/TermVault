# All paths and LOCALAPPDATA are isolated. No user settings, registry, or PATH writes.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'update-apply.ps1')
$sandbox = Join-Path ([IO.Path]::GetTempPath()) ('termvault-apply-test-' + [guid]::NewGuid().ToString('N'))
$previousLocal = $env:LOCALAPPDATA
$env:LOCALAPPDATA = Join-Path $sandbox 'local'
New-Item -ItemType Directory -Path $sandbox | Out-Null

function New-ApplyFixture {
    $root = Join-Path $sandbox ([guid]::NewGuid().ToString('N'))
    $stage = Join-Path $sandbox ('termvault-update-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $root, $stage | Out-Null
    $files = @()
    foreach ($relative in (@(Get-UpdatePayload) + @('.termvault-install.json'))) {
        $old = Join-Path $root $relative
        $new = Join-Path $stage "payload/$relative"
        New-Item -ItemType Directory -Path (Split-Path $old -Parent), (Split-Path $new -Parent) -Force | Out-Null
        # Exact historical layout: v0.1.2 never shipped docs/CSV.md.
        $before = $null
        if ($relative -cne 'docs/CSV.md') { [IO.File]::WriteAllText($old, "old:$relative") }
        [IO.File]::WriteAllText($new, "new:$relative")
        if ($relative -eq '.termvault-install.json') {
            [IO.File]::WriteAllText($old, '{"product":"TermVault","schema":1,"version":"0.1.2","future":{"keep":true}}')
            [IO.File]::WriteAllText($new, '{"product":"TermVault","schema":1,"version":"99.0.0","payload_layout":2,"future":{"keep":true}}')
        }
        if ($relative -eq 'browser/extension/manifest.json') { [IO.File]::WriteAllText($new, '{"version":"99.0.0"}') }
        if ($relative -cne 'docs/CSV.md') { $before = Get-UpdateHash $old }
        $files += @{ relative = $relative; before = $before; after = Get-UpdateHash $new }
    }
    [IO.File]::WriteAllText((Join-Path $root 'user-vault.kdbx'), 'KEEP VAULT')
    [IO.File]::WriteAllText((Join-Path $root 'settings.json'), 'KEEP SETTINGS')
    [IO.File]::WriteAllText((Join-Path $root 'unrelated.txt'), 'KEEP OTHER')
    $plan = @{ schema = 1; root = $root; version = '99.0.0'; parent = 1; files = $files }
    $planPath = Join-Path $stage 'plan.json'
    $plan | ConvertTo-Json -Depth 10 | Set-Content -Encoding UTF8 -LiteralPath $planPath
    @{ Root = $root; Stage = $stage; Plan = $planPath; Data = $plan }
}
function Assert-Preserved {
    param($Fixture)
    foreach ($pair in @(@('user-vault.kdbx', 'KEEP VAULT'), @('settings.json', 'KEEP SETTINGS'), @('unrelated.txt', 'KEEP OTHER'))) {
        if ([IO.File]::ReadAllText((Join-Path $Fixture.Root $pair[0])) -cne $pair[1]) { throw 'User file changed.' }
    }
}
function Assert-Throws {
    param([scriptblock]$Action)
    $threw = $false
    try { & $Action } catch { $threw = $true }
    if (!$threw) { throw 'Expected update refusal.' }
}
try {
    $success = New-ApplyFixture
    Invoke-TermVaultUpdate -PlanFile $success.Plan -SkipWait
    if ((Read-UpdateJson (Join-Path $success.Root '.termvault-install.json')).version -ne '99.0.0') { throw 'Update did not complete.' }
    foreach ($file in $success.Data.files) {
        if ((Get-UpdateHash (Join-Path $success.Root $file.relative)) -cne $file.after) { throw 'Incomplete payload update.' }
    }
    Assert-Preserved $success
    if (!(Test-Path -LiteralPath (Join-Path $success.Root 'docs/CSV.md') -PathType Leaf)) { throw 'Legacy payload addition missing.' }
    if ((Read-UpdateJson (Join-Path $success.Root '.termvault-install.json')).payload_layout -ne 2) { throw 'Payload layout marker not upgraded.' }

    foreach ($collisionKind in @('same-bytes', 'unknown-bytes', 'directory')) {
        $collision = New-ApplyFixture
        $collisionPath = Join-Path $collision.Root 'docs/CSV.md'
        if ($collisionKind -eq 'directory') { New-Item -ItemType Directory -Path $collisionPath | Out-Null }
        elseif ($collisionKind -eq 'same-bytes') { Copy-Item -LiteralPath (Join-Path $collision.Stage 'payload/docs/CSV.md') -Destination $collisionPath }
        else { [IO.File]::WriteAllText($collisionPath, 'KEEP UNKNOWN CSV GUIDE') }
        Assert-Throws { Invoke-TermVaultUpdate -PlanFile $collision.Plan -SkipWait }
        if (!(Test-Path -LiteralPath $collisionPath)) { throw 'Unowned collision was deleted.' }
        if ([IO.File]::ReadAllText((Join-Path $collision.Root 'tvault.exe')) -cne 'old:tvault.exe') { throw 'Collision changed owned files.' }
        if ($collisionKind -eq 'unknown-bytes' -and [IO.File]::ReadAllText($collisionPath) -cne 'KEEP UNKNOWN CSV GUIDE') { throw 'Unowned collision changed.' }
        Assert-Preserved $collision
    }

    $rollback = New-ApplyFixture
    $additionIndex = 0
    for ($i = 0; $i -lt $rollback.Data.files.Count; $i++) {
        if ($rollback.Data.files[$i].relative -ceq 'docs/CSV.md') { $additionIndex = $i + 1 }
    }
    if ($additionIndex -le 0) { throw 'Missing addition fixture.' }
    Assert-Throws { Invoke-TermVaultUpdate -PlanFile $rollback.Plan -SkipWait -FailAfter ($additionIndex + 1) }
    foreach ($file in $rollback.Data.files) {
        if ($null -eq $file.before) {
            if (Test-Path -LiteralPath (Join-Path $rollback.Root $file.relative)) { throw 'Rollback did not remove the new addition.' }
        } elseif ((Get-UpdateHash (Join-Path $rollback.Root $file.relative)) -cne $file.before) { throw 'Rollback did not restore original payload.' }
    }
    Assert-Preserved $rollback
    if (@(Get-ChildItem -LiteralPath $rollback.Root -Recurse -Force | Where-Object Name -Like '.termvault-next-*').Count) { throw 'Temporary payload leaked.' }
    $editedAddition = New-ApplyFixture
    Assert-Throws {
        Invoke-TermVaultUpdate -PlanFile $editedAddition.Plan -SkipWait -FailAfter $additionIndex -AfterChange {
            param($target, $count)
            if ($target.EndsWith('CSV.md', [StringComparison]::Ordinal)) { [IO.File]::WriteAllText($target, 'KEEP CONCURRENT EDIT') }
        }
    }
    if ([IO.File]::ReadAllText((Join-Path $editedAddition.Root 'docs/CSV.md')) -cne 'KEEP CONCURRENT EDIT') { throw 'Rollback deleted a changed new file.' }
    if (!(Test-Path -LiteralPath (Join-Path $editedAddition.Stage 'RECOVERY.txt'))) { throw 'Incomplete rollback recovery notice missing.' }
    if (!(Test-Path -LiteralPath (Join-Path $editedAddition.Stage 'rollback/0.bak'))) { throw 'Incomplete rollback lost original backups.' }
    Assert-Preserved $editedAddition

    $changed = New-ApplyFixture
    [IO.File]::WriteAllText((Join-Path $changed.Stage 'payload/tvault.exe'), 'TAMPERED')
    Assert-Throws { Invoke-TermVaultUpdate -PlanFile $changed.Plan -SkipWait }
    if ([IO.File]::ReadAllText((Join-Path $changed.Root 'tvault.exe')) -cne 'old:tvault.exe') { throw 'Tamper modified installation.' }
    Assert-Preserved $changed

    $hostFixture = New-ApplyFixture
    $bridge = Join-Path $env:LOCALAPPDATA 'TermVault/browser'
    New-Item -ItemType Directory -Path $bridge -Force | Out-Null
    @{ install_directory = $hostFixture.Root } | ConvertTo-Json | Set-Content -Encoding UTF8 -LiteralPath (Join-Path $bridge '.termvault-owner.json')
    $hostPath = Join-Path $bridge 'tvault-browser-host.exe'
    [IO.File]::Copy((Join-Path $hostFixture.Root 'tvault.exe'), $hostPath)
    @{ name = 'com.termvault.browser'; path = $hostPath } | ConvertTo-Json | Set-Content -Encoding UTF8 -LiteralPath (Join-Path $bridge 'com.termvault.browser.json')
    [IO.File]::WriteAllText((Join-Path $bridge 'browser-config.json'), 'KEEP BRIDGE CONFIG')
    Invoke-TermVaultUpdate -PlanFile $hostFixture.Plan -SkipWait
    if ((Get-UpdateHash $hostPath) -cne (Get-UpdateHash (Join-Path $hostFixture.Root 'tvault.exe'))) { throw 'Owned browser host not updated.' }
    if ([IO.File]::ReadAllText((Join-Path $bridge 'browser-config.json')) -cne 'KEEP BRIDGE CONFIG') { throw 'Bridge config changed.' }
    Assert-Preserved $hostFixture

    $unknown = New-ApplyFixture
    $unknown.Data.files[0].relative = '../unrelated.txt'
    $unknown.Data | ConvertTo-Json -Depth 10 | Set-Content -Encoding UTF8 -LiteralPath $unknown.Plan
    Assert-Throws { Invoke-TermVaultUpdate -PlanFile $unknown.Plan -SkipWait }
    Assert-Preserved $unknown
    $unknownAddition = New-ApplyFixture
    $unknownAddition.Data.files[0].before = $null
    $unknownAddition.Data | ConvertTo-Json -Depth 10 | Set-Content -Encoding UTF8 -LiteralPath $unknownAddition.Plan
    Assert-Throws { Invoke-TermVaultUpdate -PlanFile $unknownAddition.Plan -SkipWait }
    Assert-Preserved $unknownAddition
    # Exercise the actual embedded-helper entrypoint, PID wait, and cleanup.
    $waitFixture = New-ApplyFixture
    $powershell = (Get-Process -Id $PID).Path
    $waitingParent = Start-Process -FilePath $powershell -ArgumentList @('-NoProfile', '-NonInteractive', '-Command', 'Start-Sleep -Seconds 3') -PassThru -WindowStyle Hidden
    $waitFixture.Data.parent = $waitingParent.Id
    $waitFixture.Data | ConvertTo-Json -Depth 10 | Set-Content -Encoding UTF8 -LiteralPath $waitFixture.Plan
    $helper = Join-Path $waitFixture.Stage 'apply.ps1'
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'update-apply.ps1') -Destination $helper
    $watch = [Diagnostics.Stopwatch]::StartNew()
    & $powershell -NoProfile -NonInteractive -File $helper -PlanPath $waitFixture.Plan
    if ($LASTEXITCODE -ne 0) { throw 'Actual helper entrypoint failed.' }
    if (!$waitingParent.HasExited -and !(Get-Process -Id $waitingParent.Id -ErrorAction SilentlyContinue)) { $waitingParent.Refresh() }
    if ($watch.Elapsed.TotalSeconds -lt 1.5) { throw 'Helper did not wait for parent exit.' }
    if (Test-Path -LiteralPath $waitFixture.Stage) { throw 'Actual helper did not clean its staging directory.' }
    if ((Read-UpdateJson (Join-Path $waitFixture.Root '.termvault-install.json')).version -ne '99.0.0') { throw 'Actual helper did not apply.' }
    Assert-Preserved $waitFixture
    Write-Host 'PASS: legacy layout additions, unowned collisions, post-addition rollback, full payload, tamper refusal, owned browser host, PID wait/cleanup, preserved vault/config.'
} finally {
    $env:LOCALAPPDATA = $previousLocal
    # Only this invocation's generated isolated test tree.
    Remove-Item -LiteralPath $sandbox -Recurse -Force
}
