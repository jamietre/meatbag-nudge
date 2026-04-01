# Window Focus on Nudge — Design Spec

**Date:** 2026-04-01  
**Status:** Approved

## Overview

Optionally focus the terminal window running Claude Code when a nudge fires. Useful when you've switched away and want the escalation to pull you back automatically.

## Configuration

### CLI flags

| Flag | Description |
|---|---|
| `--focus <list>` | Comma-separated list of events that trigger focus. Values: `notification`, `escalation`. Bare `--focus` (no value) defaults to `escalation`. |
| `--focus-cmd "..."` | Custom shell command to run for focusing. Overrides built-in behaviour. Timing is still controlled by `--focus`. |

### Environment variables

| Variable | Description |
|---|---|
| `MEATBAG_FOCUS` | Comma-separated list: `notification`, `escalation`, or `notification,escalation`. Empty/unset = disabled. |
| `MEATBAG_FOCUS_CMD` | Custom shell command override. |
| `MEATBAG_FOCUS_HWND` | Internal. Set automatically at hook time; inherited by the detached escalation child. On Windows: Win32 HWND integer. On WSL: Windows PID of the terminal host process. |

### Defaults

Focusing is **off by default**. When `--focus` is passed without a value, it enables escalation-only focusing (equivalent to `--focus escalation`).

## Architecture

### New function: `focus_window()`

A single function handles both platforms and the custom-command override:

1. If `MEATBAG_FOCUS_CMD` is set: run it as a shell command (detached, fire-and-forget).
2. Otherwise, dispatch to the platform implementation using `MEATBAG_FOCUS_HWND`.

Call sites pass no arguments — all context comes from the environment.

### Flag parsing

Extends the existing `parse_flag` helper. `--focus` is parsed as an optional-value flag:
- If present with a value: use that value (`notification`, `escalation`, `notification,escalation`)
- If present without a value (next token starts with `--` or is absent): default to `escalation`
- Sets `MEATBAG_FOCUS` env var for child process inheritance

### Hook flow

**`stop` / `permission` handlers:**

1. Capture focus target (see platform sections below) → set `MEATBAG_FOCUS_HWND`
2. If `MEATBAG_FOCUS` contains `notification`: call `focus_window()` immediately
3. Spawn escalation child (inherits `MEATBAG_FOCUS`, `MEATBAG_FOCUS_HWND`, `MEATBAG_FOCUS_CMD`)

**`_escalate` subcommand:**

After flash/sound actions, if `MEATBAG_FOCUS` contains `escalation`: call `focus_window()`.

## Platform implementations

### Windows (native binary)

**HWND capture (at hook time):**

1. Call `GetConsoleWindow()` — works for cmd, Git Bash, older Windows Terminal
2. If NULL (VS Code, newer Windows Terminal with ConPTY): walk the process tree via `CreateToolhelp32Snapshot`, iterating parent PIDs until finding one with a non-zero, visible `MainWindowHandle`
3. Store the HWND as `MEATBAG_FOCUS_HWND=<integer>`

**Focus:**

Call `SetForegroundWindow(hwnd)` using the stored HWND. If the HWND is 0 or missing, silently no-op.

### WSL (Linux binary)

**Windows PID capture (at hook time):**

1. Read `PPID` from `/proc/self/status`
2. Read `/proc/<ppid>/status`, extract the `NtTgid:` field — the Windows PID of the terminal host process
3. Store as `MEATBAG_FOCUS_HWND=<windows_pid>`

**Focus:**

If `MEATBAG_FOCUS_HWND` is missing or empty, silently no-op. Otherwise, spawn `powershell.exe -NoProfile -NonInteractive -Command "..."` with an inline script that:
- Walks up the Windows process tree from the stored PID to find an ancestor with a visible `MainWindowHandle`
- Calls `SetForegroundWindow` on it

**Latency note:** PowerShell startup adds ~1–2s. Acceptable for escalation; noticeable for notification timing. Users with latency requirements should use `--focus-cmd` with a faster alternative (e.g. a pre-compiled helper or `wscript`).

## Env var inheritance

`MEATBAG_FOCUS_HWND` is set in the hook process's environment before spawning the detached escalation child. Both Windows (`CreateProcessW` with `env=NULL`) and Linux (`Command::new()`) inherit the parent environment, so no changes to the escalation spawn call are needed.

## Out of scope

- macOS support (not a target platform for this feature)
- Focusing a specific tmux pane/window (users can achieve this via `--focus-cmd`)
- VS Code panel focus (VS Code's window can be raised but the specific terminal panel cannot be focused via Win32)
