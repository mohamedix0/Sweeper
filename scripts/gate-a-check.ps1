#!/usr/bin/env pwsh
# Static Gate A checks (CI-enforced).
# - Bans std::fs::remove_* and raw deletion primitives in production code outside allowed modules.
# - Bans sweeper-winfs dependency from sweeper-desktop.
# - Enforces privacy/scrub rules on tracked files (no personal emails, local machine paths, internal IDs, private tokens).

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$failed = $false

function Fail([string]$msg) {
  Write-Host "GATE-A FAIL: $msg" -ForegroundColor Red
  $script:failed = $true
}

function Invoke-CheckStdFsRemovals {
  $appBackend = Join-Path $repoRoot 'app\backend'
  $appBridge = Join-Path $repoRoot 'app\bridge'
  $prodPaths = @($appBackend, $appBridge)
  $removePattern = 'std::fs::remove_(file|dir_all|dir)\b'
  $removeHitsProd = Get-ChildItem -Path $prodPaths -Recurse -Include *.rs -File -ErrorAction SilentlyContinue |
    Where-Object {
      $_.FullName -match '([/\\])src([/\\])' -and
      $_.FullName -notmatch '([/\\])target([/\\])' -and
      $_.FullName -notmatch '([/\\])app([/\\])backend([/\\])sweeper-testkit([/\\])' -and
      $_.FullName -notmatch '([/\\])tests([/\\])'
    } |
    Select-String -Pattern $removePattern -AllMatches
  if ($removeHitsProd) {
    foreach ($h in $removeHitsProd) { Write-Host "$($h.Path):$($h.LineNumber): $($h.Line.Trim())" }
    Fail "std::fs::remove_* found in production source (app/backend or app/bridge src/, excluding sweeper-testkit and tests/)."
  }
}

function Invoke-CheckDeletionSymbols {
  $appBackend = Join-Path $repoRoot 'app\backend'
  $appBridge = Join-Path $repoRoot 'app\bridge'
  $prodPaths = @($appBackend, $appBridge)
  $deletionSymbols = '(delete_by_handle|remove_by_handle|set_file_disposition_delete|open_for_delete|clear_readonly_attribute|FileDispositionInfoEx|SetFileInformationByHandle\s*\(.*FileDisposition|sweeper_winfs::delete_file|sweeper_winfs::delete_dir|retire_sweeper_log_file)'
  $deletionHits = Get-ChildItem -Path $prodPaths -Recurse -Include *.rs -File -ErrorAction SilentlyContinue |
    Where-Object {
      $_.FullName -notmatch '([/\\])target([/\\])' -and
      $_.FullName -notmatch '([/\\])app([/\\])backend([/\\])sweeper-winfs([/\\])' -and
      $_.FullName -notmatch '([/\\])app([/\\])backend([/\\])sweeper-core([/\\])src([/\\])remover([/\\.]|$)' -and
      $_.FullName -notmatch '([/\\])app([/\\])backend([/\\])sweeper-core([/\\])src([/\\])remover\.rs$' -and
      $_.FullName -notmatch '([/\\])app([/\\])backend([/\\])sweeper-core([/\\])src([/\\])journal\.rs$'
    } |
    Select-String -Pattern $deletionSymbols -AllMatches
  if ($deletionHits) {
    foreach ($h in $deletionHits) { Write-Host "$($h.Path):$($h.LineNumber): $($h.Line.Trim())" }
    Fail "sweeper-winfs deletion/log-retirement symbol found outside allowed modules (winfs, core::remover, core::journal)."
  }
}

function Invoke-CheckJournalRetireAudit {
  $journalPath = Join-Path $repoRoot 'app\backend\sweeper-core\src\journal.rs'
  $journalRetireHits = Select-String -Path $journalPath -Pattern 'retire_sweeper_log_file' -AllMatches -ErrorAction SilentlyContinue
  if (-not $journalRetireHits -or $journalRetireHits.Count -ne 1) {
    Fail "Expected exactly 1 audited retire_sweeper_log_file call in app/backend/sweeper-core/src/journal.rs."
  }
  $journalAllowHits = Select-String -Path $journalPath -Pattern '#\s*\[allow\(clippy::disallowed_methods\)\]' -AllMatches -ErrorAction SilentlyContinue
  if (-not $journalAllowHits -or $journalAllowHits.Count -lt 1) {
    Fail "Missing audited #[allow(clippy::disallowed_methods)] in app/backend/sweeper-core/src/journal.rs for log retirement call."
  }
}

function Invoke-CheckRawWin32Deletes {
  $appBackend = Join-Path $repoRoot 'app\backend'
  $appBridge = Join-Path $repoRoot 'app\bridge'
  $prodPaths = @($appBackend, $appBridge)
  $rawDeletePattern = '\b(DeleteFileW|RemoveDirectoryW)\b'
  $rawDeleteHits = Get-ChildItem -Path $prodPaths -Recurse -Include *.rs -File -ErrorAction SilentlyContinue |
    Where-Object {
      $_.FullName -notmatch '([/\\])target([/\\])' -and
      $_.FullName -notmatch '([/\\])app([/\\])backend([/\\])sweeper-winfs([/\\])src([/\\])'
    } |
    Select-String -Pattern $rawDeletePattern -AllMatches
  if ($rawDeleteHits) {
    foreach ($h in $rawDeleteHits) { Write-Host "$($h.Path):$($h.LineNumber): $($h.Line.Trim())" }
    Fail "Raw Win32 DeleteFileW/RemoveDirectoryW found outside app/backend/sweeper-winfs/src."
  }
}

function Invoke-CheckWinfsBounds {
  $winfsLib = Join-Path $repoRoot 'app\backend\sweeper-winfs\src\lib.rs'
  $deleteFileWHits = Select-String -Path $winfsLib -Pattern '\bDeleteFileW\b' -AllMatches -ErrorAction SilentlyContinue
  $removeDirWHits = Select-String -Path $winfsLib -Pattern '\bRemoveDirectoryW\b' -AllMatches -ErrorAction SilentlyContinue
  if (-not $deleteFileWHits -or $deleteFileWHits.Count -ne 3) {
    Fail "Expected exactly 3 DeleteFileW occurrences in app/backend/sweeper-winfs/src/lib.rs (import, call in retire_sweeper_log_file, and context string)."
  }
  $deleteApiCallHits = Select-String -Path $winfsLib -Pattern 'unsafe\s*\{\s*DeleteFileW\(' -AllMatches -ErrorAction SilentlyContinue
  if (-not $deleteApiCallHits -or $deleteApiCallHits.Count -ne 1) {
    Fail "Expected exactly 1 raw DeleteFileW API call site in app/backend/sweeper-winfs/src/lib.rs."
  }
  $retireFn = Select-String -Path $winfsLib -Pattern 'pub fn retire_sweeper_log_file' -AllMatches -ErrorAction SilentlyContinue
  if (-not $retireFn -or $retireFn.Count -ne 1) {
    Fail "Expected retire_sweeper_log_file to exist in app/backend/sweeper-winfs/src/lib.rs."
  }
  $retireDeleteContext = Select-String -Path $winfsLib -Pattern 'DeleteFileW\(retire_sweeper_log_file\)' -AllMatches -ErrorAction SilentlyContinue
  if (-not $retireDeleteContext -or $retireDeleteContext.Count -ne 1) {
    Fail "Expected DeleteFileW usage to be scoped to retire_sweeper_log_file context."
  }
  $createJunctionGate = Select-String -Path $winfsLib -Pattern '#\[cfg\(any\(test, feature = "test-support"\)\)\]' -Context 0,1 -AllMatches -ErrorAction SilentlyContinue |
    Where-Object { $_.Context.PostContext -match 'pub fn create_directory_junction' }
  if (-not $createJunctionGate) {
    Fail "create_directory_junction must be gated with #[cfg(any(test, feature = `"test-support`"))]."
  }
  $setTimeGate = Select-String -Path $winfsLib -Pattern '#\[cfg\(any\(test, feature = "test-support"\)\)\]' -Context 0,1 -AllMatches -ErrorAction SilentlyContinue |
    Where-Object { $_.Context.PostContext -match 'pub fn set_last_write_time' }
  if (-not $setTimeGate) {
    Fail "set_last_write_time must be gated with #[cfg(any(test, feature = `"test-support`"))]."
  }
  if (-not $removeDirWHits -or $removeDirWHits.Count -ne 3) {
    Fail "Expected exactly 3 RemoveDirectoryW occurrences in app/backend/sweeper-winfs/src/lib.rs (gated import + 2 call sites in gated helper)."
  }
  $removeDirCalls = Select-String -Path $winfsLib -Pattern 'RemoveDirectoryW\(' -AllMatches -ErrorAction SilentlyContinue
  if (-not $removeDirCalls -or $removeDirCalls.Count -ne 2) {
    Fail "Expected exactly 2 RemoveDirectoryW call sites in app/backend/sweeper-winfs/src/lib.rs (both inside gated test-support helper cleanup)."
  }
}

function Invoke-CheckDesktopCargoDeps {
  $desktopCargo = Join-Path $repoRoot 'app\bridge\sweeper-desktop\Cargo.toml'
  $desktopCargoText = Get-Content -Raw -Path $desktopCargo
  if ($desktopCargoText -match '(?m)^\s*sweeper-winfs\s*=') {
    Fail "app/bridge/sweeper-desktop/Cargo.toml must not depend on sweeper-winfs."
  }
  $depsSection = $desktopCargoText
  $buildDepsIdx = $desktopCargoText.IndexOf('[build-dependencies]')
  $devDepsIdx = $desktopCargoText.IndexOf('[dev-dependencies]')
  $cutIdx = -1
  if ($buildDepsIdx -ge 0) { $cutIdx = $buildDepsIdx }
  if ($devDepsIdx -ge 0 -and ($cutIdx -lt 0 -or $devDepsIdx -lt $cutIdx)) { $cutIdx = $devDepsIdx }
  if ($cutIdx -ge 0) { $depsSection = $desktopCargoText.Substring(0, $cutIdx) }
  $desktopInternalDeps = @()
  foreach ($m in [regex]::Matches($depsSection, '(?m)^\s*(sweeper-[A-Za-z0-9_-]+)\s*=\s*\{[^}]*workspace\s*=\s*true')) {
    $desktopInternalDeps += $m.Groups[1].Value
  }
  $desktopInternalDeps = @($desktopInternalDeps | Sort-Object -Unique)
  $allowedDesktopInternal = @('sweeper-contract', 'sweeper-core')
  if (@($desktopInternalDeps).Count -ne $allowedDesktopInternal.Count -or (@(Compare-Object -ReferenceObject $allowedDesktopInternal -DifferenceObject $desktopInternalDeps)).Count -ne 0) {
    Fail "app/bridge/sweeper-desktop/Cargo.toml internal workspace deps must be exactly: $($allowedDesktopInternal -join ', '). Found: $($desktopInternalDeps -join ', ')"
  }
}

function Get-ForbidUnsafeTargets([string]$root) {
  $targets = @()
  Get-ChildItem -Path (Join-Path $root 'app\backend') -Recurse -Filter 'lib.rs' -File -ErrorAction SilentlyContinue | ForEach-Object {
    if ($_.FullName -notmatch '([/\\])sweeper-winfs([/\\])src([/\\])lib\.rs$') { $targets += $_.FullName }
  }
  Get-ChildItem -Path (Join-Path $root 'app\bridge') -Recurse -Filter 'lib.rs' -File -ErrorAction SilentlyContinue | ForEach-Object { $targets += $_.FullName }
  $main = Join-Path $root 'app\bridge\sweeper-desktop\src\main.rs'
  if (Test-Path -LiteralPath $main) { $targets += $main }
  return $targets
}

function Test-ForbidUnsafeCode([string]$root) {
  foreach ($fp in (Get-ForbidUnsafeTargets $root)) {
    $txt = Get-Content -LiteralPath $fp -Raw -ErrorAction SilentlyContinue
    if ($null -eq $txt -or $txt -notmatch '#!\s*\[forbid\(unsafe_code\)\]') {
      Fail "Missing #![forbid(unsafe_code)] in $fp"
    }
  }
}

function Test-LineHasSafetyComment([string[]]$lines, [int]$idx) {
  $prev = $idx - 1
  for ($checked = 0; $prev -ge 0 -and $checked -lt 15; $checked++) {
    if ($lines[$prev] -match '//\s*SAFETY:') { return $true }
    $prev--
  }
  return $false
}

function Test-WinfsSafetyComments([string]$root) {
  $winfsSrc = Join-Path $root 'app\backend\sweeper-winfs\src'
  if (-not (Test-Path -LiteralPath $winfsSrc)) { return }
  Get-ChildItem -Path $winfsSrc -Recurse -Filter '*.rs' -File -ErrorAction SilentlyContinue | ForEach-Object {
    $lines = Get-Content -LiteralPath $_.FullName -ErrorAction SilentlyContinue
    for ($i = 0; $i -lt $lines.Count; $i++) {
      $line = $lines[$i]
      $isUnsafe = ($line -match '\bunsafe\s+fn\b' -or $line -match '\bunsafe\s*\{')
      if ($isUnsafe -and -not (Test-LineHasSafetyComment $lines $i)) {
        Fail "Missing // SAFETY: comment above unsafe at $($_.FullName):$($i + 1)"
      }
    }
  }
}

function Invoke-OptionalCargoGeiger([string]$root) {
  $geiger = Get-Command cargo-geiger -ErrorAction SilentlyContinue
  if ($null -eq $geiger) {
    Write-Host "GATE-A: cargo-geiger not found in PATH; skipping cargo geiger check (static checks still enforced)." -ForegroundColor Yellow
    return
  }
  Write-Host "GATE-A: running cargo geiger..."
  Push-Location (Join-Path $root 'app')
  try {
    & cargo geiger --locked 2>&1 | Write-Host
    if ($LASTEXITCODE -ne 0) { Fail "cargo geiger reported issues (exit code $LASTEXITCODE)." }
  } finally { Pop-Location }
}

function Invoke-PrivacyScrubChecks {
  $tracked = git -C $repoRoot ls-files
  $personalEmailPattern = '(?i)(@' + 'gmail\.com\b|@' + 'outlook\.com\b|@' + 'hotmail\.com\b|@' + 'yahoo\.com\b|@' + 'icloud\.com\b|@' + 'proton\.me\b|@' + 'protonmail\.com\b)'
  $userNameFrag = 'pub' + '22'
  $projectFrag = 'Hermes' + ' Projects'
  $localUserPathPattern = '(?i)C:\\Users\\(?!Alice\b|Bob\b|<|name>|<user>|Public\b|Default\b|Default User\b|All Users\b|User\b|Test\b|me\b|A\b|B\b|X\b|Admin\b|Administrator\b|user\b|test\b|admin\b)[A-Za-z0-9._-]+'
  $threadIdFrag = 'decf13a1' + '-4de9-4616-b251-262b42c6ac64'
  $tokenPatterns = @('gh' + 'p_[A-Za-z0-9]{20,}', 'github' + '_pat_[A-Za-z0-9_]{20,}', 'BEGIN [A-Z ]*PRIVATE KEY')
  $privacyHits = @()
  foreach ($rel in $tracked) {
    if (-not $rel) { continue }
    $full = Join-Path $repoRoot $rel
    if (-not (Test-Path -LiteralPath $full -PathType Leaf)) { continue }
    $ext = [System.IO.Path]::GetExtension($rel).ToLowerInvariant()
    $skipExt = @('.png','.jpg','.jpeg','.gif','.ico','.icns','.webp','.bmp','.exe','.dll','.pdb','.zip','.7z','.gz','.woff','.woff2','.ttf','.otf','.eot')
    if ($skipExt -contains $ext) { continue }
    $text = $null
    try { $text = Get-Content -LiteralPath $full -Raw -ErrorAction Stop } catch { continue }
    if ($null -eq $text) { continue }
    foreach ($m in [regex]::Matches($text, '[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}')) {
      $addr = $m.Value
      if ($addr -match $personalEmailPattern) {
        $privacyHits += "${rel}: personal email domain detected: $addr"
        continue
      }
    }
    if ($text -match $userNameFrag) { $privacyHits += "${rel}: forbidden local username fragment detected" }
    if ($text -match [regex]::Escape($projectFrag)) { $privacyHits += "${rel}: forbidden local project path fragment detected" }
    foreach ($m in [regex]::Matches($text, $localUserPathPattern)) {
      $privacyHits += "${rel}: forbidden local user path detected: $($m.Value)"
    }
    if ($text -match [regex]::Escape($threadIdFrag)) { $privacyHits += "${rel}: internal thread identifier detected" }
    foreach ($tp in $tokenPatterns) {
      if ($text -match $tp) { $privacyHits += "${rel}: private token/key material pattern detected" }
    }
  }
  if ($privacyHits.Count -gt 0) {
    foreach ($p in ($privacyHits | Select-Object -Unique)) { Write-Host $p }
    Fail "Privacy/scrub checks failed. Tracked files must not contain personal emails, local machine paths/usernames, internal thread IDs, or private tokens."
  }
}

function Invoke-GateAChecks {
  Invoke-CheckStdFsRemovals
  Invoke-CheckDeletionSymbols
  Invoke-CheckJournalRetireAudit
  Invoke-CheckRawWin32Deletes
  Invoke-CheckWinfsBounds
  Invoke-CheckDesktopCargoDeps
  Test-ForbidUnsafeCode $repoRoot
  Test-WinfsSafetyComments $repoRoot
  Invoke-OptionalCargoGeiger $repoRoot
  Invoke-PrivacyScrubChecks
  if ($failed) { exit 1 }
  Write-Host "GATE-A checks passed." -ForegroundColor Green
}

Invoke-GateAChecks
