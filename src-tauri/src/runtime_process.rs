//! 采集服务进程（`wb-agent-runtime`）的清理。
//!
//! 采集服务由桌面插件按需拉起：宿主退出后它还要等最后一个客户端空闲 20 秒才自行
//! 退出，Windows 会因此锁住安装目录里的 exe——覆盖安装时 NSIS 复制文件失败，表现
//! 成「更新 / 安装出错」。所以更新安装、正常退出与重启前都主动结束它；手动安装和
//! 卸载由安装器钩子兜底（`src-tauri/installer-hooks.nsh`）。
//!
//! 只结束本应用安装目录里的进程：`agent-studio-runtime` 是 0.1.65 及更早版本的旧名，
//! 上游同名组件也可能在跑，因此必须连同目录一起判定；`wb-agent-runtime`（含开发态
//! 带 target 后缀的产物）同样按目录判定，避免波及另一份独立安装的副本。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// 等进程真正退出（文件句柄关闭）的上限：Windows 的 `TerminateProcess` 是异步的。
const EXIT_WAIT: Duration = Duration::from_secs(2);
/// 等待期间的重新枚举间隔。
const EXIT_POLL: Duration = Duration::from_millis(100);

/// 结束本应用的采集服务进程，返回发现并处理的进程数。
///
/// 尽力而为：枚举与结束失败都静默——清理是安装的前置动作，不能反过来让安装报错。
///
/// 结束到文件句柄真正释放之间有一个窗口，而宿主每秒会重连并重新拉起采集服务，
/// 所以等待期间再现的进程也一并结束；枚举为空立即返回，正常情况下不额外耗时。
pub fn terminate() -> usize {
    #[cfg(any(windows, unix))]
    {
        let dirs = runtime_dirs();
        let deadline = Instant::now() + EXIT_WAIT;
        let mut handled = 0;
        loop {
            let targets = collect(&dirs);
            if targets.is_empty() {
                return handled;
            }
            for (pid, image) in &targets {
                if terminate_pid(*pid) {
                    handled += 1;
                    eprintln!("[采集服务] 已结束 {}（pid {pid}）", image.display());
                } else {
                    eprintln!("[采集服务] 结束失败 {}（pid {pid}）", image.display());
                }
            }
            if Instant::now() >= deadline {
                return handled;
            }
            std::thread::sleep(EXIT_POLL);
        }
    }
    #[cfg(not(any(windows, unix)))]
    {
        0
    }
}

/// 启动时枚举到的遗留采集服务进程，见 [`stale`] / [`terminate_stale`]。
pub struct Stale {
    #[cfg(any(windows, unix))]
    pids: Vec<u32>,
}

/// 记下构建应用之前就在跑的采集服务进程。
///
/// 手动替换安装包（macOS 拖 dmg、Linux 换 AppImage / `dpkg -i`）没有安装器钩子可以
/// 插入，上一次运行的采集服务会被新版本连上，所以要清一次。这里**只枚举**：此刻还没有
/// 单实例判定（单实例插件在自己的 `setup` 里决定 notify-or-exit），贸然结束会打掉
/// 「正在运行的实例」的采集服务。
pub fn stale() -> Stale {
    #[cfg(any(windows, unix))]
    {
        Stale {
            pids: collect(&runtime_dirs())
                .into_iter()
                .map(|(pid, _)| pid)
                .collect(),
        }
    }
    #[cfg(not(any(windows, unix)))]
    {
        Stale {}
    }
}

/// 结束 [`stale`] 记下的进程，返回结束的数量；枚举之后才出现的进程不在此列。
///
/// 放在应用 `setup` 里调用：单实例插件此时已判定过身份，第二实例走不到这里。按映像
/// 路径复核再结束，避免 PID 复用误杀无关进程。
pub fn terminate_stale(stale: &Stale) -> usize {
    #[cfg(any(windows, unix))]
    {
        let dirs = runtime_dirs();
        let mut handled = 0;
        for pid in &stale.pids {
            let Some(image) = image_of(*pid) else {
                continue;
            };
            if !is_runtime_process(&image, &dirs) {
                continue;
            }
            if terminate_pid(*pid) {
                handled += 1;
                eprintln!(
                    "[采集服务] 已结束上次遗留的进程 {}（pid {pid}）",
                    image.display()
                );
            }
        }
        handled
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = stale;
        0
    }
}

/// 判定采集服务进程时认可的两个目录：应用安装目录（当前 exe 所在处）与采集服务自身
/// 的目录（开发态由 `companion::runtime_path` 指向 `src-tauri/binaries`）。
fn runtime_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    {
        dirs.push(dir);
    }
    if let Some(dir) = crate::companion::runtime_path()
        .parent()
        .map(Path::to_path_buf)
    {
        if !dirs.iter().any(|existing| same_path(existing, &dir)) {
            dirs.push(dir);
        }
    }
    dirs
}

/// 映像路径是否属于本应用的采集服务（忽略大小写与 `.exe` 后缀）。
fn is_runtime_process(image: &Path, dirs: &[PathBuf]) -> bool {
    let Some(name) = image
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
    else {
        return false;
    };
    let stem = name.strip_suffix(".exe").unwrap_or(&name);
    let named = stem.starts_with("wb-agent-runtime") || stem.starts_with("agent-studio-runtime");
    named
        && image
            .parent()
            .is_some_and(|dir| dirs.iter().any(|candidate| same_path(dir, candidate)))
}

/// 路径比较：Windows 文件系统大小写不敏感，其余平台按字面比较。
#[cfg(windows)]
fn same_path(a: &Path, b: &Path) -> bool {
    a.as_os_str().eq_ignore_ascii_case(b.as_os_str())
}

#[cfg(not(windows))]
fn same_path(a: &Path, b: &Path) -> bool {
    a == b
}

/// 枚举采集服务进程：返回 (pid, 映像路径)。
#[cfg(windows)]
fn collect(dirs: &[PathBuf]) -> Vec<(u32, PathBuf)> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    let mut found = Vec::new();
    let own = std::process::id();
    // SAFETY: 快照句柄在本函数内关闭；PROCESSENTRY32W 的 dwSize 在首次调用前填好。
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return found;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut present = Process32FirstW(snapshot, &mut entry) != 0;
        while present {
            let pid = entry.th32ProcessID;
            if pid != 0 && pid != own {
                if let Some(image) = image_of(pid) {
                    if is_runtime_process(&image, dirs) {
                        found.push((pid, image));
                    }
                }
            }
            present = Process32NextW(snapshot, &mut entry) != 0;
        }
        CloseHandle(snapshot);
    }
    found
}

/// 单个进程的映像路径；取不到（权限不足 / 已退出）时返回 None。
#[cfg(windows)]
fn image_of(pid: u32) -> Option<PathBuf> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    // 长路径会超过 MAX_PATH，容量按 WCHAR 计。
    let mut buffer = vec![0u16; 32 * 1024];
    let mut size = buffer.len() as u32;
    // SAFETY: 句柄只在本函数使用并关闭；缓冲区长度由 `size` 如实给出。
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle == 0 {
            return None;
        }
        let ok = QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut size) != 0;
        CloseHandle(handle);
        if !ok {
            return None;
        }
    }
    buffer.truncate(size as usize);
    Some(PathBuf::from(String::from_utf16_lossy(&buffer)))
}

#[cfg(windows)]
fn terminate_pid(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};

    // SAFETY: 句柄只在本函数使用并关闭；pid 已按映像路径确认属于本应用的采集服务。
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if handle == 0 {
            return false;
        }
        let killed = TerminateProcess(handle, 0) != 0;
        CloseHandle(handle);
        killed
    }
}

/// macOS / Linux 走 `ps`：`comm` 是可执行文件路径，与 Windows 的映像路径语义一致。
#[cfg(unix)]
fn collect(dirs: &[PathBuf]) -> Vec<(u32, PathBuf)> {
    let Ok(output) = std::process::Command::new("ps")
        .args(["-axo", "pid=,comm="])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            // pid 右对齐、后接一个空格；路径本身可能含空格，只切第一个分隔。
            let (pid, image) = line.trim_start().split_once(' ')?;
            let pid: u32 = pid.parse().ok()?;
            let image = Path::new(image.trim_start());
            is_runtime_process(image, dirs).then(|| (pid, image.to_path_buf()))
        })
        .collect()
}

#[cfg(unix)]
fn image_of(pid: u32) -> Option<PathBuf> {
    let output = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&output.stdout);
    let path = path.trim();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

#[cfg(unix)]
fn terminate_pid(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install_dir() -> Vec<PathBuf> {
        vec![PathBuf::from("/opt/workbuddy-switch")]
    }

    #[test]
    fn runtime_inside_install_dir_is_matched() {
        for image in [
            "/opt/workbuddy-switch/wb-agent-runtime.exe",
            "/opt/workbuddy-switch/wb-agent-runtime",
            "/opt/workbuddy-switch/agent-studio-runtime.exe",
            "/opt/workbuddy-switch/agent-studio-runtime",
        ] {
            assert!(
                is_runtime_process(Path::new(image), &install_dir()),
                "{image}"
            );
        }
    }

    /// 开发态 sidecar 带 target 后缀，同样要认出来。
    #[test]
    fn target_suffixed_dev_runtime_is_matched() {
        assert!(is_runtime_process(
            Path::new("/opt/workbuddy-switch/wb-agent-runtime-x86_64-pc-windows-msvc.exe"),
            &install_dir()
        ));
    }

    /// 安装目录之外的采集服务（另一份安装、钩子副本）不归本次安装管。
    #[test]
    fn runtime_outside_install_dir_is_left_alone() {
        for image in [
            "/home/me/.agent-studio/bin/agent-studio-runtime-v1.exe",
            "/usr/local/bin/wb-agent-runtime.exe",
            "/opt/workbuddy-switch/../other/agent-studio-runtime.exe",
        ] {
            assert!(
                !is_runtime_process(Path::new(image), &install_dir()),
                "{image}"
            );
        }
    }

    #[test]
    fn unrelated_processes_are_left_alone() {
        for image in [
            "/opt/workbuddy-switch/workbuddy-switch.exe",
            "/opt/workbuddy-switch/agent-studio.exe",
            "/opt/workbuddy-switch/hook.py",
        ] {
            assert!(
                !is_runtime_process(Path::new(image), &install_dir()),
                "{image}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_compare_case_insensitively() {
        let dirs = vec![PathBuf::from(r"C:\Users\me\AppData\Local\workbuddy-switch")];
        assert!(is_runtime_process(
            Path::new(r"C:\Users\me\AppData\Local\WorkBuddy-Switch\wb-agent-runtime.exe"),
            &dirs
        ));
    }

    /// 两个端到端用例都会结束「安装目录里所有采集服务」，且共用测试可执行文件所在
    /// 目录，必须串行，否则互相把对方的假进程杀掉。
    #[cfg(windows)]
    static PROCESS_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// 在测试可执行文件所在目录里造一个长期存活、映像名符合判定条件的进程
    /// （系统命令改个名字即可，判定只看映像名与目录）。
    #[cfg(windows)]
    fn spawn_fake_runtime(name: &str) -> (std::process::Child, PathBuf) {
        use std::process::Stdio;

        let dir = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let fake = dir.join(name);
        let shell = std::env::var_os("ComSpec")
            .unwrap_or_else(|| std::ffi::OsString::from(r"C:\Windows\System32\cmd.exe"));
        std::fs::copy(&shell, &fake).unwrap();
        let child = std::process::Command::new(&fake)
            .args(["/c", "ping", "-n", "60", "127.0.0.1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        // 映像路径要等进程真正创建后才可查询。
        std::thread::sleep(Duration::from_millis(300));
        (child, fake)
    }

    /// 轮询到进程退出；超时返回 false。
    #[cfg(windows)]
    fn wait_exited(child: &mut std::process::Child, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            match child.try_wait() {
                Ok(None) if Instant::now() < deadline => std::thread::sleep(EXIT_POLL),
                Ok(None) => return false,
                _ => return true,
            }
        }
    }

    /// 端到端：真实进程能被枚举、结束。更新 / 安装不再被文件占用卡住，靠的就是
    /// 这条链路——验证结束与句柄释放都生效。
    #[cfg(windows)]
    #[test]
    fn terminate_ends_a_running_runtime_process() {
        let _serial = PROCESS_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (mut child, fake) = spawn_fake_runtime("wb-agent-runtime-terminate.exe");

        assert!(terminate() >= 1, "应结束冒充采集服务的进程");

        let exited = wait_exited(&mut child, Duration::from_secs(5));
        let _ = child.kill();
        let _ = std::fs::remove_file(&fake);
        assert!(exited, "采集服务进程应已被结束");
    }

    /// 启动清理是两步：枚举不动进程，`setup` 里的第二歩才结束——第二实例不会走到
    /// `setup`，这样就不会打掉正在运行实例的运行时。
    #[cfg(windows)]
    #[test]
    fn startup_cleanup_defers_the_kill_to_setup() {
        let _serial = PROCESS_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (mut child, fake) = spawn_fake_runtime("wb-agent-runtime-stale.exe");

        let stale = stale();
        assert!(!stale.pids.is_empty(), "启动前就存在的采集服务应被记下");
        assert!(child.try_wait().unwrap().is_none(), "枚举阶段不得结束进程");

        assert!(terminate_stale(&stale) >= 1, "setup 阶段应结束遗留进程");
        let exited = wait_exited(&mut child, Duration::from_secs(5));
        let _ = child.kill();
        let _ = std::fs::remove_file(&fake);
        assert!(exited, "遗留的采集服务进程应已被结束");
    }
}
