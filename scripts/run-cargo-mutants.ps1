[CmdletBinding()]
param(
    [double]$MinScore = 95.0,
    [string]$OutputDir = "",
    [switch]$ListOnly
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Write-Step([string]$Message) {
    Write-Host "==> $Message" -ForegroundColor Cyan
}

function Resolve-RepoRoot {
    return (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
}

function Get-OutcomeCounts {
    param([Parameter(Mandatory = $true)][string]$OutcomesPath)

    $raw = Get-Content -LiteralPath $OutcomesPath -Raw -Encoding UTF8
    $json = $raw | ConvertFrom-Json

    # cargo-mutants outcomes.json is typically an array of outcome records; tolerate wrapped shapes defensively.
    $outcomes = @()
    if ($null -eq $json) {
        $outcomes = @()
    } elseif ($json -is [System.Array]) {
        $outcomes = @($json)
    } elseif ($json.PSObject.Properties.Name -contains 'outcomes') {
        $outcomes = @($json.outcomes)
    } elseif ($json.PSObject.Properties.Name -contains 'Outcome') {
        $outcomes = @($json)
    } else {
        # Fallback: treat single object/unknown shape as no outcomes rather than crashing parsing.
        $outcomes = @()
    }

    $counts = @{
        caught   = 0
        missed   = 0
        timeout  = 0
        unviable = 0
        other    = 0
        total    = 0
    }
    $missedDetails = @()

    foreach ($o in $outcomes) {
        $counts.total++
        $summary = $null
        if ($null -ne $o -and ($o.PSObject.Properties.Name -contains 'summary')) {
            $summary = [string]$o.summary
        } elseif ($null -ne $o -and ($o.PSObject.Properties.Name -contains 'outcome')) {
            $summary = [string]$o.outcome
        } elseif ($null -ne $o -and ($o.PSObject.Properties.Name -contains 'Outcome')) {
            $summary = [string]$o.Outcome
        } else {
            $summary = ""
        }
        $key = $summary.ToLowerInvariant()
        switch ($key) {
            'caught'   { $counts.caught++ }
            'missed'   {
                $counts.missed++
                $missedDetails += $o
            }
            'timeout'  {
                $counts.timeout++
                $missedDetails += $o
            }
            'unviable' { $counts.unviable++ }
            default    { $counts.other++ }
        }
    }

    return [pscustomobject]@{
        Counts        = $counts
        MissedDetails = $missedDetails
    }
}

$repoRoot = Resolve-RepoRoot
if ([string]::IsNullOrWhiteSpace($OutputDir)) {
    $OutputDir = Join-Path $repoRoot "target\mutants"
}
$OutputDir = [System.IO.Path]::GetFullPath($OutputDir)

if ([string]::IsNullOrWhiteSpace($env:CARGO_TARGET_DIR)) {
    $env:CARGO_TARGET_DIR = Join-Path $repoRoot "target"
}
$env:CARGO_TARGET_DIR = [System.IO.Path]::GetFullPath($env:CARGO_TARGET_DIR)

Write-Step "Repo root: $repoRoot"
Write-Step "CARGO_TARGET_DIR: $env:CARGO_TARGET_DIR"
Write-Step "Mutation output directory: $OutputDir"
Write-Step "Minimum mutation score: $MinScore%"

$cargoMutants = Get-Command cargo-mutants -ErrorAction SilentlyContinue
if (-not $cargoMutants) {
    throw "cargo-mutants is not installed or not on PATH. Install it (e.g. 'cargo install --locked cargo-mutants' or via taiki-e/install-action in CI) and re-run."
}

$versionOutput = & cargo mutants --version 2>&1
if ($LASTEXITCODE -ne 0) {
    $versionText = ($versionOutput | Out-String).Trim()
    throw "Failed to execute 'cargo mutants --version'. Output: $versionText"
}
$versionText = ($versionOutput | Out-String).Trim()
if (-not [string]::IsNullOrWhiteSpace($versionText)) {
    Write-Host $versionText
}

# Target only sweeper-core safety-critical path/policy modules.
$workspaceDir = Join-Path $repoRoot "app"
$filters = @("**/sweeper-core/src/policy.rs", "**/sweeper-core/src/paths.rs")

if ($ListOnly) {
    Write-Step "Listing targeted mutants (dry-run) for sweeper-core policy.rs and paths.rs"
    $listArgs = @(
        "mutants",
        "--dir", $workspaceDir,
        "--package", "sweeper-core",
        "--list",
        "--json"
    )
    foreach ($f in $filters) {
        $listArgs += @("--file", $f)
    }
    Write-Host ("cargo " + ($listArgs -join ' '))
    & cargo @listArgs
    exit $LASTEXITCODE
}

# Clean prior output directory to avoid stale outcomes.
if (Test-Path -LiteralPath $OutputDir) {
    Write-Step "Removing previous mutation output: $OutputDir"
    Remove-Item -LiteralPath $OutputDir -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null

$mutantsArgs = @(
    "mutants",
    "--dir", $workspaceDir,
    "--package", "sweeper-core",
    "--output", $OutputDir,
    "--no-shuffle"
)
foreach ($f in $filters) {
    $mutantsArgs += @("--file", $f)
}

Write-Step "Running cargo-mutants on sweeper-core policy.rs and paths.rs"
Write-Host ("cargo " + ($mutantsArgs -join ' '))

$mutantsSucceeded = $true
& cargo @mutantsArgs
if ($LASTEXITCODE -ne 0) {
    # cargo-mutants may exit non-zero when mutants are missed/timeouts occur; outcomes.json is still the source of truth.
    $mutantsSucceeded = $false
    Write-Warning "cargo mutants exited with code $LASTEXITCODE; parsing outcomes.json for authoritative results."
}

$outcomesPath = Join-Path $OutputDir "mutants.out\outcomes.json"
if (-not (Test-Path -LiteralPath $outcomesPath)) {
    # Some layouts may place outcomes.json differently; search under output dir as fallback.
    $found = Get-ChildItem -LiteralPath $OutputDir -Recurse -Filter outcomes.json -File -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($found) {
        $outcomesPath = $found.FullName
    }
}
if (-not (Test-Path -LiteralPath $outcomesPath)) {
    throw "Mutation outcomes file not found. Expected under: $OutputDir (looked for mutants.out\\outcomes.json)."
}

$parsed = Get-OutcomeCounts -OutcomesPath $outcomesPath
$counts = $parsed.Counts
$missedDetails = @($parsed.MissedDetails)

[int]$caught = $counts.caught
[int]$missed = $counts.missed
[int]$timeout = $counts.timeout
[int]$unviable = $counts.unviable
[int]$other = $counts.other
[int]$total = $counts.total
[int]$viable = $caught + $missed + $timeout

$score = 0.0
$passed = $false
if ($viable -gt 0) {
    $score = [math]::Round((($caught / [double]$viable) * 100.0), 2)
    $passed = ($score -ge $MinScore)
}

if ($missedDetails.Count -gt 0) {
    Write-Host ""
    Write-Warning "Missed/timeout mutants detected:"
    foreach ($m in $missedDetails) {
        $mSummary = ""
        if ($null -ne $m -and ($m.PSObject.Properties.Name -contains 'summary')) { $mSummary = [string]$m.summary }
        elseif ($null -ne $m -and ($m.PSObject.Properties.Name -contains 'outcome')) { $mSummary = [string]$m.outcome }
        elseif ($null -ne $m -and ($m.PSObject.Properties.Name -contains 'Outcome')) { $mSummary = [string]$m.Outcome }
        $mName = ""
        if ($null -ne $m -and ($m.PSObject.Properties.Name -contains 'name')) { $mName = [string]$m.name }
        elseif ($null -ne $m -and ($m.PSObject.Properties.Name -contains 'scenario')) { $mName = [string]$m.scenario }
        elseif ($null -ne $m -and ($m.PSObject.Properties.Name -contains 'function')) { $mName = [string]$m.function }
        $mLog = ""
        if ($null -ne $m -and ($m.PSObject.Properties.Name -contains 'log_path')) { $mLog = [string]$m.log_path }
        elseif ($null -ne $m -and ($m.PSObject.Properties.Name -contains 'logPath')) { $mLog = [string]$m.logPath }
        elseif ($null -ne $m -and ($m.PSObject.Properties.Name -contains 'log_file')) { $mLog = [string]$m.log_file }
        Write-Host (" - summary={0} name={1} log={2}" -f $mSummary, $mName, $mLog)
    }
}

$summary = [ordered]@{
    timestampUtc        = [DateTime]::UtcNow.ToString("o")
    package             = "sweeper-core"
    targets             = @("src/policy.rs", "src/paths.rs")
    filters             = $filters
    minScore            = $MinScore
    score               = $score
    passed              = $passed
    viableMutants       = $viable
    counts              = [ordered]@{
        caught   = $caught
        missed   = $missed
        timeout  = $timeout
        unviable = $unviable
        other    = $other
        total    = $total
    }
    cargoMutantsVersion = $versionText
    cargoMutantsExitOk  = $mutantsSucceeded
    outcomesPath        = $outcomesPath
    outputDir           = $OutputDir
    missedOrTimeout     = $missedDetails
}

$summaryPath = Join-Path $OutputDir "mutation-summary.json"
$summary | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath $summaryPath -Encoding UTF8

Write-Host ""
Write-Host "Mutation summary" -ForegroundColor Cyan
Write-Host ("  Score: {0}% (threshold: {1}%)" -f $score, $MinScore)
Write-Host ("  Caught: {0}, Missed: {1}, Timeout: {2}, Unviable: {3}, Other: {4}, Viable: {5}" -f $caught, $missed, $timeout, $unviable, $other, $viable)
Write-Host ("  Passed: {0}" -f $passed)
Write-Host ("  outcomes.json: {0}" -f $outcomesPath)
Write-Host ("  mutation-summary.json: {0}" -f $summaryPath)

if ($viable -eq 0) {
    throw "No viable mutants were reported for the targeted files (policy.rs, paths.rs). Failing mutation gate."
}
if (-not $passed) {
    throw ("Mutation score {0}% is below required threshold {1}%." -f $score, $MinScore)
}

Write-Step "Mutation gate passed"
exit 0
