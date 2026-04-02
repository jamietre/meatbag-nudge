# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **`install-hooks` / `remove-hooks` subcommands** — binary now manages `settings.json` directly using `serde_json`, eliminating the `jq` prerequisite for hook configuration

### Fixed
- **Focus no-op when app already has focus** — skip `SetForegroundWindow` entirely if the foreground window belongs to the same process as the target (prevents intercepting keystrokes in VSCode editor when Claude runs in an integrated terminal)
- **Focus causing window resize** — `SW_RESTORE` now only fires when the window is actually minimized (guarded by `IsIconic`); previously it would un-fullscreen maximised windows
- **Uninstall ordering** — hooks are now removed before the binary is deleted, so `remove-hooks` can run successfully

## [0.1.0] - 2026-03-24

### Added
- Initial release
- Desktop notification sound on Claude stop events, with escalation after configurable delay
- Sound + screen flash escalation for permission requests
- Human presence detection cancels pending escalations and suppresses sounds during cooldown period
- `done` prompt intercept to cancel notification timer without sending to Claude
- Cross-platform: Linux/WSL (PulseAudio via `paplay`) and Windows (Win32 `PlaySoundW`)
- Screen flash via Win32 layered window on Windows
- WSL auto-conversion of Windows-style sound paths
- tmux context detection — exposes session/pane/window info to custom commands
- Full CLI flag configuration (`--delay`, `--cooldown`, `--flash`, `--repeat`, `--sound`, `--escalation-sound`)
- Environment variable overrides for all settings
- Custom notification/escalation command support via `MEATBAG_NOTIFICATION_CMD` / `MEATBAG_ESCALATION_CMD`
- `install.sh` — builds, installs binary + sounds, and auto-configures Claude Code hooks

[Unreleased]: https://github.com/jamietre/meatbag-nudge/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/jamietre/meatbag-nudge/releases/tag/v0.1.0
