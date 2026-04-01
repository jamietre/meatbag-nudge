use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_claude-notify");
/// Each tick unit = 100ms wall-clock time in tests.
const TICK_MS: &str = "100";

struct TestEnv {
    state_dir: TempDir,
    notif_sentinel: PathBuf,
    esc_sentinel: PathBuf,
}

impl TestEnv {
    fn new() -> Self {
        let state_dir = tempfile::tempdir().unwrap();
        let notif_sentinel = state_dir.path().join("notif-sentinel");
        let esc_sentinel = state_dir.path().join("esc-sentinel");
        TestEnv { state_dir, notif_sentinel, esc_sentinel }
    }

    fn run(&self, args: &[&str]) {
        self.run_with_stdin(args, "");
    }

    fn run_with_stdin(&self, args: &[&str], stdin_data: &str) {
        let notif_cmd = format!("touch {}", self.notif_sentinel.display());
        let esc_cmd = format!("touch {}", self.esc_sentinel.display());

        let mut cmd = Command::new(BIN);
        cmd.args(args)
            .env("MEATBAG_STATE_DIR", self.state_dir.path())
            .env("MEATBAG_TICK_MS", TICK_MS)
            .env("MEATBAG_NOTIFICATION_CMD", &notif_cmd)
            .env("MEATBAG_ESCALATION_CMD", &esc_cmd)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let mut child = cmd.spawn().unwrap();
        if !stdin_data.is_empty() {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(stdin_data.as_bytes());
            }
        }
        let _ = child.wait();
    }

    fn wait_for_file(path: &PathBuf, timeout_ms: u64) -> bool {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            if path.exists() { return true; }
            if Instant::now() >= deadline { return false; }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn wait_for_notif(&self, timeout_ms: u64) -> bool {
        Self::wait_for_file(&self.notif_sentinel, timeout_ms)
    }

    fn wait_for_esc(&self, timeout_ms: u64) -> bool {
        Self::wait_for_file(&self.esc_sentinel, timeout_ms)
    }

    fn assert_notif_absent_after(&self, wait_ms: u64) {
        thread::sleep(Duration::from_millis(wait_ms));
        assert!(!self.notif_sentinel.exists(), "notification sentinel should not exist");
    }

    fn assert_esc_absent_after(&self, wait_ms: u64) {
        thread::sleep(Duration::from_millis(wait_ms));
        assert!(!self.esc_sentinel.exists(), "escalation sentinel should not exist");
    }
}

/// Notification fires immediately when cooldown is 0.
#[test]
fn notification_fires() {
    let env = TestEnv::new();
    env.run(&["stop", "--cooldown", "0", "--delay", "999"]);
    assert!(env.wait_for_notif(500), "notification sentinel should appear");
}

/// Notification is suppressed when a prompt was recorded within the cooldown window.
#[test]
fn notification_suppressed_by_cooldown() {
    let env = TestEnv::new();
    env.run(&["prompt"]);
    thread::sleep(Duration::from_millis(50));
    // cooldown=9999 seconds — far beyond any real elapsed time
    env.run(&["stop", "--cooldown", "9999", "--delay", "999"]);
    env.assert_notif_absent_after(400);
}

/// Escalation fires after the delay when not cancelled.
/// delay=2 ticks * TICK_MS=100ms = 200ms actual sleep.
#[test]
fn escalation_fires() {
    let env = TestEnv::new();
    env.run(&["stop", "--delay", "2", "--cooldown", "0"]);
    assert!(env.wait_for_esc(700), "escalation sentinel should appear after delay");
}

/// Escalation does not fire when cancelled before the delay expires.
#[test]
fn escalation_cancelled() {
    let env = TestEnv::new();
    env.run(&["stop", "--delay", "5", "--cooldown", "0"]);
    thread::sleep(Duration::from_millis(100));
    env.run(&["cancel"]);
    // delay=5 ticks = 500ms; wait 800ms to be sure it would have fired
    env.assert_esc_absent_after(800);
}

/// dismiss cancels the pending escalation AND records an interaction
/// (so a subsequent stop with a large cooldown is suppressed).
#[test]
fn dismiss_cancels_escalation() {
    let env = TestEnv::new();
    env.run(&["stop", "--delay", "5", "--cooldown", "0"]);
    thread::sleep(Duration::from_millis(50));
    env.run(&["dismiss"]);
    // delay=5 ticks = 500ms; wait 800ms to be sure it would have fired
    env.assert_esc_absent_after(800);
    // dismiss also records interaction; next stop should be suppressed
    fs::remove_file(&env.notif_sentinel).ok();
    env.run(&["stop", "--cooldown", "9999", "--delay", "999"]);
    thread::sleep(Duration::from_millis(300));
    assert!(!env.notif_sentinel.exists(), "notification should be suppressed after dismiss");
}

/// Permission escalation fires after its delay (same mechanism as stop escalation).
#[test]
fn permission_escalation_fires() {
    let env = TestEnv::new();
    env.run(&["permission", "--delay", "2", "--cooldown", "0"]);
    assert!(env.wait_for_esc(700), "permission escalation sentinel should appear");
}

/// stop with a non-question message does not start an escalation.
#[test]
fn stop_no_escalation_for_statement() {
    let env = TestEnv::new();
    let payload = r#"{"last_assistant_message": "Task complete."}"#;
    env.run_with_stdin(&["stop", "--delay", "2", "--cooldown", "0"], payload);
    // delay=2 ticks = 200ms; wait 600ms — if escalation were coming it would have arrived
    env.assert_esc_absent_after(600);
}

/// stop with a question-ending message starts an escalation.
#[test]
fn stop_escalation_for_question() {
    let env = TestEnv::new();
    let payload = r#"{"last_assistant_message": "Should I proceed?"}"#;
    env.run_with_stdin(&["stop", "--delay", "2", "--cooldown", "0"], payload);
    assert!(env.wait_for_esc(700), "escalation sentinel should appear for question message");
}
