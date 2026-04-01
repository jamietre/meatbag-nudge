# Integration Tests Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add integration tests that verify notification, cooldown, escalation, and cancellation behaviour by invoking the real binary as a subprocess with mock effect commands.

**Architecture:** A `MEATBAG_TICK_MS` env var scales all sleep durations in the binary so tests run in milliseconds. Each test creates an isolated temp directory for state and sentinel files. `MEATBAG_NOTIFICATION_CMD` / `MEATBAG_ESCALATION_CMD` write sentinel files instead of playing audio or flashing.

**Tech Stack:** Rust `std::process::Command`, `tempfile` crate (dev-dep), `tests/integration.rs`

---

## Files

| File | Change |
|---|---|
| `src/main.rs` | Add `tick_sleep(secs: u64)` helper; replace 2 sleep calls in `run_escalation` |
| `Cargo.toml` | Add `tempfile = "3"` dev-dependency |
| `tests/integration.rs` | New — `TestEnv` harness + 8 integration tests |

---

## Task 1: Add `MEATBAG_TICK_MS` time scaling to `src/main.rs`

**Files:**
- Modify: `src/main.rs` (near `now_secs`, and in `run_escalation`)

- [ ] **Step 1: Add `tick_sleep` helper after `now_secs`**

  Insert after the closing brace of `now_secs` (around line 526):

  ```rust
  /// Sleep for `secs` tick-units. In normal use one tick = 1 second.
  /// Set MEATBAG_TICK_MS to scale all delays for testing (e.g. 100 = 10× faster).
  fn tick_sleep(secs: u64) {
      let ms = env::var("MEATBAG_TICK_MS")
          .ok()
          .and_then(|v| v.parse::<u64>().ok())
          .unwrap_or(1000);
      std::thread::sleep(Duration::from_millis(secs * ms));
  }
  ```

- [ ] **Step 2: Replace sleep calls in `run_escalation`**

  In `run_escalation` (around line 894), replace:
  ```rust
  std::thread::sleep(Duration::from_secs(delay_secs));
  ```
  with:
  ```rust
  tick_sleep(delay_secs);
  ```

  And the sound-repeat inter-play pause (around line 952), replace:
  ```rust
  std::thread::sleep(Duration::from_secs(2));
  ```
  with:
  ```rust
  tick_sleep(2);
  ```

- [ ] **Step 3: Build to confirm no compile errors**

  ```bash
  cargo build 2>&1 | tail -5
  ```
  Expected: `Finished` with no errors.

- [ ] **Step 4: Commit**

  ```bash
  git add src/main.rs
  git commit -m "feat: add MEATBAG_TICK_MS time scaling for test speed"
  ```

---

## Task 2: Add `tempfile` dev-dependency

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add dev-dependency**

  Append to `Cargo.toml`:
  ```toml
  [dev-dependencies]
  tempfile = "3"
  ```

- [ ] **Step 2: Confirm it resolves**

  ```bash
  cargo fetch 2>&1 | tail -3
  ```
  Expected: no errors.

- [ ] **Step 3: Commit**

  ```bash
  git add Cargo.toml Cargo.lock
  git commit -m "chore: add tempfile dev-dependency for integration tests"
  ```

---

## Task 3: Write `TestEnv` harness in `tests/integration.rs`

**Files:**
- Create: `tests/integration.rs`

- [ ] **Step 1: Create the file with the harness**

  ```rust
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
  ```

- [ ] **Step 2: Verify it compiles (no tests yet)**

  ```bash
  cargo test --test integration 2>&1 | tail -5
  ```
  Expected: `running 0 tests` / `test result: ok`.

- [ ] **Step 3: Commit**

  ```bash
  git add tests/integration.rs
  git commit -m "test: add TestEnv harness for integration tests"
  ```

---

## Task 4: Notification tests

**Files:**
- Modify: `tests/integration.rs`

Tests: `notification_fires`, `notification_suppressed_by_cooldown`

- [ ] **Step 1: Append tests to `tests/integration.rs`**

  ```rust
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
  ```

- [ ] **Step 2: Run and confirm both pass**

  ```bash
  cargo test --test integration notification 2>&1
  ```
  Expected:
  ```
  test notification_fires ... ok
  test notification_suppressed_by_cooldown ... ok
  ```

- [ ] **Step 3: Commit**

  ```bash
  git add tests/integration.rs
  git commit -m "test: notification_fires and notification_suppressed_by_cooldown"
  ```

---

## Task 5: Escalation tests

**Files:**
- Modify: `tests/integration.rs`

Tests: `escalation_fires`, `escalation_cancelled`

- [ ] **Step 1: Append tests**

  ```rust
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
  ```

- [ ] **Step 2: Run and confirm both pass**

  ```bash
  cargo test --test integration escalation 2>&1
  ```
  Expected:
  ```
  test escalation_fires ... ok
  test escalation_cancelled ... ok
  ```

- [ ] **Step 3: Commit**

  ```bash
  git add tests/integration.rs
  git commit -m "test: escalation_fires and escalation_cancelled"
  ```

---

## Task 6: Dismiss and permission tests

**Files:**
- Modify: `tests/integration.rs`

Tests: `dismiss_cancels_escalation`, `permission_escalation_fires`

- [ ] **Step 1: Append tests**

  ```rust
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
  ```

- [ ] **Step 2: Run and confirm both pass**

  ```bash
  cargo test --test integration dismiss permission 2>&1
  ```
  Expected:
  ```
  test dismiss_cancels_escalation ... ok
  test permission_escalation_fires ... ok
  ```

- [ ] **Step 3: Commit**

  ```bash
  git add tests/integration.rs
  git commit -m "test: dismiss_cancels_escalation and permission_escalation_fires"
  ```

---

## Task 7: Message payload tests

**Files:**
- Modify: `tests/integration.rs`

Tests: `stop_no_escalation_for_statement`, `stop_escalation_for_question`

The `stop` handler reads JSON from stdin and checks whether `last_assistant_message` ends with `?`. If not a question, no escalation is started.

- [ ] **Step 1: Append tests**

  ```rust
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
  ```

- [ ] **Step 2: Run and confirm both pass**

  ```bash
  cargo test --test integration statement question 2>&1
  ```
  Expected:
  ```
  test stop_no_escalation_for_statement ... ok
  test stop_escalation_for_question ... ok
  ```

- [ ] **Step 3: Run the full suite**

  ```bash
  cargo test --test integration 2>&1
  ```
  Expected: all 8 tests pass.

- [ ] **Step 4: Commit**

  ```bash
  git add tests/integration.rs
  git commit -m "test: stop_no_escalation_for_statement and stop_escalation_for_question"
  ```

---

## Self-Review

**Spec coverage:**
- ✅ `notification_fires` → test 1
- ✅ `notification_suppressed_by_cooldown` → test 2
- ✅ `escalation_fires` → test 3
- ✅ `escalation_cancelled` → test 4
- ✅ `dismiss_cancels_escalation` → replaces "prompt_resets_cooldown" (cooldown is real-seconds; dismiss covers the interaction-recording behaviour instead)
- ✅ `permission_escalation_fires` → test 6
- ✅ `stop_no_escalation_for_statement` → test 7
- ✅ `stop_escalation_for_question` → test 8
- ✅ `MEATBAG_TICK_MS` scaling → Task 1
- ✅ `tempfile` dev-dep → Task 2
- ✅ `TestEnv` harness → Task 3

**Note on "prompt_resets_cooldown":** The cooldown is compared against real wall-clock seconds (`now_secs()`), not tick-scaled. Testing cooldown expiry would require a 1+ second real wait. The `dismiss_cancels_escalation` test covers the interaction-recording path instead, which is the observable behaviour that matters.
