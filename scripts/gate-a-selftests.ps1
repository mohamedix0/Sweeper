# Gate A - self-test suite.
# Dot-sourced by gate-a-check.ps1. Fixtures are assembled from neutral fragments so this
# file never stores a real machine path, username, or internal identifier.

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

function Test-SelfTestEscapedLowercaseContinuations([ref]$ok) {
  $rules = Get-PrivacyRules
  $pattern = $rules.AbsoluteDrivePathPattern
  $allowed = $rules.AllowedAbsolutePathRoots
  $sep = [System.IO.Path]::DirectorySeparatorChar
  $quote = [char]34

  # CodeRabbit PR #48 thread 4607517845: bare and nested escaped paths whose first segment uses an
  # allowlisted lead word followed by a lowercase continuation must be flagged.
  $escapedBareLower = $quote + 'C:' + $sep + $sep + 'Windows confidential' + $quote
  $bareMatchCount = @([regex]::Matches($escapedBareLower, $pattern)).Count
  Assert-SelfTest ($bareMatchCount -ge 1) "escaped bare lowercase-continuation path matches pattern" $ok
  $bareLowerHits = @(Get-PrivacyHitsForAbsolutePaths 'fixture.txt' $escapedBareLower $pattern $allowed)
  Assert-SelfTest ($bareLowerHits.Count -ge 1) "escaped bare lowercase-continuation path flagged directly" $ok
  $bareLowerWired = @(Get-PrivacyHitsForFile 'fixture.txt' $escapedBareLower $rules | Where-Object { $_ -match 'forbidden absolute drive path' })
  Assert-SelfTest ($bareLowerWired.Count -ge 1) "escaped bare lowercase-continuation path flagged by file scan" $ok

  $escapedNestedLower = $quote + 'C:' + $sep + $sep + 'Windows confidential' + $sep + $sep + 'private-data' + $sep + $sep + 'notes.txt' + $quote + ';'
  $nestedMatchCount = @([regex]::Matches($escapedNestedLower, $pattern)).Count
  Assert-SelfTest ($nestedMatchCount -ge 1) "escaped nested lowercase-continuation path matches pattern" $ok
  $nestedLowerHits = @(Get-PrivacyHitsForAbsolutePaths 'fixture.txt' $escapedNestedLower $pattern $allowed)
  Assert-SelfTest ($nestedLowerHits.Count -ge 1) "escaped nested lowercase-continuation path flagged directly" $ok
  $nestedLowerWired = @(Get-PrivacyHitsForFile 'fixture.txt' $escapedNestedLower $rules | Where-Object { $_ -match 'forbidden absolute drive path' })
  Assert-SelfTest ($nestedLowerWired.Count -ge 1) "escaped nested lowercase-continuation path flagged by file scan" $ok

  # CodeRabbit PR #48 thread 4607652129: a two-word lowercase continuation in a bare escaped
  # literal is a path segment, not prose. "confidential files" has no subject-verb prose shape.
  $escapedTwoWord = $quote + 'C:' + $sep + $sep + 'Windows confidential files' + $quote + ';'
  $twoWordMatchCount = @([regex]::Matches($escapedTwoWord, $pattern)).Count
  Assert-SelfTest ($twoWordMatchCount -ge 1) "two-word continuation path matches pattern" $ok
  $twoWordDirect = @(Get-PrivacyHitsForAbsolutePaths 'fixture.txt' $escapedTwoWord $pattern $allowed)
  Assert-SelfTest ($twoWordDirect.Count -ge 1) "two-word continuation path flagged directly" $ok
  $twoWordWired = @(Get-PrivacyHitsForFile 'fixture.txt' $escapedTwoWord $rules | Where-Object { $_ -match 'forbidden absolute drive path' })
  Assert-SelfTest ($twoWordWired.Count -ge 1) "two-word continuation path flagged by file scan" $ok
}

function Test-SelfTestEscapedContinuedAndNonAscii([ref]$ok) {
  $rules = Get-PrivacyRules
  $pattern = $rules.AbsoluteDrivePathPattern
  $allowed = $rules.AllowedAbsolutePathRoots
  $sep = [System.IO.Path]::DirectorySeparatorChar
  $bs = '\'
  $quote = [char]34

  # CodeRabbit PR #48 thread 4607568585: continuation into more path segments after an allowlisted
  # lead word must be reported even when the captured segment looks like prose.
  $continuedCases = @(
    ($quote + 'C:' + $sep + $sep + 'Windows backup' + $sep + $sep + 'private-data' + $sep + $sep + 'notes.txt' + $quote + ';')
    ($quote + 'C:' + $sep + $sep + 'Windows backup folder' + $sep + $sep + 'private-data' + $sep + $sep + 'notes.txt' + $quote + ';')
  )
  foreach ($p in $continuedCases) {
    $direct = @(Get-PrivacyHitsForAbsolutePaths 'fixture.txt' $p $pattern $allowed)
    Assert-SelfTest ($direct.Count -ge 1) "continued escaped path must be flagged directly: $p" $ok
    $wired = @(Get-PrivacyHitsForFile 'fixture.txt' $p $rules | Where-Object { $_ -match 'forbidden absolute drive path' })
    Assert-SelfTest ($wired.Count -ge 1) "continued escaped path must be flagged by file scan: $p" $ok
  }

  # CodeRabbit PR #48 thread 4607568579: a valid non-ASCII folder name must not suppress detection.
  $accent = [char]0x00E9
  $nonAsciiFolder = 'private-' + $accent + 'quipe'
  $escapedNonAscii = $quote + 'D:' + $sep + $sep + $nonAsciiFolder + $sep + $sep + 'notes.txt' + $quote + ';'
  $nonAsciiMatchCount = @([regex]::Matches($escapedNonAscii, $pattern)).Count
  Assert-SelfTest ($nonAsciiMatchCount -ge 1) "escaped valid non-ASCII path matches pattern" $ok
  $nonAsciiDirect = @(Get-PrivacyHitsForAbsolutePaths 'fixture.txt' $escapedNonAscii $pattern $allowed)
  Assert-SelfTest ($nonAsciiDirect.Count -ge 1) "escaped valid non-ASCII path flagged directly" $ok
  $nonAsciiWired = @(Get-PrivacyHitsForFile 'fixture.txt' $escapedNonAscii $rules | Where-Object { $_ -match 'forbidden absolute drive path' })
  Assert-SelfTest ($nonAsciiWired.Count -ge 1) "escaped valid non-ASCII path flagged by file scan" $ok

  # CodeRabbit PR #48 thread 4607658800: a legal non-ASCII filename is allowed as the FIRST
  # character of a segment, and U+FF0F is a legal Windows filename character. Both real paths
  # below must be flagged; only the exact synthetic "Temp" + U+FF0F + "foo" fixture is exempt.
  $leadingNonAsciiPath = $quote + 'D:' + $sep + $sep + $accent + 'quipe' + $sep + $sep + 'notes.txt' + $quote + ';'
  $leadingNonAsciiMatches = @([regex]::Matches($leadingNonAsciiPath, $pattern)).Count
  Assert-SelfTest ($leadingNonAsciiMatches -ge 1) "non-ASCII leading segment matches pattern" $ok
  $leadingNonAsciiDirect = @(Get-PrivacyHitsForAbsolutePaths 'fixture.txt' $leadingNonAsciiPath $pattern $allowed)
  Assert-SelfTest ($leadingNonAsciiDirect.Count -ge 1) "non-ASCII leading segment flagged directly" $ok
  $leadingNonAsciiWired = @(Get-PrivacyHitsForFile 'fixture.txt' $leadingNonAsciiPath $rules | Where-Object { $_ -match 'forbidden absolute drive path' })
  Assert-SelfTest ($leadingNonAsciiWired.Count -ge 1) "non-ASCII leading segment flagged by file scan" $ok

  $slashLikePath = $quote + 'D:' + $sep + $sep + 'private' + [char]0xFF0F + 'notes' + $sep + $sep + 'file.txt' + $quote + ';'
  $slashLikeMatches = @([regex]::Matches($slashLikePath, $pattern)).Count
  Assert-SelfTest ($slashLikeMatches -ge 1) "slash-lookalike segment matches pattern" $ok
  $slashLikeDirect = @(Get-PrivacyHitsForAbsolutePaths 'fixture.txt' $slashLikePath $pattern $allowed)
  Assert-SelfTest ($slashLikeDirect.Count -ge 1) "slash-lookalike segment flagged directly" $ok
  $slashLikeWired = @(Get-PrivacyHitsForFile 'fixture.txt' $slashLikePath $rules | Where-Object { $_ -match 'forbidden absolute drive path' })
  Assert-SelfTest ($slashLikeWired.Count -ge 1) "slash-lookalike segment flagged by file scan" $ok

  $malformed = $quote + 'C:' + $bs + $bs + 'Temp' + [char]0xFF0F + 'foo' + $quote
  $malformedWired = @(Get-PrivacyHitsForFile 'fixture.txt' $malformed $rules)
  Assert-SelfTest ($malformedWired.Count -eq 0) "malformed fullwidth-solidus fixture stays unflagged" $ok

  # CodeRabbit 4607723328: the malformed-fixture exemption applies only to the bare fixture.
  # A following path separator makes it a real descendant path and must be reported instead.
  $malformedDescendant = $quote + 'C:' + $bs + $bs + 'Temp' + [char]0xFF0F + 'foo' + $bs + $bs + 'notes.txt' + $quote + ';'
  $descMatches = @([regex]::Matches($malformedDescendant, $pattern))
  Assert-SelfTest ($descMatches.Count -ge 1) "malformed-fixture descendant matches absolute-drive pattern" $ok
  if ($descMatches.Count -ge 1) {
    Assert-SelfTest ($descMatches[0].Groups['seg'].Value -ceq ('Temp' + [char]0xFF0F + 'foo')) "malformed-fixture descendant captures the first segment whole" $ok
  }
  $descDirect = @(Get-PrivacyHitsForAbsolutePaths 'fixture.txt' $malformedDescendant $pattern $allowed)
  Assert-SelfTest ($descDirect.Count -ge 1) "malformed-fixture descendant flagged by direct detector" $ok
  $descWired = @(Get-PrivacyHitsForFile 'fixture.txt' $malformedDescendant $rules | Where-Object { $_ -match 'forbidden absolute drive path' })
  Assert-SelfTest ($descWired.Count -ge 1) "malformed-fixture descendant flagged through file-level privacy scan" $ok

  # CodeRabbit 4607724831: a copular "is" continuation is a path-like string, not prose, so it
  # must be reported. Genuine action/auxiliary-verb prose and system paths stay unflagged.
  $copulaPath = $quote + 'C:' + $bs + $bs + 'Windows is private' + $quote
  $copulaMatches = @([regex]::Matches($copulaPath, $pattern))
  Assert-SelfTest ($copulaMatches.Count -ge 1) "copula-continuation path matches absolute-drive pattern" $ok
  $copulaDirect = @(Get-PrivacyHitsForAbsolutePaths 'fixture.txt' $copulaPath $pattern $allowed)
  Assert-SelfTest ($copulaDirect.Count -ge 1) "copula-continuation path flagged by direct detector" $ok
  $copulaWired = @(Get-PrivacyHitsForFile 'fixture.txt' $copulaPath $rules | Where-Object { $_ -match 'forbidden absolute drive path' })
  Assert-SelfTest ($copulaWired.Count -ge 1) "copula-continuation path flagged through file-level privacy scan" $ok

  $proseControls = @(
    ($quote + 'C:' + $bs + $bs + 'Users must be rejected by sandbox safety validation' + $quote)
    ('note: ' + $quote + 'detail; C:' + $bs + $bs + 'Windows remains Protected.' + $quote)
    ('C' + ':' + $sep + 'Windows' + $sep + 'System32')
  )
  foreach ($p in $proseControls) {
    $ctrlDirect = @(Get-PrivacyHitsForAbsolutePaths 'fixture.txt' $p $pattern $allowed)
    Assert-SelfTest ($ctrlDirect.Count -eq 0) "prose/system control unflagged by direct detector: $p" $ok
    $ctrlWired = @(Get-PrivacyHitsForFile 'fixture.txt' $p $rules | Where-Object { $_ -match 'forbidden absolute drive path' })
    Assert-SelfTest ($ctrlWired.Count -eq 0) "prose/system control unflagged by file scan: $p" $ok
  }
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
  Test-SelfTestEscapedLowercaseContinuations $okRef
  Test-SelfTestEscapedContinuedAndNonAscii $okRef
  Test-SelfTestForeignCheckoutUnderAllowedRoot $okRef
  if ($ok) {
    Write-Host "GATE-A self-tests passed." -ForegroundColor Green
    return
  }
  Write-Host "GATE-A self-tests failed." -ForegroundColor Red
  exit 1
}
