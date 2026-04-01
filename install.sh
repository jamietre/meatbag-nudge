#!/usr/bin/env bash
set -euo pipefail

INSTALL_DIR="${HOME}/.local/bin"
SETTINGS_FILE="${HOME}/.claude/settings.json"
BINARY_NAME="claude-notify"
REPO="jamietre/meatbag-nudge"

# Detect platform
case "$(uname -s)" in
    MSYS*|MINGW*|CYGWIN*) EXE_EXT=".exe"; PLATFORM="windows" ;;
    *)                     EXE_EXT="";     PLATFORM="linux"   ;;
esac

is_wsl() {
    # WSL_DISTRO_NAME is set by WSL itself and works even with custom kernels
    [ -n "${WSL_DISTRO_NAME:-}" ]
}

# Find the Windows claude-notify.exe binary from WSL.
# Returns "claude-notify.exe" if on PATH via interop, or the full WSL path if found
# in the standard Windows install location. Prints nothing if not found.
find_windows_binary() {
    if command -v claude-notify.exe &>/dev/null 2>&1; then
        echo "claude-notify.exe"
        return
    fi
    local win_home
    win_home=$(wslpath "$(cmd.exe /c 'echo %USERPROFILE%' 2>/dev/null | tr -d '\r\n')" 2>/dev/null) || true
    if [ -n "$win_home" ] && [ -f "$win_home/.local/bin/claude-notify.exe" ]; then
        echo "$win_home/.local/bin/claude-notify.exe"
        return
    fi
    echo ""
}

handle_wsl_install() {
    echo "  WSL detected — claude-notify uses the Windows binary for native audio and screen flash."
    echo ""

    local win_binary
    win_binary=$(find_windows_binary)

    if [ -z "$win_binary" ]; then
        cat <<'EOF'

  ERROR: Windows binary not found.

  In WSL, claude-notify delegates to the native Windows binary for audio
  and screen flash. Please install it on Windows first:

    1. Open a Windows terminal (PowerShell, cmd, or Git Bash — not WSL)
    2. Run: bash install.sh
    3. Re-run this installer in WSL

  Ensure %USERPROFILE%\.local\bin is on your Windows PATH so that
  claude-notify.exe is accessible from WSL.

EOF
        exit 1
    fi

    echo "  Using Windows binary: $win_binary"

    if $UNINSTALL; then
        echo "Removing hooks..."
        remove_hooks
        echo "Done."
        exit 0
    fi

    if ! $NO_HOOKS; then
        configure_hooks "$win_binary"
    fi

    echo ""
    echo "Done! Test with: claude-notify.exe stop --delay 3"
    exit 0
}

usage() {
    echo "Usage: bash install.sh [OPTIONS]"
    echo ""
    echo "Options:"
    echo "  --dir <path>    Install directory (default: ~/.local/bin)"
    echo "  --no-hooks      Skip adding hooks to settings.json"
    echo "  --defaults      Skip interactive config, use defaults"
    echo "  --build         Build from source instead of downloading a release"
    echo "  --uninstall     Remove binary and hooks"
    echo "  --help          Show this help"
}

# Prompt for a value with a default
prompt_value() {
    local label="$1" default="$2" varname="$3"
    local value
    read -rp "  $label [$default]: " value </dev/tty
    printf -v "$varname" '%s' "${value:-$default}"
}

configure_hooks() {
    local binary="${1:-$BINARY_NAME}"
    if ! command -v jq &>/dev/null; then
        echo "WARNING: jq not found — cannot auto-configure."
        echo "Add hooks manually to $SETTINGS_FILE (see README)."
        return
    fi

    if [ ! -f "$SETTINGS_FILE" ]; then
        mkdir -p "$(dirname "$SETTINGS_FILE")"
        echo '{}' > "$SETTINGS_FILE"
    fi

    # Prompt for settings
    local cooldown stop_delay perm_delay flash_count repeat_count focus_choice
    if $USE_DEFAULTS; then
        cooldown=30 stop_delay=300 perm_delay=30 flash_count=1 repeat_count=1
        focus_choice="notification"
    else
        echo ""
        echo "Configure notification settings (press Enter for defaults):"
        echo ""
        echo "  After you interact with Claude, sounds are suppressed for a cooldown"
        echo "  period so you aren't dinged while actively working."
        prompt_value "Cooldown (seconds)" "30" cooldown
        echo ""
        echo "  When Claude finishes and asks a question, an escalation (louder sound"
        echo "  + screen flash) fires after this delay if you haven't responded."
        prompt_value "Stop escalation delay (seconds)" "300" stop_delay
        echo ""
        echo "  When Claude requests tool permission, escalation fires after this delay."
        prompt_value "Permission escalation delay (seconds)" "30" perm_delay
        echo ""
        echo "  How many times the screen flashes and escalation sound plays."
        prompt_value "Screen flash count" "1" flash_count
        prompt_value "Escalation sound repeat" "1" repeat_count
        echo ""
        echo "  Focus the terminal window when a nudge fires?"
        echo "  Options: none, notification, escalation, both"
        prompt_value "Focus on" "notification" focus_choice
    fi

    # Normalise focus_choice -> --focus flag
    local focus_flag=""
    case "$focus_choice" in
        notification) focus_flag=" --focus notification" ;;
        escalation)   focus_flag=" --focus escalation" ;;
        both)         focus_flag=" --focus notification,escalation" ;;
        none|"")      focus_flag="" ;;
        *)            focus_flag=" --focus $focus_choice" ;;
    esac

    # Detect audio player (native Linux only — Windows binary handles audio internally)
    local player_flag=""
    local uname_s; uname_s="$(uname -s)"
    if [[ "$uname_s" != MSYS* && "$uname_s" != MINGW* && "$uname_s" != CYGWIN* ]] && \
       [[ "$binary" != *.exe ]]; then
        local detected_player
        detected_player=$(detect_audio_player)
        if [ -n "$detected_player" ]; then
            echo "  Audio player: $detected_player"
            player_flag=" --player $detected_player"
        else
            echo "  WARNING: No audio player found (paplay, aplay, pw-play). Sound will not play."
            echo "           Install one, then re-run the installer or add --player <cmd> to hooks manually."
        fi
    fi

    # Build hook commands with CLI flags
    local stop_cmd="\"$binary\" stop --delay $stop_delay --cooldown $cooldown --flash $flash_count --repeat $repeat_count${player_flag}${focus_flag}"
    local perm_cmd="\"$binary\" permission --delay $perm_delay --cooldown $cooldown --flash $flash_count --repeat $repeat_count${player_flag}${focus_flag}"
    local cancel_cmd="\"$binary\" cancel"
    local dismiss_cmd="\"$binary\" dismiss"
    local prompt_cmd="\"$binary\" prompt"

    # Check if hooks already reference claude-notify
    if grep -q "claude-notify" "$SETTINGS_FILE" 2>/dev/null; then
        echo ""
        read -rp "Hooks already exist. Overwrite? [y/N]: " overwrite </dev/tty
        if [[ ! "$overwrite" =~ ^[Yy] ]]; then
            echo "Keeping existing hooks."
            return
        fi
    fi

    local tmp
    tmp=$(mktemp)
    jq --arg stop "$stop_cmd" --arg perm "$perm_cmd" \
       --arg cancel "$cancel_cmd" --arg dismiss "$dismiss_cmd" \
       --arg prompt "$prompt_cmd" '
      .hooks.PostToolUse = [{"hooks": [{"type": "command", "command": $cancel}]}] |
      .hooks.PostToolUseFailure = [{"hooks": [{"type": "command", "command": $dismiss}]}] |
      .hooks.Stop = [{"hooks": [{"type": "command", "command": $stop}]}] |
      .hooks.PermissionRequest = [{"hooks": [{"type": "command", "command": $perm}]}] |
      .hooks.UserPromptSubmit = [{"hooks": [{"type": "command", "command": $prompt}]}]
    ' "$SETTINGS_FILE" > "$tmp" && mv "$tmp" "$SETTINGS_FILE"
    echo "Hooks configured in $SETTINGS_FILE"

    report_default_sounds "$INSTALL_DIR"
}

detect_default_sound() {
    local kind="$1"   # "notification" or "escalation"
    local install_dir="$2"
    local is_wsl=false
    grep -qi microsoft /proc/version 2>/dev/null && is_wsl=true

    # Custom sounds next to binary take priority
    for ext in wav oga ogg; do
        local f="$install_dir/sounds/${kind}.${ext}"
        [ -f "$f" ] && echo "$f" && return
    done

    # WSL: Windows Media sounds
    if $is_wsl; then
        local wsl_candidates=()
        if [ "$kind" = "escalation" ]; then
            wsl_candidates=(
                "/mnt/c/Windows/Media/Windows Message Nudge.wav"
                "/mnt/c/Windows/Media/Windows Exclamation.wav"
                "/mnt/c/Windows/Media/Windows Critical Stop.wav"
                "/mnt/c/Windows/Media/tada.wav"
            )
        else
            wsl_candidates=(
                "/mnt/c/Windows/Media/Speech Misrecognition.wav"
                "/mnt/c/Windows/Media/Windows Notify System Generic.wav"
                "/mnt/c/Windows/Media/chimes.wav"
                "/mnt/c/Windows/Media/Windows Notify.wav"
            )
        fi
        for f in "${wsl_candidates[@]}"; do
            [ -f "$f" ] && echo "$f" && return
        done
    fi

    # Linux system sounds
    local linux_candidates=()
    if [ "$kind" = "escalation" ]; then
        linux_candidates=(
            "/usr/share/sounds/alsa/Rear_Center.wav"
            "/usr/share/sounds/freedesktop/stereo/bell.oga"
            "/usr/share/sounds/freedesktop/stereo/complete.oga"
        )
    else
        linux_candidates=(
            "/usr/share/sounds/alsa/Front_Center.wav"
            "/usr/share/sounds/freedesktop/stereo/message.oga"
            "/usr/share/sounds/freedesktop/stereo/message-new-instant.oga"
        )
    fi
    for f in "${linux_candidates[@]}"; do
        [ -f "$f" ] && echo "$f" && return
    done

    echo ""
}

report_default_sounds() {
    local install_dir="$1"
    echo ""
    echo "Sound configuration:"

    local notif_sound esc_sound
    notif_sound=$(detect_default_sound "notification" "$install_dir")
    esc_sound=$(detect_default_sound "escalation" "$install_dir")

    if [ -n "$notif_sound" ]; then
        echo "  Notification sound : $notif_sound"
    else
        echo "  Notification sound : NOT FOUND"
    fi
    if [ -n "$esc_sound" ]; then
        echo "  Escalation sound   : $esc_sound"
    else
        echo "  Escalation sound   : NOT FOUND"
    fi

    if [ -z "$notif_sound" ] || [ -z "$esc_sound" ]; then
        echo ""
        echo "  To use custom sounds, either:"
        echo "    Place notification.wav / escalation.wav in $install_dir/sounds/"
        echo "    or add --sound / --escalation-sound flags to the hook commands in:"
        echo "    $SETTINGS_FILE"
    fi
}

remove_hooks() {
    if ! command -v jq &>/dev/null; then
        echo "WARNING: jq not found — remove claude-notify hooks manually from $SETTINGS_FILE"
        return
    fi
    if [ ! -f "$SETTINGS_FILE" ]; then
        return
    fi

    local tmp
    tmp=$(mktemp)
    jq '
      .hooks |= with_entries(
        select(.value | tostring | test("claude-notify") | not)
      )
    ' "$SETTINGS_FILE" > "$tmp" && mv "$tmp" "$SETTINGS_FILE"
    echo "Hooks removed from $SETTINGS_FILE"
}

build_from_source() {
    local script_dir
    script_dir="$(cd "$(dirname "$0")" && pwd)"

    if ! command -v cargo &>/dev/null; then
        echo "ERROR: cargo not found. Install the Rust toolchain:" >&2
        echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh" >&2
        echo "  or: mise use -g rust@latest" >&2
        exit 1
    fi

    echo "Building from source..." >&2
    cd "$script_dir"
    cargo build --release >&2
    local built="target/release/${BINARY_NAME}${EXE_EXT}"
    if [ ! -f "$built" ]; then
        echo "ERROR: Build succeeded but binary not found at $built" >&2
        exit 1
    fi
    echo "$script_dir/$built"
}

download_release() {
    local downloader=""
    if command -v curl &>/dev/null; then
        downloader="curl"
    elif command -v wget &>/dev/null; then
        downloader="wget"
    else
        echo "ERROR: curl or wget is required to download releases." >&2
        exit 1
    fi

    # Fetch the latest release tag
    local api_url="https://api.github.com/repos/${REPO}/releases/latest"
    local release_json
    if [ "$downloader" = "curl" ]; then
        release_json=$(curl -fsSL "$api_url")
    else
        release_json=$(wget -qO- "$api_url")
    fi

    local tag
    tag=$(echo "$release_json" | grep '"tag_name"' | head -1 | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')
    if [ -z "$tag" ]; then
        echo "ERROR: Could not determine latest release tag. Check your internet connection" >&2
        echo "or visit https://github.com/${REPO}/releases to download manually." >&2
        exit 1
    fi

    echo "Latest release: $tag" >&2

    # Pick the right asset
    local asset_name
    if [ "$PLATFORM" = "windows" ]; then
        asset_name="claude-notify-windows-x86_64.zip"
    else
        asset_name="claude-notify-linux-x86_64.tar.gz"
    fi

    local download_url="https://github.com/${REPO}/releases/download/${tag}/${asset_name}"
    local tmp_dir
    tmp_dir=$(mktemp -d)
    local archive="${tmp_dir}/${asset_name}"

    echo "Downloading $asset_name..." >&2
    if [ "$downloader" = "curl" ]; then
        curl -fsSL -o "$archive" "$download_url"
    else
        wget -qO "$archive" "$download_url"
    fi

    # Extract
    if [[ "$asset_name" == *.tar.gz ]]; then
        tar -xzf "$archive" -C "$tmp_dir" >&2
    else
        unzip -q "$archive" -d "$tmp_dir" >&2
    fi

    local binary="${tmp_dir}/${BINARY_NAME}${EXE_EXT}"
    if [ ! -f "$binary" ]; then
        echo "ERROR: Binary not found in archive. Contents:" >&2
        ls "$tmp_dir" >&2
        exit 1
    fi

    echo "$binary"
}

# Parse args
NO_HOOKS=false
USE_DEFAULTS=false
UNINSTALL=false
BUILD=false
while [[ $# -gt 0 ]]; do
    case "$1" in
        --dir)       INSTALL_DIR="$2"; shift 2 ;;
        --no-hooks)  NO_HOOKS=true; shift ;;
        --defaults)  USE_DEFAULTS=true; shift ;;
        --build)     BUILD=true; shift ;;
        --uninstall) UNINSTALL=true; shift ;;
        --help)      usage; exit 0 ;;
        *)           echo "Unknown option: $1"; usage; exit 1 ;;
    esac
done

BINARY="${INSTALL_DIR}/${BINARY_NAME}${EXE_EXT}"

cat <<'BANNER'

  claude-meatbag-nudge — Desktop notifications for Claude Code

  When Claude stops or asks a question, a notification sound plays.
  If you don't respond within a configurable delay, an escalation
  fires — a louder sound and screen flash to get your attention.
  When you interact (submit a prompt, approve a tool), the pending
  escalation is cancelled. A cooldown period suppresses repeated
  sounds while you're actively working; it won't make a sound if
  there was an interaction within the cooldown period.

  claude-meatbag-nudge will also detect when a process is complete,
  e.g. claude's response ends with a "." instead of a "?". In that case,
  you'll get the usual notification, but it won't escalate.

  By default, system sounds are used (Windows Media sounds on Windows,
  freedesktop/ALSA sounds on Linux). Place custom sounds in
  ~/.local/bin/sounds/ as notification.wav and escalation.wav to
  override them.

  This installer will:
    1. Download the latest release binary from GitHub (or build from source with --build)
    2. Copy the binary to ~/.local/bin/
    3. Add notification hooks to ~/.claude/settings.json (requires jq)

BANNER

if is_wsl; then
    handle_wsl_install
fi

if $UNINSTALL; then
    echo "Uninstalling claude-meatbag-nudge..."
    rm -f "$BINARY"
    remove_hooks
    echo "Done."
    exit 0
fi

# Get the binary
if $BUILD; then
    binary_path=$(build_from_source)
else
    binary_path=$(download_release)
fi

# Install binary
echo "Installing to $INSTALL_DIR..."
mkdir -p "$INSTALL_DIR"
if [ -f "$BINARY" ]; then
    mv "$BINARY" "${BINARY}.old" 2>/dev/null || true
fi
cp "$binary_path" "$BINARY"
chmod +x "$BINARY"
rm -f "${BINARY}.old"

echo "Installed: $BINARY"

# Verify on PATH
if ! command -v "$BINARY_NAME" &>/dev/null; then
    echo ""
    echo "WARNING: $BINARY_NAME is not on your PATH."
    echo "Add $INSTALL_DIR to your PATH, e.g.:"
    echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
fi

# Configure hooks
if ! $NO_HOOKS; then
    configure_hooks
fi

echo ""
echo "Done! Test with: claude-notify stop --delay 3"
