#!/usr/bin/env pwsh
# Static Gate A grep checks (CI-enforced).
# Fails if std::fs::remove_* appears in production code under backend/ or bridge/,
# if sweeper-winfs deletion symbols appear outside core::remover / sweeper-winfs,
# or if raw Win32 DeleteFileW/RemoveDirectoryW appear outside sweeper-winfs/src (and bounded inside it).

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$failed = $false

function Fail([string]$msg) {
  Write-Host "GATE-A FAIL: $msg" -ForegroundColor Red
  $script:failed = $true
}

$prodPaths = @(
  (Join-Path $repoRoot 'backend'),
  (Join-Path $repoRoot 'bridge')
)

# 1) Ban std::fs::remove_* in production source under backend/ and bridge/.
# Gate A: enforce zero matches in src/ (excluding sweeper-testkit and integration tests/ dirs).
$removePattern = 'std::fs::remove_(file|dir_all|dir)\b'
$removeHitsProd = Get-ChildItem -Path $prodPaths -Recurse -Include *.rs -File |
  Where-Object {
    $_.FullName -match '([/\\])src([/\\])' -and
    $_.FullName -notmatch '\\target\\' -and
    $_.FullName -notmatch '\\backend\\sweeper-testkit\\' -and
    $_.FullName -notmatch '([/\\])tests([/\\])'
  } |
  Select-String -Pattern $removePattern -AllMatches
if ($removeHitsProd) {
  foreach ($h in $removeHitsProd) { Write-Host "$($h.Path):$($h.LineNumber): $($h.Line.Trim())" }
  Fail "std::fs::remove_* found in production source (backend/ or bridge/ src/, excluding sweeper-testkit and tests/)."
}

# 2) Ban sweeper-winfs deletion symbols outside core::remover / sweeper-winfs (plus audited journal callsite for log retirement).
$deletionSymbols = '(delete_by_handle|remove_by_handle|set_file_disposition_delete|open_for_delete|clear_readonly_attribute|FileDispositionInfoEx|SetFileInformationByHandle\s*\(.*FileDisposition|sweeper_winfs::delete_file|sweeper_winfs::delete_dir|retire_sweeper_log_file)'
$deletionHits = Get-ChildItem -Path $prodPaths -Recurse -Include *.rs -File |
  Where-Object {
    $_.FullName -notmatch '\\target\\' -and
    $_.FullName -notmatch '\\backend\\sweeper-winfs\\' -and
    $_.FullName -notmatch '\\backend\\sweeper-core\\src\\remover([/\\.]|$)' -and
    $_.FullName -notmatch '\\backend\\sweeper-core\\src\\remover\.rs$' -and
    $_.FullName -notmatch '\\backend\\sweeper-core\\src\\journal\.rs$'
  } |
  Select-String -Pattern $deletionSymbols -AllMatches
if ($deletionHits) {
  foreach ($h in $deletionHits) { Write-Host "$($h.Path):$($h.LineNumber): $($h.Line.Trim())" }
  Fail "sweeper-winfs deletion/log-retirement symbol found outside allowed modules (winfs, core::remover, core::journal)."
}

# 2b) retire_sweeper_log_file is only allowed in the audited core::journal callsite.
$journalRetireHits = Select-String -Path (Join-Path $repoRoot 'backend/sweeper-core/src/journal.rs') -Pattern 'retire_sweeper_log_file' -AllMatches
if (-not $journalRetireHits -or $journalRetireHits.Count -ne 1) {
  Fail "Expected exactly 1 audited retire_sweeper_log_file call in backend/sweeper-core/src/journal.rs."
}
$journalAllowHits = Select-String -Path (Join-Path $repoRoot 'backend/sweeper-core/src/journal.rs') -Pattern '#\s*\[allow\(clippy::disallowed_methods\)\]' -AllMatches
if (-not $journalAllowHits -or $journalAllowHits.Count -lt 1) {
  Fail "Missing audited #[allow(clippy::disallowed_methods)] in backend/sweeper-core/src/journal.rs for log retirement call."
}

# 3) Ban raw Win32 DeleteFileW/RemoveDirectoryW anywhere in backend/ or bridge/ outside sweeper-winfs/src.
$rawDeletePattern = '\b(DeleteFileW|RemoveDirectoryW)\b'
$rawDeleteHits = Get-ChildItem -Path $prodPaths -Recurse -Include *.rs -File |
  Where-Object {
    $_.FullName -notmatch '\\target\\' -and
    $_.FullName -notmatch '\\backend\\sweeper-winfs\\src\\'
  } |
  Select-String -Pattern $rawDeletePattern -AllMatches
if ($rawDeleteHits) {
  foreach ($h in $rawDeleteHits) { Write-Host "$($h.Path):$($h.LineNumber): $($h.Line.Trim())" }
  Fail "Raw Win32 DeleteFileW/RemoveDirectoryW found outside backend/sweeper-winfs/src."
}

# 3b) Bound raw Win32 deletion calls inside sweeper-winfs/src/lib.rs.
$winfsLib = Join-Path $repoRoot 'backend/sweeper-winfs/src/lib.rs'
$deleteFileWHits = Select-String -Path $winfsLib -Pattern '\bDeleteFileW\b' -AllMatches
$removeDirWHits = Select-String -Path $winfsLib -Pattern '\bRemoveDirectoryW\b' -AllMatches
if (-not $deleteFileWHits -or $deleteFileWHits.Count -ne 3) {
  Fail "Expected exactly 3 DeleteFileW occurrences in sweeper-winfs/src/lib.rs (import, call in retire_sweeper_log_file, and context string)."
}
$deleteApiCallHits = Select-String -Path $winfsLib -Pattern 'unsafe\s*\{\s*DeleteFileW\(' -AllMatches
if (-not $deleteApiCallHits -or $deleteApiCallHits.Count -ne 1) {
  Fail "Expected exactly 1 raw DeleteFileW API call site in sweeper-winfs/src/lib.rs."
}
$retireFn = Select-String -Path $winfsLib -Pattern 'pub fn retire_sweeper_log_file' -AllMatches
if (-not $retireFn -or $retireFn.Count -ne 1) {
  Fail "Expected retire_sweeper_log_file to exist in sweeper-winfs/src/lib.rs."
}
$retireDeleteContext = Select-String -Path $winfsLib -Pattern 'DeleteFileW\(retire_sweeper_log_file\)' -AllMatches
if (-not $retireDeleteContext -or $retireDeleteContext.Count -ne 1) {
  Fail "Expected DeleteFileW usage to be scoped to retire_sweeper_log_file context."
}
$createJunctionGate = Select-String -Path $winfsLib -Pattern '#\[cfg\(any\(test, feature = "test-support"\)\)\]' -Context 0,1 -AllMatches |
  Where-Object { $_.Context.PostContext -match 'pub fn create_directory_junction' }
if (-not $createJunctionGate) {
  Fail "create_directory_junction must be gated with #[cfg(any(test, feature = `"test-support`"))]."
}
$setTimeGate = Select-String -Path $winfsLib -Pattern '#\[cfg\(any\(test, feature = "test-support"\)\)\]' -Context 0,1 -AllMatches |
  Where-Object { $_.Context.PostContext -match 'pub fn set_last_write_time' }
if (-not $setTimeGate) {
  Fail "set_last_write_time must be gated with #[cfg(any(test, feature = `"test-support`"))]."
}
if (-not $removeDirWHits -or $removeDirWHits.Count -ne 3) {
  Fail "Expected exactly 3 RemoveDirectoryW occurrences in sweeper-winfs/src/lib.rs (gated import + 2 call sites in gated helper)."
}
$removeDirCalls = Select-String -Path $winfsLib -Pattern 'RemoveDirectoryW\(' -AllMatches
if (-not $removeDirCalls -or $removeDirCalls.Count -ne 2) {
  Fail "Expected exactly 2 RemoveDirectoryW call sites in sweeper-winfs/src/lib.rs (both inside gated test-support helper cleanup)."
}

if ($failed) { exit 1 }
Write-Host "GATE-A grep checks passed." -ForegroundColor Green
