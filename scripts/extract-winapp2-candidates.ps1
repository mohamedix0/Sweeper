#!/usr/bin/env pwsh
<#
.SYNOPSIS
Read-only Winapp2.ini candidate extractor (review report only).

.DESCRIPTION
Parses a Winapp2.ini file and emits a Markdown review report of candidate cleanup entries.
This script NEVER executes any cleanup actions and NEVER modifies allowlist.toml.

.PARAMETER InputPath
Path to Winapp2.ini (read-only input).

.PARAMETER OutputPath
Path to write the generated Markdown review report.

.EXAMPLE
./scripts/extract-winapp2-candidates.ps1 -InputPath .\reference\Winapp2.ini -OutputPath .\artifacts\winapp2-candidates.md
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)]
  [string]$InputPath,

  [Parameter(Mandatory = $true)]
  [string]$OutputPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not (Test-Path -LiteralPath $InputPath -PathType Leaf)) {
  throw "InputPath does not exist or is not a file: $InputPath"
}

$resolvedInput = (Resolve-Path -LiteralPath $InputPath).Path
$outDir = Split-Path -Parent $OutputPath
if ($outDir -and -not (Test-Path -LiteralPath $outDir)) {
  New-Item -ItemType Directory -Path $outDir -Force | Out-Null
}
$resolvedOutput = [System.IO.Path]::GetFullPath($OutputPath)

$lines = Get-Content -LiteralPath $resolvedInput -Encoding UTF8
$entries = @()
$current = $null

foreach ($raw in $lines) {
  $line = $raw.Trim()
  if ($line -match '^\[(.+)\]$') {
    if ($null -ne $current) { $entries += $current }
    $current = [ordered]@{
      Section = $Matches[1]
      LangSecRef = ''
      Detects = @()
      FileKeys = @()
      Excludes = @()
      Warnings = @()
      Default = ''
    }
    continue
  }
  if ($null -eq $current -or $line -eq '' -or $line.StartsWith(';')) { continue }

  if ($line -match '^LangSecRef=(.*)$') { $current.LangSecRef = $Matches[1].Trim(); continue }
  if ($line -match '^Default=(.*)$') { $current.Default = $Matches[1].Trim(); continue }
  if ($line -match '^(Detect|DetectFile|DetectOS|SpecialDetect)=(.*)$') {
    $current.Detects += ("{0}={1}" -f $Matches[1], $Matches[2].Trim()); continue
  }
  if ($line -match '^(FileKey\d*|RegKey\d*)=(.*)$') {
    $current.FileKeys += ("{0}={1}" -f $Matches[1], $Matches[2].Trim()); continue
  }
  if ($line -match '^(ExcludeKey\d*)=(.*)$') {
    $current.Excludes += ("{0}={1}" -f $Matches[1], $Matches[2].Trim()); continue
  }
  if ($line -match '^Warning=(.*)$') { $current.Warnings += $Matches[1].Trim(); continue }
}
if ($null -ne $current) { $entries += $current }

$sb = New-Object System.Text.StringBuilder
[void]$sb.AppendLine('# Winapp2 Candidate Extraction Report (Read-Only)')
[void]$sb.AppendLine()
[void]$sb.AppendLine('> Generated for manual review only. This report does **not** modify `allowlist.toml`, does **not** enable targets, and must never be auto-executed.')
[void]$sb.AppendLine()
[void]$sb.AppendLine("- Source: `$InputPath` = $resolvedInput")
[void]$sb.AppendLine("- Generated (UTC): $([DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ssZ'))")
[void]$sb.AppendLine("- Entries parsed: $($entries.Count)")
[void]$sb.AppendLine()
[void]$sb.AppendLine('## Review Notes')
[void]$sb.AppendLine('- Treat every entry as **untrusted candidate data** until manually reviewed against Sweeper safety policy.')
[void]$sb.AppendLine('- Reject any candidate that targets protected paths/containers, uses unsafe wildcards, or lacks clear regeneration semantics.')
[void]$sb.AppendLine('- Do not copy Winapp2 actions directly into Sweeper; use this as a research index only.')
[void]$sb.AppendLine()
[void]$sb.AppendLine('## Candidates')

foreach ($e in $entries) {
  [void]$sb.AppendLine()
  [void]$sb.AppendLine("### [$($e.Section)]")
  if ($e.LangSecRef) { [void]$sb.AppendLine("- LangSecRef: $($e.LangSecRef)") }
  if ($e.Default) { [void]$sb.AppendLine("- Default: $($e.Default)") }
  if ($e.Warnings.Count -gt 0) {
    [void]$sb.AppendLine('- Warnings:')
    foreach ($w in $e.Warnings) { [void]$sb.AppendLine("  - $w") }
  }
  if ($e.Detects.Count -gt 0) {
    [void]$sb.AppendLine('- Detect/OS/SpecialDetect:')
    foreach ($d in $e.Detects) { [void]$sb.AppendLine("  - ``$d``") }
  }
  if ($e.FileKeys.Count -gt 0) {
    [void]$sb.AppendLine('- File/Reg keys:')
    foreach ($fk in $e.FileKeys) { [void]$sb.AppendLine("  - ``$fk``") }
  }
  if ($e.Excludes.Count -gt 0) {
    [void]$sb.AppendLine('- Exclude keys:')
    foreach ($ex in $e.Excludes) { [void]$sb.AppendLine("  - ``$ex``") }
  }
}

[System.IO.File]::WriteAllText($resolvedOutput, $sb.ToString(), [System.Text.UTF8Encoding]::new($false))
Write-Host "Wrote Winapp2 candidate report: $resolvedOutput" -ForegroundColor Green
Write-Host "Entries parsed: $($entries.Count)"
