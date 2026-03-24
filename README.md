# claude-meatbag-nudge

Desktop notifications for Claude Code. Plays a sound when Claude needs attention and escalates if you don't respond.

## Quick install

One-liner — downloads the latest release binary from GitHub (no Rust required):

```bash
curl -fsSL https://raw.githubusercontent.com/jamietre/meatbag-nudge/master/install.sh | bash
```

Or, if you have the repo checked out:

```bash
bash install.sh
```

The script will:
1. Download the latest release binary for your platform from GitHub
2. Copy the binary to `~/.local/bin/`
3. Add hooks to `~/.claude/settings.json` (requires `jq`)

Options:
- `--dir <path>` — custom install directory (default: `~/.local/bin`)
- `--no-hooks` — skip adding hooks to settings.json
- `--uninstall` — remove binary, sounds, and hooks

The installer uses bare command names in hooks (`claude-notify stop` etc.) so a shared `settings.json` works across platforms as long as the binary is on PATH.

### Prerequisites

- **curl** or **wget** — for downloading the release binary (pre-installed on most systems)
- **jq** — for auto-configuring hooks in settings.json
- **Linux/WSL**: `paplay` (PulseAudio) for sound — typically pre-installed with WSLg

To build from source instead of downloading, pass `--build` (requires the **Rust** toolchain via [mise](https://mise.jdx.dev/) or [rustup](https://rustup.rs/)).

## How it works

Hooks into Claude Code's lifecycle events:

- **Stop** — plays a notification sound. If Claude's message ends with a question, schedules an escalation (sound + screen flash) after a delay (default 5 min).
- **Permission Request** — plays a notification sound and always schedules an escalation (default 30s).
- **User Prompt / Tool Use** — records that a human is present and cancels any pending escalation. Sounds are suppressed if you've interacted within the cooldown period.
- **"done" prompt** — typing `done` as a prompt cancels the notification timer and blocks the message from reaching Claude.

## Configuration

Settings are configured via CLI flags on the hook commands in `~/.claude/settings.json`. The install script prompts for these values and writes them into the hooks automatically. To change settings later, edit the hook commands in `settings.json` directly or re-run the installer.

| Flag | Default | Description |
|---|---|---|
| `--delay N` | `300` (stop) / `30` (permission) | Seconds before escalation fires |
| `--cooldown N` | `30` | Suppress sounds for N seconds after human interaction |
| `--flash N` | `1` | Number of screen flashes on escalation |
| `--repeat N` | `1` | Number of times to play escalation sound |
| `--sound PATH` | bundled `notification.wav` | Path to WAV for notification sound |
| `--escalation-sound PATH` | bundled `escalation.wav` | Path to WAV for escalation sound |

Example hook command in `settings.json`:

```json
"command": "claude-notify stop --delay 300 --cooldown 5 --flash 1 --repeat 1"
```

Flags are also useful for testing from the command line:

```bash
claude-notify stop --delay 3 --flash 2 --repeat 2
```

### Environment variables

All settings can also be set via environment variables (e.g. in your shell profile). CLI flags take precedence over env vars. These are mainly useful for advanced configuration or custom notification commands.

| Variable | Equivalent flag |
|---|---|
| `MEATBAG_STOP_DELAY` | `--delay` (on stop) |
| `MEATBAG_PERMISSION_DELAY` | `--delay` (on permission) |
| `MEATBAG_COOLDOWN` | `--cooldown` |
| `MEATBAG_FLASH_COUNT` | `--flash` |
| `MEATBAG_ESCALATION_REPEAT` | `--repeat` |
| `MEATBAG_NOTIFICATION_SOUND` | `--sound` |
| `MEATBAG_ESCALATION_SOUND` | `--escalation-sound` |
| `MEATBAG_NOTIFICATION_CMD` | Custom shell command for notification (overrides sound) |
| `MEATBAG_ESCALATION_CMD` | Custom shell command for escalation (overrides default) |
| `MEATBAG_STATE_DIR` | Directory for state files (default: `/tmp/claude-notify`) |

### Sound paths

By default, system sounds are used:

| Platform | Notification | Escalation |
|---|---|---|
| Windows | `C:\Windows\Media\Speech Misrecognition.wav` (fallback: `Windows Notify System Generic.wav`) | `C:\Windows\Media\Windows Message Nudge.wav` (fallback: `Windows Exclamation.wav`) |
| Linux/WSL | `/usr/share/sounds/freedesktop/stereo/message.oga` | `/usr/share/sounds/freedesktop/stereo/bell.oga` |

To use custom sounds, either:
- Place `notification.wav` / `escalation.wav` in a `sounds/` subdirectory next to the binary (e.g. `~/.local/bin/sounds/`), or
- Set `--sound` / `--escalation-sound` flags, or `MEATBAG_NOTIFICATION_SOUND` / `MEATBAG_ESCALATION_SOUND` env vars.

Custom sound files in `sounds/` are checked before system sounds and may be `.wav`, `.oga`, or `.ogg`.

Sound paths accept both POSIX and Windows formats. When running in WSL, Windows paths (e.g. `C:\Windows\Media\sound.wav`) are automatically converted to WSL paths (`/mnt/c/Windows/Media/sound.wav`).

### Terminal context variables

When running inside tmux, the handler auto-detects the pane context and passes it to custom commands as environment variables.

| Variable | Description |
|---|---|
| `MEATBAG_TMUX_PANE` | Pane ID (e.g. `%0`) |
| `MEATBAG_TMUX_SESSION` | Session name |
| `MEATBAG_TMUX_WINDOW` | Window index |
| `MEATBAG_TMUX_WINDOW_NAME` | Window name |
| `MEATBAG_TMUX_CLIENT` | Client TTY |
| `MEATBAG_TMUX_SOCKET` | Socket path |

## Building from source

```bash
mise install       # install Rust toolchain
mise run build     # build for current platform → target/release/claude-notify[.exe]
```

The native build uses your platform's default toolchain (MSVC on Windows, GNU on Linux).

### Cross-compilation

To build for the other platform from Linux:

```bash
mise run setup-cross     # install cross-compilation targets (one-time)
mise run cross-windows   # → target/x86_64-pc-windows-gnu/release/claude-notify.exe
mise run cross-linux     # → target/x86_64-unknown-linux-gnu/release/claude-notify
```

Cross-compiling to Windows from Linux requires `mingw-w64` (`sudo apt install mingw-w64`). Cross-compiling to Linux from Windows requires WSL or [cross](https://github.com/cross-rs/cross).

## Platform notes

### Linux / WSL

- Sound playback via `paplay` (PulseAudio / WSLg)
- Detached processes via `setsid`
- WSL auto-converts Windows-style paths in env vars

### Windows

- Sound playback via Win32 `PlaySoundW` (winmm.dll)
- Screen flash via temporary layered window (user32.dll)
- Detached processes via `CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS`
- State directory defaults to `%TEMP%\claude-notify`

## Uninstall

```bash
bash install.sh --uninstall
```

## Files

| File | Purpose |
|---|---|
| `src/main.rs` | Rust source — cross-platform notification handler |
| `Cargo.toml` | Rust project config |
| `.mise.toml` | Mise tooling config with build tasks |
| `.cargo/config.toml` | Cross-compilation linker settings |
| `install.sh` | Build + install script |
