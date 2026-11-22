; 安装 / 卸载前结束本应用的常驻组件（钩子由 tauri.conf.json 的
; `bundle > windows > nsis > installerHooks` 引入）。
;
; wb-agent-runtime.exe 由桌面宿主拉起，宿主退出后它还会等最后一个客户端空闲 20 秒
; 才退出；文件被占用时 NSIS 覆盖 / 删除失败，安装器报「写入文件出错」。
; agent-studio-runtime.exe 是 0.1.65 及更早版本的旧名，新包已不再包含它，顺手删掉
; 免得残留（旧版本运行时被替换前正是它锁着文件）。
;
; 顺序很关键：先结束主程序，再结束采集服务。采集服务由主程序每秒重连拉起，
; 反过来先杀服务会被立刻重新拉起，复制文件时照样被占用。

!macro NSIS_HOOK_STOP_COMPONENTS
  DetailPrint "Closing WorkBuddy Switch..."
  !if "${INSTALLMODE}" == "currentUser"
    nsis_tauri_utils::KillProcessCurrentUser "${MAINBINARYNAME}.exe"
    Pop $R0
    nsis_tauri_utils::KillProcessCurrentUser "wb-agent-runtime.exe"
    Pop $R0
    nsis_tauri_utils::KillProcessCurrentUser "agent-studio-runtime.exe"
    Pop $R0
  !else
    nsis_tauri_utils::KillProcess "${MAINBINARYNAME}.exe"
    Pop $R0
    nsis_tauri_utils::KillProcess "wb-agent-runtime.exe"
    Pop $R0
    nsis_tauri_utils::KillProcess "agent-studio-runtime.exe"
    Pop $R0
  !endif
  ; 句柄释放是异步的，给系统一点时间再复制 / 删除文件。
  Sleep 500
  Delete "$INSTDIR\agent-studio-runtime.exe"
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro NSIS_HOOK_STOP_COMPONENTS
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro NSIS_HOOK_STOP_COMPONENTS
!macroend
