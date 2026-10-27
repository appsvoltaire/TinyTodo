#!/usr/bin/env bash
# Builds TinyTodo and installs it for the current user (no root):
#   ~/.local/bin/tinytodo, a menu entry, and optionally an autostart entry.
# The Slack tokens entered here are checked with Slack and stored in the
# Secret Service (KWallet on KDE, GNOME Keyring on GNOME) by
# `tinytodo --save-tokens`; they are passed through the environment, never
# on a command line or in a file.
#
#   installer/install-linux.sh              install or upgrade
#   installer/install-linux.sh --uninstall  remove (config.toml is kept)
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
bin="$HOME/.local/bin/tinytodo"
share="${XDG_DATA_HOME:-$HOME/.local/share}"
data="$share/tinytodo"
menu="$share/applications/tinytodo.desktop"
icon="$share/icons/hicolor/256x256/apps/tinytodo.png"
autostart="${XDG_CONFIG_HOME:-$HOME/.config}/autostart/tinytodo.desktop"

ask() { # ask "question" default(y/n)
    local reply hint=$([[ $2 == y ]] && echo "Y/n" || echo "y/N")
    read -rp "$1 [$hint] " reply
    [[ ${reply:-$2} =~ ^[Yy] ]]
}

stop_running_app() {
    pkill -x tinytodo 2>/dev/null && sleep 1 || true
}

desktop_entry() {
    printf '[Desktop Entry]\nType=Application\nName=TinyTodo\nComment=%s\nExec="%s"\nIcon=tinytodo\nCategories=Utility;\n' \
        "Popups from your assistant's Slack channel" "$bin"
}

if [[ ${1:-} == --uninstall ]]; then
    stop_running_app
    if [[ -x $bin ]] && ask "Also remove your Slack tokens from the keyring?" y; then
        "$bin" --forget-tokens
    fi
    rm -f "$bin" "$menu" "$icon" "$autostart"
    rm -rf "$data"
    echo "TinyTodo removed. Settings are kept in ~/.config/tinytodo/."
    exit 0
fi

if ! busctl --user status org.freedesktop.secrets >/dev/null 2>&1; then
    echo "No Secret Service is running (org.freedesktop.secrets). Start KWallet or" >&2
    echo "GNOME Keyring first, or set SLACK_BOT_TOKEN and SLACK_APP_TOKEN instead." >&2
    exit 1
fi

echo "Building TinyTodo…"
cargo build --release --manifest-path "$root/Cargo.toml"
built="$root/target/release/tinytodo"

cat <<'EOF'

Connect to Slack. Find the tokens at api.slack.com/apps > TinyTodo: the Bot
User OAuth Token (OAuth & Permissions) and an App-Level Token with
connections:write (Basic Information). Leave both empty to keep the tokens
already saved on this machine.
EOF
while true; do
    read -rsp "Bot token (xoxb-…): " bot; echo
    read -rsp "App-level token (xapp-…): " app; echo
    bot=${bot//[[:space:]]/} app=${app//[[:space:]]/}
    [[ -z $bot && -z $app ]] && break
    if [[ $bot != xoxb-* || $app != xapp-* ]]; then
        echo 'The bot token starts with "xoxb-" and the app-level token with "xapp-". Enter both, or leave both empty.'
        continue
    fi
    # Ask Slack whether the tokens work before installing anything.
    if SLACK_BOT_TOKEN=$bot SLACK_APP_TOKEN=$app "$built" --check-tokens; then
        break
    fi
    ask "Slack did not accept these tokens. Use them anyway?" n && break
done

stop_running_app
install -Dm755 "$built" "$bin"
install -Dm644 "$root/installer/tinytodo.png" "$icon"
install -Dm644 "$root/LICENSE" "$data/LICENSE"
install -Dm644 "$root/NOTICE" "$data/NOTICE"
mkdir -p "$(dirname "$menu")"
desktop_entry >"$menu"

if [[ -n $bot ]]; then
    SLACK_BOT_TOKEN=$bot SLACK_APP_TOKEN=$app "$bin" --save-tokens
fi
unset bot app

# The same file the app's Settings > "Launch TinyTodo when I sign in" toggles.
if ask "Start TinyTodo when you sign in?" y; then
    mkdir -p "$(dirname "$autostart")"
    printf '[Desktop Entry]\nType=Application\nName=TinyTodo\nExec="%s"\nX-GNOME-Autostart-enabled=true\n' "$bin" >"$autostart"
else
    rm -f "$autostart"
fi

echo "Installed $bin"
if ask "Start TinyTodo now?" y; then
    setsid -f "$bin" >/dev/null 2>&1 </dev/null
fi
