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

function Get-RsFiles([string[]]$paths) {
  return @(Get-ChildItem -Path $paths -Recurse -Include *.rs -File -ErrorAction SilentlyContinue)
}

function Test-NotTargetPath([string]$path) {
  return ($path -notmatch '([/\\])target([/\\])')
}

function Test-ProdSrcPath([string]$path) {
  $ok = ($path -match '([/\\])src([/\\])')
  if (-not $ok) { return $false }
  $ok = Test-NotTargetPath $path
  if (-not $ok) { return $false }
  $ok = ($path -notmatch '([/\\])app([/\\])backend([/\\])sweeper-testkit([/\\])')
  if (-not $ok) { return $false }
  return ($path -notmatch '([/\\])tests([/\\])')
}

function Get-ProdSrcRsFiles([string[]]$paths) {
  return @(Get-RsFiles $paths | Where-Object { Test-ProdSrcPath $_.FullName })
}

function Write-SelectStringHits($hits) {
  foreach ($h in @($hits)) { Write-Host "$($h.Path):$($h.LineNumber): $($h.Line.Trim())" }
}

function Assert-SelectStringCount([string]$path, [string]$pattern, [int]$expected, [string]$message) {
  $hits = @(Select-String -Path $path -Pattern $pattern -AllMatches -ErrorAction SilentlyContinue)
  if ($hits.Count -eq $expected) { return }
  Fail $message
}

function Assert-SelectStringMinCount([string]$path, [string]$pattern, [int]$min, [string]$message) {
  $hits = @(Select-String -Path $path -Pattern $pattern -AllMatches -ErrorAction SilentlyContinue)
  if ($hits.Count -ge $min) { return }
  Fail $message
}

function Assert-TextAbsent([string]$path, [string]$pattern, [string]$message) {
  $text = Get-Content -Raw -Path $path
  if ($text -notmatch $pattern) { return }
  Fail $message
}

function Get-AppProdPaths() {
  $appBackend = Join-Path $repoRoot 'app\backend'
  $appBridge = Join-Path $repoRoot 'app\bridge'
  return @($appBackend, $appBridge)
}

function Invoke-CheckStdFsRemovals {
  $removePattern = 'std::fs::remove_(file|dir_all|dir)\b'
  $files = Get-ProdSrcRsFiles (Get-AppProdPaths)
  $hits = @($files | Select-String -Pattern $removePattern -AllMatches)
  if ($hits.Count -eq 0) { return }
  Write-SelectStringHits $hits
  Fail "std::fs::remove_* found in production source (app/backend or app/bridge src/, excluding sweeper-testkit and tests/)."
}

function Test-DeletionAllowedPath([string]$path) {
  if (-not (Test-NotTargetPath $path)) { return $false }
  if ($path -match '([/\\])app([/\\])backend([/\\])sweeper-winfs([/\\])') { return $false }
  if ($path -match '([/\\])app([/\\])backend([/\\])sweeper-core([/\\])src([/\\])remover([/\\.]|$)') { return $false }
  if ($path -match '([/\\])app([/\\])backend([/\\])sweeper-core([/\\])src([/\\])remover\.rs$') { return $false }
  if ($path -match '([/\\])app([/\\])backend([/\\])sweeper-core([/\\])src([/\\])journal\.rs$') { return $false }
  return $true
}

function Invoke-CheckDeletionSymbols {
  $deletionSymbols = '(delete_by_handle|remove_by_handle|set_file_disposition_delete|open_for_delete|clear_readonly_attribute|FileDispositionInfoEx|SetFileInformationByHandle\s*\(.*FileDisposition|sweeper_winfs::delete_file|sweeper_winfs::delete_dir|retire_sweeper_log_file)'
  $files = @(Get-RsFiles (Get-AppProdPaths) | Where-Object { Test-DeletionAllowedPath $_.FullName })
  $hits = @($files | Select-String -Pattern $deletionSymbols -AllMatches)
  if ($hits.Count -eq 0) { return }
  Write-SelectStringHits $hits
  Fail "sweeper-winfs deletion/log-retirement symbol found outside allowed modules (winfs, core::remover, core::journal)."
}

function Invoke-CheckJournalRetireAudit {
  $journalPath = Join-Path $repoRoot 'app\backend\sweeper-core\src\journal.rs'
  Assert-SelectStringCount $journalPath 'retire_sweeper_log_file' 1 "Expected exactly 1 audited retire_sweeper_log_file call in app/backend/sweeper-core/src/journal.rs."
  Assert-SelectStringMinCount $journalPath '#\s*\[allow\(clippy::disallowed_methods\)\]' 1 "Missing audited #[allow(clippy::disallowed_methods)] in app/backend/sweeper-core/src/journal.rs for log retirement call."
}

function Test-RawDeleteAllowedPath([string]$path) {
  if (-not (Test-NotTargetPath $path)) { return $false }
  return ($path -notmatch '([/\\])app([/\\])backend([/\\])sweeper-winfs([/\\])src([/\\])')
}

function Invoke-CheckRawWin32Deletes {
  $rawDeletePattern = '\b(DeleteFileW|RemoveDirectoryW)\b'
  $files = @(Get-RsFiles (Get-AppProdPaths) | Where-Object { Test-RawDeleteAllowedPath $_.FullName })
  $hits = @($files | Select-String -Pattern $rawDeletePattern -AllMatches)
  if ($hits.Count -eq 0) { return }
  Write-SelectStringHits $hits
  Fail "Raw Win32 DeleteFileW/RemoveDirectoryW found outside app/backend/sweeper-winfs/src."
}

function Assert-WinfsGatedFunction([string]$winfsLib, [string]$fnName, [string]$message) {
  $gatePattern = '#\[cfg\(any\(test, feature = "test-support"\)\)\]'
  $hits = @(Select-String -Path $winfsLib -Pattern $gatePattern -Context 0,1 -AllMatches -ErrorAction SilentlyContinue | Where-Object { $_.Context.PostContext -match "pub fn $fnName" })
  if ($hits.Count -gt 0) { return }
  Fail $message
}

function Invoke-CheckWinfsBounds {
  $winfsLib = Join-Path $repoRoot 'app\backend\sweeper-winfs\src\lib.rs'
  Assert-SelectStringCount $winfsLib '\bDeleteFileW\b' 3 "Expected exactly 3 DeleteFileW occurrences in app/backend/sweeper-winfs/src/lib.rs (import, call in retire_sweeper_log_file, and context string)."
  Assert-SelectStringCount $winfsLib 'unsafe\s*\{\s*DeleteFileW\(' 1 "Expected exactly 1 raw DeleteFileW API call site in app/backend/sweeper-winfs/src/lib.rs."
  Assert-SelectStringCount $winfsLib 'pub fn retire_sweeper_log_file' 1 "Expected retire_sweeper_log_file to exist in app/backend/sweeper-winfs/src/lib.rs."
  Assert-SelectStringCount $winfsLib 'DeleteFileW\(retire_sweeper_log_file\)' 1 "Expected DeleteFileW usage to be scoped to retire_sweeper_log_file context."
  Assert-WinfsGatedFunction $winfsLib 'create_directory_junction' "create_directory_junction must be gated with #[cfg(any(test, feature = `"test-support`"))]."
  Assert-WinfsGatedFunction $winfsLib 'set_last_write_time' "set_last_write_time must be gated with #[cfg(any(test, feature = `"test-support`"))]."
  Assert-SelectStringCount $winfsLib '\bRemoveDirectoryW\b' 3 "Expected exactly 3 RemoveDirectoryW occurrences in app/backend/sweeper-winfs/src/lib.rs (gated import + 2 call sites in gated helper)."
  Assert-SelectStringCount $winfsLib 'RemoveDirectoryW\(' 2 "Expected exactly 2 RemoveDirectoryW call sites in app/backend/sweeper-winfs/src/lib.rs (both inside gated test-support helper cleanup)."
}

function Get-DesktopDepsCutIndex([string]$text) {
  $cutIdx = -1
  $buildDepsIdx = $text.IndexOf('[build-dependencies]')
  $devDepsIdx = $text.IndexOf('[dev-dependencies]')
  if ($buildDepsIdx -ge 0) { $cutIdx = $buildDepsIdx }
  if ($devDepsIdx -lt 0) { return $cutIdx }
  if ($cutIdx -lt 0) { return $devDepsIdx }
  if ($devDepsIdx -lt $cutIdx) { return $devDepsIdx }
  return $cutIdx
}

function Get-DesktopDepsSection([string]$text) {
  $cutIdx = Get-DesktopDepsCutIndex $text
  if ($cutIdx -lt 0) { return $text }
  return $text.Substring(0, $cutIdx)
}

function Get-DesktopInternalDeps([string]$depsSection) {
  $deps = @()
  foreach ($m in [regex]::Matches($depsSection, '(?m)^\s*(sweeper-[A-Za-z0-9_-]+)\s*=\s*\{[^}]*workspace\s*=\s*true')) {
    $deps += $m.Groups[1].Value
  }
  return @($deps | Sort-Object -Unique)
}

function Assert-DesktopInternalDeps([string[]]$found) {
  $allowed = @('sweeper-contract', 'sweeper-core')
  $countOk = ($found.Count -eq $allowed.Count)
  $diffCount = 0
  if ($countOk) { $diffCount = @(Compare-Object -ReferenceObject $allowed -DifferenceObject $found).Count }
  $ok = $countOk
  if ($ok) { $ok = ($diffCount -eq 0) }
  if ($ok) { return }
  Fail "app/bridge/sweeper-desktop/Cargo.toml internal workspace deps must be exactly: $($allowed -join ', '). Found: $($found -join ', ')"
}

function Invoke-CheckDesktopCargoDeps {
  $desktopCargo = Join-Path $repoRoot 'app\bridge\sweeper-desktop\Cargo.toml'
  Assert-TextAbsent $desktopCargo '(?m)^\s*sweeper-winfs\s*=' "app/bridge/sweeper-desktop/Cargo.toml must not depend on sweeper-winfs."
  $text = Get-Content -Raw -Path $desktopCargo
  $depsSection = Get-DesktopDepsSection $text
  $found = Get-DesktopInternalDeps $depsSection
  Assert-DesktopInternalDeps $found
}

function Get-ForbidUnsafeTargets([string]$root) {
  $targets = @()
  $backendLibs = @(Get-ChildItem -Path (Join-Path $root 'app\backend') -Recurse -Filter 'lib.rs' -File -ErrorAction SilentlyContinue)
  foreach ($f in $backendLibs) {
    $isWinfs = ($f.FullName -match '([/\\])sweeper-winfs([/\\])src([/\\])lib\.rs$')
    if ($isWinfs) { continue }
    $targets += $f.FullName
  }
  $bridgeLibs = @(Get-ChildItem -Path (Join-Path $root 'app\bridge') -Recurse -Filter 'lib.rs' -File -ErrorAction SilentlyContinue)
  foreach ($f in $bridgeLibs) { $targets += $f.FullName }
  $main = Join-Path $root 'app\bridge\sweeper-desktop\src\main.rs'
  if (Test-Path -LiteralPath $main) { $targets += $main }
  return @($targets)
}

function Assert-FileHasForbidUnsafe([string]$fp) {
  $txt = Get-Content -LiteralPath $fp -Raw -ErrorAction SilentlyContinue
  if ($null -eq $txt) {
    Fail "Missing #![forbid(unsafe_code)] in $fp"
    return
  }
  if ($txt -match '#!\s*\[forbid\(unsafe_code\)\]') { return }
  Fail "Missing #![forbid(unsafe_code)] in $fp"
}

function Test-ForbidUnsafeCode([string]$root) {
  $targets = @(Get-ForbidUnsafeTargets $root)
  foreach ($fp in $targets) { Assert-FileHasForbidUnsafe $fp }
}

function Test-LineHasSafetyComment([string[]]$lines, [int]$idx) {
  $prev = $idx - 1
  $checked = 0
  while ($prev -ge 0) {
    if ($checked -ge 15) { return $false }
    if ($lines[$prev] -match '//\s*SAFETY:') { return $true }
    $prev--
    $checked++
  }
  return $false
}

function Test-IsUnsafeLine([string]$line) {
  if ($line -match '\bunsafe\s+fn\b') { return $true }
  return ($line -match '\bunsafe\s*\{')
}

function Assert-UnsafeLineHasSafety([string]$path, [string[]]$lines, [int]$i) {
  $isUnsafe = Test-IsUnsafeLine $lines[$i]
  if (-not $isUnsafe) { return }
  $hasSafety = Test-LineHasSafetyComment $lines $i
  if ($hasSafety) { return }
  Fail "Missing // SAFETY: comment above unsafe at ${path}:$($i + 1)"
}

function Assert-UnsafeLinesHaveSafety([string]$path, [string[]]$lines) {
  for ($i = 0; $i -lt $lines.Count; $i++) {
    Assert-UnsafeLineHasSafety $path $lines $i
  }
}

function Test-WinfsSafetyComments([string]$root) {
  $winfsSrc = Join-Path $root 'app\backend\sweeper-winfs\src'
  $exists = Test-Path -LiteralPath $winfsSrc
  if (-not $exists) { return }
  $files = @(Get-ChildItem -Path $winfsSrc -Recurse -Filter '*.rs' -File -ErrorAction SilentlyContinue)
  foreach ($f in $files) {
    $lines = @(Get-Content -LiteralPath $f.FullName -ErrorAction SilentlyContinue)
    Assert-UnsafeLinesHaveSafety $f.FullName $lines
  }
}

function Invoke-CargoGeiger([string]$root) {
  Write-Host "GATE-A: running cargo geiger..."
  Push-Location (Join-Path $root 'app')
  try {
    & cargo geiger --locked 2>&1 | Write-Host
    if ($LASTEXITCODE -ne 0) { Fail "cargo geiger reported issues (exit code $LASTEXITCODE)." }
  } finally { Pop-Location }
}

function Invoke-OptionalCargoGeiger([string]$root) {
  $geiger = Get-Command cargo-geiger -ErrorAction SilentlyContinue
  if ($null -eq $geiger) {
    Write-Host "GATE-A: cargo-geiger not found in PATH; skipping cargo geiger check (static checks still enforced)." -ForegroundColor Yellow
    return
  }
  Invoke-CargoGeiger $root
}

function Get-PrivacyRules() {
  $personalEmailPattern = '(?i)(@' + 'gmail\.com\b|@' + 'outlook\.com\b|@' + 'hotmail\.com\b|@' + 'yahoo\.com\b|@' + 'icloud\.com\b|@' + 'proton\.me\b|@' + 'protonmail\.com\b)'
  $userNameFrag = 'pub' + '22'
  $projectFrag = 'Hermes' + ' Projects'
  $localUserPathPattern = '(?i)C:\\Users\\(?!Alice\b|Bob\b|<|name>|<user>|Public\b|Default\b|Default User\b|All Users\b|User\b|Test\b|me\b|A\b|B\b|X\b|Admin\b|Administrator\b|user\b|test\b|admin\b)[A-Za-z0-9._-]+'
  $threadIdFrag = 'decf13a1' + '-4de9-4616-b251-262b42c6ac64'
  return [ordered]@{
    PersonalEmailPattern = $personalEmailPattern
    UserNameFrag = $userNameFrag
    ProjectFrag = $projectFrag
    LocalUserPathPattern = $localUserPathPattern
    ThreadIdFrag = $threadIdFrag
    TokenPatterns = @('gh' + 'p_[A-Za-z0-9]{20,}', 'github' + '_pat_[A-Za-z0-9_]{20,}', 'BEGIN [A-Z ]*PRIVATE KEY')
  }
}

function Test-SkippablePrivacyExt([string]$rel) {
  $ext = [System.IO.Path]::GetExtension($rel).ToLowerInvariant()
  $skipExt = @('.png','.jpg','.jpeg','.gif','.ico','.icns','.webp','.bmp','.exe','.dll','.pdb','.zip','.7z','.gz','.woff','.woff2','.ttf','.otf','.eot')
  return ($skipExt -contains $ext)
}

function Get-PrivacyFileText([string]$full) {
  try { return Get-Content -LiteralPath $full -Raw -ErrorAction Stop } catch { return $null }
}

function Get-PrivacyHitsForEmails([string]$rel, [string]$text, [string]$personalEmailPattern) {
  $hits = @()
  foreach ($m in [regex]::Matches($text, '[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}')) {
    $addr = $m.Value
    $isPersonal = ($addr -match $personalEmailPattern)
    if (-not $isPersonal) { continue }
    $hits += "${rel}: personal email domain detected: $addr"
  }
  return @($hits)
}

function Get-PrivacyHitsForTokens([string]$rel, [string]$text, [string[]]$tokenPatterns) {
  $hits = @()
  foreach ($tp in $tokenPatterns) {
    $matched = ($text -match $tp)
    if (-not $matched) { continue }
    $hits += "${rel}: private token/key material pattern detected"
  }
  return @($hits)
}

function Get-PrivacyHitsForLocalPaths([string]$rel, [string]$text, [string]$localUserPathPattern) {
  $hits = @()
  foreach ($m in [regex]::Matches($text, $localUserPathPattern)) {
    $hits += "${rel}: forbidden local user path detected: $($m.Value)"
  }
  return @($hits)
}

function Get-PrivacyHitsForSimplePatterns([string]$rel, [string]$text, $rules) {
  $hits = @()
  if ($text -match $rules.UserNameFrag) { $hits += "${rel}: forbidden local username fragment detected" }
  if ($text -match [regex]::Escape($rules.ProjectFrag)) { $hits += "${rel}: forbidden local project path fragment detected" }
  if ($text -match [regex]::Escape($rules.ThreadIdFrag)) { $hits += "${rel}: internal thread identifier detected" }
  return @($hits)
}

function Get-PrivacyHitsForFile([string]$rel, [string]$text, $rules) {
  $hits = @()
  $hits += Get-PrivacyHitsForEmails $rel $text $rules.PersonalEmailPattern
  $hits += Get-PrivacyHitsForSimplePatterns $rel $text $rules
  $hits += Get-PrivacyHitsForLocalPaths $rel $text $rules.LocalUserPathPattern
  $hits += Get-PrivacyHitsForTokens $rel $text $rules.TokenPatterns
  return @($hits)
}

function Get-PrivacyHitsForTrackedFile([string]$rel, $rules) {
  if (-not $rel) { return @() }
  $full = Join-Path $repoRoot $rel
  if (-not (Test-Path -LiteralPath $full -PathType Leaf)) { return @() }
  if (Test-SkippablePrivacyExt $rel) { return @() }
  $text = Get-PrivacyFileText $full
  if ($null -eq $text) { return @() }
  return @(Get-PrivacyHitsForFile $rel $text $rules)
}

function Invoke-PrivacyScrubChecks {
  $tracked = @(git -C $repoRoot ls-files)
  $rules = Get-PrivacyRules
  $privacyHits = @()
  foreach ($rel in $tracked) {
    $privacyHits += Get-PrivacyHitsForTrackedFile $rel $rules
  }
  if ($privacyHits.Count -eq 0) { return }
  foreach ($p in ($privacyHits | Select-Object -Unique)) { Write-Host $p }
  Fail "Privacy/scrub checks failed. Tracked files must not contain personal emails, local machine paths/usernames, internal thread IDs, or private tokens."
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