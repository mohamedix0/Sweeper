param(
  [switch]$SelfTest
)

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

function Test-RustNewline([char]$c) {
  return ($c -eq "`r") -or ($c -eq "`n")
}

function Get-RustScanState([string]$text) {
  $out = $text.ToCharArray()
  return @{ Out = $out; Len = $out.Length; Index = 0; Depth = 0; InString = $false; InLine = $false; RawHashes = -1 }
}

function Step-RustBlock([hashtable]$state) {
  $i = $state.Index
  $j = $i + 1
  $open = (($state.Out[$i] -eq '/') -and ($j -lt $state.Len) -and ($state.Out[$j] -eq '*'))
  if ($open) {
    $state.Out[$i] = ' '; $state.Out[$j] = ' '; $state.Depth++; $state.Index = $i + 2; return
  }
  $close = (($state.Out[$i] -eq '*') -and ($j -lt $state.Len) -and ($state.Out[$j] -eq '/'))
  if ($close) {
    $state.Out[$i] = ' '; $state.Out[$j] = ' '; $state.Depth--; $state.Index = $i + 2; return
  }
  if (-not (Test-RustNewline $state.Out[$i])) { $state.Out[$i] = ' ' }
  $state.Index = $i + 1
}

function Step-RustLineComment([hashtable]$state) {
  $i = $state.Index
  $isNl = Test-RustNewline $state.Out[$i]
  if ($isNl) { $state.InLine = $false; $state.Index = $i + 1; return }
  $state.Out[$i] = ' '; $state.Index = $i + 1
}

function Step-RustString([hashtable]$state) {
  $i = $state.Index
  $j = $i + 1
  $esc = (($state.Out[$i] -eq '\') -and ($j -lt $state.Len))
  if ($esc) {
    if (-not (Test-RustNewline $state.Out[$i])) { $state.Out[$i] = ' ' }
    if (-not (Test-RustNewline $state.Out[$j])) { $state.Out[$j] = ' ' }
    $state.Index = $i + 2; return
  }
  if ($state.Out[$i] -eq '"') {
    $state.Out[$i] = ' '; $state.InString = $false; $state.Index = $i + 1; return
  }
  if (-not (Test-RustNewline $state.Out[$i])) { $state.Out[$i] = ' ' }
  $state.Index = $i + 1
}

function Test-RustRawHashesAt([hashtable]$state, [int]$i) {
  $j = $i
  while (($j -lt $state.Len) -and ($state.Out[$j] -eq '#')) { $j++ }
  $ok = (($j -lt $state.Len) -and ($state.Out[$j] -eq '"'))
  if (-not $ok) { return -1 }
  return ($j - $i)
}

function Get-RustRawPrefixStart([hashtable]$state) {
  $i = $state.Index
  if ($state.Out[$i] -eq 'r') { return $i }
  $j = $i + 1
  $isB = (($state.Out[$i] -eq 'b') -and ($j -lt $state.Len) -and ($state.Out[$j] -eq 'r'))
  if ($isB) { return $j }
  $isC = (($state.Out[$i] -eq 'c') -and ($j -lt $state.Len) -and ($state.Out[$j] -eq 'r'))
  if ($isC) { return $j }
  return -1
}

function Try-StartRustRawString([hashtable]$state) {
  $rIdx = Get-RustRawPrefixStart $state
  if ($rIdx -lt 0) { return $false }
  $prev = $state.Index - 1
  $ident = (($prev -ge 0) -and ($state.Out[$prev] -match '[A-Za-z0-9_]'))
  if ($ident) { return $false }
  $hashes = Test-RustRawHashesAt $state ($rIdx + 1)
  if ($hashes -lt 0) { return $false }
  $end = $rIdx + 1 + $hashes
  for ($k = $state.Index; $k -le $end; $k++) { $state.Out[$k] = ' ' }
  $state.RawHashes = $hashes; $state.Index = $end + 1; return $true
}

function Test-RustRawCloseAt([hashtable]$state) {
  $i = $state.Index
  if ($state.Out[$i] -ne '"') { return $false }
  for ($k = 0; $k -lt $state.RawHashes; $k++) {
    $j = $i + 1 + $k
    $inBounds = ($j -lt $state.Len)
    if (-not $inBounds) { return $false }
    if ($state.Out[$j] -ne '#') { return $false }
  }
  return $true
}

function Complete-RustRawClose([hashtable]$state) {
  $i = $state.Index
  $end = $i + $state.RawHashes
  for ($k = $i + 1; $k -le $end; $k++) { $state.Out[$k] = ' ' }
  $state.RawHashes = -1
  $state.Index = $end + 1
}

function Step-RustRawString([hashtable]$state) {
  $isClose = Test-RustRawCloseAt $state
  $i = $state.Index
  if (-not (Test-RustNewline $state.Out[$i])) { $state.Out[$i] = ' ' }
  if ($isClose) { Complete-RustRawClose $state; return }
  $state.Index = $i + 1
}

function Try-StartRustLineComment([hashtable]$state) {
  $i = $state.Index
  $j = $i + 1
  $isLine = (($j -lt $state.Len) -and ($state.Out[$i] -eq '/') -and ($state.Out[$j] -eq '/'))
  if (-not $isLine) { return $false }
  $state.Out[$i] = ' '; $state.Out[$j] = ' '
  $state.InLine = $true; $state.Index = $i + 2
  return $true
}

function Try-StartRustBlockComment([hashtable]$state) {
  $i = $state.Index
  $j = $i + 1
  $isBlock = (($j -lt $state.Len) -and ($state.Out[$i] -eq '/') -and ($state.Out[$j] -eq '*'))
  if (-not $isBlock) { return $false }
  $state.Out[$i] = ' '; $state.Out[$j] = ' '
  $state.Depth = 1; $state.Index = $i + 2
  return $true
}

function Try-StartRustQuoteString([hashtable]$state) {
  $i = $state.Index
  if ($state.Out[$i] -ne '"') { return $false }
  $state.Out[$i] = ' '; $state.InString = $true
  $state.Index = $i + 1
  return $true
}

function Step-RustNormal([hashtable]$state) {
  if (Try-StartRustRawString $state) { return }
  if (Try-StartRustLineComment $state) { return }
  if (Try-StartRustBlockComment $state) { return }
  if (Try-StartRustQuoteString $state) { return }
  $state.Index = $state.Index + 1
}

function Step-RustScan([hashtable]$state) {
  if ($state.RawHashes -ge 0) { Step-RustRawString $state; return }
  if ($state.Depth -gt 0) { Step-RustBlock $state; return }
  if ($state.InLine) { Step-RustLineComment $state; return }
  if ($state.InString) { Step-RustString $state; return }
  Step-RustNormal $state
}

function Get-RustScanText([string]$text) {
  $state = Get-RustScanState $text
  while ($state.Index -lt $state.Len) { Step-RustScan $state }
  return -join $state.Out
}

function Test-HasActiveForbidUnsafe([string]$text) {
  $scan = Get-RustScanText $text
  return ($scan -match '(?m)^\s*#!\[forbid\(unsafe_code\)\]')
}

function Assert-FileHasForbidUnsafe([string]$fp) {
  $txt = Get-Content -LiteralPath $fp -Raw -ErrorAction SilentlyContinue
  if ($null -eq $txt) {
    Fail "Missing #![forbid(unsafe_code)] in $fp"
    return
  }
  if (Test-HasActiveForbidUnsafe $txt) { return }
  Fail "Missing #![forbid(unsafe_code)] in $fp"
}

function Test-ForbidUnsafeCode([string]$root) {
  $targets = @(Get-ForbidUnsafeTargets $root)
  foreach ($fp in $targets) { Assert-FileHasForbidUnsafe $fp }
}

function Get-UnsafeStartIndices([string]$text) {
  $detect = Get-RustScanText $text
  $matches = [regex]::Matches($detect, '(?s)\bunsafe\b\s*(?:\{|fn\b)')
  $indices = @()
  foreach ($m in $matches) {
    $lineNum = ($detect.Substring(0, $m.Index) -split "`n").Count
    $indices += ($lineNum - 1)
  }
  return @($indices | Sort-Object -Unique)
}

function New-UnsafeIndexSet([int[]]$indices) {
  $set = @{}
  foreach ($i in $indices) { $set[$i] = $true }
  return $set
}

function Test-HasSafetyBeforeUnsafe([string[]]$lines, [hashtable]$unsafeSet, [int]$idx) {
  $prev = $idx - 1
  $checked = 0
  while ($prev -ge 0) {
    if ($checked -ge 15) { return $false }
    if ($unsafeSet.ContainsKey($prev)) { return $false }
    if ($lines[$prev] -match '//\s*SAFETY:') { return $true }
    $prev--
    $checked++
  }
  return $false
}

function Get-WinfsUnsafeIssuesForText([string]$text, [string]$path) {
  $lines = @($text -split "`r?`n")
  $indices = @(Get-UnsafeStartIndices $text)
  $set = New-UnsafeIndexSet $indices
  $issues = @()
  foreach ($idx in $indices) {
    $ok = Test-HasSafetyBeforeUnsafe $lines $set $idx
    if ($ok) { continue }
    $issues += "Missing // SAFETY: comment above unsafe at ${path}:$($idx + 1)"
  }
  return @($issues)
}

function Get-WinfsUnsafeIssues([string]$path, [string]$text, [string[]]$lines) {
  return @(Get-WinfsUnsafeIssuesForText $text $path)
}

function Assert-WinfsFileSafety([string]$path) {
  $text = Get-Content -LiteralPath $path -Raw -ErrorAction SilentlyContinue
  if ($null -eq $text) { return }
  $issues = @(Get-WinfsUnsafeIssuesForText $text $path)
  foreach ($issue in $issues) { Fail $issue }
}

function Test-WinfsSafetyComments([string]$root) {
  $winfsSrc = Join-Path $root 'app\backend\sweeper-winfs\src'
  $exists = Test-Path -LiteralPath $winfsSrc
  if (-not $exists) { return }
  $files = @(Get-ChildItem -Path $winfsSrc -Recurse -Filter '*.rs' -File -ErrorAction SilentlyContinue)
  foreach ($f in $files) { Assert-WinfsFileSafety $f.FullName }
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

function Assert-SelfTest([bool]$condition, [string]$name, [ref]$ok) {
  if ($condition) {
    Write-Host "SELFTEST PASS: $name"
    return
  }
  Write-Host "SELFTEST FAIL: $name" -ForegroundColor Red
  $ok.Value = $false
}

function Test-SelfTestForbidUnsafe([ref]$ok) {
  $commented = "// #![forbid(unsafe_code)]`nfn main() {}"
  $inString = "const X: &str = `"#![forbid(unsafe_code)]`";`nfn main() {}"
  $active = "#![forbid(unsafe_code)]`nfn main() {}"
  Assert-SelfTest (-not (Test-HasActiveForbidUnsafe $commented)) "forbid-unsafe commented out must fail" $ok
  Assert-SelfTest (-not (Test-HasActiveForbidUnsafe $inString)) "forbid-unsafe in string literal must fail" $ok
  Assert-SelfTest (Test-HasActiveForbidUnsafe $active) "forbid-unsafe active attribute must pass" $ok
}

function Get-UnsafeIssueCount([string]$path, [string]$content) {
  $issues = @(Get-WinfsUnsafeIssuesForText $content $path)
  return $issues.Count
}

function Test-SelfTestSharedSafety([ref]$ok) {
  $content = "// SAFETY: first block only`nunsafe { first(); }`nunsafe { second(); }`n"
  $count = Get-UnsafeIssueCount 'shared_safety.rs' $content
  Assert-SelfTest ($count -eq 1) "second unsafe block without its own SAFETY must be reported" $ok
}

function Test-SelfTestUnsafeBlockComment([ref]$ok) {
  $bad = "unsafe /* FFI */ { GetLastError() }`n"
  $good = "// SAFETY: ffi call`nunsafe /* FFI */ { GetLastError() }`n"
  $badCount = Get-UnsafeIssueCount 'inline_block_comment.rs' $bad
  $goodCount = Get-UnsafeIssueCount 'inline_block_comment_ok.rs' $good
  Assert-SelfTest ($badCount -eq 1) "unsafe with block comment and no SAFETY must fail" $ok
  Assert-SelfTest ($goodCount -eq 0) "unsafe with block comment and SAFETY must pass" $ok
}

function Test-SelfTestUnsafeMultiline([ref]$ok) {
  $bad = "unsafe`n{`n  GetLastError()`n}`n"
  $good = "// SAFETY: multiline block`nunsafe`n{`n  GetLastError()`n}`n"
  $badCount = Get-UnsafeIssueCount 'multiline_unsafe.rs' $bad
  $goodCount = Get-UnsafeIssueCount 'multiline_unsafe_ok.rs' $good
  Assert-SelfTest ($badCount -eq 1) "multiline unsafe without SAFETY must fail" $ok
  Assert-SelfTest ($goodCount -eq 0) "multiline unsafe with SAFETY must pass" $ok
}

function Test-SelfTestNestedBlockComments([ref]$ok) {
  $nestedForbid = "/* outer comment`n   /* nested comment */`n#![forbid(unsafe_code)]`n*/`nfn main() {}`n"
  $strBlock = "const X: &str = `"/*`";`n#![forbid(unsafe_code)]`nfn main() {}`n"
  $unsafeNested = "unsafe /* outer /* inner */ */ { GetLastError() }`n"
  $safeNested = "// SAFETY: nested comment in unsafe`nunsafe /* outer /* inner */ */ { GetLastError() }`n"
  Assert-SelfTest (-not (Test-HasActiveForbidUnsafe $nestedForbid)) "forbid-unsafe in nested block comment must fail" $ok
  Assert-SelfTest (Test-HasActiveForbidUnsafe $strBlock) "forbid-unsafe after string containing block opener must pass" $ok
  $unsafeCount = Get-UnsafeIssueCount 'nested_unsafe.rs' $unsafeNested
  $safeCount = Get-UnsafeIssueCount 'nested_unsafe_ok.rs' $safeNested
  Assert-SelfTest ($unsafeCount -eq 1) "unsafe with nested block comment and no SAFETY must fail" $ok
  Assert-SelfTest ($safeCount -eq 0) "unsafe with nested block comment and SAFETY must pass" $ok
}

function Test-SelfTestRawStrings([ref]$ok) {
  $cr = "const X: &str = r#`"`n`"`n#![forbid(unsafe_code)]`n`"#;`nfn main() {}`n"
  $hashes = "const Y: &str = r##`" `"# #![forbid(unsafe_code)] `"##;`nfn main() {}`n"
  $br = "const Z: &[u8] = br#`"/* `"#;`n#![forbid(unsafe_code)]`nfn main() {}`n"
  $unsafe = "const U: &str = r`"unsafe { x() }`";`nunsafe { y(); }`n"
  Assert-SelfTest (-not (Test-HasActiveForbidUnsafe $cr)) "raw string with embedded quote must ignore forbid" $ok
  Assert-SelfTest (-not (Test-HasActiveForbidUnsafe $hashes)) "r## raw string must ignore forbid" $ok
  Assert-SelfTest (Test-HasActiveForbidUnsafe $br) "active forbid after br# raw string must pass" $ok
  $unsafeCount = Get-UnsafeIssueCount 'raw_unsafe.rs' $unsafe
  Assert-SelfTest ($unsafeCount -eq 1) "unsafe in raw string ignored, real unsafe checked" $ok
}

function Invoke-GateASelfTests {
  $ok = $true
  $okRef = [ref]$ok
  Test-SelfTestForbidUnsafe $okRef
  Test-SelfTestSharedSafety $okRef
  Test-SelfTestUnsafeBlockComment $okRef
  Test-SelfTestUnsafeMultiline $okRef
  Test-SelfTestNestedBlockComments $okRef
  Test-SelfTestRawStrings $okRef
  if ($ok) {
    Write-Host "GATE-A self-tests passed." -ForegroundColor Green
    return
  }
  Write-Host "GATE-A self-tests failed." -ForegroundColor Red
  exit 1
}

function Invoke-GateAMain {
  if ($SelfTest) {
    Invoke-GateASelfTests
    return
  }
  Invoke-GateAChecks
}

Invoke-GateAMain