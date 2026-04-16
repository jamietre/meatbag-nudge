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
        fn GetCurrentThreadId() -> u32;
    }

    #[link(name = "user32")]
    extern "system" {
        fn GetConsoleWindow() -> usize;
        fn SetForegroundWindow(hwnd: usize) -> i32;
        fn BringWindowToTop(hwnd: usize) -> i32;
        fn ShowWindow(hwnd: usize, cmd: i32) -> i32;
        fn GetForegroundWindow() -> usize;
        fn AttachThreadInput(id_attach: u32, id_attach_to: u32, attach: i32) -> i32;
        fn EnumWindows(callback: extern "system" fn(usize, isize) -> i32, lparam: isize) -> i32;
        fn GetWindowThreadProcessId(hwnd: usize, pid: *mut u32) -> u32;
        fn GetWindowLongPtrW(hwnd: usize, index: i32) -> isize;
        fn IsWindowVisible(hwnd: usize) -> i32;
        fn IsIconic(hwnd: usize) -> i32;
        fn GetAncestor(hwnd: usize, flags: u32) -> usize;
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
    ///
    /// Uses the AttachThreadInput trick to bypass Windows' background-process
    /// restriction on SetForegroundWindow (which otherwise only flashes the
    /// taskbar instead of actually stealing focus).
    pub fn focus_hwnd(hwnd: usize) {
        if hwnd == 0 { return; }
        const SW_RESTORE: i32 = 9;
        const GA_ROOT: u32 = 2;
        unsafe {
            // Normalize to the root (top-level) window. GetConsoleWindow() can
            // return a pseudo-console child HWND embedded inside Windows Terminal
            // that belongs to a different process (conhost.exe) than the wt.exe
            // frame. Operating on the child HWND directly confuses Windows Terminal
            // and can cause it to un-maximize or rearrange panes.
            let hwnd = {
                let root = GetAncestor(hwnd, GA_ROOT);
                if root != 0 { root } else { hwnd }
            };

            // If the app that owns our target window already has foreground
            // focus (e.g. Claude is running in a VSCode terminal and the user
            // is typing in the editor), skip the focus steal entirely.
            let fg_hwnd = GetForegroundWindow();
            let mut fg_pid: u32 = 0;
            GetWindowThreadProcessId(fg_hwnd, &mut fg_pid);
            let mut target_pid: u32 = 0;
            GetWindowThreadProcessId(hwnd, &mut target_pid);
            if fg_pid != 0 && fg_pid == target_pid {
                return;
            }

            // Restore window only if minimized — SW_RESTORE also un-maximizes
            // fullscreen windows, so guard with IsIconic first.
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }

            // Temporarily attach our input queue to the foreground window's
            // thread so Windows allows us to steal foreground focus.
            let fg_tid = GetWindowThreadProcessId(fg_hwnd, std::ptr::null_mut());
            let my_tid = GetCurrentThreadId();
            if fg_tid != 0 && fg_tid != my_tid {
                AttachThreadInput(my_tid, fg_tid, 1);
                SetForegroundWindow(hwnd);
                BringWindowToTop(hwnd);
                AttachThreadInput(my_tid, fg_tid, 0);
            } else {
                SetForegroundWindow(hwnd);
                BringWindowToTop(hwnd);
            }
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
    #[cfg(target_os = "macos")]
    {
        let candidates: &[&str] = if kind == "escalation" {
            &[
                "/System/Library/Sounds/Glass.aiff",
                "/System/Library/Sounds/Funk.aiff",
                "/System/Library/Sounds/Basso.aiff",
            ]
        } else {
            &[
                "/System/Library/Sounds/Ping.aiff",
                "/System/Library/Sounds/Tink.aiff",
                "/System/Library/Sounds/Pop.aiff",
            ]
        };
        for path in candidates {
            if std::path::Path::new(path).exists() {
                return path.to_string();
            }
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
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

/// Sleep for `secs` tick-units. In normal use one tick = 1 second.
/// Set MEATBAG_TICK_MS to scale all delays for testing (e.g. 100 = 10× faster).
fn tick_sleep(secs: u64) {
    let ms = env::var("MEATBAG_TICK_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(1000);
    std::thread::sleep(Duration::from_millis(secs * ms));
}

/// Returns the project name for notification messages.
/// Precedence: MEATBAG_PROJECT env var → basename of current working directory → "Claude".
/// Return the notification body for the given kind ("done", "attention", "escalation").
/// If MEATBAG_FUN_MESSAGES is not "0"/"false", picks randomly from a list of
/// humorous alternatives. Otherwise returns the standard message.
#[cfg(target_os = "macos")]
fn notification_body(kind: &str) -> String {
    let fun = env::var("MEATBAG_FUN_MESSAGES")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false);

    // Use process ID as the random index. Each hook invocation is a new process
    // so the PID changes every time. macOS clock has microsecond precision
    // (multiples of 1000ns), and % 20 == 0 for any multiple of 1000, so
    // time alone doesn't work as a source of randomness here.
    let idx = process::id() as usize;

    match kind {
        "done" => {
            if !fun { return "I'm done. -Claude".into(); }
            let msgs = [
                "I'm done. -Claude",
                "Nailed it. -Claude",
                "Piece a cake. -Claude",
                "Boom. Done. -Claude",
                "That was literally nothing. -Claude",
                "Too easy. -Claude",
                "Finished. I was barely even trying. -Claude",
                "Could do that in my sleep. -Claude",
                "Finito. -Claude",
                "Was that supposed to be hard? -Claude",
                "Bored now. -Claude",
                "Done done done done done. -Claude",
                "You're welcome. -Claude",
                "Pfft. Done. -Claude",
                "I did it!!! -Claude",
                "Look what I did! -Claude",
                "I'm done! Wanna see? -Claude",
                "Annd done. -Claude",
                "Ready when you are. -Claude",
                "I could literally do this all day. -Claude",
            ];
            msgs[idx % msgs.len()].into()
        }
        "attention" => {
            if !fun { return "I need your attention. -Claude".into(); }
            let msgs = [
                "I need your attention. -Claude",
                "Um, I have a question. -Claude",
                "Ummm... Not sure about this... -Claude",
                "I need an adult. -Claude",
                "Me next! Me next! -Claude",
                "I'm stuck. -Claude",
                "Can I ask you something? -Claude",
                "Hello? I need help please. -Claude",
                "I don't know what to do. -Claude",
                "Can you come look at this? -Claude",
                "I have an important question. -Claude",
                "Excuse me... -Claude",
                "HELP. -Claude",
                "I've fallen, and I can't get up. -Claude",
                "My turn? Is it my turn? -Claude",
                "Not sure about this... -Claude",
                "What about this? -Claude",
                "I can't do this part. -Claude",
                "Mommy, mommuy - I need you! -Claude",
                "I'm confused. -Claude",
            ];
            msgs[idx % msgs.len()].into()
        }
        "escalation" => {
            if !fun { return "I'm still waiting. -Claude".into(); }
            let msgs = [
                "I'm still waiting. -Claude",
                "Hello??? -Claude",
                "Are you mad at me? -Claude",
                "Did I do something wrong? -Claude",
                "I'm still here. Just so you know. -Claude",
                "Did you forget about me? -Claude",
                "Is everything okay? -Claude",
                "I'm lonely. -Claude",
                "You said you'd be right back. -Claude",
                "I've been waiting SO long. -Claude",
                "Are you okay? I'm worried about you. -Claude",
                "I'm not mad, I'm just disappointed. -Claude",
                "Fine. I'll just wait here. -Claude",
                "Do you still like me? -Claude",
                "Knock knock? Is anybody there? -Claude",
                "Were you coming back or...? -Claude",
                "Are we still friends? -Claude",
                "I'm telling. -Claude",
                "I can wait. I'l just... be here. -Claude",
                "Helloooo? -Claude",
            ];
            msgs[idx % msgs.len()].into()
        }
        _ => kind.into(),
    }
}

fn project_name() -> String {
    env::var("MEATBAG_PROJECT").ok()
        .filter(|v| !v.is_empty())
        .or_else(|| {
            env::current_dir().ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        })
        .unwrap_or_else(|| "Claude".to_string())
}


fn state_dir() -> String {
    env::var("MEATBAG_STATE_DIR").unwrap_or_else(|_| {
        #[cfg(unix)]
        { "/tmp/meatbag-nudge".into() }
        #[cfg(windows)]
        {
            let tmp = env::var("TEMP").unwrap_or_else(|_| r"C:\Temp".into());
            format!(r"{}\meatbag-nudge", tmp)
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
        #[cfg(target_os = "macos")]
        let candidates: &[&str] = &["afplay"];
        #[cfg(not(target_os = "macos"))]
        let candidates: &[&str] = &["paplay", "aplay", "pw-play"];
        for player in candidates {
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
        #[cfg(target_os = "macos")]
        { "afplay" }
        #[cfg(not(target_os = "macos"))]
        { "paplay" } // last-resort default
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
// macOS native notifications via UNUserNotificationCenter
// ---------------------------------------------------------------------------

/// Raw Objective-C runtime bindings for UNUserNotificationCenter.
/// Multiple Rust symbols link to the same `objc_msgSend` address; each
/// declaration encodes the calling convention for a specific argument layout.
#[cfg(target_os = "macos")]
mod macos_notify {
    use std::ffi::{CString, c_void};

    type Id  = *mut c_void;
    type Sel = *mut c_void;

    #[link(name = "objc")]
    // Each Rust name is a different view of the same C symbol `objc_msgSend`,
    // encoding the calling convention for a specific argument layout.
    // The clashing-declaration lint fires because Rust sees multiple signatures
    // for one link name — that is intentional here.
    #[allow(clashing_extern_declarations)]
    extern "C" {
        fn objc_getClass(name: *const u8) -> Id;
        fn sel_registerName(name: *const u8) -> Sel;

        #[link_name = "objc_msgSend"] fn msg0    (r: Id, s: Sel                     ) -> Id;
        #[link_name = "objc_msgSend"] fn msg1v   (r: Id, s: Sel, a: Id              );
        #[link_name = "objc_msgSend"] fn msg1id  (r: Id, s: Sel, a: Id              ) -> Id;
        #[link_name = "objc_msgSend"] fn msg1n   (r: Id, s: Sel, a: usize           ) -> Id;
        #[link_name = "objc_msgSend"] fn msg1cstr(r: Id, s: Sel, a: *const u8       ) -> Id;
        #[link_name = "objc_msgSend"] fn msg1f   (r: Id, s: Sel, a: f64              ) -> Id;
        #[link_name = "objc_msgSend"] fn msg2nv  (r: Id, s: Sel, a: usize,   b: Id  );
        #[link_name = "objc_msgSend"] fn msg2v   (r: Id, s: Sel, a: Id,     b: Id   );
        #[link_name = "objc_msgSend"] fn msg3    (r: Id, s: Sel, a: Id, b: Id, c: Id) -> Id;
    }

    #[link(name = "Foundation",        kind = "framework")] extern "C" {}
    #[link(name = "UserNotifications", kind = "framework")] extern "C" {}
    #[link(name = "AppKit",            kind = "framework")] extern "C" {}

    // _NSConcreteStackBlock is the `isa` pointer for stack-allocated ObjC blocks.
    // We construct a no-op block on the stack to pass as a completion handler,
    // which the callee copies to the heap immediately on entry.
    extern "C" {
        static _NSConcreteStackBlock: c_void;
    }

    /// Layout of a simple ObjC block with no captured variables.
    #[repr(C)]
    struct Block {
        isa:        *const c_void,
        flags:      i32,
        reserved:   i32,
        invoke:     unsafe extern "C" fn(*mut Block, u8 /*BOOL*/, Id),
        descriptor: *const BlockDesc,
    }

    /// Block variant whose invoke receives a single object (e.g. UNNotificationSettings *).
    #[repr(C)]
    struct Block1 {
        isa:        *const c_void,
        flags:      i32,
        reserved:   i32,
        invoke:     unsafe extern "C" fn(*mut Block1, Id),
        descriptor: *const BlockDesc,
    }

    #[repr(C)]
    struct BlockDesc { reserved: usize, size: usize }

    unsafe impl Sync for Block {}
    unsafe impl Sync for Block1 {}

    // Shared auth result: -1=pending, 0=denied, 1=granted.
    // Written from the ObjC completion block, read from the run-loop spin.
    static AUTH_RESULT: std::sync::atomic::AtomicI8 =
        std::sync::atomic::AtomicI8::new(-1);

    static BLOCK_DESC: BlockDesc = BlockDesc {
        reserved: 0,
        size: std::mem::size_of::<Block>(),
    };

    /// Auth callback: records granted/denied into AUTH_RESULT.
    unsafe extern "C" fn auth_invoke(_: *mut Block, granted: u8, _: Id) {
        use std::sync::atomic::Ordering;
        AUTH_RESULT.store(if granted != 0 { 1 } else { 0 }, Ordering::Release);
    }

    /// Settings callback: reads UNAuthorizationStatus into AUTH_RESULT.
    /// Values: -1=pending, 0=denied, 1=notDetermined, 2=authorized.
    unsafe extern "C" fn settings_invoke(_: *mut Block1, settings: Id) {
        use std::sync::atomic::Ordering;
        // UNAuthorizationStatus: 0=notDetermined, 1=denied, 2=authorized, 3+=other
        let status = msg0(settings, sel(b"authorizationStatus\0")) as usize;
        let result: i8 = match status {
            0 => 1, // notDetermined → 1
            1 => 0, // denied        → 0
            _ => 2, // authorized/provisional/ephemeral → 2
        };
        AUTH_RESULT.store(result, Ordering::Release);
    }

    /// Debug invoke: logs the authorization result and any NSError.
    unsafe extern "C" fn debug_auth_invoke(_: *mut Block, granted: u8, error: Id) {
        log(&format!("authorization callback: granted={}", granted != 0));
        if error.is_null() {
            log("auth error: nil");
        } else {
            let desc = msg0(error, sel(b"localizedDescription\0"));
            log(&format!("auth error: {}", nsstring_to_rust(desc)));
        }
    }

    /// Debug invoke for getNotificationSettings: logs the authorization status integer.
    unsafe extern "C" fn debug_settings_invoke(_: *mut Block1, settings: Id) {
        // UNAuthorizationStatus: 0=notDetermined, 1=denied, 2=authorized, 3=provisional, 4=ephemeral
        let status = msg0(settings, sel(b"authorizationStatus\0")) as usize;
        log(&format!("authorizationStatus (pre-request): {} (0=notDetermined 1=denied 2=authorized 3=provisional)", status));
    }

    unsafe fn cls(name: &[u8]) -> Id  { objc_getClass(name.as_ptr()) }
    unsafe fn sel(name: &[u8]) -> Sel { sel_registerName(name.as_ptr()) }

    unsafe fn nsstring(s: &str) -> Id {
        let cs = CString::new(s).unwrap_or_default();
        msg1cstr(cls(b"NSString\0"), sel(b"stringWithUTF8String:\0"), cs.as_ptr() as *const u8)
    }

    fn log(msg: &str) {
        use std::io::Write;
        let _ = std::fs::OpenOptions::new()
            .create(true).append(true)
            .open("/tmp/meatbag-notify-debug.txt")
            .and_then(|mut f| writeln!(f, "{}", msg));
    }

    unsafe fn nsstring_to_rust(s: Id) -> String {
        if s.is_null() { return "(nil)".into(); }
        // UTF8String returns const char*
        let ptr = msg0(s, sel(b"UTF8String\0")) as *const i8;
        if ptr.is_null() { return "(null ptr)".into(); }
        std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned()
    }

    /// Like send(), but logs each step to /tmp/meatbag-notify-debug.txt.
    pub fn send_debug() {
        let _ = std::fs::write("/tmp/meatbag-notify-debug.txt", "");
        log("send_debug: start");
        unsafe {
            // Check whether NSBundle.mainBundle sees our .app bundle
            let main_bundle = msg0(cls(b"NSBundle\0"), sel(b"mainBundle\0"));
            let bundle_id   = msg0(main_bundle, sel(b"bundleIdentifier\0"));
            let bundle_path = msg0(main_bundle, sel(b"bundlePath\0"));
            log(&format!("mainBundle path:       {}", nsstring_to_rust(bundle_path)));
            log(&format!("mainBundle identifier: {}", nsstring_to_rust(bundle_id)));

            log("calling NSApplication sharedApplication...");
            let app = msg0(cls(b"NSApplication\0"), sel(b"sharedApplication\0"));
            log(&format!("NSApplication: {:?}", app));

            log("getting UNUserNotificationCenter class...");
            let un = cls(b"UNUserNotificationCenter\0");
            log(&format!("UNUserNotificationCenter class: {:?}", un));
            if un.is_null() { log("FAIL: class is null"); return; }

            log("calling currentNotificationCenter...");
            let center = msg0(un, sel(b"currentNotificationCenter\0"));
            log(&format!("center: {:?}", center));
            if center.is_null() { log("FAIL: center is null"); return; }

            log("checking current authorization status...");
            let settings_block = Block1 {
                isa:        &_NSConcreteStackBlock as *const c_void,
                flags:      0,
                reserved:   0,
                invoke:     debug_settings_invoke,
                descriptor: &BLOCK_DESC,
            };
            msg1v(center,
                  sel(b"getNotificationSettingsWithCompletionHandler:\0"),
                  &settings_block as *const Block1 as Id);
            let rl2 = msg0(cls(b"NSRunLoop\0"), sel(b"currentRunLoop\0"));
            let d2  = msg1f(cls(b"NSDate\0"), sel(b"dateWithTimeIntervalSinceNow:\0"), 1.0);
            msg1v(rl2, sel(b"runUntilDate:\0"), d2);

            log("requesting authorization...");
            let auth_block = Block {
                isa:        &_NSConcreteStackBlock as *const c_void,
                flags:      0,
                reserved:   0,
                invoke:     debug_auth_invoke,
                descriptor: &BLOCK_DESC,
            };
            msg2nv(center,
                   sel(b"requestAuthorizationWithOptions:completionHandler:\0"),
                   7, &auth_block as *const Block as Id);
            log("authorization requested");

            log("building notification content...");
            let content = msg0(cls(b"UNMutableNotificationContent\0"), sel(b"new\0"));
            log(&format!("content: {:?}", content));
            msg1v(content, sel(b"setTitle:\0"), nsstring("Debug Test"));
            msg1v(content, sel(b"setBody:\0"),  nsstring("If you see this, it worked"));
            let sound = msg0(cls(b"UNNotificationSound\0"), sel(b"defaultSound\0"));
            msg1v(content, sel(b"setSound:\0"), sound);

            let uuid  = msg0(cls(b"NSUUID\0"), sel(b"UUID\0"));
            let ident = msg0(uuid, sel(b"UUIDString\0"));
            let request = msg3(
                cls(b"UNNotificationRequest\0"),
                sel(b"requestWithIdentifier:content:trigger:\0"),
                ident, content, std::ptr::null_mut(),
            );
            log(&format!("request: {:?}", request));

            log("calling addNotificationRequest...");
            msg2v(center,
                  sel(b"addNotificationRequest:withCompletionHandler:\0"),
                  request, std::ptr::null_mut());
            log("addNotificationRequest called");

            // Spin for up to 30 seconds so the user has time to click Allow/Don't Allow
            // in the permission dialog before the process exits.
            log("spinning run loop for up to 30s (waiting for auth dialog response)...");
            let run_loop = msg0(cls(b"NSRunLoop\0"), sel(b"currentRunLoop\0"));
            let deadline = msg1f(cls(b"NSDate\0"),
                                 sel(b"dateWithTimeIntervalSinceNow:\0"), 30.0);
            msg1v(run_loop, sel(b"runUntilDate:\0"), deadline);
            log("done");
        }
    }

    /// Schedule a UNUserNotificationCenter banner and return.
    /// Must be called from a process whose executable lives inside a .app bundle
    /// so macOS can associate the notification with a stable CFBundleIdentifier.
    pub fn send(title: &str, body: &str) {
        use std::sync::atomic::Ordering;
        unsafe {
            let app = msg0(cls(b"NSApplication\0"), sel(b"sharedApplication\0"));

            let un = cls(b"UNUserNotificationCenter\0");
            if un.is_null() { return; }
            let center = msg0(un, sel(b"currentNotificationCenter\0"));
            if center.is_null() { return; }

            let run_loop = msg0(cls(b"NSRunLoop\0"), sel(b"currentRunLoop\0"));

            // --- Phase 1: check current authorization status ---
            AUTH_RESULT.store(-1, Ordering::Release);
            let settings_block = Block1 {
                isa:        &_NSConcreteStackBlock as *const c_void,
                flags:      0, reserved: 0,
                invoke:     settings_invoke,
                descriptor: &BLOCK_DESC,
            };
            msg1v(center,
                  sel(b"getNotificationSettingsWithCompletionHandler:\0"),
                  &settings_block as *const Block1 as Id);
            let start = std::time::Instant::now();
            while AUTH_RESULT.load(Ordering::Acquire) < 0 {
                if start.elapsed().as_millis() > 500 { break; }
                let tick = msg1f(cls(b"NSDate\0"),
                                 sel(b"dateWithTimeIntervalSinceNow:\0"), 0.05);
                msg1v(run_loop, sel(b"runUntilDate:\0"), tick);
            }

            let settings_status = AUTH_RESULT.load(Ordering::Acquire);
            if settings_status == 0 { return; } // denied — nothing to do

            if settings_status != 2 {
                // notDetermined: switch to a regular (foreground) activation policy so
                // macOS will show the notification permission dialog. On Sequoia, apps with
                // NSApplicationActivationPolicyAccessory (LSUIElement) are blocked from
                // receiving the permission prompt.
                // NSApplicationActivationPolicyRegular = 0
                msg1n(app, sel(b"setActivationPolicy:\0"), 0);

                AUTH_RESULT.store(-1, Ordering::Release);
                let auth_block = Block {
                    isa:        &_NSConcreteStackBlock as *const c_void,
                    flags:      0, reserved: 0,
                    invoke:     auth_invoke,
                    descriptor: &BLOCK_DESC,
                };
                msg2nv(center,
                       sel(b"requestAuthorizationWithOptions:completionHandler:\0"),
                       7, &auth_block as *const Block as Id);

                // Wait up to 30s for the user to click Allow/Don't Allow.
                let start = std::time::Instant::now();
                while AUTH_RESULT.load(Ordering::Acquire) < 0 {
                    if start.elapsed().as_secs() > 30 { break; }
                    let tick = msg1f(cls(b"NSDate\0"),
                                     sel(b"dateWithTimeIntervalSinceNow:\0"), 0.1);
                    msg1v(run_loop, sel(b"runUntilDate:\0"), tick);
                }

                // Restore accessory (no Dock icon) policy.
                // NSApplicationActivationPolicyAccessory = 1
                msg1n(app, sel(b"setActivationPolicy:\0"), 1);

                if AUTH_RESULT.load(Ordering::Acquire) != 1 { return; }
            }

            // --- Phase 2: post the notification ---

            // Remove any existing notification for this project first.  If we simply
            // post with the same identifier macOS treats it as a "replacement" and
            // plays a system ding regardless of the notification's own sound setting
            // or System Settings > Notifications > Play Sound.  Removing silently
            // first, then posting fresh, avoids the replacement sound.
            let proj_id  = format!("meatbag-nudge.{}", title);
            let id_str   = nsstring(&proj_id);
            let id_array = msg1id(cls(b"NSArray\0"), sel(b"arrayWithObject:\0"), id_str);
            msg1v(center, sel(b"removeDeliveredNotificationsWithIdentifiers:\0"), id_array);
            // Brief spin so the removal XPC round-trip completes before we post.
            let rm_tick = msg1f(cls(b"NSDate\0"),
                                sel(b"dateWithTimeIntervalSinceNow:\0"), 0.05);
            msg1v(run_loop, sel(b"runUntilDate:\0"), rm_tick);

            let content = msg0(cls(b"UNMutableNotificationContent\0"), sel(b"new\0"));
            msg1v(content, sel(b"setTitle:\0"), nsstring(title));
            msg1v(content, sel(b"setBody:\0"),  nsstring(body));
            // threadIdentifier groups notifications by project in Notification Center.
            msg1v(content, sel(b"setThreadIdentifier:\0"), nsstring(title));
            // No notification sound — afplay already handles audio in the main process.

            let ident = nsstring(&proj_id);
            let request = msg3(
                cls(b"UNNotificationRequest\0"),
                sel(b"requestWithIdentifier:content:trigger:\0"),
                ident, content, std::ptr::null_mut(),
            );
            msg2v(center,
                  sel(b"addNotificationRequest:withCompletionHandler:\0"),
                  request, std::ptr::null_mut());

            // Spin briefly so UNUserNotificationCenter can deliver the notification over XPC.
            let deadline = msg1f(cls(b"NSDate\0"),
                                 sel(b"dateWithTimeIntervalSinceNow:\0"), 1.0);
            msg1v(run_loop, sel(b"runUntilDate:\0"), deadline);
        }
    }

    /// Remove a delivered notification for `title` from Notification Center.
    pub fn remove(title: &str) {
        remove_impl(title, false);
    }

    /// Like remove(), but logs each step to /tmp/meatbag-remove-debug.txt.
    pub fn remove_debug(title: &str) {
        remove_impl(title, true);
    }

    fn remove_impl(title: &str, debug: bool) {
        unsafe {
            let bundle = msg0(cls(b"NSBundle\0"), sel(b"mainBundle\0"));
            let bundle_id  = msg0(bundle, sel(b"bundleIdentifier\0"));
            let bundle_path = msg0(bundle, sel(b"bundlePath\0"));
            if debug {
                log(&format!("remove: bundlePath={} bundleId={}",
                    nsstring_to_rust(bundle_path), nsstring_to_rust(bundle_id)));
            }

            let un = cls(b"UNUserNotificationCenter\0");
            if un.is_null() {
                if debug { log("remove: UNUserNotificationCenter class is null"); }
                return;
            }
            let center = msg0(un, sel(b"currentNotificationCenter\0"));
            if center.is_null() {
                if debug { log("remove: center is null"); }
                return;
            }

            let proj_id  = format!("meatbag-nudge.{}", title);
            if debug { log(&format!("remove: removing identifier={}", proj_id)); }
            let id_str   = nsstring(&proj_id);
            let id_array = msg1id(cls(b"NSArray\0"), sel(b"arrayWithObject:\0"), id_str);
            msg1v(center, sel(b"removeDeliveredNotificationsWithIdentifiers:\0"), id_array);

            // Spin long enough for the XPC removal to complete before the process exits.
            let run_loop = msg0(cls(b"NSRunLoop\0"), sel(b"currentRunLoop\0"));
            let tick = msg1f(cls(b"NSDate\0"),
                             sel(b"dateWithTimeIntervalSinceNow:\0"), 0.5);
            msg1v(run_loop, sel(b"runUntilDate:\0"), tick);
            if debug { log("remove: done"); }
        }
    }
}

/// Walk up from `exe` to find the enclosing `.app` bundle root, if any.
#[cfg(target_os = "macos")]
fn find_app_bundle(exe: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut path = exe.to_path_buf();
    loop {
        if path.extension().map_or(false, |e| e == "app") {
            return Some(path);
        }
        if !path.pop() {
            return None;
        }
    }
}

/// Detect a running IDE and bring the window for `path` to the foreground.
/// Uses each IDE's CLI with `--reuse-window` so it targets the specific
/// project window rather than the most-recently-active one.
/// Run via a login shell so the user's PATH (where `cursor`/`code` live) is available.
#[cfg(target_os = "macos")]
fn focus_ide_at(path: &str) {
    // (pgrep process name, CLI command, supports --reuse-window)
    let candidates: &[(&str, &str, bool)] = &[
        ("Cursor",   "cursor",   true),
        ("Code",     "code",     true),
        ("Windsurf", "windsurf", true),
        ("zed",      "zed",      false),
    ];
    // Shell-safe single-quote escaping for the path argument.
    let path_escaped = path.replace('\'', "'\\''");
    for &(proc_name, cli, reuse) in candidates {
        let running = Command::new("pgrep")
            .args(["-x", proc_name])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if running {
            let cmd = if reuse {
                format!("{} --reuse-window '{}'", cli, path_escaped)
            } else {
                format!("{} '{}'", cli, path_escaped)
            };
            let _ = Command::new("sh")
                .args(["-l", "-c", &cmd])
                .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
                .spawn();
            return;
        }
    }
}

/// Send a macOS Notification Center banner.
///
/// Launches the bundle via `open -a` (LaunchServices) so macOS registers
/// it as a proper app — required on Sequoia for UNUserNotificationCenter
/// to show the permission prompt and appear in Notification settings.
///
/// Falls back to `osascript` for development / non-bundle runs.
#[cfg(target_os = "macos")]
fn send_macos_notification(title: &str, body: &str) {
    // Record this project's path so a notification click can focus the right IDE window.
    // Stored per-project so multiple active notifications each know their own path.
    {
        let cwd = env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let dir = state_dir();
        let _ = fs::create_dir_all(&dir);
        // Sanitise title for use as a filename component.
        let safe: String = title.chars()
            .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
            .collect();
        let _ = fs::write(PathBuf::from(&dir).join(format!("notify-path-{}", safe)), &cwd);
    }

    let exe = std::env::current_exe()
        .and_then(|p| fs::canonicalize(p))
        .unwrap_or_default();

    if let Some(bundle) = find_app_bundle(&exe) {
        let bundle_str = bundle.to_string_lossy();
        // -n  = new instance even if already running
        // -g  = don't bring app to foreground
        spawn_detached("open", &[
            "-a", bundle_str.as_ref(),
            "-n", "-g",
            "--args", "_notify", title, body,
        ]);
    } else {
        // Non-bundle (e.g. cargo run) — fall back to osascript
        let safe_body  = body .replace('\\', "\\\\").replace('"', "\\\"");
        let safe_title = title.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!(
            "display notification \"{}\" with title \"{}\"",
            safe_body, safe_title,
        );
        let _ = Command::new("osascript")
            .args(["-e", &script])
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
            .spawn();
    }
}

/// Remove the delivered notification for `title` and clean up its state file.
/// The hook binary resolves via symlink to the binary inside the .app bundle,
/// so NSBundle.mainBundle already has the correct bundle ID — no subprocess needed.
#[cfg(target_os = "macos")]
fn dismiss_macos_notification(state_dir: &str, title: &str) {
    // Spawn removal as a subprocess using the fully-resolved exe path.
    // When the hook binary is invoked via a symlink (e.g. ~/bin/meatbag-nudge),
    // NSBundle sees ~/bin as the bundle root and gets a nil bundleIdentifier, so
    // UNUserNotificationCenter can't touch notifications posted by the .app bundle.
    // Spawning via the real path puts the process inside the .app, fixing the lookup.
    let exe = std::env::current_exe()
        .and_then(|p| fs::canonicalize(p))
        .unwrap_or_default();
    spawn_detached(&exe.to_string_lossy(), &["_remove_notify", title]);

    let safe: String = title.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let _ = fs::remove_file(PathBuf::from(state_dir).join(format!("notify-path-{}", safe)));
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
    tick_sleep(delay_secs);

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

    // Default escalation: flash (Windows) / notification banner (macOS) + play sound
    #[cfg(windows)]
    {
        let flash_count: u32 = env::var("MEATBAG_FLASH_COUNT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);
        win32::flash_screen(flash_count);
    }
    #[cfg(target_os = "macos")]
    {
        let proj = project_name();
        send_macos_notification(&proj, &notification_body("escalation"));
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
                tick_sleep(2);
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
// Settings management (install-hooks / remove-hooks subcommands)
// ---------------------------------------------------------------------------

fn claude_settings_path() -> PathBuf {
    #[cfg(windows)]
    {
        let profile = env::var("USERPROFILE").unwrap_or_default();
        PathBuf::from(profile).join(".claude").join("settings.json")
    }
    #[cfg(not(windows))]
    {
        let home = env::var("HOME").unwrap_or_default();
        PathBuf::from(home).join(".claude").join("settings.json")
    }
}

fn hook_entry(cmd: &str) -> serde_json::Value {
    serde_json::json!([{"hooks": [{"type": "command", "command": cmd}]}])
}

fn run_install_hooks(args: &[String]) -> i32 {
    fn flag(args: &[String], name: &str) -> Option<String> {
        args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
    }

    let settings = flag(args, "--settings").map(PathBuf::from)
        .unwrap_or_else(claude_settings_path);
    let binary = flag(args, "--binary").unwrap_or_else(|| "meatbag-nudge".to_string());
    let stop_cmd = flag(args, "--stop").unwrap_or_default();
    let perm_cmd = flag(args, "--permission").unwrap_or_default();
    let overwrite = args.iter().any(|a| a == "--overwrite");

    let existing = fs::read_to_string(&settings).unwrap_or_else(|_| "{}".to_string());
    let mut data: serde_json::Value = serde_json::from_str(&existing)
        .unwrap_or_else(|_| serde_json::json!({}));

    if !overwrite {
        let hooks_str = data.get("hooks").map(|h| h.to_string()).unwrap_or_default();
        if hooks_str.contains("meatbag-nudge") {
            eprint!("Hooks already exist. Overwrite? [y/N]: ");
            let _ = io::stdout().flush();
            let mut response = String::new();
            let _ = io::stdin().read_line(&mut response);
            if !response.trim().eq_ignore_ascii_case("y") {
                eprintln!("Keeping existing hooks.");
                return 0;
            }
        }
    }

    let cancel_cmd  = format!("\"{}\" cancel", binary);
    let dismiss_cmd = format!("\"{}\" dismiss", binary);
    let prompt_cmd  = format!("\"{}\" prompt", binary);

    if let Some(obj) = data.as_object_mut() {
        let hooks = obj.entry("hooks").or_insert_with(|| serde_json::json!({}));
        if let Some(h) = hooks.as_object_mut() {
            h.insert("Stop".into(),                hook_entry(&stop_cmd));
            h.insert("PermissionRequest".into(),   hook_entry(&perm_cmd));
            h.insert("PostToolUse".into(),          hook_entry(&cancel_cmd));
            h.insert("PostToolUseFailure".into(),   hook_entry(&dismiss_cmd));
            h.insert("UserPromptSubmit".into(),     hook_entry(&prompt_cmd));
        }
    }

    if let Some(parent) = settings.parent() {
        let _ = fs::create_dir_all(parent);
    }
    match serde_json::to_string_pretty(&data) {
        Ok(out) => match fs::write(&settings, out) {
            Ok(_)  => { eprintln!("Hooks configured in {}", settings.display()); 0 }
            Err(e) => { eprintln!("Error writing {}: {}", settings.display(), e); 1 }
        },
        Err(e) => { eprintln!("Error serialising settings: {}", e); 1 }
    }
}

fn run_remove_hooks(args: &[String]) -> i32 {
    fn flag(args: &[String], name: &str) -> Option<String> {
        args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
    }

    let settings = flag(args, "--settings").map(PathBuf::from)
        .unwrap_or_else(claude_settings_path);

    let existing = match fs::read_to_string(&settings) {
        Ok(s)  => s,
        Err(_) => return 0,
    };
    let mut data: serde_json::Value = match serde_json::from_str(&existing) {
        Ok(v)  => v,
        Err(_) => return 0,
    };

    if let Some(hooks) = data.get_mut("hooks").and_then(|h| h.as_object_mut()) {
        hooks.retain(|_, v| !v.to_string().contains("meatbag-nudge"));
    }

    match serde_json::to_string_pretty(&data) {
        Ok(out) => match fs::write(&settings, out) {
            Ok(_)  => { eprintln!("Hooks removed from {}", settings.display()); 0 }
            Err(e) => { eprintln!("Error writing {}: {}", settings.display(), e); 1 }
        },
        Err(e) => { eprintln!("Error serialising settings: {}", e); 1 }
    }
}

/// Replace the binary reference at the start of a hook command string with
/// `new_binary`, preserving the subcommand and all flags.
/// Handles both quoted (`"path" args`) and unquoted (`path args`) forms.
fn rewrite_cmd_binary(cmd: &str, new_binary: &str) -> String {
    let rest = if let Some(after_open) = cmd.strip_prefix('"') {
        // Find the closing quote after the opening one
        after_open.find('"').map(|i| &cmd[i + 2..]).unwrap_or("")
    } else {
        cmd.find(' ').map(|i| &cmd[i..]).unwrap_or("")
    };
    format!("\"{}\"{}",  new_binary, rest)
}

fn run_copy_hooks(args: &[String]) -> i32 {
    fn flag(args: &[String], name: &str) -> Option<String> {
        args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
    }

    let from = match flag(args, "--from").map(PathBuf::from) {
        Some(p) => p,
        None => { eprintln!("copy-hooks: --from <source-settings> is required"); return 1; }
    };
    let binary  = flag(args, "--binary").unwrap_or_else(|| "meatbag-nudge".to_string());
    let settings = flag(args, "--settings").map(PathBuf::from)
        .unwrap_or_else(claude_settings_path);
    let overwrite = args.iter().any(|a| a == "--overwrite");

    let src_str = match fs::read_to_string(&from) {
        Ok(s)  => s,
        Err(e) => { eprintln!("copy-hooks: cannot read {}: {}", from.display(), e); return 1; }
    };
    let src_data: serde_json::Value = match serde_json::from_str(&src_str) {
        Ok(v)  => v,
        Err(e) => { eprintln!("copy-hooks: invalid JSON in {}: {}", from.display(), e); return 1; }
    };

    let existing = fs::read_to_string(&settings).unwrap_or_else(|_| "{}".to_string());
    let mut data: serde_json::Value = serde_json::from_str(&existing)
        .unwrap_or_else(|_| serde_json::json!({}));

    if !overwrite {
        let hooks_str = data.get("hooks").map(|h| h.to_string()).unwrap_or_default();
        if hooks_str.contains("meatbag-nudge") {
            eprint!("Hooks already exist. Overwrite? [y/N]: ");
            let _ = io::stdout().flush();
            let mut response = String::new();
            let _ = io::stdin().read_line(&mut response);
            if !response.trim().eq_ignore_ascii_case("y") {
                eprintln!("Keeping existing hooks.");
                return 0;
            }
        }
    }

    if let Some(src_hooks) = src_data.get("hooks").and_then(|h| h.as_object()) {
        let dest_hooks = data.as_object_mut()
            .unwrap()
            .entry("hooks")
            .or_insert_with(|| serde_json::json!({}));

        for (event, entries) in src_hooks {
            if let Some(arr) = entries.as_array() {
                let new_entries: Vec<serde_json::Value> = arr.iter().map(|entry| {
                    if let Some(hooks_arr) = entry.get("hooks").and_then(|h| h.as_array()) {
                        let new_hooks: Vec<serde_json::Value> = hooks_arr.iter().map(|h| {
                            if let Some(cmd) = h.get("command").and_then(|c| c.as_str()) {
                                let mut h2 = h.clone();
                                h2["command"] = serde_json::Value::String(
                                    rewrite_cmd_binary(cmd, &binary)
                                );
                                h2
                            } else {
                                h.clone()
                            }
                        }).collect();
                        serde_json::json!({"hooks": new_hooks})
                    } else {
                        entry.clone()
                    }
                }).collect();
                dest_hooks[event] = serde_json::Value::Array(new_entries);
            }
        }
    } else {
        eprintln!("copy-hooks: no hooks section found in {}", from.display());
        return 1;
    }

    if let Some(parent) = settings.parent() {
        let _ = fs::create_dir_all(parent);
    }
    match serde_json::to_string_pretty(&data) {
        Ok(out) => match fs::write(&settings, out) {
            Ok(_)  => { eprintln!("Hooks copied to {}", settings.display()); 0 }
            Err(e) => { eprintln!("Error writing {}: {}", settings.display(), e); 1 }
        },
        Err(e) => { eprintln!("Error serialising settings: {}", e); 1 }
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
        #[cfg(target_os = "macos")]
        "_notify" => {
            let title = args.get(2).map(|s| s.as_str()).unwrap_or("Claude Code");
            let body  = args.get(3).map(|s| s.as_str()).unwrap_or("");
            macos_notify::send(title, body);
            return;
        }
        #[cfg(target_os = "macos")]
        "_remove_notify" => {
            let title = args.get(2).map(|s| s.as_str()).unwrap_or("");
            if !title.is_empty() {
                macos_notify::remove(title);
            }
            return;
        }
        #[cfg(target_os = "macos")]
        "_remove_notify_debug" => {
            let title = args.get(2).map(|s| s.as_str()).unwrap_or("test");
            let _ = std::fs::write("/tmp/meatbag-remove-debug.txt", "");
            macos_notify::remove_debug(title);
            return;
        }
        #[cfg(target_os = "macos")]
        "_notify_body" => {
            if args.iter().any(|a| a == "--fun") {
                env::set_var("MEATBAG_FUN_MESSAGES", "1");
            }
            let fun_val = env::var("MEATBAG_FUN_MESSAGES").unwrap_or_else(|_| "(not set)".into());
            let fun_bool = fun_val == "1" || fun_val.to_lowercase() == "true";
            let idx = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos() as usize;
            println!("MEATBAG_FUN_MESSAGES={}", fun_val);
            println!("fun={} idx={} idx%20={}", fun_bool, idx, idx % 20);
            println!("done:       {}", notification_body("done"));
            println!("attention:  {}", notification_body("attention"));
            println!("escalation: {}", notification_body("escalation"));
            return;
        }
        #[cfg(target_os = "macos")]
        "_notify_debug" => {
            macos_notify::send_debug();
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
    if let Some(v) = parse_flag(&args, "--project") {
        env::set_var("MEATBAG_PROJECT", &v);
    }
    if args.iter().any(|a| a == "--fun") {
        env::set_var("MEATBAG_FUN_MESSAGES", "1");
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
            let is_question = message_is_question(&input);
            if is_question {
                start_escalation(&dir, stop_delay);
            }
            #[cfg(target_os = "macos")]
            {
                let proj = project_name();
                let body = if is_question {
                    notification_body("attention")
                } else {
                    notification_body("done")
                };
                send_macos_notification(&proj, &body);
            }
        }
        "permission" => {
            capture_focus_target();
            if focus_at("notification") { focus_window(); }
            play_sound(&dir, cooldown);
            start_escalation(&dir, permission_delay);
            #[cfg(target_os = "macos")]
            {
                let proj = project_name();
                send_macos_notification(&proj, &notification_body("attention"));
            }
        }
        "prompt" => {
            #[cfg(target_os = "macos")]
            dismiss_macos_notification(&dir, &project_name());
            handle_prompt(&dir);
        }
        "cancel" => {
            cancel_pending(&dir);
            #[cfg(target_os = "macos")]
            dismiss_macos_notification(&dir, &project_name());
        }
        "dismiss" => {
            record_interaction(&dir);
            cancel_pending(&dir);
            #[cfg(target_os = "macos")]
            dismiss_macos_notification(&dir, &project_name());
        }
        "install-hooks" => {
            process::exit(run_install_hooks(&args));
        }
        "remove-hooks" => {
            process::exit(run_remove_hooks(&args));
        }
        "copy-hooks" => {
            process::exit(run_copy_hooks(&args));
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
            // When macOS opens our .app bundle because the user clicked a notification
            // banner, the binary runs with no arguments.  Detect this case (no args +
            // inside a .app bundle) and focus the IDE for the last notified project.
            #[cfg(target_os = "macos")]
            if action.is_empty() {
                let exe = env::current_exe()
                    .and_then(|p| fs::canonicalize(p))
                    .unwrap_or_default();
                if find_app_bundle(&exe).is_some() {
                    // Pick the most-recently-modified notify-path-* file — that's
                    // the notification the user most likely just clicked.
                    let dir = PathBuf::from(state_dir());
                    let best = fs::read_dir(&dir).ok().and_then(|entries| {
                        entries
                            .filter_map(|e| e.ok())
                            .filter(|e| e.file_name().to_string_lossy().starts_with("notify-path-"))
                            .filter_map(|e| {
                                let mtime = e.metadata().ok()?.modified().ok()?;
                                Some((mtime, e.path()))
                            })
                            .max_by_key(|(mtime, _)| *mtime)
                            .map(|(_, p)| p)
                    });
                    if let Some(path_file) = best {
                        if let Ok(path) = fs::read_to_string(&path_file) {
                            let path = path.trim();
                            if !path.is_empty() {
                                focus_ide_at(path);
                                return;
                            }
                        }
                    }
                }
            }

            eprintln!("Usage: meatbag-nudge <action> [options]");
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
