# Integration Tests Design

**Date:** 2026-04-01
**Status:** Approved

## Overview

Add integration tests to `claude-notify` that verify the behavioral logic of notification timing, cooldown suppression, escalation firing and cancellation. Tests invoke the real binary as a subprocess and use mock effect commands to observe outcomes without requiring audio hardware or Win32 display.

## Approach

Integration tests only (`tests/integration.rs`). Pure functions (`normalize_path`, `extract_json_string`, `message_is_question`, `focus_at`) are simple enough that their correctness is validated through the integration scenarios rather than needing separate unit tests.

## Time Scaling

The binary will respect a `MEATBAG_TICK_MS` environment variable (default: 1000, i.e. 1 tick = 1 second). All `thread::sleep` durations are multiplied by this value. Tests set `MEATBAG_TICK_MS=100`, making `--delay 2` = 200ms actual wall-clock time. This gives 10–30x headroom over OS scheduling jitter (~15ms on Windows, ~4ms on Linux) while keeping the full suite under ~5 seconds.

## Test Harness

### `TestEnv` struct

Each test creates an isolated `TestEnv`:

```rust
struct TestEnv {
    state_dir: TempDir,   // unique dir, passed as MEATBAG_STATE_DIR
    sentinel: PathBuf,    // unique file path; written by mock effect commands
}
```

`TempDir` comes from the `tempfile` crate (new dev-dependency).

### Binary location

```rust
const BIN: &str = env!("CARGO_BIN_EXE_claude-notify");
```

Cargo sets this env var automatically when running integration tests.

### Mock effects

Both notification and escalation effects are replaced with a `touch`-equivalent shell command that writes the sentinel file:

```
MEATBAG_NOTIFICATION_CMD = "touch /tmp/test-sentinel-<uuid>"
MEATBAG_ESCALATION_CMD   = "touch /tmp/test-sentinel-<uuid>"
```

This bypasses all audio and Win32 flash logic.

### Assertion helpers

- `wait_for_sentinel(timeout_ms) -> bool` — polls every 20ms until the sentinel file appears or timeout elapses
- `assert_absent_after(wait_ms)` — sleeps for `wait_ms`, then asserts sentinel does not exist

### Stdin for payload tests

`stop` reads JSON from stdin to determine `message_is_question`. Tests that exercise this path pipe JSON via `.stdin(Stdio::piped())`. Tests that don't care pass an empty string (the parser defaults to "assume question" on failure, which is acceptable for those scenarios).

## Test Scenarios

All tests use `MEATBAG_TICK_MS=100`. Delay values below are tick units.

| Test | Setup | Expected outcome |
|---|---|---|
| `notification_fires` | `stop`, no prior interaction | sentinel appears within 300ms |
| `notification_suppressed_by_cooldown` | `prompt`, then `stop --cooldown 5` | sentinel absent after 400ms |
| `escalation_fires` | `stop --delay 2 --cooldown 0` | sentinel appears within 500ms |
| `escalation_cancelled` | `stop --delay 3`, then `cancel` after 100ms | sentinel absent after 600ms |
| `prompt_resets_cooldown` | `prompt`, wait 300ms, `stop --cooldown 2` | sentinel appears (cooldown expired) |
| `permission_escalation_fires` | `permission --delay 2 --cooldown 0` | sentinel appears within 500ms |
| `stop_no_escalation_for_statement` | `stop --delay 2`, non-question JSON payload | sentinel absent after 500ms |
| `stop_escalation_for_question` | `stop --delay 2`, `?`-ending JSON payload | sentinel appears within 500ms |

## Files Changed

- `src/main.rs` — read `MEATBAG_TICK_MS` env var; use it to scale all `thread::sleep` calls
- `tests/integration.rs` — new file with all 8 tests
- `Cargo.toml` — add `tempfile` as a dev-dependency

## Out of Scope

- Tests for Win32 flash/focus behaviour (requires display)
- Tests for audio playback (requires audio hardware)
- Windows-native test execution (tests run on Linux/WSL; CI is Linux)
