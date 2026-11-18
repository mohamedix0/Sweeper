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

function Try-BlankRustSimpleChar([hashtable]$state) {
  $i = $state.Index
  $len = $state.Len
  if (($i + 2) -ge $len) { return $false }
  $c1 = $state.Out[$i + 1]
  $isBad = (($c1 -eq '\') -or ($c1 -eq "'") -or ($c1 -eq "`r") -or ($c1 -eq "`n"))
  if ($isBad) { return $false }
  if ($state.Out[$i + 2] -ne "'") { return $false }
  $state.Out[$i] = ' '; $state.Out[$i + 1] = ' '; $state.Out[$i + 2] = ' '
  $state.Index = $i + 3
  return $true
}

function Find-RustEscapedCharEnd([hashtable]$state) {
  $i = $state.Index
  $max = [Math]::Min($state.Len - 1, $i + 12)
  $j = $i + 3
  while ($j -le $max) {
    $c = $state.Out[$j]
    if (($c -eq "`r") -or ($c -eq "`n")) { return -1 }
    if ($c -eq "'") { return $j }
    $j++
  }
  return -1
}

function Try-BlankRustEscapedChar([hashtable]$state) {
  $i = $state.Index
  $len = $state.Len
  if (($i + 2) -ge $len) { return $false }
  if ($state.Out[$i + 1] -ne '\') { return $false }
  $endIdx = Find-RustEscapedCharEnd $state
  if ($endIdx -lt 0) { return $false }
  for ($k = $i; $k -le $endIdx; $k++) { $state.Out[$k] = ' ' }
  $state.Index = $endIdx + 1
  return $true
}

function Try-SkipRustCharLiteral([hashtable]$state) {
  if ($state.Out[$state.Index] -ne "'") { return $false }
  if (Try-BlankRustSimpleChar $state) { return $true }
  return (Try-BlankRustEscapedChar $state)
}

function Step-RustNormal([hashtable]$state) {
  if (Try-StartRustRawString $state) { return }
  if (Try-SkipRustCharLiteral $state) { return }
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

function Test-IsMachineSpecificUser([string]$user) {
  if ([string]::IsNullOrWhiteSpace($user)) { return $false }
  # Length is not a validity signal: short real accounts can be disclosures too.
  # Generic/service accounts are excluded explicitly; everything else is treated as machine-specific.
  $genericUsers = @('runner', 'runneradmin', 'administrator', 'admin', 'user', 'test', 'guest', 'defaultuser', 'system', 'build')
  $genericUserPattern = '(?i)^(' + ($genericUsers -join '|') + ')$'
  return ($user -notmatch $genericUserPattern)
}

function Get-PrivacyRules() {
  $personalEmailPattern = '(?i)(@' + 'gmail\.com\b|@' + 'outlook\.com\b|@' + 'hotmail\.com\b|@' + 'yahoo\.com\b|@' + 'icloud\.com\b|@' + 'proton\.me\b|@' + 'protonmail\.com\b)'
  # Derive machine-specific fragments at runtime so this gate never stores a real username or path.
  $currentUser = [Environment]::UserName
  $userNameFrag = ''
  $userNameContextPattern = ''
  if (Test-IsMachineSpecificUser $currentUser) {
    $userNameFrag = [regex]::Escape($currentUser)
    # Account-context matching avoids false positives on ordinary text (e.g. a 'build' account
    # appearing in '[build-dependencies]') while still catching real username disclosures.
    $userAccountKeys = '(?:user|username|owner|account|login|author|committer)'
    $userNameContextPattern = '(?i)(?:\b' + $userAccountKeys + '\b["'']?\s*[:=]\s*["'']?' + $userNameFrag + '\b|C:\\Users\\' + $userNameFrag + '\b|\b' + $userNameFrag + '@)'
  }
  # Use the absolute repo path (never a bare project name) so fresh clones named after the project do not self-match.
  $projectFrag = $repoRoot
  # Separators accept one OR more backslashes so ordinary string literals with doubled, escaped
  # backslashes are covered as well as single-backslash text. The placeholder/generic-account
  # negative lookahead is unchanged, so documented placeholders and build accounts are excluded.
  $localUserPathPattern = '(?i)C:\\+Users\\+(?!Alice\b|Bob\b|<|name>|<user>|Public\b|Default\b|Default User\b|All Users\b|User\b|Test\b|me\b|A\b|B\b|X\b|Admin\b|Administrator\b|user\b|test\b|admin\b)[A-Za-z0-9._-]+'
  # Machine-neutral detection of absolute checkout/workspace paths from *other* checkouts.
  # No real drive letter or machine path is stored; the drive and intermediates are matched generically.
  # Common checkout-root names are covered, not just the current project's special folders.
  # The reserved Users root is covered by the placeholder-aware user-path rule above.
  $machineWorkspacePathPattern = '(?i)[A-Za-z]:\\(?!Users\\)(?:[^\\\r\n"''<>]+\\)*(?:\b(?:workspaces?|worktrees?|checkouts?|projects?|repos?)\b|\.t3)(?:\\[^\\\r\n"''<>]+)*'
  # Deny-by-default: flag ANY drive-letter absolute path unless its (drive, first segment) is an
  # explicitly allowlisted, machine-neutral root. The ENTIRE first segment is captured as ONE group,
  # from after the drive root up to the next path separator or the end of the text (an optional
  # trailing newline is allowed). The allowlist is then an exact, case-insensitive comparison of
  # that full captured segment: no prefix matching, no special-casing of spaces or parentheses, no
  # whitespace boundary rule. Consequently "Program FilesBackup", "Program Files Backup" and
  # "Program Files (x86)Backup" are each captured whole and rejected because none equals an
  # allowlisted root, which closes the whole boundary-variant class instead of enumerating variants.
  # Characters that cannot belong to a path root in tracked text (quotes, backticks, angle brackets,
  # braces, colon) terminate the capture so prose such as "C:\Windows, C:\Users" cannot swallow the
  # following sentence. End-of-text (not end-of-line) anchoring keeps prose at line ends from being
  # mistaken for a bare path segment.
  $firstSegmentPattern = '(?<seg>[^\\\r\n"''`<>{}:]+)(?=\\|(?:\r?\n)?$)'
  # A quoted bare path has no trailing path separator, so its closing quote must also satisfy the
  # first-segment boundary or a non-allowlisted root slips past the privacy scan. Accept a quote as
  # a boundary for ANY backslash-root form: a raw-string literal (single backslash) and an ordinary
  # string literal with doubled, escaped backslashes. Gating the quote on a single-backslash-only
  # capture left escaped strings undetected even though that is the normal form in TS/C#/JSON config
  # files. The match is still anchored to a real path root (one or more backslashes after the drive)
  # and the first segment must start with a path-root character, so leading-whitespace prose cannot
  # be mistaken for a bare path. The exact full-segment allowlist comparison is unchanged; no
  # leading-token suppression is used, so a one-word allowlisted prefix cannot hide a non-allowlisted
  # root. Escaped-string matches receive an additional string-literal guard in the hit helper below.
  $rootSeparatorPattern = '\\+'
  $quoteAwareSegmentPattern = '(?<seg>[A-Za-z0-9_.][^\\\r\n"''`<>{}:]*)(?=\\|["'']|(?:\r?\n)?$)'
  $absoluteDrivePathPattern = '(?i)\b(?<drive>[A-Za-z]):' + $rootSeparatorPattern + $quoteAwareSegmentPattern
  # A checkout-style absolute path buried under an otherwise-allowlisted fixture/placeholder root,
  # e.g. a source checkout's app folder beneath a first segment that the absolute-path rule permits.
  # First-segment allowlisting alone would let such a path pass because it carries no workspace
  # keyword. The marker must be a complete path segment after the first (app/src/repo/crate).
  $foreignCheckoutPathPattern = '(?i)\b(?<drive>[A-Za-z]):\\+' + $firstSegmentPattern + '(?:\\+[^\\\r\n"''<>]+)+\\+(?:app|src|repo|crate)(?![A-Za-z0-9_\-.])'
  # Machine-neutral, context-aware detection of internal session/thread identifiers. The token
  # shape is matched, never a literal identifier.
  $internalIdContext = '(?:(?:internal[\s_-]*)?(?:session|thread|childthread|t3|ses)[\s_-]*(?:id|identifier)?)'
  $internalIdToken = '(?:[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}|[0-9a-f]{8}-[0-9a-f]{16}|ses[0-9a-f]{16,})'
  $internalIdPattern = '(?i)(?:' + $internalIdContext + '[^\r\n]{0,60}?\b' + $internalIdToken + '\b|\b' + $internalIdToken + '\b[^\r\n]{0,60}?' + $internalIdContext + ')'
  return [ordered]@{
    PersonalEmailPattern = $personalEmailPattern
    UserNameFrag = $userNameFrag
    UserNameContextPattern = $userNameContextPattern
    ProjectFrag = $projectFrag
    LocalUserPathPattern = $localUserPathPattern
    MachineWorkspacePathPattern = $machineWorkspacePathPattern
    AbsoluteDrivePathPattern = $absoluteDrivePathPattern
    ForeignCheckoutPathPattern = $foreignCheckoutPathPattern
    AllowedAbsolutePathRoots = @(Get-AllowedAbsolutePathRoots)
    SystemAbsolutePathRoots = @(Get-SystemAbsolutePathRoots)
    InternalIdPattern = $internalIdPattern
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

function Get-PrivacyHitsForUserName([string]$rel, [string]$text, [string]$userNameContextPattern) {
  $hits = @()
  if ($userNameContextPattern -and $text -match $userNameContextPattern) {
    $hits += "${rel}: forbidden local username disclosure detected in account context"
  }
  return @($hits)
}

function Get-SystemAbsolutePathRoots() {
  # Legitimate Windows/system roots. These stay allowlisted at any depth so real system paths
  # (e.g. deeply nested Windows or Program Files subtrees) are never over-blocked.
  return @(
    'c:\windows', 'c:\windows.', 'c:\windows.old', 'c:\windowsapps',
    'c:\program files', 'c:\program files (x86)', 'c:\programdata',
    'c:\recovery', 'c:\system volume information'
  )
}

function Get-AllowedAbsolutePathRoots() {
  # Machine-neutral allowlist of (drive, first segment) roots that legitimately occur in tracked
  # text: Windows/system roots, documented placeholders, and synthetic test fixtures shipped in the
  # repo. This is deliberately data, not logic: no real username, machine path, or internal ID.
  # Fixture/placeholder exceptions are bounded by the foreign-checkout rule: a first segment here
  # is not a blanket pass for an arbitrary source checkout buried underneath it.
  $system = @(Get-SystemAbsolutePathRoots)
  $placeholders = @(
    'c:\temp', 'c:\temp.', 'c:\temp2', 'c:\cache', 'c:\users', 'c:\sandbox',
    'c:\sweeper', 'c:\sweepernonexistentcustompathcancel', 'c:\sweepernonexistentrecyclebincancel',
    'd:\sandbox', 'd:\sweepertest', 'd:\oldwork'
  )
  $fixtures = @(
    # Exact full-segment entries for synthetic adversarial fixtures whose first segment is "." /
    # ".." or carries a trailing space (e.g. C:\Temp \x). Exact comparison only, never a prefix.
    'c:\.', 'c:\..', 'c:\temp ',
    'c:\0foo', 'c:\a', 'c:\bulk', 'c:\com', 'c:\con', 'c:\conx', 'c:\dev', 'c:\does',
    'c:\foo', 'c:\hiberfil.sys', 'c:\logs', 'c:\lpt', 'c:\lpt1.txt', 'c:\missing',
    'c:\mystery', 'c:\nonexistentbrowser', 'c:\nonexistentdepththree', 'c:\other',
    'c:\pagefile.sys', 'c:\path', 'c:\progra~1', 'c:\safea', 'c:\safeb', 'c:\someapp',
    'c:\swapfile.sys', 'c:\tools', 'c:\vms'
  )
  return @($system + $placeholders + $fixtures)
}

function Test-IsAllowedAbsolutePath([string]$key, [string[]]$allowedRoots) {
  return ($allowedRoots -contains $key.ToLowerInvariant())
}

function Get-PrivacyHitsForAbsolutePaths([string]$rel, [string]$text, [string]$pattern, [string[]]$allowedRoots) {
  $hits = @()
  foreach ($m in [regex]::Matches($text, $pattern)) {
    $key = ($m.Groups['drive'].Value + ':\' + $m.Groups['seg'].Value).ToLowerInvariant()
    if (Test-IsAllowedAbsolutePath $key $allowedRoots) { continue }
    # An escaped-string root (doubled backslashes) is a genuine path literal only when the drive
    # path STARTS the string literal (preceded by a quote) and the segment is ASCII. Mid-literal
    # mentions, non-ASCII segments (invalid-path/mojibake fixtures) and prose that names an
    # allowlisted root followed by a lowercase continuation (e.g. a sentence beginning with a
    # neutral root word) are not machine paths and must stay unflagged. The prose rule requires an
    # allowlisted leading token AND a lowercase next word, so a one-word allowlisted prefix followed
    # by a real path segment (e.g. a capitalized Backup-style folder) is still flagged. Single-
    # backslash text is direct path text and keeps the exact full-segment allowlist verdict with no
    # suppression at all.
    if ($m.Value.Length -ge 4 -and $m.Value.Substring(2, 2) -ceq '\\') {
      if ($m.Index -eq 0 -or ($text[$m.Index - 1] -ne [char]34 -and $text[$m.Index - 1] -ne [char]39)) { continue }
      $seg = $m.Groups['seg'].Value
      if ($seg -match '[^\x00-\x7F]') { continue }
      $proseLead = [regex]::Match($seg, '^(\S+)\s+(.)')
      if ($proseLead.Success) {
        $leadKey = ($m.Groups['drive'].Value + ':\' + $proseLead.Groups[1].Value).ToLowerInvariant()
        if ((Test-IsAllowedAbsolutePath $leadKey $allowedRoots) -and $proseLead.Groups[2].Value -cmatch '[a-z]') { continue }
      }
    }
    $hits += "${rel}: forbidden absolute drive path detected: $($m.Value)"
  }
  return @($hits)
}

function Get-PrivacyHitsForMachineWorkspacePaths([string]$rel, [string]$text, [string]$pattern, [string[]]$systemRoots = @()) {
  $hits = @()
  foreach ($m in [regex]::Matches($text, $pattern)) {
    # Mirror the absolute-path detectors: a path under a legitimate system root is not a
    # foreign workspace even if a later segment happens to be named like a checkout.
    $firstSegment = [regex]::Match($m.Value, '^[A-Za-z]:\\+[^\\/]+')
    if ($firstSegment.Success -and (Test-IsAllowedAbsolutePath $firstSegment.Value $systemRoots)) { continue }
    $hits += "${rel}: forbidden absolute workspace path detected: $($m.Value)"
  }
  return @($hits)
}

function Get-PrivacyHitsForForeignCheckoutPaths([string]$rel, [string]$text, $rules) {
  $hits = @()
  foreach ($m in [regex]::Matches($text, $rules.ForeignCheckoutPathPattern)) {
    $key = ($m.Groups['drive'].Value + ':\' + $m.Groups['seg'].Value).ToLowerInvariant()
    # Only relevant when the first segment is otherwise allowlisted but is not a system root:
    # a fixture/placeholder root must not silently hide a foreign source checkout.
    if (-not (Test-IsAllowedAbsolutePath $key $rules.AllowedAbsolutePathRoots)) { continue }
    if (Test-IsAllowedAbsolutePath $key $rules.SystemAbsolutePathRoots) { continue }
    $hits += "${rel}: forbidden foreign checkout path under allowlisted root: $($m.Value)"
  }
  return @($hits)
}

function Get-PrivacyHitsForInternalIds([string]$rel, [string]$text, [string]$pattern) {
  $hits = @()
  # Concatenated string-literal fragments (e.g. 'aaa' + '-bbb-...') are a common way to hide
  # an identifier, so join them before applying the context-aware token pattern.
  $normalized = [regex]::Replace($text, "'\s*\+\s*'", '')
  $normalized = [regex]::Replace($normalized, '"\s*\+\s*"', '')
  $normalized = $normalized -replace "['`"]", ''
  foreach ($m in [regex]::Matches($normalized, $pattern)) {
    $hits += "${rel}: internal session/thread identifier detected"
  }
  return @($hits)
}

function Get-PrivacyHitsForSimplePatterns([string]$rel, [string]$text, $rules) {
  $hits = @()
  if ($rules.ProjectFrag -and $text -match [regex]::Escape($rules.ProjectFrag)) { $hits += "${rel}: forbidden local project path fragment detected" }
  return @($hits)
}

function Get-PrivacyHitsForFile([string]$rel, [string]$text, $rules) {
  $hits = @()
  $hits += Get-PrivacyHitsForEmails $rel $text $rules.PersonalEmailPattern
  $hits += Get-PrivacyHitsForSimplePatterns $rel $text $rules
  $hits += Get-PrivacyHitsForUserName $rel $text $rules.UserNameContextPattern
  $hits += Get-PrivacyHitsForLocalPaths $rel $text $rules.LocalUserPathPattern
  $hits += Get-PrivacyHitsForMachineWorkspacePaths $rel $text $rules.MachineWorkspacePathPattern $rules.SystemAbsolutePathRoots
  $hits += Get-PrivacyHitsForAbsolutePaths $rel $text $rules.AbsoluteDrivePathPattern $rules.AllowedAbsolutePathRoots
  $hits += Get-PrivacyHitsForForeignCheckoutPaths $rel $text $rules
  $hits += Get-PrivacyHitsForInternalIds $rel $text $rules.InternalIdPattern
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

function Test-SelfTestCharLiterals([ref]$ok) {
  $quoteFixture = "const QUOTE: char = '`"';`nconst TEXT: &str = `"`n#![forbid(unsafe_code)]`n`";`n"
  Assert-SelfTest (-not (Test-HasActiveForbidUnsafe $quoteFixture)) "char literal quote before string must ignore forbid in string" $ok

  $esc = "const ESC: char = '\'';`nconst T: &str = `"`n#![forbid(unsafe_code)]`n`";`n"
  $byteQuote = "const B: u8 = b'`"';`nconst T: &str = `"`n#![forbid(unsafe_code)]`n`";`n"
  Assert-SelfTest (-not (Test-HasActiveForbidUnsafe $esc)) "escaped char literal must not open string" $ok
  Assert-SelfTest (-not (Test-HasActiveForbidUnsafe $byteQuote)) "byte char literal quote must not open string" $ok

  $active = "#![forbid(unsafe_code)]`nfn f<'a>(_x: &'a str) {}`nconst QUOTE: char = '`"';`n"
  Assert-SelfTest (Test-HasActiveForbidUnsafe $active) "active forbid with lifetimes and char literal must pass" $ok

  $unsafeFixture = "const Q: char = '`"';`nfn main() {`n  unsafe { GetLastError() }`n}`n"
  $unsafeCount = Get-UnsafeIssueCount 'char_unsafe.rs' $unsafeFixture
  Assert-SelfTest ($unsafeCount -eq 1) "real unsafe after char literal quote must still be reported" $ok
}

function Test-SelfTestPrivacyRules([ref]$ok) {
  $rules = Get-PrivacyRules

  # Short real accounts must not be exempted by length, and must be caught in account context.
  Assert-SelfTest (Test-IsMachineSpecificUser 'eve') "short account must not be exempted by length" $ok
  $shortFrag = [regex]::Escape('eve')
  $shortPattern = '(?i)(?:\b(?:user|username|owner|account|login|author|committer)\b["'']?\s*[:=]\s*["'']?' + $shortFrag + '\b)'
  $shortHits = @(Get-PrivacyHitsForUserName 'fixture.txt' 'owner=eve' $shortPattern)
  Assert-SelfTest ($shortHits.Count -eq 1) "short username in owner context must produce a privacy hit" $ok

  # Generic service accounts must be excluded, and a bare section header must not match.
  Assert-SelfTest (-not (Test-IsMachineSpecificUser 'build')) "generic build account must be excluded" $ok
  $buildFrag = [regex]::Escape('build')
  $buildPattern = '(?i)(?:\b(?:user|username|owner|account|login|author|committer)\b["'']?\s*[:=]\s*["'']?' + $buildFrag + '\b|C:\\Users\\' + $buildFrag + '\b|\b' + $buildFrag + '@)'
  $depHits = @(Get-PrivacyHitsForUserName 'Cargo.toml' '[build-dependencies]' $buildPattern)
  Assert-SelfTest ($depHits.Count -eq 0) "generic account in section header must not match" $ok
  $buildHits = @(Get-PrivacyHitsForUserName 'fixture.txt' 'owner=build' $buildPattern)
  Assert-SelfTest ($buildHits.Count -eq 1) "account-context username must still be detected" $ok

  # Absolute workspace paths from another checkout must be detected; system paths must not.
  $drive = 'D' + ':' + '\'
  $otherCheckout = $drive + 'workspace\Sweeper\src\main.rs'
  $pathHits = @(Get-PrivacyHitsForMachineWorkspacePaths 'fixture.txt' $otherCheckout $rules.MachineWorkspacePathPattern)
  Assert-SelfTest ($pathHits.Count -ge 1) "other-checkout workspace path must be detected" $ok
  $systemPath = 'C' + ':' + '\Windows\System32'
  $sysHits = @(Get-PrivacyHitsForMachineWorkspacePaths 'fixture.txt' $systemPath $rules.MachineWorkspacePathPattern)
  Assert-SelfTest ($sysHits.Count -eq 0) "ordinary system path must not match" $ok

  # Regression for PR #48 thread 4606419932: checkout roots outside the original marker list.
  $sep = [System.IO.Path]::DirectorySeparatorChar
  $foreignDrive = 'D' + ':' + $sep
  $missedExample = $foreignDrive + 'projects' + $sep + 'private-checkout' + $sep + 'app'
  $missedHits = @(Get-PrivacyHitsForMachineWorkspacePaths 'fixture.txt' $missedExample $rules.MachineWorkspacePathPattern)
  Assert-SelfTest ($missedHits.Count -ge 1) "foreign projects/private-checkout path must be detected" $ok
  $foreignCheckout = $foreignDrive + 'agent' + $sep + 'checkout'
  $checkoutHits = @(Get-PrivacyHitsForMachineWorkspacePaths 'fixture.txt' $foreignCheckout $rules.MachineWorkspacePathPattern)
  Assert-SelfTest ($checkoutHits.Count -ge 1) "foreign agent/checkout path must be detected" $ok
  $placeholder = 'C' + ':' + $sep + 'Users' + $sep + '<real-user>' + $sep + 'projects'
  $placeholderHits = @(Get-PrivacyHitsForMachineWorkspacePaths 'fixture.txt' $placeholder $rules.MachineWorkspacePathPattern)
  Assert-SelfTest ($placeholderHits.Count -eq 0) "machine-neutral user placeholder must not match" $ok

  # Regression for PR #48 thread 4607041898: the workspace detector must honor the same
  # system-root exemptions as the absolute-path detectors.
  $sysWorkspace = 'C' + ':' + $sep + 'Windows' + $sep + 'Temp' + $sep + 'projects' + $sep + 'tool.exe'
  $sysWorkspaceHits = @(Get-PrivacyHitsForMachineWorkspacePaths 'fixture.txt' $sysWorkspace $rules.MachineWorkspacePathPattern $rules.SystemAbsolutePathRoots)
  Assert-SelfTest ($sysWorkspaceHits.Count -eq 0) "system-root workspace path must not be flagged" $ok
  $sysWorkspaceFileHits = @(Get-PrivacyHitsForFile 'fixture.txt' $sysWorkspace $rules)
  Assert-SelfTest ($sysWorkspaceFileHits.Count -eq 0) "system-root workspace path must pass full privacy scan" $ok
  $nonSysWorkspace = $drive + 'projects' + $sep + 'private-checkout' + $sep + 'app'
  $nonSysHits = @(Get-PrivacyHitsForMachineWorkspacePaths 'fixture.txt' $nonSysWorkspace $rules.MachineWorkspacePathPattern $rules.SystemAbsolutePathRoots)
  Assert-SelfTest ($nonSysHits.Count -ge 1) "non-system workspace path must still be flagged" $ok

  # Internal session/thread identifiers must be detected in context, including concatenated literals.
  $uuid = '0a1b2c3d' + '-4e5f-4a6b-8c9d-0e1f2a3b4c5d'
  $tagged = 'threadId=' + $uuid
  $idHits = @(Get-PrivacyHitsForInternalIds 'fixture.txt' $tagged $rules.InternalIdPattern)
  Assert-SelfTest ($idHits.Count -ge 1) "internal thread id in context must be detected" $ok
  $quote = [char]39
  $plus = ' + '
  $fragA = '0a1b2c3d'
  $fragB = '-4e5f-4a6b-8c9d-0e1f2a3b4c5d'
  $concatSource = 'threadId = ' + $quote + $fragA + $quote + $plus + $quote + $fragB + $quote
  $concatHits = @(Get-PrivacyHitsForInternalIds 'fixture.txt' $concatSource $rules.InternalIdPattern)
  Assert-SelfTest ($concatHits.Count -ge 1) "concatenated internal id literal must be detected" $ok
  $dquote = [char]34
  $concatDoubleSource = 'threadId = ' + $dquote + $fragA + $dquote + $plus + $dquote + $fragB + $dquote
  $concatDoubleHits = @(Get-PrivacyHitsForInternalIds 'fixture.txt' $concatDoubleSource $rules.InternalIdPattern)
  Assert-SelfTest ($concatDoubleHits.Count -ge 1) "double-quoted concatenated internal id literal must be detected" $ok
  $guidOnly = '{' + '12345678-1234-1234-1234-123456789abc' + '}.TM.blf'
  $guidHits = @(Get-PrivacyHitsForInternalIds 'fixture.txt' $guidOnly $rules.InternalIdPattern)
  Assert-SelfTest ($guidHits.Count -eq 0) "non-internal GUID must not match" $ok
}

function Test-SelfTestAbsoluteDrivePaths([ref]$ok) {
  $rules = Get-PrivacyRules
  $pattern = $rules.AbsoluteDrivePathPattern
  $allowed = $rules.AllowedAbsolutePathRoots
  $sep = [System.IO.Path]::DirectorySeparatorChar

  # CodeRabbit re-check thread 4606419932: both cases must now be flagged. Paths are assembled
  # from neutral fragments so no real machine path is ever stored in this gate.
  $caseOne = 'D' + ':' + $sep + 'Users' + $sep + 'fixture-person' + $sep + 'projects' + $sep + 'private-checkout' + $sep + 'app'
  $caseOneHits = @(Get-PrivacyHitsForAbsolutePaths 'fixture.txt' $caseOne $pattern $allowed)
  Assert-SelfTest ($caseOneHits.Count -ge 1) "foreign-drive Users checkout path must be detected" $ok

  $caseTwo = 'D' + ':' + $sep + 'src' + $sep + 'private-app' + $sep + 'app'
  $caseTwoHits = @(Get-PrivacyHitsForAbsolutePaths 'fixture.txt' $caseTwo $pattern $allowed)
  Assert-SelfTest ($caseTwoHits.Count -ge 1) "foreign-drive unmarked checkout path must be detected" $ok

  # Prove the rule is wired into the file-level privacy scan, not just callable in isolation.
  $wiredOne = @(Get-PrivacyHitsForFile 'fixture.txt' $caseOne $rules | Where-Object { $_ -match 'absolute drive path' })
  $wiredTwo = @(Get-PrivacyHitsForFile 'fixture.txt' $caseTwo $rules | Where-Object { $_ -match 'absolute drive path' })
  Assert-SelfTest ($wiredOne.Count -ge 1) "case one flagged through file-level privacy scan" $ok
  Assert-SelfTest ($wiredTwo.Count -ge 1) "case two flagged through file-level privacy scan" $ok

  # Regression for PR #48 thread 4607041892: an allowlisted multi-word system root must not be
  # captured as a mere prefix of a longer first segment (e.g. "Program FilesBackup").
  $prefixBypass = 'C' + ':' + $sep + 'Program FilesBackup' + $sep + 'private-data' + $sep + 'notes.txt'
  $prefixHits = @(Get-PrivacyHitsForAbsolutePaths 'fixture.txt' $prefixBypass $pattern $allowed)
  Assert-SelfTest ($prefixHits.Count -ge 1) "long segment starting with an allowlisted root must be detected" $ok
  $prefixWired = @(Get-PrivacyHitsForFile 'fixture.txt' $prefixBypass $rules | Where-Object { $_ -match 'absolute drive path' })
  Assert-SelfTest ($prefixWired.Count -ge 1) "long segment must be flagged through file-level privacy scan" $ok

  # CodeRabbit reply 4607145526 (thread 4607041892): a longer first segment that begins with an
  # allowlisted root but contains a space ("Program Files Backup") must be captured whole. The
  # boundary is a real path separator, never whitespace, so the root prefix cannot hide it.
  $spaceSegment = 'C' + ':' + $sep + 'Program Files Backup' + $sep + 'private-data' + $sep + 'notes.txt'
  $spaceSegmentHits = @(Get-PrivacyHitsForAbsolutePaths 'fixture.txt' $spaceSegment $pattern $allowed)
  Assert-SelfTest ($spaceSegmentHits.Count -ge 1) "space-containing longer first segment must match absolute-drive pattern" $ok
  $foreignSpaceSegment = 'C' + ':' + $sep + 'Program Files Backup' + $sep + 'private-app' + $sep + 'app'
  $foreignSegmentMatches = @([regex]::Matches($foreignSpaceSegment, $rules.ForeignCheckoutPathPattern))
  Assert-SelfTest ($foreignSegmentMatches.Count -ge 1) "space-containing longer first segment must match foreign-checkout pattern" $ok
  $spaceSegmentWired = @(Get-PrivacyHitsForFile 'fixture.txt' $spaceSegment $rules | Where-Object { $_ -match 'absolute drive path' })
  Assert-SelfTest ($spaceSegmentWired.Count -ge 1) "space-containing longer first segment must be flagged through file-level privacy scan" $ok

  # Legitimate system paths and machine-neutral placeholders must never be flagged.
  $systemPaths = @(
    ('C' + ':' + $sep + 'Windows' + $sep + 'System32'),
    ('C' + ':' + $sep + 'Windows' + $sep + 'Temp'),
    ('C' + ':' + $sep + 'Program Files' + $sep + 'NSIS')
  )
  foreach ($p in $systemPaths) {
    $hits = @(Get-PrivacyHitsForAbsolutePaths 'fixture.txt' $p $pattern $allowed)
    Assert-SelfTest ($hits.Count -eq 0) "system path must not be flagged: $p" $ok
  }
  $placeholders = @(
    ('D' + ':' + $sep + 'OldWork' + $sep + 'OldRepo'),
    ('C' + ':' + $sep + 'Users' + $sep + '<real-user>' + $sep + 'projects'),
    ('C' + ':' + $sep + 'Users' + $sep + 'Alice'),
    ('C' + ':' + $sep + 'Temp'),
    ('C' + ':' + $sep + 'Cache')
  )
  foreach ($p in $placeholders) {
    $hits = @(Get-PrivacyHitsForAbsolutePaths 'fixture.txt' $p $pattern $allowed)
    Assert-SelfTest ($hits.Count -eq 0) "placeholder must not be flagged: $p" $ok
  }

  # Regression for PR #48 thread 4607041885: the transition guide writes forbidden-path
  # examples as angle-bracket placeholders so its own Step 1 search cannot flag the guide.
  $fictionalExample = '<drive>:\<old-root>\<old-repo>\node_modules'
  $fictionalHits = @(Get-PrivacyHitsForFile 'TRANSITION.md' $fictionalExample $rules)
  Assert-SelfTest ($fictionalHits.Count -eq 0) "fictional instructional example must not be flagged" $ok
}

function Test-SelfTestFullFirstSegmentAllowlist([ref]$ok) {
  $rules = Get-PrivacyRules
  $pattern = $rules.AbsoluteDrivePathPattern
  $allowed = $rules.AllowedAbsolutePathRoots
  $sep = [System.IO.Path]::DirectorySeparatorChar

  # Root-cause regression: every first segment is captured WHOLE, then compared exactly against the
  # allowlist. Assert both the captured text and the resulting verdict for every cited variant.
  $variants = @(
    @{ Name = 'Program FilesBackup'; Text = ('C' + ':' + $sep + 'Program FilesBackup' + $sep + 'private-data' + $sep + 'notes.txt'); Segment = 'Program FilesBackup' }
    @{ Name = 'Program Files Backup with trailing separator'; Text = ('C' + ':' + $sep + 'Program Files Backup' + $sep + 'private-data' + $sep + 'notes.txt'); Segment = 'Program Files Backup' }
    @{ Name = 'bare Program Files Backup at end of text'; Text = ('C' + ':' + $sep + 'Program Files Backup'); Segment = 'Program Files Backup' }
    @{ Name = 'Program Files (x86)Backup'; Text = ('C' + ':' + $sep + 'Program Files (x86)Backup' + $sep + 'private-data' + $sep + 'notes.txt'); Segment = 'Program Files (x86)Backup' }
  )
  foreach ($v in $variants) {
    $matches = @([regex]::Matches($v.Text, $pattern))
    Assert-SelfTest ($matches.Count -ge 1) "full-segment pattern matches: $($v.Name)" $ok
    if ($matches.Count -ge 1) {
      Assert-SelfTest ($matches[0].Groups['seg'].Value -ceq $v.Segment) "entire first segment captured for: $($v.Name)" $ok
    }
    $directHits = @(Get-PrivacyHitsForAbsolutePaths 'fixture.txt' $v.Text $pattern $allowed)
    Assert-SelfTest ($directHits.Count -ge 1) "exact allowlist rejects: $($v.Name)" $ok
    $wiredHits = @(Get-PrivacyHitsForFile 'fixture.txt' $v.Text $rules | Where-Object { $_ -match 'forbidden absolute drive path' })
    Assert-SelfTest ($wiredHits.Count -ge 1) "file-level privacy scan flags: $($v.Name)" $ok
  }

  # A bare segment at the very end of a CRLF file is still an end-of-text occurrence.
  $crlfText = 'next line' + "`r`n" + ('C' + ':' + $sep + 'Program Files Backup') + "`r`n"
  $crlfMatches = @([regex]::Matches($crlfText, $pattern))
  Assert-SelfTest ($crlfMatches.Count -ge 1) "full-segment pattern matches bare segment at CRLF end of text" $ok
  if ($crlfMatches.Count -ge 1) {
    Assert-SelfTest ($crlfMatches[0].Groups['seg'].Value -ceq 'Program Files Backup') "entire first segment captured at CRLF end of text" $ok
  }

  # Legitimate system roots must still match the pattern but pass the exact allowlist at file level.
  $controls = @(
    ('C' + ':' + $sep + 'Program Files' + $sep + 'NSIS'),
    ('C' + ':' + $sep + 'Program Files (x86)' + $sep + 'Internet Explorer' + $sep + 'iexplore.exe'),
    ('C' + ':' + $sep + 'Windows' + $sep + 'System32')
  )
  foreach ($p in $controls) {
    $matches = @([regex]::Matches($p, $pattern))
    Assert-SelfTest ($matches.Count -ge 1) "full-segment pattern matches legitimate control: $p" $ok
    $directHits = @(Get-PrivacyHitsForAbsolutePaths 'fixture.txt' $p $pattern $allowed)
    Assert-SelfTest ($directHits.Count -eq 0) "exact allowlist accepts legitimate control: $p" $ok
    $wiredHits = @(Get-PrivacyHitsForFile 'fixture.txt' $p $rules | Where-Object { $_ -match 'forbidden absolute drive path' })
    Assert-SelfTest ($wiredHits.Count -eq 0) "file-level privacy scan accepts legitimate control: $p" $ok
  }
}

function Test-SelfTestQuotedBareAbsolutePaths([ref]$ok) {
  $rules = Get-PrivacyRules
  $pattern = $rules.AbsoluteDrivePathPattern
  $allowed = $rules.AllowedAbsolutePathRoots
  $sep = [System.IO.Path]::DirectorySeparatorChar
  $quote = [char]34

  # Regression for PR #48 thread 4607264274: a quoted bare absolute path has no trailing
  # separator, so the closing quote must also satisfy the first-segment boundary. Otherwise
  # the non-allowlisted root is captured but the match is discarded and slips past the scan.
  $quotedBarePaths = @(
    ('C' + ':' + $sep + 'Program Files Backup'),
    ('C' + ':' + $sep + 'Private Folder')
  )
  foreach ($bare in $quotedBarePaths) {
    $quoted = 'r' + $quote + $bare + $quote
    $matches = @([regex]::Matches($quoted, $pattern))
    Assert-SelfTest ($matches.Count -ge 1) "quoted bare path matches absolute-drive pattern: $bare" $ok
    if ($matches.Count -ge 1) {
      Assert-SelfTest ($matches[0].Groups['seg'].Value -ceq $bare.Substring(3)) "entire first segment captured for quoted bare path: $bare" $ok
    }
    $directHits = @(Get-PrivacyHitsForAbsolutePaths 'fixture.txt' $quoted $pattern $allowed)
    Assert-SelfTest ($directHits.Count -ge 1) "quoted bare path flagged by direct detector: $bare" $ok
    $wiredHits = @(Get-PrivacyHitsForFile 'fixture.txt' $quoted $rules | Where-Object { $_ -match 'forbidden absolute drive path' })
    Assert-SelfTest ($wiredHits.Count -ge 1) "quoted bare path flagged through file-level privacy scan: $bare" $ok
  }

  # Legitimate quoted system paths must still pass the exact allowlist (no over-blocking).
  $quotedSystemPaths = @(
    ('C' + ':' + $sep + 'Windows' + $sep + 'System32'),
    ('C' + ':' + $sep + 'Program Files' + $sep + 'NSIS')
  )
  foreach ($p in $quotedSystemPaths) {
    $quoted = 'r' + $quote + $p + $quote
    $hits = @(Get-PrivacyHitsForFile 'fixture.txt' $quoted $rules | Where-Object { $_ -match 'forbidden absolute drive path' })
    Assert-SelfTest ($hits.Count -eq 0) "quoted legitimate system path must not be flagged: $p" $ok
  }
}

function Test-SelfTestOneWordAllowlistedPrefix([ref]$ok) {
  $rules = Get-PrivacyRules
  $pattern = $rules.AbsoluteDrivePathPattern
  $allowed = $rules.AllowedAbsolutePathRoots
  $sep = [System.IO.Path]::DirectorySeparatorChar
  $quote = [char]34

  # Regression for CodeRabbit thread 4607401610: a non-allowlisted path whose first segment merely
  # BEGINS with a one-word allowlisted root (e.g. "Windows Backup" starts with the allowlisted
  # "Windows", "Temp confidential" with "Temp") must still be FLAGGED. The whole first segment is
  # captured and compared exactly, so the one-word prefix must never grant a pass. All fixtures are
  # synthetic and assembled from neutral fragments; no real machine path is stored in this gate.
  $oneWordPrefixCases = @(
    @{ Name = 'bare plain'; Text = ('C' + ':' + $sep + 'Windows Backup'); Quoted = $false }
    @{ Name = 'bare quoted raw string'; Text = ('C' + ':' + $sep + 'Windows Backup'); Quoted = $true }
    @{ Name = 'nested'; Text = ('C' + ':' + $sep + 'Windows Backup' + $sep + 'private-data' + $sep + 'notes.txt'); Quoted = $false }
    @{ Name = 'neutral example nested'; Text = ('C' + ':' + $sep + 'Temp confidential' + $sep + 'notes.txt'); Quoted = $false }
  )
  foreach ($c in $oneWordPrefixCases) {
    $fixture = if ($c.Quoted) { 'r' + $quote + $c.Text + $quote } else { $c.Text }
    $directHits = @(Get-PrivacyHitsForAbsolutePaths 'fixture.txt' $fixture $pattern $allowed)
    Assert-SelfTest ($directHits.Count -ge 1) "one-word prefix case flagged by direct detector: $($c.Name)" $ok
    $wiredHits = @(Get-PrivacyHitsForFile 'fixture.txt' $fixture $rules | Where-Object { $_ -match 'forbidden absolute drive path' })
    Assert-SelfTest ($wiredHits.Count -ge 1) "one-word prefix case flagged through file-level privacy scan: $($c.Name)" $ok
  }

  # Legitimate system paths whose entire first segment IS allowlisted must remain unflagged.
  $controls = @(
    ('C' + ':' + $sep + 'Windows' + $sep + 'System32'),
    ('C' + ':' + $sep + 'Program Files' + $sep + 'NSIS')
  )
  foreach ($p in $controls) {
    $directHits = @(Get-PrivacyHitsForAbsolutePaths 'fixture.txt' $p $pattern $allowed)
    Assert-SelfTest ($directHits.Count -eq 0) "one-word prefix control accepted by direct detector: $p" $ok
    $wiredHits = @(Get-PrivacyHitsForFile 'fixture.txt' $p $rules | Where-Object { $_ -match 'forbidden absolute drive path' })
    Assert-SelfTest ($wiredHits.Count -eq 0) "one-word prefix control accepted by file-level privacy scan: $p" $ok
  }
}

function Test-SelfTestEscapedStringAbsolutePaths([ref]$ok) {
  $rules = Get-PrivacyRules
  $pattern = $rules.AbsoluteDrivePathPattern
  $allowed = $rules.AllowedAbsolutePathRoots
  $sep = [System.IO.Path]::DirectorySeparatorChar
  $bs = '\'
  $quote = [char]34

  # Independent-review hard defect: an ordinary string literal uses doubled, escaped backslashes.
  # The old single-backslash-gated quote boundary let an escaped-string path with a non-allowlisted
  # root evade BOTH the absolute-path detector and the file-level scan, contradicting the
  # deny-by-default rule. All fixtures are synthetic and assembled from neutral fragments; no real
  # machine path is stored.
  $escapedBackup = 'let p = ' + $quote + 'C:' + $sep + $sep + 'Program Files Backup' + $quote + ';'
  $directHits = @(Get-PrivacyHitsForAbsolutePaths 'fixture.txt' $escapedBackup $pattern $allowed)
  Assert-SelfTest ($directHits.Count -ge 1) "escaped-string absolute path flagged by direct detector" $ok
  $wiredHits = @(Get-PrivacyHitsForFile 'fixture.txt' $escapedBackup $rules | Where-Object { $_ -match 'forbidden absolute drive path' })
  Assert-SelfTest ($wiredHits.Count -ge 1) "escaped-string absolute path flagged through file-level privacy scan" $ok

  # The reserved Users root is allowlisted by the absolute-path rule, so an escaped-string user
  # path must be caught by the local-user-path detector instead. The previous single-backslash-only
  # pattern missed the doubled-backslash form entirely.
  $escapedUserPath = $quote + 'C:' + $sep + $sep + 'Users' + $sep + $sep + 'fixture-person' + $sep + $sep + 'projects' + $quote
  $localDirect = @(Get-PrivacyHitsForLocalPaths 'fixture.txt' $escapedUserPath $rules.LocalUserPathPattern)
  Assert-SelfTest ($localDirect.Count -ge 1) "escaped-string user path flagged by direct local-user detector" $ok
  $localWired = @(Get-PrivacyHitsForFile 'fixture.txt' $escapedUserPath $rules | Where-Object { $_ -match 'forbidden local user path' })
  Assert-SelfTest ($localWired.Count -ge 1) "escaped-string user path flagged through file-level privacy scan" $ok

  # Legitimate system controls stay unflagged in both plain and escaped-string forms.
  $controls = @(
    ('C:' + $sep + 'Windows' + $sep + 'System32'),
    ('C:' + $sep + 'Program Files' + $sep + 'NSIS')
  )
  foreach ($p in $controls) {
    foreach ($text in @($p, ($quote + ($p -replace [regex]::Escape($sep), ($bs + $bs)) + $quote))) {
      $allHits = @(Get-PrivacyHitsForFile 'fixture.txt' $text $rules)
      Assert-SelfTest ($allHits.Count -eq 0) "escaped/control system path must not be flagged: $text" $ok
    }
  }

  # Extended-path and neutral fixtures must remain unflagged (no over-broad delimiter or prefix).
  $extendedFixtures = @(
    ('C:' + $sep + 'Temp' + $sep + '??' + $sep + 'x'),
    ($bs + $bs + '?' + $bs + 'C:' + $bs + 'foo')
  )
  foreach ($p in $extendedFixtures) {
    $allHits = @(Get-PrivacyHitsForFile 'fixture.txt' $p $rules)
    Assert-SelfTest ($allHits.Count -eq 0) "extended-path fixture must not be flagged: $p" $ok
  }

  # Escaped-string prose and invalid-path fixtures must stay unflagged, while a one-word allowlisted
  # prefix followed by a real (capitalized) path segment is still flagged. Built from neutral
  # fragments so no real machine path is stored in this gate.
  $proseFixtures = @(
    ($quote + 'C:' + $bs + $bs + 'Users must be rejected by sandbox safety validation' + $quote)
    ('note: ' + $quote + 'detail; C:' + $bs + $bs + 'Windows remains Protected.' + $quote)
    ('error: C:' + $bs + $bs + '...' + ')')
    ($quote + 'C:' + $bs + $bs + 'Temp' + [char]0xFF0F + 'foo' + $quote)
  )
  foreach ($p in $proseFixtures) {
    $absHits = @(Get-PrivacyHitsForAbsolutePaths 'fixture.txt' $p $pattern $allowed)
    Assert-SelfTest ($absHits.Count -eq 0) "escaped-string prose/fixture must not be flagged: $p" $ok
  }
  $escapedWindowsBackup = $quote + 'C:' + $bs + $bs + 'Windows Backup' + $quote
  $prefixHits = @(Get-PrivacyHitsForAbsolutePaths 'fixture.txt' $escapedWindowsBackup $pattern $allowed)
  Assert-SelfTest ($prefixHits.Count -ge 1) "escaped one-word prefix path must still be flagged" $ok
}

function Test-SelfTestForeignCheckoutUnderAllowedRoot([ref]$ok) {
  $rules = Get-PrivacyRules
  $sep = [System.IO.Path]::DirectorySeparatorChar

  # File-level regression for PR #48 thread 4606741541: a checkout-style path beneath an
  # allowlisted fixture root must be flagged even though it carries no workspace keyword.
  # Built from neutral fragments so no real machine path is ever stored in this gate.
  $foreignUnderFixtureRoot = 'C' + ':' + $sep + 'Dev' + $sep + 'private-app' + $sep + 'app'
  $foreignHits = @(Get-PrivacyHitsForFile 'fixture.txt' $foreignUnderFixtureRoot $rules | Where-Object { $_ -match 'foreign checkout path under allowlisted root' })
  Assert-SelfTest ($foreignHits.Count -ge 1) "checkout under allowlisted fixture root must be flagged at file level" $ok

  # The same shape under a documented placeholder root must also be flagged.
  $foreignUnderPlaceholder = 'C' + ':' + $sep + 'Temp' + $sep + 'private-app' + $sep + 'app'
  $placeholderRootHits = @(Get-PrivacyHitsForFile 'fixture.txt' $foreignUnderPlaceholder $rules | Where-Object { $_ -match 'foreign checkout path under allowlisted root' })
  Assert-SelfTest ($placeholderRootHits.Count -ge 1) "checkout under allowlisted placeholder root must be flagged at file level" $ok

  # Legitimate fixtures and system paths must remain allowed (no over-blocking).
  $legit = @(
    ('C' + ':' + $sep + 'Dev'),
    ('C' + ':' + $sep + 'A' + $sep + 'B' + $sep + 'C'),
    ('C' + ':' + $sep + 'Windows' + $sep + 'System32'),
    ('C' + ':' + $sep + 'Program Files' + $sep + 'NSIS')
  )
  foreach ($p in $legit) {
    $hits = @(Get-PrivacyHitsForFile 'fixture.txt' $p $rules)
    Assert-SelfTest ($hits.Count -eq 0) "legitimate fixture/system path must not be flagged: $p" $ok
  }
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
  Test-SelfTestCharLiterals $okRef
  Test-SelfTestPrivacyRules $okRef
  Test-SelfTestAbsoluteDrivePaths $okRef
  Test-SelfTestFullFirstSegmentAllowlist $okRef
  Test-SelfTestQuotedBareAbsolutePaths $okRef
  Test-SelfTestOneWordAllowlistedPrefix $okRef
  Test-SelfTestEscapedStringAbsolutePaths $okRef
  Test-SelfTestForeignCheckoutUnderAllowedRoot $okRef
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