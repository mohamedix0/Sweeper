<#
.SYNOPSIS
    Verifies (and optionally installs) every tool required by Sweeper on a freshly relocated machine.

.DESCRIPTION
    Read-first companion to docs/TRANSITION.md. Check mode is the default and is read-only:
    it prints OK / MISSING with an exact remediation command for each dependency.
    With -Install it attempts non-interactive installs for the safe, well-known cases
    (corepack/pnpm, rustup components, cargo tauri-cli). Tools that require a human
    (GitHub auth, Tauri/WebView2/NSIS system prerequisites, Edge install) are reported
    with instructions and are never silently assumed to be present.

    Never writes secrets, never modifies repository files, never runs the coderabbit CLI.

.PARAMETER Install
    Attempt non-interactive installation for tools that support it.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts/bootstrap-new-machine.ps1
    powershell -ExecutionPolicy Bypass -File scripts/bootstrap-new-machine.ps1 -Install
#>
param([switch]$Install)

$ErrorActionPreference = "Continue"
$script:RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$script:Failures = 0

function Write-Check {
    param([string]$Name, [bool]$Ok, [string]$Detail, [string]$Remediation)
    if ($Ok) {
        Write-Host "OK      $Name - $Detail"
    } else {
        Write-Host "MISSING $Name - $Detail"
        if ($Remediation) { Write-Host ("        -> " + $Remediation) }
        $script:Failures++
    }
}

function Get-VersionOrNull {
    param([string]$Command, [string[]]$Arguments)
    try {
        $out = & $Command @Arguments 2>&1
        if ($LASTEXITCODE -ne 0) { return $null }
        return (($out | Select-Object -First 1) -join '').Trim()
    } catch { return $null }
}

function Get-NumericVersion {
    param([string]$Text)
    $match = [regex]::Match($Text, '\d+\.\d+(?:\.\d+)?')
    if (-not $match.Success) { return $null }
    try { return [version]$match.Value } catch { return $null }
}

function Test-ExactVersion {
    param([string]$Text, [string]$Expected)
    if ([string]::IsNullOrEmpty($Text)) { return $false }
    $version = Get-NumericVersion -Text $Text
    return ($null -ne $version -and $version -eq [version]$Expected)
}

function Test-MinimumMajorVersion {
    param([string]$Text, [int]$MinimumMajor)
    if ([string]::IsNullOrEmpty($Text)) { return $false }
    $version = Get-NumericVersion -Text $Text
    return ($null -ne $version -and $version.Major -ge $MinimumMajor)
}

function Test-MajorVersion {
    param([string]$Text, [int]$Major)
    if ([string]::IsNullOrEmpty($Text)) { return $false }
    $version = Get-NumericVersion -Text $Text
    return ($null -ne $version -and $version.Major -eq $Major)
}

function Test-GitIdentityOk {
    param([string]$Email, [string]$Name)
    if ($Email -notmatch '^[A-Za-z0-9._%+-]+@users\.noreply\.github\.com$') { return $false }
    if ([string]::IsNullOrWhiteSpace($Name)) { return $false }
    return ($Name -notmatch '\\|C:|D:')
}

function Test-RustToolchainOk {
    param([string]$Rustc, [string]$Cargo)
    if (-not (Test-ExactVersion -Text $Rustc -Expected '1.99.1')) { return $false }
    return (Test-ExactVersion -Text $Cargo -Expected '1.99.1')
}

function Get-GitConfigValue {
    param([string]$Key)
    $raw = & git config $Key 2>$null
    if ($null -eq $raw) { return '' }
    return ([string]$raw).Trim()
}

function Get-FirstExistingPath {
    param([string[]]$Paths)
    foreach ($candidate in $Paths) {
        if (Test-Path -LiteralPath $candidate) { return $candidate }
    }
    return $null
}

function Invoke-TryInstall {
    param([string]$Label, [scriptblock]$Action, [switch]$Install)
    if (-not $Install) { return }
    try {
        Write-Host "INSTALL $Label ..."
        & $Action
        if ($LASTEXITCODE -ne 0) { Write-Host "        install failed for $Label (exit $LASTEXITCODE)" }
    } catch {
        Write-Host "        install failed for $Label : $($_.Exception.Message)"
    }
}

function Write-Banner {
    Write-Host "Sweeper new-machine bootstrap (repo: $($script:RepoRoot))"
    $modeMessages = @('CHECK ONLY (use -Install to attempt fixes)', 'CHECK + INSTALL')
    Write-Host "Mode: $($modeMessages[[int][bool]$Install])"
    Write-Host ""
}

function Test-ScmTools {
    $git = Get-VersionOrNull -Command 'git' -Arguments @('--version')
    Write-Check -Name 'git' -Ok ($null -ne $git) -Detail $git -Remediation 'Install Git for Windows: https://gitforwindows.org/'

    $ghVer = Get-VersionOrNull -Command 'gh' -Arguments @('--version')
    Write-Check -Name 'gh (GitHub CLI)' -Ok ($null -ne $ghVer) -Detail $ghVer -Remediation 'winget install --id GitHub.cli'
    if ($null -ne $ghVer) {
        $auth = & gh auth status 2>&1
        $authOk = ($LASTEXITCODE -eq 0) -and (($auth | Out-String) -match 'Logged in')
        $authDetail = 'not authenticated'
        if ($authOk) { $authDetail = 'logged in' }
        Write-Check -Name 'gh authentication' -Ok $authOk -Detail $authDetail -Remediation 'gh auth login (needs a human with credentials)'
    }
}

function Test-GitIdentity {
    $email = Get-GitConfigValue -Key 'user.email'
    $name = Get-GitConfigValue -Key 'user.name'
    $identityOk = Test-GitIdentityOk -Email $email -Name $name
    Write-Check -Name 'machine-neutral git identity' -Ok $identityOk -Detail "email=$email name=$name" -Remediation 'git config user.email "<id>+<user>@users.noreply.github.com"; git config user.name "Sweeper Agent"'
}

function Test-RustToolchain {
    $rustc = Get-VersionOrNull -Command 'rustc' -Arguments @('--version')
    $cargo = Get-VersionOrNull -Command 'cargo' -Arguments @('--version')
    $rustOk = Test-RustToolchainOk -Rustc $rustc -Cargo $cargo
    Write-Check -Name 'Rust toolchain' -Ok $rustOk -Detail "rustc=$rustc cargo=$cargo (CI pins 1.99.1)" -Remediation 'rustup toolchain install 1.99.1 && rustup default 1.99.1'
    if ($null -ne $cargo) {
        $fmt = Get-VersionOrNull -Command 'cargo' -Arguments @('fmt','--version')
        $clippy = & cargo clippy --version 2>&1
        Write-Check -Name 'rustfmt + clippy' -Ok ($null -ne $fmt -and $LASTEXITCODE -eq 0) -Detail "fmt=$fmt" -Remediation 'rustup component add rustfmt clippy'
        Invoke-TryInstall -Label 'rustfmt/clippy' -Action { rustup component add rustfmt clippy } -Install:$Install
    }
}

function Test-TauriCli {
    $cargo = Get-VersionOrNull -Command 'cargo' -Arguments @('--version')
    $tauri = $null
    if ($null -ne $cargo) { $tauri = Get-VersionOrNull -Command 'cargo' -Arguments @('tauri','--version') }
    Write-Check -Name 'Tauri CLI' -Ok ($null -ne $tauri) -Detail $tauri -Remediation 'cargo install tauri-cli --locked'
    Invoke-TryInstall -Label 'tauri-cli' -Action { cargo install tauri-cli --locked } -Install:$Install
}

function Test-Nsis {
    $nsisPaths = @(
        "$env:ProgramFiles\NSIS\makensis.exe",
        "${env:ProgramFiles(x86)}\NSIS\makensis.exe"
    )
    $nsis = Get-FirstExistingPath -Paths $nsisPaths
    $nsisDetail = 'not found'
    if ($null -ne $nsis) { $nsisDetail = $nsis }
    Write-Check -Name 'NSIS (makensis.exe)' -Ok ($null -ne $nsis) -Detail $nsisDetail -Remediation 'winget install --id NSIS.NSIS'
}

function Test-NodePnpm {
    $node = Get-VersionOrNull -Command 'node' -Arguments @('--version')
    $nodeOk = Test-MinimumMajorVersion -Text $node -MinimumMajor 20
    Write-Check -Name 'Node.js' -Ok $nodeOk -Detail $node -Remediation 'winget install OpenJS.NodeJS.LTS'
    $pnpm = Get-VersionOrNull -Command 'pnpm' -Arguments @('--version')
    $pnpmOk = Test-MajorVersion -Text $pnpm -Major 9
    Write-Check -Name 'pnpm' -Ok $pnpmOk -Detail $pnpm -Remediation 'corepack enable; corepack prepare pnpm@9 --activate'
    Invoke-TryInstall -Label 'pnpm via corepack' -Action { corepack enable; corepack prepare pnpm@9 --activate } -Install:$Install
}

function Test-Edge {
    $edgePaths = @(
        "$env:ProgramFiles\Microsoft\Edge\Application\msedge.exe",
        "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe"
    )
    $edge = Get-FirstExistingPath -Paths $edgePaths
    $edgeDetail = 'not found'
    if ($null -ne $edge) { $edgeDetail = $edge }
    Write-Check -Name 'Microsoft Edge (Playwright msedge channel)' -Ok ($null -ne $edge) -Detail $edgeDetail -Remediation 'Install Microsoft Edge; Playwright must use channel "msedge" and must NOT download browsers'
}

function Test-RepoState {
    $target = Join-Path $script:RepoRoot 'target'
    Write-Host "INFO    Cargo target dir must be: $target"
    Write-Host "        set `$env:CARGO_TARGET_DIR = (Join-Path (git rev-parse --show-toplevel) 'target') before any cargo command"
    $transition = Join-Path $script:RepoRoot 'docs\TRANSITION.md'
    Write-Check -Name 'mandatory transition doc' -Ok (Test-Path -LiteralPath $transition) -Detail $transition -Remediation 'docs/TRANSITION.md is missing - restore it from the repository'
}

function Write-CloudGateNotes {
    Write-Host ""
    Write-Host "CLOUD GATES (verify on GitHub, not with local tools):"
    Write-Host "  - CodeRabbit: GitHub App review on PRs. NEVER run the 'coderabbit' CLI locally (AGENTS.md rule)."
    Write-Host "    Trigger on OSS PRs with: gh pr comment <n> --body '@coderabbitai review' when the check is skipped."
    Write-Host "  - CodeScene: PR check 'CodeScene Code Health Review (main)' must be present and green."
    Write-Host "  - GitHub Actions CI: gh pr checks <n> must show gate-a-and-backend, gate-g-frontend, mutation-gate, package-windows all green."
    Write-Host ""
}

function Invoke-Bootstrap {
    Write-Banner
    Test-ScmTools
    Test-GitIdentity
    Test-RustToolchain
    Test-TauriCli
    Test-Nsis
    Test-NodePnpm
    Test-Edge
    Test-RepoState
    Write-CloudGateNotes
}

Invoke-Bootstrap

$resultMessages = @(
    "RESULT: environment ready. You may resume work after re-reading docs/TRANSITION.md fully.",
    "RESULT: $($script:Failures) check(s) missing. Fix them before any autonomous work (docs/TRANSITION.md)."
)
$resultIndex = [int]($script:Failures -gt 0)
Write-Host $resultMessages[$resultIndex]
exit $resultIndex
