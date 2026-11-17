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

function Write-Check {
    param([string]$Name, [bool]$Ok, [string]$Detail, [string]$Remediation, [ref]$FailureCount)
    if ($Ok) {
        Write-Host "OK      $Name - $Detail"
    } else {
        Write-Host "MISSING $Name - $Detail"
        if ($Remediation) { Write-Host ("        -> " + $Remediation) }
        $FailureCount.Value++
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

function Invoke-BootstrapMain {
    param([switch]$Install)

    $RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
    $Failures = 0

    Write-Host "Sweeper new-machine bootstrap (repo: $RepoRoot)"
    Write-Host "Mode: $(if ($Install) { 'CHECK + INSTALL' } else { 'CHECK ONLY (use -Install to attempt fixes)' })"
    Write-Host ""

    # --- git / gh ---------------------------------------------------------------
    $git = Get-VersionOrNull -Command 'git' -Arguments @('--version')
    Write-Check -Name 'git' -Ok ($null -ne $git) -Detail $git -Remediation 'Install Git for Windows: https://gitforwindows.org/' -FailureCount ([ref]$Failures)

    $ghVer = Get-VersionOrNull -Command 'gh' -Arguments @('--version')
    Write-Check -Name 'gh (GitHub CLI)' -Ok ($null -ne $ghVer) -Detail $ghVer -Remediation 'winget install --id GitHub.cli' -FailureCount ([ref]$Failures)
    if ($null -ne $ghVer) {
        $auth = & gh auth status 2>&1
        $authOk = ($LASTEXITCODE -eq 0) -and (($auth | Out-String) -match 'Logged in')
        $authDetail = 'not authenticated'
        if ($authOk) { $authDetail = 'logged in' }
        Write-Check -Name 'gh authentication' -Ok $authOk -Detail $authDetail -Remediation 'gh auth login (needs a human with credentials)' -FailureCount ([ref]$Failures)
    }

    $email = (& git config user.email 2>$null).Trim()
    $name  = (& git config user.name 2>$null).Trim()
    $identityOk = ($email -match 'users\.noreply\.github\.com') -and ($name -notmatch '\\|C:|D:')
    Write-Check -Name 'machine-neutral git identity' -Ok $identityOk -Detail "email=$email name=$name" -Remediation 'git config user.email "<id>+<user>@users.noreply.github.com"; git config user.name "Sweeper Agent"' -FailureCount ([ref]$Failures)

    # --- Rust -------------------------------------------------------------------
    $rustc = Get-VersionOrNull -Command 'rustc' -Arguments @('--version')
    $cargo = Get-VersionOrNull -Command 'cargo' -Arguments @('--version')
    Write-Check -Name 'Rust toolchain' -Ok ($null -ne $rustc -and $null -ne $cargo) -Detail "rustc=$rustc cargo=$cargo (CI pins 1.99.1)" -Remediation 'rustup toolchain install 1.99.1 && rustup default 1.99.1' -FailureCount ([ref]$Failures)
    if ($null -ne $cargo) {
        $fmt = Get-VersionOrNull -Command 'cargo' -Arguments @('fmt','--version')
        $clippy = & cargo clippy --version 2>&1
        Write-Check -Name 'rustfmt + clippy' -Ok ($null -ne $fmt -and $LASTEXITCODE -eq 0) -Detail "fmt=$fmt" -Remediation 'rustup component add rustfmt clippy' -FailureCount ([ref]$Failures)
        Invoke-TryInstall -Label 'rustfmt/clippy' -Action { rustup component add rustfmt clippy } -Install:$Install
    }
    $tauri = $null
    if ($null -ne $cargo) { $tauri = Get-VersionOrNull -Command 'cargo' -Arguments @('tauri','--version') }
    Write-Check -Name 'Tauri CLI' -Ok ($null -ne $tauri) -Detail $tauri -Remediation 'cargo install tauri-cli --locked' -FailureCount ([ref]$Failures)
    Invoke-TryInstall -Label 'tauri-cli' -Action { cargo install tauri-cli --locked } -Install:$Install

    # --- Node / pnpm ------------------------------------------------------------
    $node = Get-VersionOrNull -Command 'node' -Arguments @('--version')
    Write-Check -Name 'Node.js' -Ok ($null -ne $node) -Detail $node -Remediation 'winget install OpenJS.NodeJS.LTS' -FailureCount ([ref]$Failures)
    $pnpm = Get-VersionOrNull -Command 'pnpm' -Arguments @('--version')
    Write-Check -Name 'pnpm' -Ok ($null -ne $pnpm) -Detail $pnpm -Remediation 'corepack enable; corepack prepare pnpm@9 --activate' -FailureCount ([ref]$Failures)
    Invoke-TryInstall -Label 'pnpm via corepack' -Action { corepack enable; corepack prepare pnpm@9 --activate } -Install:$Install

    # --- Edge for Playwright ----------------------------------------------------
    $edgePaths = @(
      "$env:ProgramFiles\Microsoft\Edge\Application\msedge.exe",
      "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe"
    )
    $edge = $edgePaths | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    $edgeDetail = 'not found'
    if ($null -ne $edge) { $edgeDetail = $edge }
    Write-Check -Name 'Microsoft Edge (Playwright msedge channel)' -Ok ($null -ne $edge) -Detail $edgeDetail -Remediation 'Install Microsoft Edge; Playwright must use channel "msedge" and must NOT download browsers' -FailureCount ([ref]$Failures)

    # --- repository-local state -------------------------------------------------
    $target = Join-Path $RepoRoot 'target'
    Write-Host "INFO    Cargo target dir must be: $target"
    Write-Host "        set `$env:CARGO_TARGET_DIR = (Join-Path (git rev-parse --show-toplevel) 'target') before any cargo command"
    $transition = Join-Path $RepoRoot 'docs\TRANSITION.md'
    Write-Check -Name 'mandatory transition doc' -Ok (Test-Path -LiteralPath $transition) -Detail $transition -Remediation 'docs/TRANSITION.md is missing - restore it from the repository' -FailureCount ([ref]$Failures)

    # --- GitHub-side gates (human/cloud, never local CLI) -----------------------
    Write-Host ""
    Write-Host "CLOUD GATES (verify on GitHub, not with local tools):"
    Write-Host "  - CodeRabbit: GitHub App review on PRs. NEVER run the 'coderabbit' CLI locally (AGENTS.md rule)."
    Write-Host "    Trigger on OSS PRs with: gh pr comment <n> --body '@coderabbitai review' when the check is skipped."
    Write-Host "  - CodeScene: PR check 'CodeScene Code Health Review (main)' must be present and green."
    Write-Host "  - GitHub Actions CI: gh pr checks <n> must show gate-a-and-backend, gate-g-frontend, mutation-gate, package-windows all green."
    Write-Host ""

    if ($Failures -gt 0) {
        Write-Host "RESULT: $Failures check(s) missing. Fix them before any autonomous work (docs/TRANSITION.md)."
        exit 1
    }
    Write-Host "RESULT: environment ready. You may resume work after re-reading docs/TRANSITION.md fully."
    exit 0
}

Invoke-BootstrapMain -Install:$Install
