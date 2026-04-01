# Window Focus on Nudge — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `--focus [EVENTS]` and `--focus-cmd CMD` flags that optionally focus the terminal window running Claude Code when a notification or escalation fires.

**Architecture:** All changes are confined to `src/main.rs`. The `win32` module gains HWND discovery (`GetConsoleWindow` + process-tree walk via `CreateToolhelp32Snapshot`/`EnumWindows`) and `SetForegroundWindow`. Three new cross-platform functions handle the feature: `capture_focus_target()` stores the window handle in `MEATBAG_FOCUS_HWND` at hook time (so the detached escalation child inherits it automatically), `focus_window()` performs the actual focus, and `focus_at(event)` checks whether focusing should fire for a given event. On WSL, `focus_window()` spawns `powershell.exe` using `WScript.Shell.AppActivate` (fast, no C# compilation).

**Tech Stack:** Rust, Win32 API (user32.dll, kernel32.dll), PowerShell + WScript.Shell COM (WSL focus path)

---

### Task 1: Add HWND discovery and SetForegroundWindow to the win32 module

**Files:**
- Modify: `src/main.rs` (inside `#[cfg(windows)] mod win32 { }`, lines 14–264)

- [ ] **Step 1: Add atomic statics and use statement for the EnumWindows callback**

After the two existing `use` statements inside `mod win32` (lines 16–17), add:

```rust
    use std::sync::atomic::{AtomicUsize, Ordering};

    static ENUM_FOUND_HWND: AtomicUsize = AtomicUsize::new(0);
    static ENUM_TARGET_PID: AtomicUsize = AtomicUsize::new(0);
```

- [ ] **Step 2: Add ProcessEntry32W struct**

After the `Msg` struct (after line 145), add:

```rust
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
```

- [ ] **Step 3: Add new extern declarations**

After the `#[link(name = "gdi32")]` block (after line 168), add:

```rust
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> usize;
        fn Process32FirstW(snapshot: usize, entry: *mut ProcessEntry32W) -> i32;
        fn Process32NextW(snapshot: usize, entry: *mut ProcessEntry32W) -> i32;
    }

    extern "system" {
        fn GetConsoleWindow() -> usize;
        fn SetForegroundWindow(hwnd: usize) -> i32;
        fn EnumWindows(callback: extern "system" fn(usize, isize) -> i32, lparam: isize) -> i32;
        fn GetWindowThreadProcessId(hwnd: usize, pid: *mut u32) -> u32;
        fn GetWindowLongW(hwnd: usize, index: i32) -> i32;
        fn IsWindowVisible(hwnd: usize) -> i32;
    }
```

Note: `CloseHandle` is already declared in the first `extern "system"` block (line 61) and reused by `parent_pid` below without a re-declaration.

- [ ] **Step 4: Add helper functions before the closing `}` of the win32 module**

Before the final `}` of `mod win32` (line 264), add:

```rust
    extern "system" fn enum_window_cb(hwnd: usize, _: isize) -> i32 {
        let target = ENUM_TARGET_PID.load(Ordering::Relaxed) as u32;
        unsafe {
            let mut pid: u32 = 0;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid == target && IsWindowVisible(hwnd) != 0 {
                const GWL_STYLE: i32 = -16;
                const WS_CHILD: u32 = 0x40000000;
                let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
                if style & WS_CHILD == 0 {
                    ENUM_FOUND_HWND.store(hwnd, Ordering::Relaxed);
                    return 0; // stop enumeration
                }
            }
        }
        1 // continue
    }

    fn main_window_for_pid(pid: u32) -> usize {
        ENUM_FOUND_HWND.store(0, Ordering::Relaxed);
        ENUM_TARGET_PID.store(pid as usize, Ordering::Relaxed);
        unsafe { EnumWindows(enum_window_cb, 0); }
        ENUM_FOUND_HWND.load(Ordering::Relaxed)
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
```

- [ ] **Step 5: Build and verify no new errors**

```bash
cd /home/jamiet/code/meatbag-nudge
mise run build 2>&1 | tail -40
```

Expected: build succeeds. There may be pre-existing warnings; no new errors.

- [ ] **Step 6: Commit**

```bash
git add src/main.rs
git commit -m "feat: add Win32 HWND discovery and focus helpers to win32 module"
```

---

### Task 2: Add cross-platform focus helpers

**Files:**
- Modify: `src/main.rs` — add after `play_wav` (after line ~523), before the `// State management` comment

- [ ] **Step 1: Add focus_at**

```rust
/// Returns true if focus should fire for the given event.
/// event is "notification" or "escalation".
/// MEATBAG_FOCUS is a comma-separated list, e.g. "notification,escalation".
fn focus_at(event: &str) -> bool {
    env::var("MEATBAG_FOCUS")
        .map(|v| v.split(',').any(|e| e.trim() == event))
        .unwrap_or(false)
}
```

- [ ] **Step 2: Add capture_focus_target and wsl_nt_parent_pid**

```rust
/// Capture the terminal window identifier into MEATBAG_FOCUS_HWND so the
/// detached escalation child inherits it. Call this at the start of
/// stop/permission handlers, before start_escalation.
fn capture_focus_target() {
    if !env::var("MEATBAG_FOCUS").map(|v| !v.is_empty()).unwrap_or(false) {
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
```

- [ ] **Step 3: Add focus_window and build_wsl_focus_script**

```rust
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
```

- [ ] **Step 4: Build and verify**

```bash
mise run build 2>&1 | tail -40
```

Expected: clean build, no new errors.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs
git commit -m "feat: add focus_at, capture_focus_target, and focus_window helpers"
```

---

### Task 3: Add --focus and --focus-cmd flag parsing

**Files:**
- Modify: `src/main.rs` — inside `main()`, around lines 748–773

- [ ] **Step 1: Add parse_flag_opt_val inner function alongside parse_flag**

Inside `main()`, directly after the existing `fn parse_flag` inner function (around line 751), add:

```rust
    fn parse_flag_opt_val(args: &[String], flag: &str, default_val: &str) -> Option<String> {
        args.iter().position(|a| a == flag).map(|i| {
            match args.get(i + 1) {
                Some(v) if !v.starts_with('-') => v.clone(),
                _ => default_val.to_string(),
            }
        })
    }
```

- [ ] **Step 2: Parse --focus and --focus-cmd alongside existing flags**

After the existing `--player` flag block (around line 773), add:

```rust
    if let Some(v) = parse_flag_opt_val(&args, "--focus", "escalation") {
        env::set_var("MEATBAG_FOCUS", &v);
    }
    if let Some(v) = parse_flag(&args, "--focus-cmd") {
        env::set_var("MEATBAG_FOCUS_CMD", &v);
    }
```

- [ ] **Step 3: Add to usage help text**

In the `_ =>` arm of the main `match action` block, add to the Options section:

```rust
    eprintln!("  --focus [EVENTS]  Focus terminal on nudge; EVENTS is comma-separated list of notification,escalation (default when flag present: escalation)");
    eprintln!("  --focus-cmd CMD   Shell command to focus terminal, overrides built-in focus");
```

- [ ] **Step 4: Build and verify**

```bash
mise run build 2>&1 | tail -40
```

Expected: clean build.

- [ ] **Step 5: Verify flag parsing behaves correctly**

```bash
# --focus alone should default to "escalation"
MEATBAG_STATE_DIR=/tmp/test-focus ./target/release/claude-notify --focus stop 2>&1 | head -5
printenv MEATBAG_FOCUS  # won't work since it's in child env, but build confirms no crash

# Verify unknown action exits with usage
./target/release/claude-notify --focus 2>&1 | grep -i focus
```

Expected: the `--focus` line appears in the help output.

- [ ] **Step 6: Commit**

```bash
git add src/main.rs
git commit -m "feat: add --focus and --focus-cmd flag parsing"
```

---

### Task 4: Integrate focus into stop and permission handlers

**Files:**
- Modify: `src/main.rs` — the `stop` and `permission` match arms inside `main()`, around lines 812–821

- [ ] **Step 1: Update the stop handler**

Replace:
```rust
        "stop" => {
            let input = read_stdin_timeout(Duration::from_millis(100));
            play_sound(&dir, cooldown);
            if message_is_question(&input) {
                start_escalation(&dir, stop_delay);
            }
        }
```

With:
```rust
        "stop" => {
            let input = read_stdin_timeout(Duration::from_millis(100));
            capture_focus_target();
            if focus_at("notification") { focus_window(); }
            play_sound(&dir, cooldown);
            if message_is_question(&input) {
                start_escalation(&dir, stop_delay);
            }
        }
```

- [ ] **Step 2: Update the permission handler**

Replace:
```rust
        "permission" => {
            play_sound(&dir, cooldown);
            start_escalation(&dir, permission_delay);
        }
```

With:
```rust
        "permission" => {
            capture_focus_target();
            if focus_at("notification") { focus_window(); }
            play_sound(&dir, cooldown);
            start_escalation(&dir, permission_delay);
        }
```

- [ ] **Step 3: Build and verify**

```bash
mise run build 2>&1 | tail -40
```

Expected: clean build.

- [ ] **Step 4: Smoke test — notification focus**

Switch focus to a different window, then run:

```bash
MEATBAG_FOCUS=notification ./target/release/claude-notify stop
```

Expected: the terminal window comes to the foreground.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs
git commit -m "feat: call capture_focus_target and focus_window in stop/permission handlers"
```

---

### Task 5: Integrate focus into run_escalation

**Files:**
- Modify: `src/main.rs` — `run_escalation` function, around lines 638–701

- [ ] **Step 1: Add focus call after the sound loop at the end of run_escalation**

The end of `run_escalation` currently looks like:

```rust
    if !sound.is_empty() {
        for i in 0..sound_repeat {
            if i > 0 {
                std::thread::sleep(Duration::from_secs(2));
            }
            play_wav(&sound);
        }
    }

    let _ = fs::remove_file(pid_path(&dir));
```

Replace with:

```rust
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
```

- [ ] **Step 2: Also add focus to the MEATBAG_ESCALATION_CMD early-return path**

In `run_escalation`, the custom escalation command block currently ends with:

```rust
        let _ = shell_result;
        let _ = fs::remove_file(pid_path(&dir));
        return;
```

Replace with:

```rust
        let _ = shell_result;
        if focus_at("escalation") {
            focus_window();
        }
        let _ = fs::remove_file(pid_path(&dir));
        return;
```

- [ ] **Step 3: Build and verify**

```bash
mise run build 2>&1 | tail -40
```

Expected: clean build.

- [ ] **Step 4: Smoke test — escalation focus**

Switch focus to a different window after running the command. The terminal should come to the foreground after the delay.

```bash
MEATBAG_FOCUS=escalation ./target/release/claude-notify stop --delay 5
# Switch to another window within 5 seconds
# Expected: terminal comes to foreground after 5s
```

- [ ] **Step 5: Smoke test — both**

```bash
MEATBAG_FOCUS=notification,escalation ./target/release/claude-notify stop --delay 5
# Expected: terminal focuses immediately (notification), then again after 5s (escalation)
```

- [ ] **Step 6: Smoke test — focus-cmd override**

```bash
MEATBAG_FOCUS=escalation MEATBAG_FOCUS_CMD="echo 'focus fired'" ./target/release/claude-notify stop --delay 3
# Expected: "focus fired" appears in terminal after 3s (as a detached child, may appear asynchronously)
```

- [ ] **Step 7: Commit**

```bash
git add src/main.rs
git commit -m "feat: call focus_window in run_escalation after flash/sound"
```

---

### Task 6: Update README

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Add --focus and --focus-cmd to the flags table**

In the Configuration section, add two rows to the flags table:

```markdown
| `--focus [EVENTS]` | *(off)* | Focus terminal window on nudge; `EVENTS` is a comma-separated list of `notification`, `escalation`. Bare `--focus` defaults to `escalation`. |
| `--focus-cmd CMD` | *(none)* | Shell command to run for focusing. Overrides built-in behaviour; timing controlled by `--focus`. |
```

- [ ] **Step 2: Add MEATBAG_FOCUS and MEATBAG_FOCUS_CMD to the env vars table**

```markdown
| `MEATBAG_FOCUS` | `--focus` |
| `MEATBAG_FOCUS_CMD` | `--focus-cmd` |
```

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs: add --focus and --focus-cmd to README"
```

---

## Self-Review

**Spec coverage:**
- `--focus [EVENTS]` with comma-separated values and bare-flag default → Task 3 ✓
- `--focus-cmd CMD` / `MEATBAG_FOCUS_CMD` override → Tasks 2, 3 ✓
- `MEATBAG_FOCUS_HWND` set at hook time, inherited by escalation child → Task 2 ✓
- Windows: `GetConsoleWindow` + process tree fallback → Task 1 ✓
- Windows: `SetForegroundWindow` → Task 1 ✓
- WSL: NtTgid PID capture → Task 2 ✓
- WSL: PowerShell `WScript.Shell.AppActivate` focus → Task 2 ✓
- WSL2 fallback (no NtTgid): activate Windows Terminal by name → Task 2 ✓
- Notification timing → Task 4 ✓
- Escalation timing (default and MEATBAG_ESCALATION_CMD path) → Task 5 ✓
- Silent no-op when HWND is 0 or MEATBAG_FOCUS_HWND missing → `win32::focus_hwnd` checks hwnd != 0; `focus_window` on WSL handles missing env var in script fallback ✓
- README → Task 6 ✓

**Placeholder scan:** No TBDs, no "implement later", all code blocks present.

**Type consistency:**
- `focus_at(event: &str) -> bool` — defined Task 2, used Tasks 4, 5 ✓
- `focus_window()` — defined Task 2, used Tasks 4, 5 ✓
- `capture_focus_target()` — defined Task 2, used Task 4 ✓
- `win32::find_terminal_hwnd() -> usize` — defined Task 1, used Task 2 ✓
- `win32::focus_hwnd(hwnd: usize)` — defined Task 1, used Task 2 ✓
- `build_wsl_focus_script() -> String` — defined and used in Task 2 ✓
