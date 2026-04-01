use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::{self, Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::process::CommandExt;
#[cfg(unix)]
use std::sync::OnceLock;

#[cfg(windows)]
mod win32 {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    const DETACHED_PROCESS: u32 = 0x00000008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x01000000;

    #[repr(C)]
    struct StartupInfoW {
        cb: u32,
        reserved: usize,
        desktop: usize,
        title: usize,
        x: u32, y: u32, x_size: u32, y_size: u32,
        x_count_chars: u32, y_count_chars: u32,
        fill_attribute: u32,
        flags: u32,
        show_window: u16,
        cb_reserved2: u16,
        lp_reserved2: usize,
        std_input: usize,
        std_output: usize,
        std_error: usize,
    }

    #[repr(C)]
    struct ProcessInformation {
        process: usize,
        thread: usize,
        process_id: u32,
        thread_id: u32,
    }

    extern "system" {
        fn CreateProcessW(
            app: *const u16,
            cmd: *mut u16,
            proc_attrs: usize,
            thread_attrs: usize,
            inherit_handles: i32,
            flags: u32,
            env: usize,
            dir: *const u16,
            si: *mut StartupInfoW,
            pi: *mut ProcessInformation,
        ) -> i32;
        fn CloseHandle(handle: usize) -> i32;
    }

    #[link(name = "winmm")]
    extern "system" {
        fn PlaySoundW(sound: *const u16, hmod: usize, flags: u32) -> i32;
    }

    /// Play a WAV file synchronously (blocks until finished).
    pub fn play_sound_sync(path: &str) {
        const SND_FILENAME: u32 = 0x00020000;
        const SND_NODEFAULT: u32 = 0x0002;
        let path_wide: Vec<u16> = OsStr::new(path)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        unsafe {
            PlaySoundW(path_wide.as_ptr(), 0, SND_FILENAME | SND_NODEFAULT);
        }
    }

    fn create_process(cmd_line: &str, flags: u32) -> Option<u32> {
        let mut cmd_wide: Vec<u16> = OsStr::new(cmd_line)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let mut si = unsafe { std::mem::zeroed::<StartupInfoW>() };
        si.cb = std::mem::size_of::<StartupInfoW>() as u32;
        let mut pi = unsafe { std::mem::zeroed::<ProcessInformation>() };

        let ok = unsafe {
            CreateProcessW(
                std::ptr::null(),
                cmd_wide.as_mut_ptr(),
                0,
                0,
                0, // bInheritHandles = FALSE
                flags,
                0,
                std::ptr::null(),
                &mut si,
                &mut pi,
            )
        };

        if ok != 0 {
            unsafe {
                CloseHandle(pi.thread);
                CloseHandle(pi.process);
            }
            Some(pi.process_id)
        } else {
            None
        }
    }

    // -- Screen flash via a temporary full-screen window --

    #[repr(C)]
    struct WndClassExW {
        cb_size: u32,
        style: u32,
        wnd_proc: extern "system" fn(usize, u32, usize, isize) -> isize,
        cls_extra: i32,
        wnd_extra: i32,
        instance: usize,
        icon: usize,
        cursor: usize,
        background: usize,
        menu_name: *const u16,
        class_name: *const u16,
        icon_sm: usize,
    }

    #[repr(C)]
    struct Msg {
        hwnd: usize,
        message: u32,
        wparam: usize,
        lparam: isize,
        time: u32,
        pt_x: i32,
        pt_y: i32,
    }

    #[repr(C)]
    struct ProcessEntry32W {
        dw_size: u32,
        cnt_usage: u32,
        th32_process_id: u32,
        th32_default_heap_id: usize,
        th32_module_id: u32,
        cnt_threads: u32,
        th32_parent_process_id: u32,
        pc_pri_class_base: i32,
        dw_flags: u32,
        sz_exe_file: [u16; 260],
    }

    #[link(name = "user32")]
    extern "system" {
        fn RegisterClassExW(wc: *const WndClassExW) -> u16;
        fn CreateWindowExW(
            ex_style: u32, class: *const u16, title: *const u16,
            style: u32, x: i32, y: i32, w: i32, h: i32,
            parent: usize, menu: usize, instance: usize, param: usize,
        ) -> usize;
        fn DestroyWindow(hwnd: usize) -> i32;
        fn DefWindowProcW(hwnd: usize, msg: u32, wparam: usize, lparam: isize) -> isize;
        fn GetSystemMetrics(index: i32) -> i32;
        fn SetLayeredWindowAttributes(hwnd: usize, key: u32, alpha: u8, flags: u32) -> i32;
        fn GetModuleHandleW(name: *const u16) -> usize;
        fn PeekMessageW(msg: *mut Msg, hwnd: usize, min: u32, max: u32, remove: u32) -> i32;
        fn TranslateMessage(msg: *const Msg) -> i32;
        fn DispatchMessageW(msg: *const Msg) -> i32;
    }

    #[link(name = "gdi32")]
    extern "system" {
        fn GetStockObject(index: i32) -> usize;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> usize;
        fn Process32FirstW(snapshot: usize, entry: *mut ProcessEntry32W) -> i32;
        fn Process32NextW(snapshot: usize, entry: *mut ProcessEntry32W) -> i32;
    }

    #[link(name = "user32")]
    extern "system" {
        fn GetConsoleWindow() -> usize;
        fn SetForegroundWindow(hwnd: usize) -> i32;
        fn EnumWindows(callback: extern "system" fn(usize, isize) -> i32, lparam: isize) -> i32;
        fn GetWindowThreadProcessId(hwnd: usize, pid: *mut u32) -> u32;
        fn GetWindowLongPtrW(hwnd: usize, index: i32) -> isize;
        fn IsWindowVisible(hwnd: usize) -> i32;
    }

    struct EnumWindowsParam {
        target_pid: u32,
        found_hwnd: usize,
    }

    extern "system" fn flash_wnd_proc(hwnd: usize, msg: u32, wp: usize, lp: isize) -> isize {
        unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
    }

    fn pump_messages() {
        unsafe {
            let mut msg = std::mem::zeroed::<Msg>();
            while PeekMessageW(&mut msg, 0, 0, 0, 1) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    /// Flash the screen with a white overlay. Each flash is ~100ms.
    pub fn flash_screen(count: u32) {
        const SM_CXSCREEN: i32 = 0;
        const SM_CYSCREEN: i32 = 1;
        const WS_POPUP: u32 = 0x80000000;
        const WS_VISIBLE: u32 = 0x10000000;
        const WS_EX_TOPMOST: u32 = 0x00000008;
        const WS_EX_TOOLWINDOW: u32 = 0x00000080;
        const WS_EX_LAYERED: u32 = 0x00080000;
        const WS_EX_TRANSPARENT: u32 = 0x00000020;
        const LWA_ALPHA: u32 = 0x00000002;
        const WHITE_BRUSH: i32 = 0;

        unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            let class_name: Vec<u16> = OsStr::new("MeatbagFlash")
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let title: Vec<u16> = [0u16].to_vec();

            let wc = WndClassExW {
                cb_size: std::mem::size_of::<WndClassExW>() as u32,
                style: 0,
                wnd_proc: flash_wnd_proc,
                cls_extra: 0,
                wnd_extra: 0,
                instance,
                icon: 0,
                cursor: 0,
                background: GetStockObject(WHITE_BRUSH),
                menu_name: std::ptr::null(),
                class_name: class_name.as_ptr(),
                icon_sm: 0,
            };
            RegisterClassExW(&wc);

            let cx = GetSystemMetrics(SM_CXSCREEN);
            let cy = GetSystemMetrics(SM_CYSCREEN);

            for i in 0..count {
                if i > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(450));
                }

                let hwnd = CreateWindowExW(
                    WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_TRANSPARENT,
                    class_name.as_ptr(),
                    title.as_ptr(),
                    WS_POPUP | WS_VISIBLE,
                    0, 0, cx, cy,
                    0, 0, instance, 0,
                );

                if hwnd != 0 {
                    SetLayeredWindowAttributes(hwnd, 0, 153, LWA_ALPHA); // ~60% opacity
                    pump_messages();
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    DestroyWindow(hwnd);
                    pump_messages();
                }
            }
        }
    }

    /// Create a fully detached process with bInheritHandles=FALSE.
    /// Tries to break out of the parent's Job Object first (so the parent
    /// isn't blocked waiting for the child). Falls back without breakaway
    /// if the job doesn't allow it.
    pub fn create_detached(cmd_line: &str) -> Option<u32> {
        // Try escaping parent's job object first
        create_process(
            cmd_line,
            DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB,
        )
        .or_else(|| {
            // Fallback without breakaway
            create_process(cmd_line, DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        })
    }

    extern "system" fn enum_window_cb(hwnd: usize, lparam: isize) -> i32 {
        let param = unsafe { &mut *(lparam as *mut EnumWindowsParam) };
        unsafe {
            let mut pid: u32 = 0;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid == param.target_pid && IsWindowVisible(hwnd) != 0 {
                const GWL_STYLE: i32 = -16;
                const WS_CHILD: u32 = 0x40000000;
                let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
                if style & WS_CHILD == 0 {
                    param.found_hwnd = hwnd;
                    return 0; // stop enumeration
                }
            }
        }
        1 // continue
    }

    fn main_window_for_pid(pid: u32) -> usize {
        let mut param = EnumWindowsParam { target_pid: pid, found_hwnd: 0 };
        unsafe { EnumWindows(enum_window_cb, &mut param as *mut EnumWindowsParam as isize); }
        param.found_hwnd
    }

    fn parent_pid(pid: u32) -> Option<u32> {
        const TH32CS_SNAPPROCESS: u32 = 0x00000002;
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == usize::MAX { return None; }
            let mut entry = std::mem::zeroed::<ProcessEntry32W>();
            entry.dw_size = std::mem::size_of::<ProcessEntry32W>() as u32;
            let mut result = None;
            if Process32FirstW(snap, &mut entry) != 0 {
                loop {
                    if entry.th32_process_id == pid {
                        let ppid = entry.th32_parent_process_id;
                        if ppid != 0 { result = Some(ppid); }
                        break;
                    }
                    if Process32NextW(snap, &mut entry) == 0 { break; }
                }
            }
            CloseHandle(snap);
            result
        }
    }

    /// Find the HWND of the terminal hosting this process.
    /// Tries GetConsoleWindow first; walks the process tree if that returns NULL.
    pub fn find_terminal_hwnd() -> usize {
        let hwnd = unsafe { GetConsoleWindow() };
        if hwnd != 0 { return hwnd; }
        let mut pid = std::process::id();
        for _ in 0..10 {
            match parent_pid(pid) {
                Some(p) => {
                    let hwnd = main_window_for_pid(p);
                    if hwnd != 0 { return hwnd; }
                    pid = p;
                }
                None => break,
            }
        }
        0
    }

    /// Bring the given window to the foreground. No-ops if hwnd is 0.
    pub fn focus_hwnd(hwnd: usize) {
        if hwnd != 0 {
            unsafe { SetForegroundWindow(hwnd); }
        }
    }
}

// ---------------------------------------------------------------------------
// Path / environment helpers
// ---------------------------------------------------------------------------

/// Resolve the install directory (parent of the binary's symlink target).
fn install_dir() -> Option<PathBuf> {
    let exe = env::current_exe().ok()?;
    let resolved = fs::canonicalize(&exe).ok()?;
    // Binary is at target/release/claude-notify — install dir is 3 levels up
    resolved.parent()?.parent()?.parent().map(PathBuf::from)
}

#[cfg(unix)]
fn is_wsl() -> bool {
    static WSL: OnceLock<bool> = OnceLock::new();
    *WSL.get_or_init(|| {
        fs::read_to_string("/proc/version")
            .map(|v| v.to_lowercase().contains("microsoft"))
            .unwrap_or(false)
    })
}

/// Normalize a path: convert Windows paths (e.g. C:\foo\bar) to WSL paths
/// (/mnt/c/foo/bar) when running in WSL. Passes through unchanged otherwise.
fn normalize_path(path: &str) -> String {
    let trimmed = path.trim();
    #[cfg(unix)]
    {
        if trimmed.len() >= 3
            && trimmed.as_bytes()[0].is_ascii_alphabetic()
            && trimmed.as_bytes()[1] == b':'
            && (trimmed.as_bytes()[2] == b'\\' || trimmed.as_bytes()[2] == b'/')
            && is_wsl()
        {
            let drive = (trimmed.as_bytes()[0] as char).to_ascii_lowercase();
            let rest = &trimmed[3..];
            return format!("/mnt/{}/{}", drive, rest.replace('\\', "/"));
        }
    }
    trimmed.to_string()
}

/// Look up a sound by kind ("notification" or "escalation").
/// Checks for a user-installed custom sound next to the binary first,
/// then falls back to platform system sounds.
fn default_sound(kind: &str) -> String {
    // Check next to the binary first (direct install or user-placed custom sounds)
    for ext in &["wav", "oga", "ogg"] {
        let name = format!("{}.{}", kind, ext);
        if let Ok(exe) = env::current_exe() {
            if let Some(exe_dir) = exe.parent() {
                let path = exe_dir.join("sounds").join(&name);
                if path.exists() {
                    return path.to_string_lossy().into_owned();
                }
            }
        }
        // Then check the resolved install dir (symlink install)
        if let Some(dir) = install_dir() {
            let path = dir.join("sounds").join(&name);
            if path.exists() {
                return path.to_string_lossy().into_owned();
            }
        }
    }
    // Fall back to platform system sounds
    system_sound(kind)
}

/// Return a path to a suitable system sound for the given kind, or empty string if none found.
fn system_sound(kind: &str) -> String {
    #[cfg(windows)]
    {
        let candidates: &[&str] = if kind == "escalation" {
            &[
                r"C:\Windows\Media\Windows Message Nudge.wav",
                r"C:\Windows\Media\Windows Exclamation.wav",
                r"C:\Windows\Media\Windows Critical Stop.wav",
                r"C:\Windows\Media\tada.wav",
            ]
        } else {
            &[
                r"C:\Windows\Media\Speech Misrecognition.wav",
                r"C:\Windows\Media\Windows Notify System Generic.wav",
                r"C:\Windows\Media\chimes.wav",
                r"C:\Windows\Media\Windows Notify.wav",
            ]
        };
        for path in candidates {
            if std::path::Path::new(path).exists() {
                return path.to_string();
            }
        }
    }
    #[cfg(unix)]
    {
        // WAV first (works with aplay, paplay, pw-play), then OGG (paplay/pw-play only)
        let candidates: &[&str] = if kind == "escalation" {
            &[
                "/usr/share/sounds/alsa/Rear_Center.wav",
                "/usr/share/sounds/freedesktop/stereo/bell.oga",
                "/usr/share/sounds/freedesktop/stereo/complete.oga",
            ]
        } else {
            &[
                "/usr/share/sounds/alsa/Front_Center.wav",
                "/usr/share/sounds/freedesktop/stereo/message.oga",
                "/usr/share/sounds/freedesktop/stereo/message-new-instant.oga",
            ]
        };
        for path in candidates {
            if std::path::Path::new(path).exists() {
                return path.to_string();
            }
        }
    }
    String::new()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn state_dir() -> String {
    env::var("MEATBAG_STATE_DIR").unwrap_or_else(|_| {
        #[cfg(unix)]
        { "/tmp/claude-notify".into() }
        #[cfg(windows)]
        {
            let tmp = env::var("TEMP").unwrap_or_else(|_| r"C:\Temp".into());
            format!(r"{}\claude-notify", tmp)
        }
    })
}

fn pid_path(dir: &str) -> PathBuf {
    PathBuf::from(dir).join("pending.pid")
}

fn interaction_path(dir: &str) -> PathBuf {
    PathBuf::from(dir).join("last-interaction")
}

// ---------------------------------------------------------------------------
// Process helpers (platform-specific)
// ---------------------------------------------------------------------------

/// Spawn a command fully detached from the parent process.
/// On Windows, uses CreateProcessW with bInheritHandles=FALSE to prevent
/// the child from holding the parent's pipe handles open (which would cause
/// Claude Code to hang waiting for the hook to finish).
fn spawn_detached(cmd: &str, args: &[&str]) -> Option<u32> {
    #[cfg(unix)]
    {
        let mut command = Command::new(cmd);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        unsafe {
            command.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        command.spawn().ok().map(|c| c.id())
    }

    #[cfg(windows)]
    {
        let mut cmd_line = format!("\"{}\"", cmd);
        for arg in args {
            cmd_line.push_str(&format!(" \"{}\"", arg));
        }
        win32::create_detached(&cmd_line)
    }
}

/// Spawn a shell command fully detached.
fn spawn_detached_shell(cmd: &str) -> Option<u32> {
    #[cfg(unix)]
    {
        spawn_detached("sh", &["-c", cmd])
    }
    #[cfg(windows)]
    {
        spawn_detached("cmd", &["/C", cmd])
    }
}

/// Terminate a process by PID.
fn kill_process(pid: u32) {
    #[cfg(unix)]
    {
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

// ---------------------------------------------------------------------------
// Sound playback
// ---------------------------------------------------------------------------

/// Detect the best available audio player on this system (cached).
#[cfg(unix)]
fn detect_audio_player() -> &'static str {
    static PLAYER: OnceLock<&'static str> = OnceLock::new();
    PLAYER.get_or_init(|| {
        for player in &["paplay", "aplay", "pw-play"] {
            if Command::new("which")
                .arg(player)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
            {
                return player;
            }
        }
        "paplay" // last-resort default
    })
}

/// Play a sound file in a detached child process.
fn play_wav(path: &str) {
    if path.is_empty() {
        return;
    }
    #[cfg(unix)]
    {
        let player = env::var("MEATBAG_PLAYER")
            .unwrap_or_else(|_| detect_audio_player().to_string());
        spawn_detached(&player, &[path]);
    }
    #[cfg(windows)]
    {
        // Spawn self with _play subcommand — plays via PlaySoundW in a detached process
        let exe = env::current_exe().unwrap_or_default();
        let exe_str = exe.to_string_lossy().to_string();
        spawn_detached(&exe_str, &["_play", path]);
    }
}

/// Returns true if focus should fire for the given event.
/// event is "notification" or "escalation".
/// MEATBAG_FOCUS is a comma-separated list, e.g. "notification,escalation".
fn focus_at(event: &str) -> bool {
    env::var("MEATBAG_FOCUS")
        .map(|v| v.split(',').any(|e| e.trim() == event))
        .unwrap_or(false)
}

/// Capture the terminal window identifier into MEATBAG_FOCUS_HWND so the
/// detached escalation child inherits it. Call this at the start of
/// stop/permission handlers, before start_escalation.
fn capture_focus_target() {
    if env::var("MEATBAG_FOCUS").map(|v| v.is_empty()).unwrap_or(true) {
        return;
    }
    // Custom cmd needs no HWND — it handles its own window lookup.
    if env::var("MEATBAG_FOCUS_CMD").map(|v| !v.is_empty()).unwrap_or(false) {
        return;
    }

    #[cfg(windows)]
    {
        let hwnd = win32::find_terminal_hwnd();
        if hwnd != 0 {
            env::set_var("MEATBAG_FOCUS_HWND", hwnd.to_string());
        }
    }

    #[cfg(unix)]
    {
        if !is_wsl() { return; }
        if let Some(nt_pid) = wsl_nt_parent_pid() {
            env::set_var("MEATBAG_FOCUS_HWND", nt_pid.to_string());
        }
    }
}

/// On WSL1, read the Windows NT PID of the parent process from /proc/<ppid>/status
/// via the NtTgid field. Returns None on WSL2 (field absent) or any read failure.
#[cfg(unix)]
fn wsl_nt_parent_pid() -> Option<u32> {
    let self_status = fs::read_to_string("/proc/self/status").ok()?;
    let ppid: u32 = self_status
        .lines()
        .find(|l| l.starts_with("PPid:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()?;
    let parent_status = fs::read_to_string(format!("/proc/{}/status", ppid)).ok()?;
    parent_status
        .lines()
        .find(|l| l.starts_with("NtTgid:"))?
        .split_whitespace()
        .nth(1)?
        .parse::<u32>()
        .ok()
        .filter(|&p| p != 0)
}

/// Focus the terminal window. On Windows, uses the stored MEATBAG_FOCUS_HWND.
/// On WSL, spawns powershell.exe with WScript.Shell.AppActivate.
/// If MEATBAG_FOCUS_CMD is set, runs that shell command instead.
fn focus_window() {
    if let Ok(cmd) = env::var("MEATBAG_FOCUS_CMD") {
        if !cmd.is_empty() {
            spawn_detached_shell(&cmd);
            return;
        }
    }

    #[cfg(windows)]
    {
        if let Ok(s) = env::var("MEATBAG_FOCUS_HWND") {
            if let Ok(hwnd) = s.parse::<usize>() {
                win32::focus_hwnd(hwnd);
            }
        }
    }

    #[cfg(unix)]
    {
        if !is_wsl() { return; }
        let script = build_wsl_focus_script();
        spawn_detached("powershell.exe", &[
            "-NoProfile", "-NonInteractive", "-Command", &script,
        ]);
    }
}

/// Build a PowerShell one-liner that activates the terminal window.
/// Uses WScript.Shell.AppActivate (fast, no C# compilation).
/// If MEATBAG_FOCUS_HWND is set (WSL1), walks the process tree from that PID.
/// Otherwise (WSL2) activates Windows Terminal by name.
#[cfg(unix)]
fn build_wsl_focus_script() -> String {
    match env::var("MEATBAG_FOCUS_HWND") {
        Ok(pid_str) if !pid_str.is_empty() => {
            // WSL1: stored NtTgid is a Windows PID — walk up to find a focusable window
            format!(
                r#"$sh = New-Object -ComObject WScript.Shell
$pid = {pid}
for ($i = 0; $i -lt 10; $i++) {{
    if ($sh.AppActivate($pid)) {{ break }}
    $p = Get-CimInstance Win32_Process -Filter "ProcessId=$pid" -ErrorAction SilentlyContinue
    if (-not $p -or $p.ParentProcessId -eq 0) {{ break }}
    $pid = $p.ParentProcessId
}}"#,
                pid = pid_str
            )
        }
        _ => {
            // WSL2 fallback: activate Windows Terminal by title
            "$null = (New-Object -ComObject WScript.Shell).AppActivate('Windows Terminal')".to_string()
        }
    }
}

// ---------------------------------------------------------------------------
// State management
// ---------------------------------------------------------------------------

fn record_interaction(dir: &str) {
    let _ = fs::write(interaction_path(dir), now_secs().to_string());
}

fn cancel_pending(dir: &str) {
    let pid_file = pid_path(dir);
    if let Ok(content) = fs::read_to_string(&pid_file) {
        if let Ok(pid) = content.trim().parse::<u32>() {
            kill_process(pid);
        }
    }
    let _ = fs::remove_file(&pid_file);
}

// ---------------------------------------------------------------------------
// Stdin / JSON helpers
// ---------------------------------------------------------------------------

/// Read stdin with a short timeout. Returns empty string if stdin doesn't
/// close in time (common on Windows where pipes may not close properly).
fn read_stdin_timeout(timeout: Duration) -> String {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = String::new();
        let _ = io::stdin().read_to_string(&mut buf);
        let _ = tx.send(buf);
    });
    rx.recv_timeout(timeout).unwrap_or_default()
}

/// Extract a JSON string value by key (naive parser, handles escapes).
fn extract_json_string<'a>(input: &'a str, key: &str) -> Option<&'a str> {
    let pattern = format!("\"{}\"", key);
    let start = input.find(&pattern)?;
    let rest = &input[start + pattern.len()..];
    let colon = rest.find(':')?;
    let after_colon = rest[colon + 1..].trim_start();
    if !after_colon.starts_with('"') {
        return None;
    }
    let bytes = after_colon.as_bytes();
    let mut i = 1; // skip opening quote
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
        } else if bytes[i] == b'"' {
            return Some(&after_colon[1..i]);
        } else {
            i += 1;
        }
    }
    None
}

/// Check if the last_assistant_message in a hook payload ends with a question.
fn message_is_question(input: &str) -> bool {
    match extract_json_string(input, "last_assistant_message") {
        Some(msg) => {
            // Strip trailing JSON escape sequences (\n, \r, \t) and whitespace
            let trimmed = msg
                .trim_end_matches("\\n")
                .trim_end_matches("\\r")
                .trim_end_matches("\\t")
                .trim_end();
            trimmed.ends_with('?')
        }
        None => true, // if we can't parse, assume it needs attention
    }
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

fn play_sound(dir: &str, cooldown: u64) {
    if let Ok(content) = fs::read_to_string(interaction_path(dir)) {
        if let Ok(last) = content.trim().parse::<u64>() {
            if now_secs() - last < cooldown {
                return;
            }
        }
    }

    if let Ok(cmd) = env::var("MEATBAG_NOTIFICATION_CMD") {
        if !cmd.is_empty() {
            spawn_detached_shell(&cmd);
            return;
        }
    }

    let sound = env::var("MEATBAG_NOTIFICATION_SOUND")
        .map(|s| normalize_path(&s))
        .unwrap_or_else(|_| default_sound("notification"));
    play_wav(&sound);
}

fn start_escalation(dir: &str, delay_secs: u64) {
    cancel_pending(dir);

    // Spawn self with _escalate subcommand as a detached child
    let exe = env::current_exe().unwrap_or_default();
    let exe_str = exe.to_string_lossy().to_string();
    let delay_str = delay_secs.to_string();
    if let Some(pid) = spawn_detached(&exe_str, &["_escalate", &delay_str]) {
        let _ = fs::write(pid_path(dir), pid.to_string());
    }
}

/// Internal: runs as a detached child process, sleeps, then fires escalation.
fn run_escalation(delay_secs: u64) {
    std::thread::sleep(Duration::from_secs(delay_secs));

    let dir = state_dir();
    let still_pending = fs::read_to_string(pid_path(&dir))
        .ok()
        .and_then(|c| c.trim().parse::<u32>().ok())
        == Some(process::id());

    if !still_pending {
        return;
    }

    if let Ok(cmd) = env::var("MEATBAG_ESCALATION_CMD") {
        if !cmd.is_empty() {
            #[cfg(unix)]
            let shell_result = Command::new("sh")
                .args(["-c", &cmd])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            #[cfg(windows)]
            let shell_result = Command::new("cmd")
                .args(["/C", &cmd])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let _ = shell_result;
            if focus_at("escalation") {
                focus_window();
            }
            let _ = fs::remove_file(pid_path(&dir));
            return;
        }
    }

    // Default escalation: flash screen + play sound
    #[cfg(windows)]
    {
        let flash_count: u32 = env::var("MEATBAG_FLASH_COUNT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);
        win32::flash_screen(flash_count);
    }

    let sound_repeat: u32 = env::var("MEATBAG_ESCALATION_REPEAT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);

    let sound = env::var("MEATBAG_ESCALATION_SOUND")
        .map(|s| normalize_path(&s))
        .unwrap_or_else(|_| default_sound("escalation"));
    if !sound.is_empty() {
        for i in 0..sound_repeat {
            if i > 0 {
                std::thread::sleep(Duration::from_secs(2));
            }
            play_wav(&sound);
        }
    }

    if focus_at("escalation") {
        focus_window();
    }

    let _ = fs::remove_file(pid_path(&dir));
}

fn handle_prompt(dir: &str) {
    record_interaction(dir);
    cancel_pending(dir);

    let input = read_stdin_timeout(Duration::from_millis(100));
    if let Some(prompt) = extract_json_string(&input, "prompt") {
        if prompt.trim().eq_ignore_ascii_case("done") {
            let response =
                r#"{"decision":"block","reason":"Notification timer cancelled."}"#;
            let _ = io::stdout().write_all(response.as_bytes());
        }
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    let args: Vec<String> = env::args().collect();
    let action = args.get(1).map(|s| s.as_str()).unwrap_or("");

    // Internal subcommands (run as detached children)
    match action {
        "_escalate" => {
            let delay: u64 = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(300);
            run_escalation(delay);
            return;
        }
        #[cfg(windows)]
        "_play" => {
            if let Some(path) = args.get(2) {
                win32::play_sound_sync(path);
            }
            return;
        }
        #[cfg(windows)]
        "_flash" => {
            let count: u32 = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(1);
            win32::flash_screen(count);
            return;
        }
        _ => {}
    }

    // CLI flags override env vars (and flow through to child processes)
    fn parse_flag(args: &[String], flag: &str) -> Option<String> {
        args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1).cloned())
    }
    fn parse_flag_opt_val(args: &[String], flag: &str, default_val: &str) -> Option<String> {
        args.iter().position(|a| a == flag).map(|i| {
            match args.get(i + 1) {
                Some(v) if !v.starts_with('-') => v.clone(),
                _ => default_val.to_string(),
            }
        })
    }
    if let Some(v) = parse_flag(&args, "--flash") {
        env::set_var("MEATBAG_FLASH_COUNT", &v);
    }
    if let Some(v) = parse_flag(&args, "--repeat") {
        env::set_var("MEATBAG_ESCALATION_REPEAT", &v);
    }
    if let Some(v) = parse_flag(&args, "--delay") {
        env::set_var("MEATBAG_STOP_DELAY", &v);
        env::set_var("MEATBAG_PERMISSION_DELAY", &v);
    }
    if let Some(v) = parse_flag(&args, "--cooldown") {
        env::set_var("MEATBAG_COOLDOWN", &v);
    }
    if let Some(v) = parse_flag(&args, "--sound") {
        env::set_var("MEATBAG_NOTIFICATION_SOUND", &v);
    }
    if let Some(v) = parse_flag(&args, "--escalation-sound") {
        env::set_var("MEATBAG_ESCALATION_SOUND", &v);
    }
    if let Some(v) = parse_flag(&args, "--player") {
        env::set_var("MEATBAG_PLAYER", &v);
    }
    if let Some(v) = parse_flag_opt_val(&args, "--focus", "escalation") {
        env::set_var("MEATBAG_FOCUS", &v);
    }
    if let Some(v) = parse_flag(&args, "--focus-cmd") {
        env::set_var("MEATBAG_FOCUS_CMD", &v);
    }

    let dir = state_dir();
    let _ = fs::create_dir_all(&dir);

    // Detect controlling TTY (Linux only)
    #[cfg(target_os = "linux")]
    if env::var("MEATBAG_TTY").is_err() {
        if let Ok(stat) = fs::read_to_string("/proc/self/stat") {
            let fields: Vec<&str> = stat.split(' ').collect();
            if let Some(tty_str) = fields.get(6) {
                if let Ok(tty_nr) = tty_str.parse::<u32>() {
                    let major = (tty_nr >> 8) & 0xfff;
                    let minor = tty_nr & 0xff;
                    if major >= 136 {
                        env::set_var(
                            "MEATBAG_TTY",
                            format!("/dev/pts/{}", (major - 136) * 256 + minor),
                        );
                    }
                }
            }
        }
    }

    let cooldown: u64 = env::var("MEATBAG_COOLDOWN")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    let stop_delay: u64 = env::var("MEATBAG_STOP_DELAY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300);
    let permission_delay: u64 = env::var("MEATBAG_PERMISSION_DELAY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);

    match action {
        "stop" => {
            let input = read_stdin_timeout(Duration::from_millis(100));
            capture_focus_target();
            if focus_at("notification") { focus_window(); }
            play_sound(&dir, cooldown);
            if message_is_question(&input) {
                start_escalation(&dir, stop_delay);
            }
        }
        "permission" => {
            capture_focus_target();
            if focus_at("notification") { focus_window(); }
            play_sound(&dir, cooldown);
            start_escalation(&dir, permission_delay);
        }
        "prompt" => handle_prompt(&dir),
        "cancel" => cancel_pending(&dir),
        "dismiss" => {
            record_interaction(&dir);
            cancel_pending(&dir);
        }
        "debug" => {
            // Dump action + stdin to a log file for inspecting hook payloads
            let label = args.get(2).map(|s| s.as_str()).unwrap_or("unknown");
            let mut input = String::new();
            let _ = io::stdin().read_to_string(&mut input);
            let log = PathBuf::from(&dir).join("debug.log");
            let entry = format!(
                "--- {} [{}] ---\n{}\n\n",
                label,
                now_secs(),
                if input.is_empty() { "(no stdin)" } else { &input }
            );
            let _ = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log)
                .and_then(|mut f| f.write_all(entry.as_bytes()));
            eprintln!("Logged to {}", log.display());
        }
        _ => {
            eprintln!("Usage: claude-notify <action> [options]");
            eprintln!();
            eprintln!("Actions:");
            eprintln!("  stop          Play notification sound, schedule escalation");
            eprintln!("  permission    Play notification sound, schedule escalation (shorter delay)");
            eprintln!("  prompt        Cancel escalation, handle 'done' blocking (reads stdin)");
            eprintln!("  cancel        Cancel pending escalation");
            eprintln!("  dismiss       Record interaction and cancel escalation");
            eprintln!();
            eprintln!("Options (override env vars):");
            eprintln!("  --delay N     Seconds before escalation fires (default: 300 stop, 30 permission)");
            eprintln!("  --flash N     Number of screen flashes on escalation (default: 1)");
            eprintln!("  --repeat N    Number of times to play escalation sound (default: 1)");
            eprintln!("  --cooldown N  Suppress notification sound for N seconds after interaction (default: 30)");
            eprintln!("  --sound PATH  WAV file for notification sound");
            eprintln!("  --escalation-sound PATH  WAV file for escalation sound");
            eprintln!("  --player CMD  Audio player command (default: paplay, aplay, or pw-play — whichever is found first)");
            eprintln!("  --focus [EVENTS]  Focus terminal on nudge; EVENTS is comma-separated list of notification,escalation (default when flag present: escalation)");
            eprintln!("  --focus-cmd CMD   Shell command to focus terminal, overrides built-in focus");
            eprintln!();
            eprintln!("Test commands:");
            eprintln!("  _play <path>  Play a WAV file synchronously");
            eprintln!("  _flash [N]    Flash screen N times (default: 1)");
            eprintln!("  _escalate <delay> [flash] [repeat]  Run escalation after delay");
            process::exit(1);
        }
    }
}
