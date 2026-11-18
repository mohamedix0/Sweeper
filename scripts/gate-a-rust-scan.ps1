# Gate A - Rust lexical scanner helpers.
# Dot-sourced by gate-a-check.ps1. Pure state-machine functions that blank comments and
# string literals so forbid(unsafe_code) and SAFETY-comment checks see only active code.

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
