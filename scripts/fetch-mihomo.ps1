[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$lock = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'mihomo-lock.json') -Raw | ConvertFrom-Json
$spec = $lock.platforms.'windows-x86_64'
$destination = Join-Path $repoRoot $spec.destination
$destinationDirectory = Split-Path -Parent $destination
$tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("via-mihomo-" + [guid]::NewGuid().ToString('N'))

try {
    New-Item -ItemType Directory -Force -Path $tempRoot, $destinationDirectory | Out-Null
    $archive = Join-Path $tempRoot $spec.asset
    Invoke-WebRequest -Uri $spec.url -OutFile $archive -MaximumRedirection 5
    $actual = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $spec.sha256) {
        throw "Mihomo SHA-256 mismatch. Expected $($spec.sha256), got $actual"
    }

    $expanded = Join-Path $tempRoot 'expanded'
    Expand-Archive -LiteralPath $archive -DestinationPath $expanded
    $candidates = @(Get-ChildItem -LiteralPath $expanded -File -Filter '*.exe' | Where-Object {
        $_.Name -like 'mihomo*'
    })
    if ($candidates.Count -ne 1) {
        throw "The pinned archive must contain exactly one Mihomo executable; found $($candidates.Count)."
    }
    Copy-Item -LiteralPath $candidates[0].FullName -Destination $destination -Force
    $executableHash = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($executableHash -ne $spec.executableSha256) {
        Remove-Item -LiteralPath $destination -Force
        throw "Mihomo executable SHA-256 mismatch. Expected $($spec.executableSha256), got $executableHash"
    }
    Write-Output "Staged Mihomo $($lock.version) at $destination"
}
finally {
    if (Test-Path -LiteralPath $tempRoot) {
        $tempBase = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
        $resolvedTempRoot = [System.IO.Path]::GetFullPath($tempRoot)
        $safePrefix = $resolvedTempRoot.StartsWith($tempBase, [System.StringComparison]::OrdinalIgnoreCase)
        $safeName = (Split-Path -Leaf $resolvedTempRoot) -like 'via-mihomo-*'
        if (-not ($safePrefix -and $safeName)) {
            throw "Refusing to remove unexpected temporary path: $resolvedTempRoot"
        }
        Remove-Item -LiteralPath $resolvedTempRoot -Recurse -Force
    }
}
