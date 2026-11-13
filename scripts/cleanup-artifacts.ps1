<#
.SYNOPSIS
    Cleans temporary Sweeper build artifacts, orphaned/merged worktrees, and merged local branches.

.DESCRIPTION
    Keeps all Rust build caches inside the project directory on D: (`$RepoRoot\target`)
    and removes unneeded artifacts automatically and safely (skipping active build caches
    while cargo/rustc processes are running):
      - Any accidental per-worktree `target/` or `fuzz/target/` on `C:` (when not actively locked)
      - Orphaned or cleanly merged git worktrees (clean working tree & ancestor of `feat/sweeper-v1`)
      - Merged local temporary `t3/*` and `ticket/*` branches not attached to active worktrees
      - Transient build/test/mutation outputs inside the project (`fuzz/target`, `mutants.out*`,
        `frontend/test-results`, `frontend/playwright-report`, `frontend/.vite`, `frontend/coverage`,
        `target/tmp`, `*.profraw`, `*.profdata`)
      - Stale `sweeper-*` temp sandbox directories in `$env:TEMP` older than 1 hour
      - Oversized `target/` cache in `$RepoRoot\target` when exceeding `-MaxTargetSizeGb` (or `-PruneTargetCache`)
#>
param(
    [switch]$PruneTargetCache,
    [int]$MaxTargetSizeGb = 8
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent $PSScriptRoot
$NormRepoRoot = ([System.IO.Path]::GetFullPath($RepoRoot)).TrimEnd('\', '/')
Push-Location $RepoRoot

function Remove-TreeSafe {
    param([string]$PathToRemove, [string]$Label)
    if (Test-Path $PathToRemove) {
        Write-Host "Cleaning ${Label}: $PathToRemove"
        cmd /c "rmdir /s /q `"\\?\$PathToRemove`"" 2>&1 | Out-Null
    }
}

try {
    $activeRustBuild = @(Get-Process -Name cargo, rustc -ErrorAction SilentlyContinue).Count -gt 0

    git worktree prune 2>&1 | Out-Null

    # 1. Inspect registered git worktrees; safely remove merged & clean temporary worktrees
    $worktrees = @()
    $current = @{}
    foreach ($line in (git worktree list --porcelain)) {
        if ([string]::IsNullOrWhiteSpace($line)) {
            if ($current.ContainsKey('path')) { $worktrees += [pscustomobject]$current }
            $current = @{}
            continue
        }
        if ($line -match '^worktree (.+)$') {
            $current['path'] = ([System.IO.Path]::GetFullPath($Matches[1])).TrimEnd('\', '/')
        } elseif ($line -match '^HEAD ([0-9a-fA-F]+)$') {
            $current['head'] = $Matches[1]
        } elseif ($line -match '^branch refs/heads/(.+)$') {
            $current['branch'] = $Matches[1]
        }
    }
    if ($current.ContainsKey('path')) { $worktrees += [pscustomobject]$current }

    foreach ($wt in $worktrees) {
        if ($wt.path.Equals($NormRepoRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
            continue
        }
        if (-not (Test-Path $wt.path)) {
            continue
        }

        if (-not $activeRustBuild) {
            Remove-TreeSafe -PathToRemove (Join-Path $wt.path "target") -Label "per-worktree target/"
            Remove-TreeSafe -PathToRemove (Join-Path $wt.path "fuzz\target") -Label "per-worktree fuzz/target/"
        }

        # Only remove the worktree itself if it has ZERO uncommitted changes AND HEAD is already an ancestor of feat/sweeper-v1
        $dirty = git -C $wt.path status --porcelain 2>$null
        if ([string]::IsNullOrWhiteSpace(($dirty -join ""))) {
            git merge-base --is-ancestor $wt.head feat/sweeper-v1 2>$null
            if ($LASTEXITCODE -eq 0) {
                Remove-TreeSafe -PathToRemove $wt.path -Label "merged clean worktree"
            }
        }
    }

    git worktree prune 2>&1 | Out-Null

    # 2. Refresh active worktree set and remove any orphaned directories under C:\Users\<user>\.t3\worktrees\CCleaner
    $activePaths = New-Object 'System.Collections.Generic.HashSet[string]' ([System.StringComparer]::OrdinalIgnoreCase)
    $activeBranches = New-Object 'System.Collections.Generic.HashSet[string]' ([System.StringComparer]::OrdinalIgnoreCase)
    foreach ($line in (git worktree list --porcelain)) {
        if ($line -match '^worktree (.+)$') {
            [void]$activePaths.Add(([System.IO.Path]::GetFullPath($Matches[1])).TrimEnd('\', '/'))
        } elseif ($line -match '^branch refs/heads/(.+)$') {
            [void]$activeBranches.Add($Matches[1])
        }
    }

    $t3WorktreeRoot = Join-Path $env:USERPROFILE ".t3\worktrees\CCleaner"
    if (Test-Path $t3WorktreeRoot) {
        Get-ChildItem $t3WorktreeRoot -Directory -ErrorAction SilentlyContinue | ForEach-Object {
            $full = ([System.IO.Path]::GetFullPath($_.FullName)).TrimEnd('\', '/')
            if (-not $activePaths.Contains($full)) {
                Remove-TreeSafe -PathToRemove $full -Label "orphaned worktree directory"
            }
        }
    }

    # 3. Clean merged local temporary branches (`t3/*` and `ticket/*`) not attached to active worktrees
    $protected = @("main", "feat/sweeper-v1")
    foreach ($b in (git branch --merged feat/sweeper-v1 --format="%(refname:short)")) {
        $branch = $b.Trim()
        if ([string]::IsNullOrWhiteSpace($branch)) { continue }
        if ($protected -contains $branch) { continue }
        if ($activeBranches.Contains($branch)) { continue }
        if ($branch -like "t3/*" -or $branch -like "ticket/*") {
            Write-Host "Deleting merged temporary branch: $branch"
            git branch -d $branch 2>&1 | Out-Null
        }
    }

    # 4. Clean transient test, fuzz, mutation, and build-temp directories inside project folder
    $transientDirs = @(
        (Join-Path $RepoRoot "frontend\test-results"),
        (Join-Path $RepoRoot "frontend\playwright-report"),
        (Join-Path $RepoRoot "frontend\.vite"),
        (Join-Path $RepoRoot "frontend\coverage"),
        (Join-Path $RepoRoot "mutants.out.old")
    )
    if (-not $activeRustBuild) {
        $transientDirs += @(
            (Join-Path $RepoRoot "fuzz\target"),
            (Join-Path $RepoRoot "target\tmp"),
            (Join-Path $RepoRoot "mutants.out")
        )
    }
    foreach ($dir in $transientDirs) {
        Remove-TreeSafe -PathToRemove $dir -Label "transient project dir"
    }

    Get-ChildItem $RepoRoot -Include *.profraw,*.profdata -File -ErrorAction SilentlyContinue | ForEach-Object {
        Write-Host "Removing profiling artifact: $($_.FullName)"
        Remove-Item $_.FullName -Force -ErrorAction SilentlyContinue
    }

    # 5. Clean stale sweeper temp sandbox directories older than 1 hour
    $cutoff = (Get-Date).AddHours(-1)
    Get-ChildItem $env:TEMP -Directory -Filter "sweeper*" -ErrorAction SilentlyContinue | Where-Object {
        $_.LastWriteTime -lt $cutoff
    } | ForEach-Object {
        Remove-TreeSafe -PathToRemove $_.FullName -Label "stale temp sandbox"
    }

    # 6. Keep target/ inside repo root bounded (all builds live in $RepoRoot\target on D:)
    $repoTarget = Join-Path $RepoRoot "target"
    if (Test-Path $repoTarget) {
        $targetBytes = (Get-ChildItem $repoTarget -Recurse -File -ErrorAction SilentlyContinue | Measure-Object -Property Length -Sum).Sum
        $targetGb = [math]::Round(($targetBytes / 1GB), 2)
        Write-Host "Project target/ size: $targetGb GB ($repoTarget)"
        if ((-not $activeRustBuild) -and ($PruneTargetCache -or $targetGb -gt $MaxTargetSizeGb)) {
            Write-Host "Pruning Rust target cache (threshold: $MaxTargetSizeGb GB)..."
            cargo clean 2>&1 | Out-Null
        } elseif ($activeRustBuild -and ($PruneTargetCache -or $targetGb -gt $MaxTargetSizeGb)) {
            Write-Host "Skipping target/ prune because cargo/rustc is actively running."
        }
    }

    Get-PSDrive C, D | Format-Table Name, @{N='UsedGB';E={[math]::Round($_.Used/1GB,2)}}, @{N='FreeGB';E={[math]::Round($_.Free/1GB,2)}}
}
finally {
    Pop-Location
}
