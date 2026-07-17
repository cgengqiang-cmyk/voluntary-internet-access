[CmdletBinding()]
param(
    [ValidateSet("Install", "Remove", "Start", "Stop", "Status")]
    [string]$Action = "Install",
    [string]$PayloadRoot = "",
    [switch]$Elevated
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$InstallRoot = "C:\ProgramData\VoluntaryInternetAccess"
$CoreDirectory = Join-Path $InstallRoot "core"
$RuntimeDirectory = Join-Path $InstallRoot "runtime"
$InstalledHelper = Join-Path $InstallRoot "via-helper.exe"
$InstalledRecovery = Join-Path $InstallRoot "via-recovery.exe"
$InstalledCore = Join-Path $CoreDirectory "mihomo.exe"
$TokenFile = Join-Path $InstallRoot "helper.auth"
$TaskName = "VoluntaryInternetAccessHelper"
$ExpectedCoreSha256 = "c14bda8dc4cc8910ccd2110fe2be083c51a1b66da59141a0b87aff6fe6126517"
$RepositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
if ([string]::IsNullOrWhiteSpace($PayloadRoot)) {
    $BuiltHelper = Join-Path $RepositoryRoot "src-tauri\target\release\via-helper.exe"
    $BuiltRecovery = Join-Path $RepositoryRoot "src-tauri\target\release\via-recovery.exe"
    $BundledCore = Join-Path $RepositoryRoot "src-tauri\binaries\mihomo-x86_64-pc-windows-msvc.exe"
}
else {
    $PayloadRoot = [IO.Path]::GetFullPath($PayloadRoot)
    $BuiltHelper = Join-Path $PayloadRoot "via-helper.exe"
    $BuiltRecovery = Join-Path $PayloadRoot "via-recovery.exe"
    $BundledCore = Join-Path $PayloadRoot "mihomo.exe"
}
$ClientAccount = [Security.Principal.WindowsIdentity]::GetCurrent().Name

function Test-IsAdministrator {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Invoke-ElevatedSelf {
    $arguments = @(
        "-NoProfile",
        "-ExecutionPolicy", "Bypass",
        "-File", ('"{0}"' -f $PSCommandPath),
        "-Action", $Action,
        "-Elevated"
    )
    if (-not [string]::IsNullOrWhiteSpace($PayloadRoot)) {
        $arguments += @("-PayloadRoot", ('"{0}"' -f $PayloadRoot))
    }
    $process = Start-Process -FilePath "powershell.exe" -Verb RunAs -Wait -PassThru -WindowStyle Hidden -ArgumentList $arguments
    exit $process.ExitCode
}

function Assert-ExactInstallRoot {
    $resolved = [IO.Path]::GetFullPath($InstallRoot).TrimEnd('\')
    if ($resolved -cne "C:\ProgramData\VoluntaryInternetAccess") {
        throw "拒绝操作非固定安装目录：$resolved"
    }
}

function New-AuthToken {
    $bytes = [byte[]]::new(32)
    $generator = [Security.Cryptography.RandomNumberGenerator]::Create()
    try {
        $generator.GetBytes($bytes)
    }
    finally {
        $generator.Dispose()
    }
    return -join ($bytes | ForEach-Object { $_.ToString("x2") })
}

function Set-InstallAcl {
    & "$env:SystemRoot\System32\icacls.exe" $InstallRoot "/inheritance:r" | Out-Null
    & "$env:SystemRoot\System32\icacls.exe" $InstallRoot "/grant:r" `
        "SYSTEM:(OI)(CI)F" `
        "BUILTIN\Administrators:(OI)(CI)F" `
        "${ClientAccount}:(OI)(CI)RX" | Out-Null
    & "$env:SystemRoot\System32\icacls.exe" $TokenFile "/inheritance:r" | Out-Null
    & "$env:SystemRoot\System32\icacls.exe" $TokenFile "/grant:r" `
        "SYSTEM:F" `
        "BUILTIN\Administrators:F" `
        "${ClientAccount}:R" | Out-Null
}

function Install-Helper {
    foreach ($source in @($BuiltHelper, $BuiltRecovery, $BundledCore)) {
        if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
            throw "缺少构建产物：$source。请先运行 cargo build --release --bins。"
        }
    }
    $actualCoreSha256 = (Get-FileHash -LiteralPath $BundledCore -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualCoreSha256 -cne $ExpectedCoreSha256) {
        throw "Mihomo 可执行文件哈希不匹配，拒绝安装。"
    }
    $expectedHelperSha256 = (Get-FileHash -LiteralPath $BuiltHelper -Algorithm SHA256).Hash.ToLowerInvariant()
    $expectedRecoverySha256 = (Get-FileHash -LiteralPath $BuiltRecovery -Algorithm SHA256).Hash.ToLowerInvariant()

    Assert-ExactInstallRoot
    New-Item -ItemType Directory -Force -Path $CoreDirectory, $RuntimeDirectory | Out-Null
    Copy-Item -LiteralPath $BuiltHelper -Destination $InstalledHelper -Force
    Copy-Item -LiteralPath $BuiltRecovery -Destination $InstalledRecovery -Force
    Copy-Item -LiteralPath $BundledCore -Destination $InstalledCore -Force
    if ((Get-FileHash -LiteralPath $InstalledHelper -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedHelperSha256) {
        throw "复制后的 helper 哈希不匹配，拒绝注册。"
    }
    if ((Get-FileHash -LiteralPath $InstalledRecovery -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedRecoverySha256) {
        throw "复制后的恢复工具哈希不匹配，拒绝注册。"
    }
    if ((Get-FileHash -LiteralPath $InstalledCore -Algorithm SHA256).Hash.ToLowerInvariant() -cne $ExpectedCoreSha256) {
        throw "复制后的 Mihomo 哈希不匹配，拒绝注册 helper。"
    }
    if (-not (Test-Path -LiteralPath $TokenFile -PathType Leaf)) {
        [IO.File]::WriteAllText($TokenFile, (New-AuthToken) + [Environment]::NewLine, [Text.UTF8Encoding]::new($false))
    }
    Set-InstallAcl

    $taskAction = New-ScheduledTaskAction -Execute $InstalledHelper -Argument "serve"
    $trigger = New-ScheduledTaskTrigger -AtLogOn -User $ClientAccount
    $principal = New-ScheduledTaskPrincipal -UserId $ClientAccount -LogonType Interactive -RunLevel Highest
    $settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit ([TimeSpan]::Zero) -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1)
    Register-ScheduledTask -TaskName $TaskName -Action $taskAction -Trigger $trigger -Principal $principal -Settings $settings -Force | Out-Null
    Start-ScheduledTask -TaskName $TaskName
    Write-Host "VIA 高权限 helper 已安装并启动。"
}

function Stop-Helper {
    if (Test-Path -LiteralPath $InstalledRecovery -PathType Leaf) {
        & $InstalledRecovery --tun-only
    }
    $task = Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
    if ($null -ne $task) {
        Stop-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
    }
}

function Remove-Helper {
    Assert-ExactInstallRoot
    Stop-Helper
    Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
    if (Test-Path -LiteralPath $InstallRoot) {
        $resolved = (Resolve-Path -LiteralPath $InstallRoot).Path.TrimEnd('\')
        if ($resolved -cne "C:\ProgramData\VoluntaryInternetAccess") {
            throw "拒绝递归删除非固定目录：$resolved"
        }
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
    Write-Host "VIA 高权限 helper 已移除。"
}

function Show-Status {
    $task = Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
    if ($null -eq $task) {
        Write-Host "helper 未安装。"
        return
    }
    Write-Host ("helper 任务状态：{0}" -f $task.State)
    Write-Host ("固定目录：{0}" -f $InstallRoot)
}

if ($Action -eq "Status") {
    Show-Status
    exit 0
}

if (-not (Test-IsAdministrator)) {
    if ($Elevated) {
        throw "管理员权限获取失败。"
    }
    Invoke-ElevatedSelf
}

switch ($Action) {
    "Install" { Install-Helper }
    "Remove" { Remove-Helper }
    "Start" {
        Start-ScheduledTask -TaskName $TaskName
        Write-Host "helper 已启动。"
    }
    "Stop" {
        Stop-Helper
        Write-Host "helper 已停止。"
    }
}
