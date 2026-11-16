<#
.SYNOPSIS
    Cleans temporary Sweeper build, test, and mutation artifacts strictly inside the repository root.

.DESCRIPTION
    Operates strictly inside the repository root resolved from this script's location and never touches
    any directory outside the project folder (never touches `C:` drive paths, `.t3`, or user/system folders).
    Removes unneeded transient artifacts automatically (skipping active build caches while
    `cargo`/`rustc` is running):
      - `app/fuzz/target` (redundant when `CARGO_TARGET_DIR` points to the repo-root `target/`)
      - `app/mutants.out*`, `app/target` (stray crate-local outputs)
      - `app/frontend/test-results`, `app/frontend/playwright-report`, `app/frontend/.vite`, `app/frontend/coverage`
      - `target/tmp` and root `*.profraw` / `*.profdata`
      - repo-root `target/` cache when exceeding `-MaxTargetSizeGb` (or when `-PruneTargetCache` is passed)
#>
param(
    [switch]$PruneTargetCache,
    [int]$MaxTargetSizeGb = 8
)

$ErrorActionPreference = "Stop"
$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$NormRepoRoot = ([System.IO.Path]::GetFullPath($RepoRoot)).TrimEnd('\', '/')
Push-Location $RepoRoot

function Remove-ProjectSubtreeSafe {
    param([string]$PathToRemove, [string]$Label)
    $full = ([System.IO.Path]::GetFullPath($PathToRemove)).TrimEnd('\', '/')
    if (-not $full.StartsWith($NormRepoRoot + '\', [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to touch path outside project directory: $full"
    }
    if (Test-Path -LiteralPath $full) {
        Write-Host "Cleaning ${Label}: $full"
        Remove-Item -LiteralPath $full -Recurse -Force -ErrorAction SilentlyContinue
    }
}

function Test-SweeperTempOwnedMarker {
    param(
        [System.IO.DirectoryInfo]$Dir,
        [string]$Kind
    )
    switch ($Kind) {
        "sandbox" {
            return (Test-Path -LiteralPath (Join-Path $Dir.FullName ".sweeper-sandbox-sentinel") -PathType Leaf)
        }
        "canary" {
            return (
                (Test-Path -LiteralPath (Join-Path $Dir.FullName "root.txt") -PathType Leaf) -and
                (Test-Path -LiteralPath (Join-Path $Dir.FullName "nested\file.txt") -PathType Leaf) -and
                (Test-Path -LiteralPath (Join-Path $Dir.FullName "nested\data.bin") -PathType Leaf)
            )
        }
        default { return $false }
    }
}

function Test-SweeperOwnerLockLive {
    param([System.IO.DirectoryInfo]$Dir)

    $lockPath = Join-Path $Dir.FullName ".sweeper-owner.lock"
    if (-not (Test-Path -LiteralPath $lockPath -PathType Leaf)) { return $false }

    try {
        $fs = [System.IO.File]::Open(
            $lockPath,
            [System.IO.FileMode]::Open,
            [System.IO.FileAccess]::ReadWrite,
            [System.IO.FileShare]::None
        )
        $fs.Dispose()
        return $false
    }
    catch [System.IO.IOException], [System.UnauthorizedAccessException] {
        # Exclusive lock held by a live Sandbox/CanaryTree owner.
        return $true
    }
    catch {
        # Be conservative on unexpected probe failures.
        return $true
    }
}

function Get-SweeperStaleTempCandidates {
    param(
        [string]$TempRoot,
        [hashtable]$Spec,
        [datetime]$Cutoff
    )

    $items = Get-ChildItem -LiteralPath $TempRoot -Directory -ErrorAction SilentlyContinue
    if (-not $items) { return @() }

    $candidates = @()
    foreach ($dir in $items) {
        if (Test-SweeperTempCandidate -Dir $dir -Spec $Spec -Cutoff $Cutoff) {
            $candidates += $dir
        }
    }
    return $candidates
}

function Test-SweeperTempCandidate {
    param(
        [System.IO.DirectoryInfo]$Dir,
        [hashtable]$Spec,
        [datetime]$Cutoff
    )

    if (-not $Dir.Name.StartsWith($Spec.Prefix, [System.StringComparison]::Ordinal)) { return $false }
    if (($Dir.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) { return $false }
    if ($Dir.LastWriteTime -ge $Cutoff) { return $false }
    return $true
}

function Test-SweeperOwnerLockFree {
    param([System.IO.DirectoryInfo]$Dir)
    return -not (Test-SweeperOwnerLockLive -Dir $Dir)
}

function Test-SweeperTempDirSafeToRemove {
    param(
        [System.IO.DirectoryInfo]$Dir,
        [hashtable]$Spec,
        [string]$TempRoot,
        [datetime]$Cutoff
    )

    $full = ([System.IO.Path]::GetFullPath($Dir.FullName)).TrimEnd('\', '/')
    $parent = ([System.IO.Path]::GetFullPath($Dir.Parent.FullName)).TrimEnd('\', '/')
    if ($parent -ne $TempRoot) { return $false }
    if (-not $full.StartsWith($TempRoot + '\', [System.StringComparison]::OrdinalIgnoreCase)) { return $false }
    if (-not (Test-SweeperTempCandidate -Dir $Dir -Spec $Spec -Cutoff $Cutoff)) { return $false }
    if (-not (Test-SweeperTempOwnedMarker -Dir $Dir -Kind $Spec.Kind)) { return $false }
    if (-not (Test-SweeperOwnerLockFree -Dir $Dir)) { return $false }
    return $true
}

function Remove-SingleSweeperTempDir {
    param(
        [System.IO.DirectoryInfo]$Dir,
        [hashtable]$Spec
    )

    $full = ([System.IO.Path]::GetFullPath($Dir.FullName)).TrimEnd('\', '/')
    Write-Host "Cleaning stale Sweeper temp $($Spec.Kind): $full"
    Remove-Item -LiteralPath $full -Recurse -Force -ErrorAction SilentlyContinue
}

function Remove-StaleSweeperTempDirs {
    param([int]$MinAgeMinutes = 60)

    $tempRoot = ([System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())).TrimEnd('\', '/')
    if (-not (Test-Path -LiteralPath $tempRoot -PathType Container)) { return }
    $cutoff = (Get-Date).AddMinutes(-$MinAgeMinutes)

    $prefixes = @(
        @{ Prefix = "sweeper-sandbox-"; Kind = "sandbox" },
        @{ Prefix = "sweeper-canary-"; Kind = "canary" }
    )

    foreach ($spec in $prefixes) {
        $candidates = Get-SweeperStaleTempCandidates -TempRoot $tempRoot -Spec $spec -Cutoff $cutoff
        foreach ($dir in $candidates) {
            if (-not (Test-SweeperTempDirSafeToRemove -Dir $dir -Spec $spec -TempRoot $tempRoot -Cutoff $cutoff)) {
                continue
            }
            Remove-SingleSweeperTempDir -Dir $dir -Spec $spec
        }
    }
}


function Get-RepoTargetSizeGb {
    param([string]$RepoTarget)
    $targetBytes = (Get-ChildItem -LiteralPath $RepoTarget -Recurse -File -ErrorAction SilentlyContinue | Measure-Object -Property Length -Sum).Sum
    if (-not $targetBytes) { $targetBytes = 0 }
    return [math]::Round(($targetBytes / 1GB), 2)
}

function Ensure-CargoTargetCachedirTag {
    param([string]$RepoTarget)
    $cachedirTag = Join-Path $RepoTarget "CACHEDIR.TAG"
    if (Test-Path -LiteralPath $cachedirTag -PathType Leaf) { return $false }
    Set-Content -LiteralPath $cachedirTag -Value "Signature: 8a477f597d28d172789f06886806bc55" -NoNewline -Encoding ASCII
    return $true
}

function Invoke-CargoCleanForTarget {
    param([string]$RepoTarget)
    return & cargo clean --manifest-path (Join-Path $NormRepoRoot "app\Cargo.toml") --target-dir $RepoTarget 2>&1
}

function Test-CargoCleanSkipForSandboxOnlyTarget {
    param([string]$Message, [string]$RepoTarget)
    if ($Message -notmatch '(missing or invalid|invalid signature in) `?CACHEDIR\.TAG') { return $false }
    return (Test-Path -LiteralPath (Join-Path $RepoTarget "sweeper-test-sandboxes"))
}

function Invoke-SafeCargoTargetClean {
    param(
        [string]$RepoTarget,
        [bool]$ActiveRustBuild
    )

    $targetGb = Get-RepoTargetSizeGb -RepoTarget $RepoTarget
    Write-Host "Project target/ size: $targetGb GB ($RepoTarget)"
    $shouldPrune = ($PruneTargetCache -or $targetGb -gt $MaxTargetSizeGb)
    if (-not $shouldPrune) { return }
    if ($ActiveRustBuild) {
        Write-Host "Skipping project target/ prune because cargo/rustc is actively running."
        return
    }
    Write-Host "Pruning project target/ cache (threshold: $MaxTargetSizeGb GB)..."
    $cargoCleanPrevEap = $ErrorActionPreference
    $createdCachedirTag = $false
    try {
        $ErrorActionPreference = 'Continue'
        $createdCachedirTag = Ensure-CargoTargetCachedirTag -RepoTarget $RepoTarget
        $cargoCleanOutput = Invoke-CargoCleanForTarget -RepoTarget $RepoTarget
        if ($LASTEXITCODE -eq 0) { return }
        $msg = ($cargoCleanOutput | Out-String).Trim()
        if (Test-CargoCleanSkipForSandboxOnlyTarget -Message $msg -RepoTarget $RepoTarget) {
            Write-Host "Skipping cargo clean: target/ is missing/invalid CACHEDIR.TAG but contains sweeper-test-sandboxes; preserving target directory."
            return
        }
        throw "cargo clean failed with exit code $LASTEXITCODE`n$msg"
    } finally {
        $ErrorActionPreference = $cargoCleanPrevEap
        if ($createdCachedirTag) { $null = $createdCachedirTag }
    }
}

try {
    $activeRustBuild = @(Get-Process -Name cargo, rustc -ErrorAction SilentlyContinue).Count -gt 0

    # 1. Clean transient frontend & mutation output directories inside app/
    $alwaysTransientDirs = @(
        (Join-Path $NormRepoRoot "app\frontend\test-results"),
        (Join-Path $NormRepoRoot "app\frontend\playwright-report"),
        (Join-Path $NormRepoRoot "app\frontend\.vite"),
        (Join-Path $NormRepoRoot "app\frontend\coverage"),
        (Join-Path $NormRepoRoot "app\mutants.out.old")
    )
    foreach ($dir in $alwaysTransientDirs) {
        Remove-ProjectSubtreeSafe -PathToRemove $dir -Label "transient dir"
    }

    # 2. Clean transient Rust build/fuzz/mutation directories inside app/ when no compiler is running
    if (-not $activeRustBuild) {
        $rustTransientDirs = @(
            (Join-Path $NormRepoRoot "app\fuzz\target"),
            (Join-Path $NormRepoRoot "app\target"),
            (Join-Path $NormRepoRoot "target\tmp"),
            (Join-Path $NormRepoRoot "app\mutants.out")
        )
        foreach ($dir in $rustTransientDirs) {
            Remove-ProjectSubtreeSafe -PathToRemove $dir -Label "transient Rust dir"
        }

        # Additional mutants.out* variants under app/
        Get-ChildItem -LiteralPath (Join-Path $NormRepoRoot "app") -Directory -Filter "mutants.out*" -ErrorAction SilentlyContinue | ForEach-Object {
            Remove-ProjectSubtreeSafe -PathToRemove $_.FullName -Label "mutants output"
        }
    }

    # 3. Clean profiling files strictly at repo root
    Get-ChildItem -LiteralPath $NormRepoRoot -File -ErrorAction SilentlyContinue | Where-Object {
        $_.Extension -eq ".profraw" -or $_.Extension -eq ".profdata"
    } | ForEach-Object {
        $full = [System.IO.Path]::GetFullPath($_.FullName)
        if ($full.StartsWith($NormRepoRoot + '\', [System.StringComparison]::OrdinalIgnoreCase)) {
            Write-Host "Removing profiling artifact: $full"
            Remove-Item -LiteralPath $full -Force -ErrorAction SilentlyContinue
        }
    }

    # 4. Keep repo-root target/ bounded
    $repoTarget = Join-Path $NormRepoRoot "target"
    if (Test-Path -LiteralPath $repoTarget) {
        Invoke-SafeCargoTargetClean -RepoTarget $repoTarget -ActiveRustBuild $activeRustBuild
    }

    # 5. Prune stale Sweeper-owned sandbox/canary directories in OS temp only (age-gated, strict guards).
    #    Never touches non-Sweeper entries in %TEMP% or any .t3 paths.
    Remove-StaleSweeperTempDirs -MinAgeMinutes 60
}
finally {
    Pop-Location
}

