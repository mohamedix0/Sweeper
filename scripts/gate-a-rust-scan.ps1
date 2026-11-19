# Gate A - Rust lexical scanner helpers.
# Dot-sourced by gate-a-check.ps1. Pure state-machine functions that blank comments and
# string literals so forbid(unsafe_code), SAFETY-comment and build-script unsafe checks see
# only active code.
# Compound character tests are named predicates so each function stays simple and reviewable.

function Test-RustNewline([char]$c) {
  return ($c -eq "`r") -or ($c -eq "`n")
}

function Blank-RustNonNewline([hashtable]$state, [int]$i) {
  if (-not (Test-RustNewline $state.Out[$i])) { $state.Out[$i] = ' ' }
}

function Test-RustBlockOpen([hashtable]$state, [int]$i, [int]$j) {
  return (($j -lt $state.Len) -and ($state.Out[$i] -eq '/') -and ($state.Out[$j] -eq '*'))
}

function Test-RustBlockClose([hashtable]$state, [int]$i, [int]$j) {
  return (($j -lt $state.Len) -and ($state.Out[$i] -eq '*') -and ($state.Out[$j] -eq '/'))
}

function Get-RustScanState([string]$text) {
  $out = $text.ToCharArray()
  return @{ Out = $out; Len = $out.Length; Index = 0; Depth = 0; InString = $false; InLine = $false; RawHashes = -1 }
}

function Step-RustBlock([hashtable]$state) {
  $i = $state.Index
  $j = $i + 1
  if (Test-RustBlockOpen $state $i $j) {
    $state.Out[$i] = ' '; $state.Out[$j] = ' '; $state.Depth++; $state.Index = $i + 2; return
  }
  if (Test-RustBlockClose $state $i $j) {
    $state.Out[$i] = ' '; $state.Out[$j] = ' '; $state.Depth--; $state.Index = $i + 2; return
  }
  Blank-RustNonNewline $state $i
  $state.Index = $i + 1
}

function Step-RustLineComment([hashtable]$state) {
  $i = $state.Index
  if (Test-RustNewline $state.Out[$i]) { $state.InLine = $false; $state.Index = $i + 1; return }
  Blank-RustNonNewline $state $i
  $state.Index = $i + 1
}

function Test-RustStringEscaped([hashtable]$state, [int]$i, [int]$j) {
  return (($state.Out[$i] -eq '\') -and ($j -lt $state.Len))
}

function Step-RustString([hashtable]$state) {
  $i = $state.Index
  $j = $i + 1
  if (Test-RustStringEscaped $state $i $j) {
    Blank-RustNonNewline $state $i
    Blank-RustNonNewline $state $j
    $state.Index = $i + 2; return
  }
  if ($state.Out[$i] -eq '"') {
    $state.Out[$i] = ' '; $state.InString = $false; $state.Index = $i + 1; return
  }
  Blank-RustNonNewline $state $i
  $state.Index = $i + 1
}

function Test-RustRawHashAt([hashtable]$state, [int]$i) {
  return (($i -lt $state.Len) -and ($state.Out[$i] -eq '#'))
}

function Test-RustRawQuoteAt([hashtable]$state, [int]$i) {
  return (($i -lt $state.Len) -and ($state.Out[$i] -eq '"'))
}

function Test-RustRawHashesAt([hashtable]$state, [int]$i) {
  $j = $i
  while (Test-RustRawHashAt $state $j) { $j++ }
  if (-not (Test-RustRawQuoteAt $state $j)) { return -1 }
  return ($j - $i)
}

function Test-RustRawPrefixAt([hashtable]$state, [int]$i, [int]$j, [char]$prefix) {
  return (($state.Out[$i] -eq $prefix) -and ($j -lt $state.Len) -and ($state.Out[$j] -eq 'r'))
}

function Get-RustRawPrefixStart([hashtable]$state) {
  $i = $state.Index
  $j = $i + 1
  if ($state.Out[$i] -eq 'r') { return $i }
  if (Test-RustRawPrefixAt $state $i $j 'b') { return $j }
  if (Test-RustRawPrefixAt $state $i $j 'c') { return $j }
  return -1
}

function Test-RustIdentBefore([hashtable]$state, [int]$prev) {
  return (($prev -ge 0) -and ($state.Out[$prev] -match '[A-Za-z0-9_]'))
}

function Try-StartRustRawString([hashtable]$state) {
  $rIdx = Get-RustRawPrefixStart $state
  if ($rIdx -lt 0) { return $false }
  if (Test-RustIdentBefore $state ($state.Index - 1)) { return $false }
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
    if ($j -ge $state.Len) { return $false }
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
  Blank-RustNonNewline $state $state.Index
  if ($isClose) { Complete-RustRawClose $state; return }
  $state.Index = $state.Index + 1
}

function Test-RustSlashPair([hashtable]$state, [int]$i, [int]$j, [char]$second) {
  return (($j -lt $state.Len) -and ($state.Out[$i] -eq '/') -and ($state.Out[$j] -eq $second))
}

function Try-StartRustLineComment([hashtable]$state) {
  $i = $state.Index
  $j = $i + 1
  if (-not (Test-RustSlashPair $state $i $j '/')) { return $false }
  $state.Out[$i] = ' '; $state.Out[$j] = ' '
  $state.InLine = $true; $state.Index = $i + 2
  return $true
}

function Try-StartRustBlockComment([hashtable]$state) {
  $i = $state.Index
  $j = $i + 1
  if (-not (Test-RustSlashPair $state $i $j '*')) { return $false }
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

function Test-RustSimpleCharFiller([char]$c) {
  return (($c -eq '\') -or ($c -eq "'") -or ($c -eq "`r") -or ($c -eq "`n"))
}

function Try-BlankRustSimpleChar([hashtable]$state) {
  $i = $state.Index
  if (($i + 2) -ge $state.Len) { return $false }
  if (Test-RustSimpleCharFiller $state.Out[$i + 1]) { return $false }
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
    if (Test-RustNewline $c) { return -1 }
    if ($c -eq "'") { return $j }
    $j++
  }
  return -1
}

function Try-BlankRustEscapedChar([hashtable]$state) {
  $i = $state.Index
  if (($i + 2) -ge $state.Len) { return $false }
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

function Test-RustHasUnsafeToken([string]$text) {
  # Conservative detection of any active `unsafe` token (unsafe block, fn, impl, trait, extern).
  # Comments and string/char literals are blanked first, so docs and examples never false-positive.
  $scan = Get-RustScanText $text
  return ($scan -match '\bunsafe\b')
}
