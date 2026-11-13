<#
.SYNOPSIS
    Cleans temporary Sweeper build, test, and mutation artifacts strictly inside the project directory.

.DESCRIPTION
    Operates strictly inside `$RepoRoot` (`<repo-root>`) and never touches
    any directory outside the project folder (never touches `C:`, `.t3`, or user/system folders).
    Removes unneeded transient artifacts automatically (skipping active build caches while
    `cargo`/`rustc` is running):
      - `fuzz/target` (redundant when `CARGO_TARGET_DIR` points to `$RepoRoot\target`)
      - `mutants.out`, `mutants.out.old` (`cargo-mutants` output)
      - `frontend/test-results`, `frontend/playwright-report`, `frontend/.vite`, `frontend/coverage`
      - `target/tmp` and root `*.profraw` / `*.profdata`
      - `$RepoRoot\target` cache when exceeding `-MaxTargetSizeGb` (or when `-PruneTargetCache` is passed)
#>
param(
    [switch]$PruneTargetCache,
    [int]$MaxTargetSizeGb = 8
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent $PSScriptRoot
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

try {
    $activeRustBuild = @(Get-Process -Name cargo, rustc -ErrorAction SilentlyContinue).Count -gt 0

    # 1. Clean transient frontend & mutation output directories inside the project folder
    $alwaysTransientDirs = @(
        (Join-Path $NormRepoRoot "frontend\test-results"),
        (Join-Path $NormRepoRoot "frontend\playwright-report"),
        (Join-Path $NormRepoRoot "frontend\.vite"),
        (Join-Path $NormRepoRoot "frontend\coverage"),
        (Join-Path $NormRepoRoot "mutants.out.old")
    )
    foreach ($dir in $alwaysTransientDirs) {
        Remove-ProjectSubtreeSafe -PathToRemove $dir -Label "transient dir"
    }

    # 2. Clean transient Rust build/fuzz/mutation directories inside the project folder when no compiler is running
    if (-not $activeRustBuild) {
        $rustTransientDirs = @(
            (Join-Path $NormRepoRoot "fuzz\target"),
            (Join-Path $NormRepoRoot "target\tmp"),
            (Join-Path $NormRepoRoot "mutants.out")
        )
        foreach ($dir in $rustTransientDirs) {
            Remove-ProjectSubtreeSafe -PathToRemove $dir -Label "transient Rust dir"
        }
    }

    # 3. Clean profiling files strictly at repo root
    Get-ChildItem -LiteralPath $NormRepoRoot -Include *.profraw,*.profdata -File -ErrorAction SilentlyContinue | ForEach-Object {
        $full = [System.IO.Path]::GetFullPath($_.FullName)
        if ($full.StartsWith($NormRepoRoot + '\', [System.StringComparison]::OrdinalIgnoreCase)) {
            Write-Host "Removing profiling artifact: $full"
            Remove-Item -LiteralPath $full -Force -ErrorAction SilentlyContinue
        }
    }

    # 4. Keep $RepoRoot\target bounded
    $repoTarget = Join-Path $NormRepoRoot "target"
    if (Test-Path -LiteralPath $repoTarget) {
        $targetBytes = (Get-ChildItem -LiteralPath $repoTarget -Recurse -File -ErrorAction SilentlyContinue | Measure-Object -Property Length -Sum).Sum
        $targetGb = [math]::Round(($targetBytes / 1GB), 2)
        Write-Host "Project target/ size: $targetGb GB ($repoTarget)"
        if ((-not $activeRustBuild) -and ($PruneTargetCache -or $targetGb -gt $MaxTargetSizeGb)) {
            Write-Host "Pruning project target/ cache (threshold: $MaxTargetSizeGb GB)..."
            cargo clean 2>&1 | Out-Null
        } elseif ($activeRustBuild -and ($PruneTargetCache -or $targetGb -gt $MaxTargetSizeGb)) {
            Write-Host "Skipping project target/ prune because cargo/rustc is actively running."
        }
    }
}
finally {
    Pop-Location
}
