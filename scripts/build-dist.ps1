[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSVersion.Major -ge 7) {
    $PSNativeCommandUseErrorActionPreference = $true
}

function Assert-Command {
    param([Parameter(Mandatory = $true)][string]$Name)

    if (-not (Get-Command -Name $Name -ErrorAction SilentlyContinue)) {
        throw "Required command is not available on PATH: $Name"
    }
}

function Invoke-Checked {
    param(
        [Parameter(Mandatory = $true)][string]$Command,
        [Parameter(Mandatory = $true)][string[]]$Arguments
    )

    Write-Host "> $Command $($Arguments -join ' ')"
    & $Command @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "Command failed with exit code ${LASTEXITCODE}: $Command"
    }
}

if ([System.Environment]::OSVersion.Platform -ne [System.PlatformID]::Win32NT) {
    throw 'scripts/build-dist.ps1 must be run on Windows.'
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$manifest = Join-Path $repoRoot 'src-tauri\Cargo.toml'
$lockPath = Join-Path $PSScriptRoot 'mihomo-lock.json'

foreach ($command in @('node', 'pnpm', 'cargo', 'rustc')) {
    Assert-Command -Name $command
}

$nodeVersionText = (& node --version).Trim()
if ($LASTEXITCODE -ne 0) {
    throw 'Unable to determine the Node.js version.'
}
$nodeVersion = [version]($nodeVersionText.TrimStart('v').Split('-')[0])
$minimumNodeVersion = [version]'22.12.0'
if ($nodeVersion -lt $minimumNodeVersion) {
    throw "Node.js 22.12.0 or newer is required; found $nodeVersionText."
}

$pnpmVersionText = (& pnpm --version).Trim()
if ($LASTEXITCODE -ne 0 -or $pnpmVersionText -ne '11.9.0') {
    throw "pnpm 11.9.0 is required; found '$pnpmVersionText'."
}

$rustVersionText = (& rustc --version).Trim()
if ($LASTEXITCODE -ne 0 -or $rustVersionText -notmatch '^rustc ([0-9]+\.[0-9]+\.[0-9]+)') {
    throw "Unable to determine the Rust version from '$rustVersionText'."
}
$rustVersion = [version]$Matches[1]
if ($rustVersion -lt [version]'1.97.1') {
    throw "Rust 1.97.1 or newer is required; found $rustVersionText."
}

$rustHost = (& rustc -vV | Select-String '^host:').Line
if ($LASTEXITCODE -ne 0 -or $rustHost -ne 'host: x86_64-pc-windows-msvc') {
    throw "The x86_64-pc-windows-msvc Rust host is required; found '$rustHost'."
}

Push-Location -LiteralPath $repoRoot
try {
    Write-Host 'Fetching the pinned Mihomo executable and verifying both archive and executable hashes...'
    & (Join-Path $PSScriptRoot 'fetch-mihomo.ps1')

    $lock = Get-Content -LiteralPath $lockPath -Raw | ConvertFrom-Json
    $spec = $lock.platforms.'windows-x86_64'
    $corePath = Join-Path $repoRoot ([string]$spec.destination)
    if (-not (Test-Path -LiteralPath $corePath -PathType Leaf)) {
        throw "Pinned Mihomo executable was not staged at $corePath."
    }
    $actualCoreHash = (Get-FileHash -LiteralPath $corePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualCoreHash -ne $spec.executableSha256) {
        throw "Staged Mihomo hash mismatch. Expected $($spec.executableSha256), got $actualCoreHash."
    }

    Invoke-Checked -Command 'pnpm' -Arguments @('install', '--frozen-lockfile')
    Invoke-Checked -Command 'pnpm' -Arguments @('build')
    Invoke-Checked -Command 'cargo' -Arguments @('fmt', '--manifest-path', $manifest, '--all', '--', '--check')
    Invoke-Checked -Command 'cargo' -Arguments @('test', '--manifest-path', $manifest, '--all-targets', '--all-features')
    Invoke-Checked -Command 'cargo' -Arguments @('clippy', '--manifest-path', $manifest, '--all-targets', '--all-features', '--', '-D', 'warnings')
    Invoke-Checked -Command 'cargo' -Arguments @('check', '--manifest-path', $manifest, '--all-targets', '--all-features')
    Invoke-Checked -Command 'pnpm' -Arguments @('tauri', 'build', '--bundles', 'nsis')

    $generatedInstallerScript = Join-Path $repoRoot 'src-tauri\target\release\nsis\x64\installer.nsi'
    if (-not (Test-Path -LiteralPath $generatedInstallerScript -PathType Leaf)) {
        throw "Tauri did not leave the generated NSIS script at $generatedInstallerScript."
    }
    $installerScript = Get-Content -LiteralPath $generatedInstallerScript -Raw
    $requiredInstallerSnippets = [ordered]@{
        'desktop main binary name' = '!define MAINBINARYNAME "voluntary-internet-access"'
        'per-machine install mode' = '!define INSTALLMODE "perMachine"'
        'desktop main binary file' = 'File "${MAINBINARYSRCPATH}"'
        'user-mode Mihomo sidecar' = 'File /a "/oname=mihomo.exe"'
        'helper installer payload' = 'File /a "/oname=helper-payload\install-helper.ps1"'
        'privileged Mihomo payload' = 'File /a "/oname=helper-payload\mihomo.exe"'
        'privileged helper payload' = 'File /a "/oname=helper-payload\via-helper.exe"'
        'recovery payload' = 'File /a "/oname=helper-payload\via-recovery.exe"'
        'pre-uninstall hook include' = 'via-uninstall.nsh"'
    }
    foreach ($entry in $requiredInstallerSnippets.GetEnumerator()) {
        if (-not $installerScript.Contains($entry.Value)) {
            throw "Generated NSIS script is missing $($entry.Key): $($entry.Value)"
        }
    }

    $bundleDirectory = Join-Path $repoRoot 'src-tauri\target\release\bundle\nsis'
    $installers = @(Get-ChildItem -LiteralPath $bundleDirectory -Filter '*.exe' -File -ErrorAction SilentlyContinue)
    if ($installers.Count -eq 0) {
        throw "Tauri completed without producing an NSIS installer in $bundleDirectory."
    }

    Write-Host 'Verified installer(s):'
    $installers | ForEach-Object { Write-Host "  $($_.FullName)" }
}
finally {
    Pop-Location
}
