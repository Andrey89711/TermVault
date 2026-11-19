# Trusted helper embedded in the currently running binary. Never dot-source
# release scripts, execute downloaded programs, elevate, relaunch, or edit PATH.
[CmdletBinding()]
param([string]$PlanPath)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Get-UpdatePayload {
    @(
        'tvault.exe', 'LICENSE', 'README.md', 'SECURITY.md', 'PRIVACY.md', 'CHANGELOG.md',
        'docs/RELEASING.md', 'docs/STORE-PUBLISHING.ru.md', 'docs/CSV.md',
        'setup.ps1', 'setup.cmd', 'uninstall.ps1', 'uninstall.cmd',
        'scripts/setup-common.ps1', 'scripts/uninstall-common.ps1',
        'browser/README.md', 'browser/QUICKSTART.ru.md', 'browser/install.ps1', 'browser/uninstall.ps1',
        'browser/extension/manifest.json', 'browser/extension/background.js',
        'browser/extension/content.js', 'browser/extension/package.json',
        'browser/extension/options.html', 'browser/extension/options.js',
        'browser/extension/icons/icon-16.png', 'browser/extension/icons/icon-32.png',
        'browser/extension/icons/icon-48.png', 'browser/extension/icons/icon-128.png'
    )
}

function Get-Legacy012OwnedPayload {
    # Historical ownership must NOT be inferred from Get-UpdatePayload.
    @(
        'tvault.exe', 'LICENSE', 'README.md', 'SECURITY.md', 'PRIVACY.md', 'CHANGELOG.md',
        'docs/RELEASING.md', 'docs/STORE-PUBLISHING.ru.md',
        'setup.ps1', 'setup.cmd', 'uninstall.ps1', 'uninstall.cmd',
        'scripts/setup-common.ps1', 'scripts/uninstall-common.ps1',
        'browser/README.md', 'browser/QUICKSTART.ru.md', 'browser/install.ps1', 'browser/uninstall.ps1',
        'browser/extension/manifest.json', 'browser/extension/background.js',
        'browser/extension/content.js', 'browser/extension/package.json',
        'browser/extension/options.html', 'browser/extension/options.js',
        'browser/extension/icons/icon-16.png', 'browser/extension/icons/icon-32.png',
        'browser/extension/icons/icon-48.png', 'browser/extension/icons/icon-128.png'
    )
}

function Assert-UpdateAbsent {
    param([string]$Path)
    # This layout introduces no directories: docs/ already belongs to v0.1.2.
    # Require every parent to preexist; never create or recursively clean them.
    $parent = Split-Path $Path -Parent
    Assert-UpdateRegular $parent -Directory
    $leaf = Split-Path $Path -Leaf
    if (@(Get-ChildItem -Force -LiteralPath $parent -ErrorAction Stop | Where-Object { $_.Name -ieq $leaf }).Count -ne 0) {
        throw 'New update member collides with an unowned path.'
    }
}

function Assert-UpdateRegular {
    param([string]$Path, [switch]$Directory)
    $item = Get-Item -Force -LiteralPath $Path
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -or ($Directory.IsPresent -ne $item.PSIsContainer)) {
        throw 'Non-regular update path.'
    }
    $parent = Split-Path $item.FullName -Parent
    if ($parent -and $parent -ne $item.FullName) { Assert-UpdateRegular -Path $parent -Directory }
}

function Read-UpdateJson {
    param([string]$Path)
    Assert-UpdateRegular $Path
    if ((Get-Item -Force -LiteralPath $Path).Length -gt 65536) { throw 'Oversized update metadata.' }
    Get-Content -Raw -Encoding UTF8 -LiteralPath $Path | ConvertFrom-Json
}

function Get-UpdateHash {
    param([string]$Path)
    Assert-UpdateRegular $Path
    if ((Get-Item -Force -LiteralPath $Path).Length -gt 67108864) { throw 'Oversized update file.' }
    (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Invoke-TermVaultUpdate {
    # FailAfter is used only by isolated tests; it is not read from the plan.
    param([string]$PlanFile, [int]$FailAfter = -1, [switch]$SkipWait, [scriptblock]$AfterChange)
    $plan = Read-UpdateJson $PlanFile
    $stage = Split-Path ([IO.Path]::GetFullPath($PlanFile)) -Parent
    Assert-UpdateRegular $stage -Directory
    if ((Split-Path $stage -Leaf) -notmatch '^termvault-update-[A-Za-z0-9_-]+$') { throw 'Invalid staging directory.' }
    $root = [IO.Path]::GetFullPath([string]$plan.root).TrimEnd('\', '/')
    if ($root -ine [string]$plan.root -or $root -ieq $stage -or $stage.StartsWith($root + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Invalid installation root.' }
    Assert-UpdateRegular $root -Directory
    if ($plan.schema -ne 1 -or [string]$plan.version -notmatch '^\d+\.\d+\.\d+$') { throw 'Invalid update plan.' }
    $markerPath = Join-Path $root '.termvault-install.json'
    $marker = Read-UpdateJson $markerPath
    if ($marker.product -cne 'TermVault' -or $marker.schema -ne 1 -or [version]$marker.version -ge [version]$plan.version) {
        throw 'Installation ownership or version mismatch.'
    }
    $layoutProperty = $marker.PSObject.Properties['payload_layout']
    $legacy = $null -eq $layoutProperty -and $marker.version -ceq '0.1.2'
    if (!$legacy -and ($null -eq $layoutProperty -or
        ($layoutProperty.Value -isnot [int] -and $layoutProperty.Value -isnot [long]) -or $layoutProperty.Value -ne 2)) { throw 'Unsupported installed payload layout.' }
    $owned = @(Get-Legacy012OwnedPayload) + @('.termvault-install.json')
    if (!$legacy) { $owned += 'docs/CSV.md' }
    $allowed = @((Get-UpdatePayload)) + @('.termvault-install.json')
    if (@($plan.files).Count -ne $allowed.Count) { throw 'Incomplete update plan.' }
    $seen = @{}
    $operations = @()
    foreach ($file in $plan.files) {
        $relative = [string]$file.relative
        $created = $null -eq $file.before
        if ($relative -cnotin $allowed -or $seen.ContainsKey($relative) -or
            (!$created -and [string]$file.before -notmatch '^[a-f0-9]{64}$') -or [string]$file.after -notmatch '^[a-f0-9]{64}$') { throw 'Invalid update member.' }
        if ($created) {
            if (!$legacy -or $relative -cne 'docs/CSV.md') { throw 'No explicit mapping for new update file.' }
            Assert-UpdateAbsent (Join-Path $root $relative)
        } elseif ($relative -cnotin $owned) { throw 'Refusing to replace an unowned update path.' }
        $seen[$relative] = $true
        $operations += [pscustomobject]@{
            Target = Join-Path $root $relative
            Source = Join-Path (Join-Path $stage 'payload') $relative
            Before = [string]$file.before
            After = [string]$file.after
            Created = $created
        }
    }
    $newMarker = Read-UpdateJson (Join-Path $stage 'payload/.termvault-install.json')
    $manifest = Read-UpdateJson (Join-Path $stage 'payload/browser/extension/manifest.json')
    if ($newMarker.product -cne 'TermVault' -or $newMarker.schema -ne 1 -or
        ($newMarker.payload_layout -isnot [int] -and $newMarker.payload_layout -isnot [long]) -or
        $newMarker.version -cne $plan.version -or $newMarker.payload_layout -ne 2 -or $manifest.version -cne $plan.version) { throw 'Payload version mismatch.' }

    # Only an existing bridge explicitly owned by this installation is changed.
    # No registry or browser configuration is modified.
    if ($env:LOCALAPPDATA) {
        $bridge = Join-Path $env:LOCALAPPDATA 'TermVault/browser'
        $ownerPath = Join-Path $bridge '.termvault-owner.json'
        if (Test-Path -LiteralPath $ownerPath) {
            $owner = Read-UpdateJson $ownerPath
            if ([string]$owner.install_directory -ieq $root) {
                Assert-UpdateRegular $bridge -Directory
                $hostPath = Join-Path $bridge 'tvault-browser-host.exe'
                $native = Read-UpdateJson (Join-Path $bridge 'com.termvault.browser.json')
                if ($native.name -cne 'com.termvault.browser' -or [string]$native.path -ine $hostPath) { throw 'Unrecognized owned browser bridge.' }
                $binary = @($operations | Where-Object { $_.Target -ieq (Join-Path $root 'tvault.exe') })[0]
                if ((Get-UpdateHash $hostPath) -cne $binary.Before) { throw 'Browser bridge differs from owned installed version; repair it first.' }
                $operations += [pscustomobject]@{ Target = $hostPath; Source = $binary.Source; Before = $binary.Before; After = $binary.After; Created = $false }
            }
        }
    }
    if (!$SkipWait) {
        $parentId = [int]$plan.parent
        if ($parentId -le 0 -or $parentId -eq $PID) { throw 'Invalid parent process.' }
        $parent = Get-Process -Id $parentId -ErrorAction SilentlyContinue
        if ($null -ne $parent -and !$parent.WaitForExit(120000)) { throw 'TermVault did not exit in time.' }
    }
    foreach ($operation in $operations) {
        if ($operation.Created) { Assert-UpdateAbsent $operation.Target }
        elseif ((Get-UpdateHash $operation.Target) -cne $operation.Before) { throw 'Update target changed; refusing installation.' }
        if ((Get-UpdateHash $operation.Source) -cne $operation.After) { throw 'Update source changed; refusing installation.' }
    }
    $rollback = Join-Path $stage 'rollback'
    New-Item -ItemType Directory -Path $rollback | Out-Null
    $changed = @()
    $temporaryFiles = @()
    $rollbackFailed = $false
    try {
        for ($i = 0; $i -lt $operations.Count; $i++) {
            $operation = $operations[$i]
            $backup = Join-Path $rollback "$i.bak"
            if (!$operation.Created) {
                [IO.File]::Copy($operation.Target, $backup, $false)
                if ((Get-UpdateHash $backup) -cne $operation.Before) { throw 'Cannot create update rollback copy.' }
            }
            # Same-directory replacement is atomic. Never execute the payload.
            $next = Join-Path (Split-Path $operation.Target -Parent) ('.termvault-next-' + [guid]::NewGuid().ToString('N'))
            $temporaryFiles += $next
            [IO.File]::Copy($operation.Source, $next, $false)
            if ((Get-UpdateHash $next) -cne $operation.After) { throw 'Update source changed.' }
            if ($operation.Created) {
                Assert-UpdateAbsent $operation.Target
                # Two-argument Move never overwrites an existing destination,
                # including a path created after the last absence check.
                [IO.File]::Move($next, $operation.Target)
            } else {
                Assert-UpdateRegular $operation.Target
                if ((Get-UpdateHash $operation.Target) -cne $operation.Before) { throw 'Update target changed.' }
                [IO.File]::Replace($next, $operation.Target, [NullString]::Value)
            }
            $changed += [pscustomobject]@{ Target = $operation.Target; Backup = $backup; Created = $operation.Created; After = $operation.After }
            # Isolated tests can model a concurrent edit without races. This
            # callback is never read from the plan or passed by the entrypoint.
            if ($null -ne $AfterChange) { & $AfterChange $operation.Target $changed.Count }
            if ($FailAfter -ge 0 -and $changed.Count -ge $FailAfter) { throw 'Injected isolated test failure.' }
        }
    } catch {
        for ($i = $changed.Count - 1; $i -ge 0; $i--) {
            try {
                Assert-UpdateRegular $changed[$i].Target
                if ((Get-UpdateHash $changed[$i].Target) -cne $changed[$i].After) { throw 'Rollback target changed; preserve it for recovery.' }
                if ($changed[$i].Created) {
                    # Delete only this transaction's unchanged newly created file.
                    [IO.File]::Delete($changed[$i].Target)
                } else {
                    [IO.File]::Copy($changed[$i].Backup, $changed[$i].Target, $true)
                }
            } catch { $rollbackFailed = $true }
        }
        if ($rollbackFailed) {
            # Preserve original copies for manual recovery, never delete them.
            [IO.File]::WriteAllText((Join-Path $stage 'RECOVERY.txt'), 'Update rollback incomplete. Original program files remain in rollback/. Vault/configuration files were not touched.')
        }
        throw
    } finally {
        foreach ($temporary in $temporaryFiles) {
            if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force }
        }
    }
}

if ($MyInvocation.InvocationName -ne '.') {
    $stageToClean = $null
    try {
        if (!$PlanPath) { throw 'Missing update plan.' }
        $stageToClean = Split-Path ([IO.Path]::GetFullPath($PlanPath)) -Parent
        if ($stageToClean -ine $PSScriptRoot -or (Split-Path $PlanPath -Leaf) -cne 'plan.json') {
            $stageToClean = $null
            throw 'The helper only accepts its own staging plan.'
        }
        Invoke-TermVaultUpdate -PlanFile $PlanPath
    } catch {
        # Fixed message only; no vault paths or credentials. Never run fallback
        # scripts or silently retry an installation after a failed transaction.
        exit 1
    } finally {
        if ($stageToClean -and (Split-Path $stageToClean -Leaf) -match '^termvault-update-[A-Za-z0-9_-]+$' -and
            !(Test-Path -LiteralPath (Join-Path $stageToClean 'RECOVERY.txt'))) {
            # Only this helper's private invocation directory, never install root.
            Remove-Item -LiteralPath $stageToClean -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}
