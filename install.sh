#!/usr/bin/env bash
set -euo pipefail

INSTALL_DIR="${HOME}/.local/bin"
SETTINGS_FILE="${HOME}/.claude/settings.json"
BINARY_NAME="claude-notify"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# Detect platform
case "$(uname -s)" in
    MSYS*|MINGW*|CYGWIN*) EXE_EXT=".exe" ;;
    *)                     EXE_EXT="" ;;
esac

usage() {
    echo "Usage: bash install.sh [OPTIONS]"
    echo ""
    echo "Options:"
    echo "  --dir <path>    Install directory (default: ~/.local/bin)"
    echo "  --no-hooks      Skip adding hooks to settings.json"
    echo "  --defaults      Skip interactive config, use defaults"
    echo "  --uninstall     Remove binary, sounds, and hooks"
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
    local cooldown stop_delay perm_delay flash_count repeat_count
    if $USE_DEFAULTS; then
        cooldown=30 stop_delay=300 perm_delay=30 flash_count=1 repeat_count=1
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
    fi

    # Build hook commands with CLI flags
    local stop_cmd="claude-notify stop --delay $stop_delay --cooldown $cooldown --flash $flash_count --repeat $repeat_count"
    local perm_cmd="claude-notify permission --delay $perm_delay --cooldown $cooldown --flash $flash_count --repeat $repeat_count"
    local cancel_cmd="claude-notify cancel"
    local dismiss_cmd="claude-notify dismiss"
    local prompt_cmd="claude-notify prompt"

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

# Parse args
NO_HOOKS=false
USE_DEFAULTS=false
UNINSTALL=false
while [[ $# -gt 0 ]]; do
    case "$1" in
        --dir)       INSTALL_DIR="$2"; shift 2 ;;
        --no-hooks)  NO_HOOKS=true; shift ;;
        --defaults)  USE_DEFAULTS=true; shift ;;
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
  sounds while you're actively working; it won't make a sounds if
  there as an interaction within the cooldown period.

  claude-meatbag-nudge will also detect when a process is complete, 
  e.g. claude's response ends with a "." instead of a "?". In that case,
  you'll get the usual notification, but it won't escalate.

  By default, system sounds are used (Windows Media sounds on Windows,
  freedesktop/ALSA sounds on Linux). You can override sounds via
  --sound / --escalation-sound flags or environment variables.
  Place custom sounds in ~/.local/bin/sounds/ as notification.wav
  and escalation.wav to use them automatically.

  This installer will:
    1. Build the claude-notify binary from source (requires Rust)
    2. Copy the binary to ~/.local/bin/
    3. Add notification hooks to ~/.claude/settings.json (requires jq)

BANNER

if $UNINSTALL; then
    echo "Uninstalling claude-meatbag-nudge..."
    rm -f "$BINARY"
    remove_hooks
    echo "Done."
    exit 0
fi

# Check dependencies
missing=()
if ! command -v cargo &>/dev/null; then
    missing+=("cargo (Rust toolchain)")
fi
if ! command -v jq &>/dev/null; then
    missing+=("jq (JSON processor)")
fi
case "$(uname -s)" in
    MSYS*|MINGW*|CYGWIN*)
        if ! find "/c/Program Files/Microsoft Visual Studio" -name "link.exe" -path "*/Hostx64/x64/*" 2>/dev/null | grep -q .; then
            missing+=("MSVC C++ build tools (link.exe)")
        fi
        ;;
esac
if [ ${#missing[@]} -gt 0 ]; then
    echo "Missing required tools:"
    for tool in "${missing[@]}"; do
        echo "  - $tool"
    done
    echo ""
    echo "Install them with:"
    echo "  Rust:  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    echo "         or: mise use -g rust@latest"
    case "$(uname -s)" in
        Linux*)
            echo "  jq:    sudo apt install jq"
            ;;
        Darwin*)
            echo "  jq:    brew install jq"
            ;;
        MSYS*|MINGW*|CYGWIN*)
            echo "  jq:    pacman -S jq"
            echo "  MSVC:  Open Visual Studio Installer → Modify →"
            echo "         check 'Desktop development with C++' → Install"
            ;;
    esac
    exit 1
fi

# Build
echo "Building..."
cd "$SCRIPT_DIR"
cargo build --release
BUILT="target/release/${BINARY_NAME}${EXE_EXT}"
if [ ! -f "$BUILT" ]; then
    echo "ERROR: Build succeeded but binary not found at $BUILT"
    exit 1
fi

# Install binary
echo "Installing to $INSTALL_DIR..."
mkdir -p "$INSTALL_DIR"
if [ -f "$BINARY" ]; then
    mv "$BINARY" "${BINARY}.old" 2>/dev/null || true
fi
cp "$BUILT" "$BINARY"
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
