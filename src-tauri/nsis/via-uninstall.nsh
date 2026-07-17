; Restore VIA-owned network state and remove the privileged helper before
; Tauri deletes the packaged recovery tools. Any failure leaves the app
; installed so the user can reopen it and run the visible repair workflow.
!macro NSIS_HOOK_PREUNINSTALL
  ; Tauri invokes this hook before its built-in running-process check. Stop the
  ; desktop process first so it cannot race the recovery/removal transaction.
  !insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"

  IfFileExists "$INSTDIR\helper-payload\via-recovery.exe" via_recovery_present via_recovery_missing

  via_recovery_missing:
    MessageBox MB_OK|MB_ICONSTOP "卸载已停止：安装目录缺少网络恢复工具。请重新安装 VIA，运行“修复网络”，然后重试卸载。"
    SetErrorLevel 1
    Quit

  via_recovery_present:
    ClearErrors
    ExecWait '"$INSTDIR\helper-payload\via-recovery.exe" --proxy-only' $0
    IfErrors via_recovery_failed
    StrCmp $0 "0" via_recovery_complete via_recovery_failed

  via_recovery_failed:
    MessageBox MB_OK|MB_ICONSTOP "卸载已停止：无法安全恢复 VIA 管理的系统代理（退出码 $0）。请重新打开 VIA，运行“修复网络”，然后重试卸载。"
    SetErrorLevel 1
    Quit

  via_recovery_complete:
    IfFileExists "$INSTDIR\helper-payload\install-helper.ps1" via_helper_installer_present via_helper_installer_missing

  via_helper_installer_missing:
    MessageBox MB_OK|MB_ICONSTOP "卸载已停止：安装目录缺少权限组件移除脚本。请重新安装 VIA，移除运行组件，然后重试卸载。"
    SetErrorLevel 1
    Quit

  via_helper_installer_present:
    ClearErrors
    ExecWait '"$WINDIR\System32\WindowsPowerShell\v1.0\powershell.exe" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$INSTDIR\helper-payload\install-helper.ps1" -Action Remove -Elevated' $0
    IfErrors via_helper_remove_failed
    StrCmp $0 "0" via_preuninstall_complete via_helper_remove_failed

  via_helper_remove_failed:
    MessageBox MB_OK|MB_ICONSTOP "卸载已停止：无法安全移除 VIA 权限组件（退出码 $0）。请重新打开 VIA，使用“移除运行组件”，然后重试卸载。"
    SetErrorLevel 1
    Quit

  via_preuninstall_complete:
!macroend
