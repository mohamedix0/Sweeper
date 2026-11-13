#!/usr/bin/env pwsh
# Static Gate A grep checks (CI-enforced).
# Fails if std::fs::remove_* appears in production code under backend/ or bridge/,
# or if sweeper-winfs deletion symbols appear outside core::remover / sweeper-winfs.

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

# 1) Ban std::fs::remove_* in production code (backend/, bridge/)
$removePattern = 'std::fs::remove_(file|dir_all|dir)\b'
$removeHits = Get-ChildItem -Path $prodPaths -Recurse -Include *.rs -File |
  Where-Object { $_.FullName -notmatch '\\target\\' -and $_.FullName -notmatch '\\tests?\\' } |
  Select-String -Pattern $removePattern -AllMatches
# Also scan test files? Gate states "anywhere in production code", so exclude tests/ dirs and files under tests paths.
$removeHitsProd = Get-ChildItem -Path $prodPaths -Recurse -Include *.rs -File |
  Where-Object { $_.FullName -notmatch '\\target\\' -and $_.FullName -notmatch '([/\\])tests([/\\])' -and $_.Name -notmatch '_test\.rs$' } |
  Select-String -Pattern $removePattern -AllMatches
if ($removeHitsProd) {
  foreach ($h in $removeHitsProd) { Write-Host "$($h.Path):$($h.LineNumber): $($h.Line.Trim())" }
  Fail "std::fs::remove_* found in production code under backend/ or bridge/."
}

# 2) Ban sweeper-winfs deletion symbols outside core::remover / sweeper-winfs.
# No deletion primitives exist in Phase 0; reserve symbol names for future phases.
$deletionSymbols = '(delete_by_handle|remove_by_handle|FileDispositionInfoEx|SetFileInformationByHandle\s*\(.*FileDisposition|sweeper_winfs::delete)'
$deletionHits = Get-ChildItem -Path $prodPaths -Recurse -Include *.rs -File |
  Where-Object {
    $_.FullName -notmatch '\\target\\' -and
    $_.FullName -notmatch '\\backend\\sweeper-winfs\\' -and
    $_.FullName -notmatch '\\backend\\sweeper-core\\src\\remover([/\\.]|$)' -and
    $_.FullName -notmatch '\\backend\\sweeper-core\\src\\remover\.rs$'
  } |
  Select-String -Pattern $deletionSymbols -AllMatches
if ($deletionHits) {
  foreach ($h in $deletionHits) { Write-Host "$($h.Path):$($h.LineNumber): $($h.Line.Trim())" }
  Fail "sweeper-winfs deletion symbol found outside backend/sweeper-winfs and backend/sweeper-core remover module."
}

if ($failed) { exit 1 }
Write-Host "GATE-A grep checks passed." -ForegroundColor Green
