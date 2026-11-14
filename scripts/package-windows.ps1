[CmdletBinding()]
param(
    [string]$OutputDir = "",
    [switch]$SkipFrontendInstall,
    [switch]$StandaloneOnly
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Write-Step([string]$Message) {
    Write-Host "==> $Message" -ForegroundColor Cyan
}

function Invoke-Checked {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [Parameter()][string[]]$Arguments = @(),
        [Parameter()][string]$ErrorMessage = "Command failed"
    )
    & $FilePath @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$ErrorMessage (exit code $LASTEXITCODE): $FilePath $($Arguments -join ' ')"
    }
}

function Resolve-RepoRoot {
    return (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
}

function Get-CargoTauriCommand {
    if (Get-Command cargo-tauri -ErrorAction SilentlyContinue) {
        return @{ FilePath = "cargo-tauri"; PrefixArgs = @() }
    }
    & cargo tauri --version *> $null
    if ($LASTEXITCODE -eq 0) {
        return @{ FilePath = "cargo"; PrefixArgs = @("tauri") }
    }
    return $null
}

function Get-MakensisPath {
    $cmd = Get-Command makensis -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    $candidates = @(
        "C:\Program Files (x86)\NSIS\makensis.exe",
        "C:\Program Files\NSIS\makensis.exe"
    )
    foreach ($c in $candidates) {
        if (Test-Path -LiteralPath $c) { return $c }
    }
    return $null
}

function Find-SignTool {
    $cmd = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    $kitsRoot = "C:\Program Files (x86)\Windows Kits\10\bin"
    if (Test-Path -LiteralPath $kitsRoot) {
        $found = Get-ChildItem -LiteralPath $kitsRoot -Recurse -Filter signtool.exe -File -ErrorAction SilentlyContinue |
            Where-Object { $_.FullName -match "\\x64\\signtool\.exe$" } |
            Sort-Object FullName -Descending |
            Select-Object -First 1
        if ($found) { return $found.FullName }
        $foundAny = Get-ChildItem -LiteralPath $kitsRoot -Recurse -Filter signtool.exe -File -ErrorAction SilentlyContinue |
            Sort-Object FullName -Descending |
            Select-Object -First 1
        if ($foundAny) { return $foundAny.FullName }
    }
    return $null
}

function Get-TauriConf([string]$RepoRoot) {
    $confPath = Join-Path $RepoRoot "app\bridge\sweeper-desktop\tauri.conf.json"
    if (-not (Test-Path -LiteralPath $confPath)) {
        throw "Tauri config not found: $confPath"
    }
    $conf = Get-Content -LiteralPath $confPath -Raw | ConvertFrom-Json
    return [pscustomobject]@{ Path = $confPath; Data = $conf }
}

function Copy-IfExists {
    param([string]$Source, [string]$DestinationDir, [string]$DestinationName)
    if (-not (Test-Path -LiteralPath $Source)) { return $null }
    $dest = Join-Path $DestinationDir $DestinationName
    Copy-Item -LiteralPath $Source -Destination $dest -Force
    return $dest
}

function Get-SigningConfig {
    $pfx = [Environment]::GetEnvironmentVariable("SWEEPER_CERT_PFX_PATH")
    $pfxPwd = [Environment]::GetEnvironmentVariable("SWEEPER_CERT_PASSWORD")
    $thumb = [Environment]::GetEnvironmentVariable("SWEEPER_CERT_THUMBPRINT")
    $ts = [Environment]::GetEnvironmentVariable("SWEEPER_TIMESTAMP_URL")
    $hasPfx = -not [string]::IsNullOrWhiteSpace($pfx)
    $hasThumb = -not [string]::IsNullOrWhiteSpace($thumb)
    if (-not $hasPfx -and -not $hasThumb) {
        return @{ Enabled = $false; Reason = "Signing credentials not configured; skipping Authenticode signing." }
    }
    if ($hasPfx -and -not (Test-Path -LiteralPath $pfx)) {
        throw "SWEEPER_CERT_PFX_PATH is set but file does not exist: $pfx"
    }
    $signtool = Find-SignTool
    if (-not $signtool) {
        throw "Signing requested but signtool.exe was not found on PATH or in Windows Kits."
    }
    return @{
        Enabled = $true
        SignTool = $signtool
        PfxPath = $pfx
        PfxPassword = $pfxPwd
        Thumbprint = $thumb
        TimestampUrl = $ts
    }
}

function Invoke-SignExe {
    param([string]$ExePath, [hashtable]$Config)
    $args = @("sign", "/fd", "SHA256")
    if (-not [string]::IsNullOrWhiteSpace($Config.TimestampUrl)) {
        $args += @("/tr", $Config.TimestampUrl, "/td", "SHA256")
    }
    if (-not [string]::IsNullOrWhiteSpace($Config.Thumbprint)) {
        $args += @("/sha1", $Config.Thumbprint)
    } elseif (-not [string]::IsNullOrWhiteSpace($Config.PfxPath)) {
        $args += @("/f", $Config.PfxPath)
        if (-not [string]::IsNullOrWhiteSpace($Config.PfxPassword)) {
            $args += @("/p", $Config.PfxPassword)
        }
    } else {
        throw "Signing configuration missing certificate selector."
    }
    $args += $ExePath
    Invoke-Checked -FilePath $Config.SignTool -Arguments $args -ErrorMessage "signtool sign failed for $ExePath"
    Invoke-Checked -FilePath $Config.SignTool -Arguments @("verify", "/pa", $ExePath) -ErrorMessage "signtool verify failed for $ExePath"
}

function Write-Sha256Sums {
    param([string]$Directory)
    $exeFiles = @(Get-ChildItem -LiteralPath $Directory -File -Filter *.exe | Sort-Object Name)
    if ($exeFiles.Count -eq 0) {
        throw "No .exe artifacts found in output directory for checksum generation: $Directory"
    }
    $lines = foreach ($f in $exeFiles) {
        $hash = (Get-FileHash -LiteralPath $f.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        "$hash  $($f.Name)"
    }
    $content = ($lines -join "`n") + "`n"
    $shaPath = Join-Path $Directory "SHA256SUMS.txt"
    $utf8NoBom = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText($shaPath, $content, $utf8NoBom)
    return $shaPath
}

function Verify-Sha256Sums {
    param([string]$Directory, [string]$ShaPath)
    $shaLines = @(Get-Content -LiteralPath $ShaPath -Encoding UTF8)
    $expected = @{}
    foreach ($line in $shaLines) {
        if ([string]::IsNullOrWhiteSpace($line)) { continue }
        $m = [regex]::Match($line, '^([0-9a-f]{64})  (.+)$')
        if (-not $m.Success) {
            throw "Invalid SHA256SUMS.txt line format: $line"
        }
        $expected[$m.Groups[2].Value] = $m.Groups[1].Value
    }
    $exeFiles = @(Get-ChildItem -LiteralPath $Directory -File -Filter *.exe | Sort-Object Name)
    if ($expected.Count -ne $exeFiles.Count) {
        throw "SHA256SUMS.txt entry count ($($expected.Count)) does not match .exe artifact count ($($exeFiles.Count))."
    }
    foreach ($f in $exeFiles) {
        if (-not $expected.ContainsKey($f.Name)) {
            throw "SHA256SUMS.txt missing entry for artifact: $($f.Name)"
        }
        $actual = (Get-FileHash -LiteralPath $f.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actual -ne $expected[$f.Name]) {
            throw "Checksum mismatch for $($f.Name): expected $($expected[$f.Name]) actual $actual"
        }
    }
    foreach ($name in $expected.Keys) {
        $path = Join-Path $Directory $name
        if (-not (Test-Path -LiteralPath $path)) {
            throw "SHA256SUMS.txt lists missing file: $name"
        }
    }
}

$repoRoot = Resolve-RepoRoot
if ([string]::IsNullOrWhiteSpace($OutputDir)) {
    $OutputDir = Join-Path $repoRoot "target\release-artifacts\windows"
}

if ([string]::IsNullOrWhiteSpace($env:CARGO_TARGET_DIR)) {
    $env:CARGO_TARGET_DIR = Join-Path $repoRoot "target"
}
$env:CARGO_TARGET_DIR = [System.IO.Path]::GetFullPath($env:CARGO_TARGET_DIR)

$manifestPath = Join-Path $repoRoot "app\Cargo.toml"
$frontendDir = Join-Path $repoRoot "app\frontend"
$desktopDir = Join-Path $repoRoot "app\bridge\sweeper-desktop"
$tauri = Get-TauriConf -RepoRoot $repoRoot
$productName = [string]$tauri.Data.productName
$version = [string]$tauri.Data.version
if ([string]::IsNullOrWhiteSpace($productName) -or [string]::IsNullOrWhiteSpace($version)) {
    throw "Missing productName or version in $($tauri.Path)"
}

Write-Step "Repository root: $repoRoot"
Write-Step "CARGO_TARGET_DIR: $env:CARGO_TARGET_DIR"
Write-Step "Output directory: $OutputDir"

if (-not $SkipFrontendInstall) {
    Write-Step "Building frontend (pnpm install --frozen-lockfile && pnpm run build)"
    Push-Location $repoRoot
    try {
        Invoke-Checked -FilePath "pnpm" -Arguments @("--dir", $frontendDir, "install", "--frozen-lockfile") -ErrorMessage "pnpm install failed"
        Invoke-Checked -FilePath "pnpm" -Arguments @("--dir", $frontendDir, "run", "build") -ErrorMessage "pnpm build failed"
    } finally {
        Pop-Location
    }
} else {
    Write-Step "Skipping frontend install/build steps (-SkipFrontendInstall)"
    $distIndex = Join-Path $frontendDir "dist\index.html"
    if (-not (Test-Path -LiteralPath $distIndex)) {
        throw "Frontend dist is missing ($distIndex). Run without -SkipFrontendInstall or build frontend first."
    }
}

Write-Step "Building standalone desktop release binary (--locked)"
Invoke-Checked -FilePath "cargo" -Arguments @("build", "--manifest-path", $manifestPath, "--package", "sweeper-desktop", "--release", "--locked") -ErrorMessage "cargo build (sweeper-desktop release) failed"

$tauriCmd = $null
$nsisPath = Get-MakensisPath
if (-not $StandaloneOnly) {
    $tauriCmd = Get-CargoTauriCommand
    if (-not $tauriCmd) {
        throw "Tauri CLI is required for default packaging mode (NSIS installer) but was not found. Install @tauri-apps/cli/cargo-tauri or run with -StandaloneOnly for standalone-only packaging."
    }
    if (-not $nsisPath) {
        throw "NSIS (makensis.exe) is required for default packaging mode but was not found on PATH or in standard install locations. Install NSIS or run with -StandaloneOnly."
    }
    Write-Step "Building NSIS bundle via Tauri CLI (--bundles nsis, --no-sign, --ci)"
    $tauriArgs = @()
    $tauriArgs += $tauriCmd.PrefixArgs
    $tauriArgs += @("build", "--manifest-path", $manifestPath, "--config", (Join-Path $desktopDir "tauri.conf.json"), "--bundles", "nsis", "--no-sign", "--ci")
    Invoke-Checked -FilePath $tauriCmd.FilePath -Arguments $tauriArgs -ErrorMessage "Tauri NSIS bundle build failed"
} else {
    Write-Step "Standalone-only mode requested (-StandaloneOnly): skipping NSIS bundle generation"
    if (-not $tauriCmd) { Write-Host "Tauri CLI: not detected (not required in -StandaloneOnly mode)." }
    if (-not $nsisPath) { Write-Host "NSIS (makensis): not detected (not required in -StandaloneOnly mode)." }
}

if (Test-Path -LiteralPath $OutputDir) {
    Write-Step "Cleaning output directory: $OutputDir"
    Remove-Item -LiteralPath $OutputDir -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null

$releaseDir = Join-Path $env:CARGO_TARGET_DIR "release"
$bundleNsisDir = Join-Path $env:CARGO_TARGET_DIR "release\bundle\nsis"
$standaloneSrc = Join-Path $releaseDir "sweeper-desktop.exe"
if (-not (Test-Path -LiteralPath $standaloneSrc)) {
    throw "Standalone executable not found after build: $standaloneSrc"
}
$standaloneName = "$productName-$version-windows-x64-standalone.exe"
$copiedStandalone = Copy-IfExists -Source $standaloneSrc -DestinationDir $OutputDir -DestinationName $standaloneName
if (-not $copiedStandalone) { throw "Failed to copy standalone executable to output directory." }
Write-Host "Standalone artifact: $copiedStandalone"

if (-not $StandaloneOnly) {
    if (-not (Test-Path -LiteralPath $bundleNsisDir)) {
        throw "NSIS bundle output directory not found: $bundleNsisDir"
    }
    $nsisInstallers = @(Get-ChildItem -LiteralPath $bundleNsisDir -File -Filter *.exe | Sort-Object LastWriteTime -Descending)
    if (-not $nsisInstallers -or $nsisInstallers.Count -eq 0) {
        throw "No NSIS installer .exe produced in $bundleNsisDir"
    }
    $installerSrc = $nsisInstallers[0].FullName
    $installerName = "$productName-$version-windows-x64-nsis-setup.exe"
    $copiedInstaller = Copy-IfExists -Source $installerSrc -DestinationDir $OutputDir -DestinationName $installerName
    if (-not $copiedInstaller) { throw "Failed to copy NSIS installer to output directory." }
    Write-Host "NSIS installer artifact: $copiedInstaller"
}

$signing = Get-SigningConfig
if ($signing.Enabled) {
    Write-Step "Signing .exe artifacts with signtool and verifying signatures"
    $exeArtifacts = @(Get-ChildItem -LiteralPath $OutputDir -File -Filter *.exe | Sort-Object Name)
    foreach ($exe in $exeArtifacts) {
        Write-Host "Signing: $($exe.FullName)"
        Invoke-SignExe -ExePath $exe.FullName -Config $signing
    }
} else {
    Write-Host $signing.Reason
}

Write-Step "Generating SHA256SUMS.txt (sorted, LF, UTF-8 without BOM) and verifying"
$shaPath = Write-Sha256Sums -Directory $OutputDir
Verify-Sha256Sums -Directory $OutputDir -ShaPath $shaPath
Write-Host "SHA256SUMS.txt: $shaPath"
Write-Host "Packaging completed successfully. Artifacts in: $OutputDir"
exit 0