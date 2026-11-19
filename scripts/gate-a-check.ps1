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


# Cohesive helpers live in sibling modules so each file stays small and focused.
. (Join-Path $PSScriptRoot 'gate-a-rust-scan.ps1')
. (Join-Path $PSScriptRoot 'gate-a-selftests.ps1')

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

function Test-IsCIEnvironment([string]$ci, [string]$githubActions) {
  # GitHub Actions sets both CI and GITHUB_ACTIONS to "true". Accept either so a fork
  # or a local CI emulator that sets only one of them is still treated as CI.
  return (($ci -ieq 'true') -or ($githubActions -ieq 'true'))
}

function Test-OwnedBuildScript([string]$path) {
  if (-not (Test-NotTargetPath $path)) { return $false }
  # ADR 0003 permits unsafe in sweeper-winfs; every other owned crate must stay unsafe-free.
  return ($path -notmatch '([/\\])sweeper-winfs([/\\])')
}

function Get-OwnedBuildScriptPaths([string]$root) {
  # cargo-geiger does not analyse build-script roots (verified with the pinned 0.14.0: a crate
  # whose build.rs contains unsafe reports zero unsafe). Scan owned build scripts directly so
  # an unsafe block there cannot slip past the ADR 0003 confinement audit.
  $paths = @()
  foreach ($appRoot in @((Join-Path $root 'app\backend'), (Join-Path $root 'app\bridge'))) {
    $found = @(Get-ChildItem -Path $appRoot -Recurse -Filter 'build.rs' -File -ErrorAction SilentlyContinue |
      Where-Object { Test-OwnedBuildScript $_.FullName } |
      ForEach-Object { $_.FullName })
    $paths += $found
  }
  return @($paths | Sort-Object -Unique)
}

function Get-BuildScriptUnsafeIssues([string[]]$paths) {
  $issues = @()
  foreach ($path in $paths) {
    $text = Get-Content -LiteralPath $path -Raw -ErrorAction SilentlyContinue
    if ($null -eq $text) { continue }
    if (Test-RustHasUnsafeToken $text) { $issues += $path }
  }
  return @($issues)
}

function Invoke-BuildScriptUnsafeCheck([string]$root) {
  $paths = @(Get-OwnedBuildScriptPaths $root)
  foreach ($issue in @(Get-BuildScriptUnsafeIssues $paths)) {
    Fail "unsafe code found in build script $issue. cargo-geiger does not scan build-script roots; unsafe must be confined to sweeper-winfs (ADR 0003)."
  }
}

function Get-GeigerWorkspaceManifests([string]$root) {
  # cargo-geiger requires a concrete package manifest (a virtual workspace manifest is rejected).
  # Enumerate the workspace members directly from the crate folders so new crates are covered.
  $roots = @((Join-Path $root 'app\backend'), (Join-Path $root 'app\bridge'))
  $manifests = @(Get-ChildItem -Path $roots -Recurse -Filter 'Cargo.toml' -File -ErrorAction SilentlyContinue |
    Where-Object { Test-NotTargetPath $_.FullName } | ForEach-Object { $_.FullName })
  return @($manifests | Sort-Object -Unique)
}

function Invoke-GeigerForManifest([string]$manifest) {
  # Native stderr must not be promoted to a terminating error under $ErrorActionPreference='Stop'.
  # It is diagnostic noise from cargo; the report itself is on stdout.
  $previous = $ErrorActionPreference
  $ErrorActionPreference = 'Continue'
  try {
    $jsonText = (& cargo geiger --locked --output-format Json --manifest-path $manifest 2>$null | Out-String)
    $exitCode = $LASTEXITCODE
  } finally {
    $ErrorActionPreference = $previous
  }
  if ($exitCode -ne 0) {
    Fail "cargo geiger failed for $manifest (exit code $exitCode)."
    return $null
  }
  if ([string]::IsNullOrWhiteSpace($jsonText)) {
    Fail "cargo geiger produced no unsafe-audit output for $manifest."
    return $null
  }
  try { return ($jsonText | ConvertFrom-Json) } catch {
    Fail "cargo geiger output for $manifest could not be parsed as JSON."
    return $null
  }
}

function Get-GeigerMetricUnsafeValue($metric) {
  # Parse one metric; an absent or non-numeric unsafe_ contributes zero.
  if ($null -eq $metric) { return 0 }
  $unsafe = $metric.Value.PSObject.Properties['unsafe_']
  if ($null -eq $unsafe) { return 0 }
  $value = 0
  if ([int]::TryParse([string]$unsafe.Value, [ref]$value)) { return $value }
  return 0
}

function Get-GeigerSectionUnsafeCount($section) {
  # Sum one section (used or unused) across all unsafe metrics.
  if ($null -eq $section) { return 0 }
  $total = 0
  foreach ($metricName in @('functions', 'exprs', 'item_impls', 'item_traits', 'methods')) {
    $metric = $section.Value.PSObject.Properties[$metricName]
    $total += Get-GeigerMetricUnsafeValue $metric
  }
  return $total
}

function Get-GeigerEntryUnsafeCount($entry) {
  # Sum both used and unused unsafe counts for one crate's geiger entry.
  # Third-party unsafe is not owned by this repo; only sweeper-* entries reach this helper.
  $total = 0
  foreach ($sectionName in @('used', 'unused')) {
    $section = $entry.unsafety.PSObject.Properties[$sectionName]
    $total += Get-GeigerSectionUnsafeCount $section
  }
  return $total
}

function Test-IsGeigerOffender([string]$name, [int]$total) {
  # sweeper-winfs is the sole crate allowed to contain unsafe code (ADR 0003).
  if ($name -eq 'sweeper-winfs') { return $false }
  return ($total -gt 0)
}

function Format-GeigerAuditResult($summary, $offenders, [int]$auditedCount) {
  # An empty/malformed report must never silently pass the gate.
  $result = @{ Ok = $false; Offenders = @(); Summary = @() }
  if ($auditedCount -eq 0) { return $result }
  $result.Ok = $true
  $result.Offenders = @($offenders)
  $result.Summary = @($summary | Sort-Object)
  return $result
}

function Get-GeigerUnsafeAudit($entries) {
  # Counts unsafe in OUR workspace crates only (third-party unsafe is not owned by this repo).
  $summary = @()
  $offenders = @()
  $seen = @{}
  foreach ($entry in @($entries)) {
    $name = [string]$entry.package.id.name
    if (($name -notlike 'sweeper-*') -or $seen.ContainsKey($name)) { continue }
    $seen[$name] = $true
    $total = Get-GeigerEntryUnsafeCount $entry
    $summary += ("{0}={1}" -f $name, $total)
    if (Test-IsGeigerOffender $name $total) {
      $offenders += "$name (unsafe count $total)"
    }
  }
  return Format-GeigerAuditResult $summary $offenders $seen.Count
}

function Get-GeigerOmittedCrateName($entry) {
  # Real cargo-geiger reports packages_without_metrics as PackageId objects ({ name, ... }).
  # Accept the plain string form too so older/synthetic reports are handled conservatively.
  if ($null -eq $entry) { return '' }
  if ($entry -is [string]) { return [string]$entry }
  return [string]$entry.name
}

function Get-GeigerOmittedOwnedCrates($report) {
  # Only owned sweeper-* crates matter; a third-party omission is not this repo's unsafe.
  $names = @(@($report.packages_without_metrics) | ForEach-Object { Get-GeigerOmittedCrateName $_ })
  return @($names | Where-Object { $_ -like 'sweeper-*' })
}

function Invoke-CargoGeiger([string]$root) {
  Write-Host "GATE-A: running cargo geiger unsafe audit for workspace crates..."
  $manifests = @(Get-GeigerWorkspaceManifests $root)
  if ($manifests.Count -eq 0) {
    Fail "No workspace crate manifests found for the cargo geiger audit."
    return
  }
  $entries = @()
  foreach ($manifest in $manifests) {
    $report = Invoke-GeigerForManifest $manifest
    if ($null -eq $report) { return }
    $entries += @($report.packages)
    $omitted = @(Get-GeigerOmittedOwnedCrates $report)
    if ($omitted.Count -gt 0) {
      Fail "cargo geiger produced no unsafe metrics for owned workspace crate(s) in $manifest ($($omitted -join ', ')). Unsafe confinement could not be verified."
      return
    }
  }

  $audit = Get-GeigerUnsafeAudit $entries
  if (-not $audit.Ok) {
    Fail "Could not parse cargo geiger output or no sweeper workspace crates were audited; unsafe confinement could not be verified."
    return
  }
  Write-Host "GATE-A: cargo geiger unsafe-audit output (workspace crate unsafe counts):"
  foreach ($line in $audit.Summary) { Write-Host "  $line" }
  if ($audit.Offenders.Count -gt 0) {
    Fail "cargo geiger found unsafe code outside sweeper-winfs: $($audit.Offenders -join ', '). Unsafe must be confined to sweeper-winfs (ADR 0003)."
  }
}

function Invoke-RequiredCargoGeiger([string]$root) {
  $geiger = Get-Command cargo-geiger -ErrorAction SilentlyContinue
  if ($null -eq $geiger) {
    if (Test-IsCIEnvironment $env:CI $env:GITHUB_ACTIONS) {
      Fail "cargo-geiger is required in CI but was not found in PATH. Ensure the 'Install cargo-geiger' step in .github/workflows/ci.yml runs before Gate A."
    } else {
      Write-Host "GATE-A: cargo-geiger not found in PATH; skipping unsafe-count audit locally (CI enforces it)." -ForegroundColor Yellow
    }
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
  # The first character of a segment may be ANY legal Windows filename character, including
  # non-ASCII letters (e.g. "equipe" with an accent). Only whitespace and path/quote delimiters
  # are excluded, so leading-whitespace prose still cannot be mistaken for a bare path.
  $quoteAwareSegmentPattern = '(?<seg>[^\s\\\r\n"''`<>{}:][^\\\r\n"''`<>{}:]*)(?=\\|["'']|(?:\r?\n)?$)'
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
  # Tracked text files are UTF-8. Reading with the Windows PowerShell 5.1 ANSI default silently
  # mojibakes multi-byte characters (e.g. U+FF0F becomes three Latin-1 chars), which both hides
  # real machine paths and breaks the narrow malformed-path exemption. Decode as UTF-8 explicitly.
  try { return [System.IO.File]::ReadAllText($full, [System.Text.Encoding]::UTF8) } catch { return $null }
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


function Test-IsEscapedPathRoot([string]$text, $match) {
  if ($match.Value.Length -lt 4) { return $false }
  return ($match.Value.Substring(2, 2) -ceq '\\')
}

function Test-IsQuotedPathRoot([string]$text, $match) {
  if ($match.Index -eq 0) { return $true }
  $prev = $text[$match.Index - 1]
  return (($prev -eq [char]34) -or ($prev -eq [char]39))
}

function Test-IsSyntheticMalformedFixture([string]$key) {
  # The ONLY intentionally exempt malformed path in the repo's adversarial fixtures is the
  # synthetic Temp + U+FF0F + foo literal under an allowlisted root. U+FF0F is a legal Windows
  # filename character, so a real path using it must stay reportable. Match the exact fixture by
  # its full case-folded key; never exempt a slash lookalike by character class. Paths here are
  # assembled from fragments at runtime so this gate never stores an absolute machine path.
  return ($key -ieq ('c:\temp' + [char]0xFF0F + 'foo'))
}

function Test-HasNestedPathSeparator([string]$text, $match) {
  $end = $match.Index + $match.Value.Length
  if ($end -ge $text.Length) { return $false }
  return ($text[$end] -eq '\')
}

function Test-IsEscapedProseSegment([string]$seg, [string]$drive, [string[]]$allowedRoots) {
  # Escaped-string prose names an allowlisted root followed by an ordinary lower-case sentence
  # (e.g. "Users must be rejected by sandbox safety validation"). A genuine path segment such as
  # "Windows confidential files" uses a path-segment noun instead. Require the continuation to
  # OPEN with an action/auxiliary verb - the subject-plus-verb shape of a sentence - so multi-word
  # path names are no longer mistaken for prose. Copular "to be" forms are excluded because they
  # double as ordinary path-segment words (e.g. a "Windows is private" continuation is a path).
  # System-root controls are allowlisted before this guard.
  $lead = [regex]::Match($seg, '^(\S+)\s+(.+)')
  if (-not $lead.Success) { return $false }
  $leadKey = ($drive + ':\' + $lead.Groups[1].Value).ToLowerInvariant()
  if (-not (Test-IsAllowedAbsolutePath $leadKey $allowedRoots)) { return $false }
  $continuation = $lead.Groups[2].Value
  if ($continuation -cnotmatch '[a-z]') { return $false }
  $firstWord = ([regex]::Match($continuation, '^[A-Za-z]+')).Value.ToLowerInvariant()
  $proseVerbs = @('must','remains','can','could','will','would','shall','should','may','might',
                  'has','have','had','does','do','did','needs','need','seems','appears',
                  'becomes','continues','stays')
  return ($proseVerbs -contains $firstWord)
}

function Test-ShouldSkipAbsolutePathMatch([string]$text, $match, [string[]]$allowedRoots) {
  $seg = $match.Groups['seg'].Value
  $drive = $match.Groups['drive'].Value
  $key = ($drive + ':\' + $seg).ToLowerInvariant()
  if (Test-IsAllowedAbsolutePath $key $allowedRoots) { return $true }
  if (-not (Test-IsEscapedPathRoot $text $match)) { return $false }
  if (-not (Test-IsQuotedPathRoot $text $match)) { return $true }
  # A following path separator makes this a real descendant path, so it must be reported even
  # when its first segment is the synthetic malformed fixture. Check nesting before the fixture
  # exemption so only the bare fixture stays exempt.
  if (Test-HasNestedPathSeparator $text $match) { return $false }
  if (Test-IsSyntheticMalformedFixture $key) { return $true }
  return (Test-IsEscapedProseSegment $seg $drive $allowedRoots)
}

function Get-PrivacyHitsForAbsolutePaths([string]$rel, [string]$text, [string]$pattern, [string[]]$allowedRoots) {
  $hits = @()
  foreach ($m in [regex]::Matches($text, $pattern)) {
    if (Test-ShouldSkipAbsolutePathMatch $text $m $allowedRoots) { continue }
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
  Invoke-BuildScriptUnsafeCheck $repoRoot
  Invoke-RequiredCargoGeiger $repoRoot
  Invoke-PrivacyScrubChecks
  if ($failed) { exit 1 }
  Write-Host "GATE-A checks passed." -ForegroundColor Green
}


function Invoke-GateAMain {
  if ($SelfTest) {
    Invoke-GateASelfTests
    return
  }
  Invoke-GateAChecks
}

Invoke-GateAMain
