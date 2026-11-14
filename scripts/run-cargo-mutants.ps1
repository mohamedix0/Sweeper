[CmdletBinding()]
param(
    [double]$MinScore = 95.0,
    [string]$OutputDir = "",
    [switch]$ListOnly,
    [switch]$SelfTest
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Write-Step([string]$Message) {
    Write-Host "==> $Message" -ForegroundColor Cyan
}

function Resolve-RepoRoot {
    return (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
}

function Get-ObjectPropertyValue {
    param(
        [Parameter(Mandatory = $false)]$Object,
        [Parameter(Mandatory = $true)][string[]]$Names
    )
    if ($null -eq $Object) { return $null }
    $props = @()
    try { $props = @($Object.PSObject.Properties.Name) } catch { return $null }
    foreach ($name in $Names) {
        if ($props -contains $name) {
            return $Object.$name
        }
    }
    return $null
}

function Test-IsBaselineScenario {
    param([Parameter(Mandatory = $false)]$Scenario)
    if ($null -eq $Scenario) { return $false }
    if ($Scenario -is [string]) {
        return ($Scenario.Trim().ToLowerInvariant() -eq "baseline")
    }
    $s = [string]$Scenario
    if ($s.Trim().ToLowerInvariant() -eq "baseline") { return $true }
    $props = @()
    try { $props = @($Scenario.PSObject.Properties.Name) } catch { return $false }
    if ($props -contains "Baseline") { return $true }
    if (($props -contains "scenario") -and ($Scenario.scenario -is [string])) {
        return ($Scenario.scenario.Trim().ToLowerInvariant() -eq "baseline")
    }
    return $false
}

function Get-NormalizedOutcomeKey {
    param([Parameter(Mandatory = $false)][string]$RawSummary)

    $trimmed = ""
    if ($null -ne $RawSummary) { $trimmed = $RawSummary.Trim() }
    $key = $trimmed.ToLowerInvariant()
    switch ($key) {
        "caught"       { return "caught" }
        "caughtmutant" { return "caught" }
        "missed"       { return "missed" }
        "missedmutant" { return "missed" }
        "timeout"      { return "timeout" }
        "unviable"     { return "unviable" }
        default        { return $null }
    }
}

function Get-MutantOutcomeSummaryText {
    param([Parameter(Mandatory = $true)]$Entry)

    $value = Get-ObjectPropertyValue -Object $Entry -Names @("summary", "outcome", "Outcome")
    if ($null -eq $value) { return "" }
    return ([string]$value).Trim()
}

function Get-FirstNonNullValue {
    param([Parameter(Mandatory = $false)]$Object, [Parameter(Mandatory = $true)][string[]]$Names)
    foreach ($name in $Names) {
        $v = Get-ObjectPropertyValue -Object $Object -Names @($name)
        if ($null -ne $v -and -not [string]::IsNullOrWhiteSpace([string]$v)) {
            return $v
        }
    }
    return $null
}

function Get-ConciseMutantDetail {
    param(
        [Parameter(Mandatory = $true)]$Entry,
        [Parameter(Mandatory = $true)][string]$SummaryText
    )

    $scenario = Get-ObjectPropertyValue -Object $Entry -Names @("scenario")
    $mutant = Get-ObjectPropertyValue -Object $Entry -Names @("mutant", "Mutation", "mutation")
    if ($null -eq $mutant -and $null -ne $scenario) {
        $mutant = Get-ObjectPropertyValue -Object $scenario -Names @("Mutant", "mutant")
    }

    $file = $null
    $line = $null
    $functionName = $null
    $replacement = $null

    if ($null -ne $mutant) {
        $file = Get-FirstNonNullValue -Object $mutant -Names @("file", "path")
        $functionName = Get-FirstNonNullValue -Object $mutant -Names @("function", "name", "function_name")
        $replacement = Get-FirstNonNullValue -Object $mutant -Names @("replacement", "replace")
        $span = Get-ObjectPropertyValue -Object $mutant -Names @("span")
        $start = $null
        if ($null -ne $span) { $start = Get-ObjectPropertyValue -Object $span -Names @("start") }
        if ($null -ne $start) {
            $line = Get-FirstNonNullValue -Object $start -Names @("line", "row")
        }
        if ($null -eq $file) { $file = Get-FirstNonNullValue -Object $mutant -Names @("file_name") }
    }
    if ($null -eq $file) { $file = Get-FirstNonNullValue -Object $Entry -Names @("file", "path") }
    if ($null -eq $functionName) { $functionName = Get-FirstNonNullValue -Object $Entry -Names @("function", "name") }
    if ($null -eq $replacement) { $replacement = Get-FirstNonNullValue -Object $Entry -Names @("replacement") }
    if ($null -eq $line) {
        $span = Get-ObjectPropertyValue -Object $Entry -Names @("span")
        $start = $null
        if ($null -ne $span) { $start = Get-ObjectPropertyValue -Object $span -Names @("start") }
        if ($null -ne $start) { $line = Get-FirstNonNullValue -Object $start -Names @("line", "row") }
    }

    $detail = [ordered]@{ summary = $SummaryText }
    if ($null -ne $file -and -not [string]::IsNullOrWhiteSpace([string]$file)) { $detail.file = [string]$file }
    if ($null -ne $line) { $detail.line = [int]$line }
    if ($null -ne $functionName -and -not [string]::IsNullOrWhiteSpace([string]$functionName)) { $detail.function = [string]$functionName }
    if ($null -ne $replacement -and -not [string]::IsNullOrWhiteSpace([string]$replacement)) { $detail.replacement = [string]$replacement }
    return [pscustomobject]$detail
}

function Get-MutationAnalysisFromJson {
    param(
        [Parameter(Mandatory = $true)]$Json,
        [Parameter(Mandatory = $true)][double]$MinScore
    )

    $counts = [ordered]@{
        caught   = 0
        missed   = 0
        timeout  = 0
        unviable = 0
        other    = 0
        total    = 0
    }
    $missedDetails = @()
    $unrecognizedSummaries = @()

    $hasOutcomesProp = ($null -ne $Json -and ($Json.PSObject.Properties.Name -contains "outcomes"))
    $outcomesArray = @()
    $outcomesArrayPresent = $false
    if ($hasOutcomesProp) {
        $rawOutcomes = $Json.outcomes
        if ($null -ne $rawOutcomes) {
            if ($rawOutcomes -is [System.Array]) { $outcomesArray = @($rawOutcomes) }
            else { $outcomesArray = @($rawOutcomes) }
        } else {
            $outcomesArray = @()
        }
        $outcomesArrayPresent = ($outcomesArray.Count -gt 0 -or $hasOutcomesProp)
    } elseif ($Json -is [System.Array]) {
        $outcomesArray = @($Json)
        $outcomesArrayPresent = $true
    }

    if ($outcomesArrayPresent) {
        foreach ($entry in $outcomesArray) {
            $scenario = Get-ObjectPropertyValue -Object $entry -Names @("scenario")
            if (Test-IsBaselineScenario -Scenario $scenario) {
                continue
            }

            $counts.total++
            $summaryText = Get-MutantOutcomeSummaryText -Entry $entry
            $bucket = Get-NormalizedOutcomeKey -RawSummary $summaryText
            switch ($bucket) {
                "caught"   { $counts.caught++ }
                "missed"   {
                    $counts.missed++
                    $missedDetails += (Get-ConciseMutantDetail -Entry $entry -SummaryText $summaryText)
                }
                "timeout"  {
                    $counts.timeout++
                    $missedDetails += (Get-ConciseMutantDetail -Entry $entry -SummaryText $summaryText)
                }
                "unviable" { $counts.unviable++ }
                default    {
                    $counts.other++
                    $label = $summaryText
                    if ([string]::IsNullOrWhiteSpace($label)) { $label = "<empty>" }
                    $unrecognizedSummaries += $label
                }
            }
        }

        if ($counts.other -gt 0) {
            $distinct = @($unrecognizedSummaries | Sort-Object -Unique)
            throw ("Unrecognized cargo-mutants outcomes.json schema or mutant summary value(s): " + ($distinct -join ", ") + ". Schema drift must not silently score 0.")
        }
    } else {
        $caughtVal = Get-ObjectPropertyValue -Object $Json -Names @("caught")
        $missedVal = Get-ObjectPropertyValue -Object $Json -Names @("missed")
        $timeoutVal = Get-ObjectPropertyValue -Object $Json -Names @("timeout")
        $unviableVal = Get-ObjectPropertyValue -Object $Json -Names @("unviable")
        $totalMutantsVal = Get-ObjectPropertyValue -Object $Json -Names @("total_mutants", "totalMutants", "total")

        $hasTopLevelCounters = ($null -ne $caughtVal -or $null -ne $missedVal -or $null -ne $timeoutVal -or $null -ne $unviableVal -or $null -ne $totalMutantsVal)
        if (-not $hasTopLevelCounters) {
            throw "Unrecognized cargo-mutants outcomes.json schema: expected '.outcomes' array or top-level numeric summary counters (caught/missed/timeout/unviable/total_mutants)."
        }

        $counts.caught = if ($null -ne $caughtVal) { [int]$caughtVal } else { 0 }
        $counts.missed = if ($null -ne $missedVal) { [int]$missedVal } else { 0 }
        $counts.timeout = if ($null -ne $timeoutVal) { [int]$timeoutVal } else { 0 }
        $counts.unviable = if ($null -ne $unviableVal) { [int]$unviableVal } else { 0 }
        $counts.other = 0
        $sum = $counts.caught + $counts.missed + $counts.timeout + $counts.unviable
        if ($null -ne $totalMutantsVal) {
            $counts.total = [Math]::Max([int]$totalMutantsVal, $sum)
        } else {
            $counts.total = $sum
        }
    }

    [int]$viable = $counts.caught + $counts.missed + $counts.timeout
    $score = 0.0
    $passed = $false
    if ($viable -gt 0) {
        $score = [math]::Round((($counts.caught / [double]$viable) * 100.0), 2)
        $passed = ($score -ge $MinScore)
    }

    return [pscustomobject]@{
        Counts        = $counts
        MissedDetails = @($missedDetails)
        Viable        = $viable
        Score         = $score
        Passed        = $passed
    }
}

function Get-MutationAnalysisFromFile {
    param(
        [Parameter(Mandatory = $true)][string]$OutcomesPath,
        [Parameter(Mandatory = $true)][double]$MinScore
    )
    $raw = Get-Content -LiteralPath $OutcomesPath -Raw -Encoding UTF8
    $json = $raw | ConvertFrom-Json
    return Get-MutationAnalysisFromJson -Json $json -MinScore $MinScore
}

function Invoke-MutationSelfTest {
    param([Parameter(Mandatory = $true)][double]$MinScore)

    $tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("sweeper-cargo-mutants-selftest-" + [Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Force -Path $tempRoot | Out-Null
    try {
        Write-Step "Running cargo-mutants parser/threshold self-tests (-SelfTest)"

        # 1) Real outcomes format with Baseline + CaughtMutant/MissedMutant/Timeout/Unviable; score threshold pass/fail
        $outcomesCase1 = [ordered]@{
            outcomes = @(
                [ordered]@{ scenario = "Baseline"; summary = "Success" },
                [ordered]@{ scenario = [ordered]@{ Mutant = [ordered]@{ file = "backend/sweeper-core/src/policy.rs"; function = @{ function_name = "a" }; span = @{ start = @{ line = 10 } }; replacement = "true" } }; summary = "CaughtMutant" },
                [ordered]@{ scenario = [ordered]@{ Mutant = [ordered]@{ file = "backend/sweeper-core/src/policy.rs"; function = @{ function_name = "b" }; span = @{ start = @{ line = 20 } }; replacement = "false" } }; summary = "CaughtMutant" },
                [ordered]@{ scenario = [ordered]@{ Mutant = [ordered]@{ file = "backend/sweeper-core/src/paths.rs"; function = @{ function_name = "c" }; span = @{ start = @{ line = 30 } }; replacement = "return 0" } }; summary = "MissedMutant" },
                [ordered]@{ scenario = [ordered]@{ Mutant = [ordered]@{ file = "backend/sweeper-core/src/paths.rs"; function = @{ function_name = "d" }; span = @{ start = @{ line = 40 } }; replacement = "loop {}" } }; summary = "Timeout" },
                [ordered]@{ scenario = [ordered]@{ Mutant = [ordered]@{ file = "backend/sweeper-core/src/paths.rs"; function = @{ function_name = "e" }; span = @{ start = @{ line = 50 } }; replacement = "()" } }; summary = "Unviable" }
            )
        }
        $case1Path = Join-Path $tempRoot "case1.json"
        $outcomesCase1 | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $case1Path -Encoding UTF8
        $r1 = Get-MutationAnalysisFromFile -OutcomesPath $case1Path -MinScore 95.0
        if ($r1.Counts.caught -ne 2 -or $r1.Counts.missed -ne 1 -or $r1.Counts.timeout -ne 1 -or $r1.Counts.unviable -ne 1 -or $r1.Counts.total -ne 5) {
            throw "SelfTest failed (case 1 counts): expected caught=2 missed=1 timeout=1 unviable=1 total=5, got $($r1.Counts | ConvertTo-Json -Compress)"
        }
        if ($r1.Score -ne 50.0 -or $r1.Passed) {
            throw "SelfTest failed (case 1 threshold): expected score 50.0 and passed=false at 95%, got score=$($r1.Score) passed=$($r1.Passed)"
        }
        $r1Pass = Get-MutationAnalysisFromFile -OutcomesPath $case1Path -MinScore 50.0
        if (-not $r1Pass.Passed) {
            throw "SelfTest failed (case 1 pass threshold): expected passed=true at 50%"
        }
        if (@($r1.MissedDetails).Count -ne 2) {
            throw "SelfTest failed (case 1 missed details): expected 2 concise missed/timeout entries, got $(@($r1.MissedDetails).Count)"
        }
        foreach ($d in @($r1.MissedDetails)) {
            if ([string]::IsNullOrWhiteSpace([string]$d.summary) -or [string]::IsNullOrWhiteSpace([string]$d.file)) {
                throw "SelfTest failed (case 1 concise details): missing summary/file in $($d | ConvertTo-Json -Compress)"
            }
        }

        # 2) Top-level aggregate counters format
        $case2Path = Join-Path $tempRoot "case2.json"
        @{ caught = 19; missed = 1; timeout = 0; unviable = 3; total_mutants = 23 } | ConvertTo-Json | Set-Content -LiteralPath $case2Path -Encoding UTF8
        $r2 = Get-MutationAnalysisFromFile -OutcomesPath $case2Path -MinScore 95.0
        if ($r2.Counts.caught -ne 19 -or $r2.Counts.missed -ne 1 -or $r2.Counts.timeout -ne 0 -or $r2.Counts.unviable -ne 3 -or $r2.Counts.total -ne 23) {
            throw "SelfTest failed (case 2 counters): got $($r2.Counts | ConvertTo-Json -Compress)"
        }
        if ($r2.Score -ne 95.0 -or -not $r2.Passed) {
            throw "SelfTest failed (case 2 score): expected 95.0/pass, got $($r2.Score)/$($r2.Passed)"
        }

        # 3) Lowercase format (caught/missed)
        $case3Path = Join-Path $tempRoot "case3.json"
        @{ outcomes = @(
            @{ scenario = "baseline"; summary = "success" },
            @{ scenario = @{ Mutant = @{ file = "backend/sweeper-core/src/policy.rs"; function = @{ function_name = "x" }; span = @{ start = @{ line = 1 } } } }; summary = "caught" },
            @{ scenario = @{ Mutant = @{ file = "backend/sweeper-core/src/paths.rs"; function = @{ function_name = "y" }; span = @{ start = @{ line = 2 } } } }; summary = "missed" }
        ) } | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath $case3Path -Encoding UTF8
        $r3 = Get-MutationAnalysisFromFile -OutcomesPath $case3Path -MinScore 95.0
        if ($r3.Counts.caught -ne 1 -or $r3.Counts.missed -ne 1 -or $r3.Counts.total -ne 2) {
            throw "SelfTest failed (case 3 lowercase): got $($r3.Counts | ConvertTo-Json -Compress)"
        }

        # 4) viable == 0 fails cleanly
        $case4Path = Join-Path $tempRoot "case4.json"
        @{ outcomes = @(
            @{ scenario = "Baseline"; summary = "Success" },
            @{ scenario = @{ Mutant = @{ file = "backend/sweeper-core/src/policy.rs" } }; summary = "Unviable" }
        ) } | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath $case4Path -Encoding UTF8
        $r4 = Get-MutationAnalysisFromFile -OutcomesPath $case4Path -MinScore 95.0
        if ($r4.Viable -ne 0 -or $r4.Passed) {
            throw "SelfTest failed (case 4 viable==0): expected viable 0 and passed false, got viable=$($r4.Viable) passed=$($r4.Passed)"
        }

        # 5) Unrecognized mutant summary string fails with clear diagnostic
        $case5Path = Join-Path $tempRoot "case5.json"
        @{ outcomes = @(
            @{ scenario = "Baseline"; summary = "Success" },
            @{ scenario = @{ Mutant = @{ file = "backend/sweeper-core/src/policy.rs" } }; summary = "WeirdStatus" }
        ) } | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath $case5Path -Encoding UTF8
        $threw = $false
        $errMsg = ""
        try {
            Get-MutationAnalysisFromFile -OutcomesPath $case5Path -MinScore 95.0 | Out-Null
        } catch {
            $threw = $true
            $errMsg = $_.Exception.Message
        }
        if (-not $threw -or ($errMsg -notmatch "Unrecognized") -or ($errMsg -notmatch "WeirdStatus")) {
            throw "SelfTest failed (case 5 unrecognized summary): expected clear diagnostic mentioning Unrecognized and WeirdStatus, got threw=$threw msg='$errMsg'"
        }

        Write-Host "SelfTest passed: parser normalization, schema handling, thresholding, viable==0, and unrecognized-summary diagnostics." -ForegroundColor Green
    } finally {
        Remove-Item -LiteralPath $tempRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}

if ($SelfTest) {
    Invoke-MutationSelfTest -MinScore $MinScore
    exit 0
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
$filters = @("backend/sweeper-core/src/policy.rs", "backend/sweeper-core/src/paths.rs")
$testArgs = @("--locked", "--lib", "--test", "gate_b_proptest", "--test", "gate_c_adversarial", "--test", "gate_c_ntfs", "--test", "settings_tier3")

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
    "--no-shuffle",
    "--timeout-multiplier", "5",
    "--minimum-test-timeout", "30"
)
foreach ($f in $filters) {
    $mutantsArgs += @("--file", $f)
}
$mutantsArgs += "--"
$mutantsArgs += $testArgs

Write-Step "Running cargo-mutants on sweeper-core policy.rs and paths.rs (restricted test suites)"
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

$parsed = Get-MutationAnalysisFromFile -OutcomesPath $outcomesPath -MinScore $MinScore
$counts = $parsed.Counts
$missedDetails = @($parsed.MissedDetails)

[int]$caught = $counts.caught
[int]$missed = $counts.missed
[int]$timeout = $counts.timeout
[int]$unviable = $counts.unviable
[int]$other = $counts.other
[int]$total = $counts.total
[int]$viable = $parsed.Viable
[double]$score = $parsed.Score
[bool]$passed = $parsed.Passed

if ($missedDetails.Count -gt 0) {
    Write-Host ""
    Write-Warning "Missed/timeout mutants detected:"
    foreach ($m in $missedDetails) {
        $mSummary = [string](Get-ObjectPropertyValue -Object $m -Names @("summary"))
        $mFile = [string](Get-ObjectPropertyValue -Object $m -Names @("file"))
        $mLine = Get-ObjectPropertyValue -Object $m -Names @("line")
        $mFunc = [string](Get-ObjectPropertyValue -Object $m -Names @("function"))
        $mRepl = [string](Get-ObjectPropertyValue -Object $m -Names @("replacement"))
        Write-Host (" - summary={0} file={1} line={2} function={3} replacement={4}" -f $mSummary, $mFile, $mLine, $mFunc, $mRepl)
    }
}

$summary = [ordered]@{
    timestampUtc        = [DateTime]::UtcNow.ToString("o")
    package             = "sweeper-core"
    targets             = @("backend/sweeper-core/src/policy.rs", "backend/sweeper-core/src/paths.rs")
    filters             = $filters
    testArgs            = $testArgs
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
