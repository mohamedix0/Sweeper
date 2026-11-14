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
        $targetBytes = (Get-ChildItem -LiteralPath $repoTarget -Recurse -File -ErrorAction SilentlyContinue | Measure-Object -Property Length -Sum).Sum
        if (-not $targetBytes) { $targetBytes = 0 }
        $targetGb = [math]::Round(($targetBytes / 1GB), 2)
        Write-Host "Project target/ size: $targetGb GB ($repoTarget)"
        if ((-not $activeRustBuild) -and ($PruneTargetCache -or $targetGb -gt $MaxTargetSizeGb)) {
            Write-Host "Pruning project target/ cache (threshold: $MaxTargetSizeGb GB)..."
            # Use explicit target dir and avoid native stderr becoming a terminating error under $ErrorActionPreference=Stop.
            $cargoCleanPrevEap = $ErrorActionPreference
            try {
                $ErrorActionPreference = 'Continue'
                & cargo clean --manifest-path (Join-Path $NormRepoRoot "app\Cargo.toml") --target-dir $repoTarget 2>&1 | Out-Null
                if ($LASTEXITCODE -ne 0) {
                    throw "cargo clean failed with exit code $LASTEXITCODE"
                }
            } finally {
                $ErrorActionPreference = $cargoCleanPrevEap
            }
        } elseif ($activeRustBuild -and ($PruneTargetCache -or $targetGb -gt $MaxTargetSizeGb)) {
            Write-Host "Skipping project target/ prune because cargo/rustc is actively running."
        }
    }
}
finally {
    Pop-Location
}
