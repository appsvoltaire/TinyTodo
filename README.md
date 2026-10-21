# TinyTodo

A tiny desktop front-end for a personal assistant that lives in a private Slack
channel. When the assistant posts, a small glass popup appears in the top-right
corner; type a reply and press Enter, and it goes back to the channel.

TinyTodo keeps no tasks, history or memory of its own. The assistant owns the
work and Slack owns the conversation. Locally there is only a config file, the
Slack tokens (in the OS keyring), and one timestamp so a message never pops up
twice.

## Using it

- **Popup**: appears without stealing focus and is sized to the newest message.
  Older unread messages are above it; scroll up to see them, or drag the grip
  at the bottom edge to make the popup taller. Click the reply field to type.
  **Enter** sends, **Esc** dismisses. **History** opens the conversation.
- **Fading**: an untouched popup fades out after 15 seconds (configurable).
  It never fades while the pointer is over it. Any click, scroll, key press or
  hover starts the countdown over. A half-typed reply is kept for next time.
- **Conversation**: the 10 most recent channel messages, fetched from Slack
  each time it opens. Open it from the tray icon (left-click on Windows, or
  *Open conversation* in the menu), or by launching TinyTodo again.
- **Tray menu**: *Open conversation*, *Settings…*, *Quit TinyTodo*. The tooltip
  shows when the app is offline.
- **Settings**: popups on/off, fade delay, notification sound, glass
  transparency, launch at sign-in, and a test notification to preview changes.
- Messages that arrive while TinyTodo is closed or offline pop up once it
  reconnects. On the very first run it starts from the newest message instead
  of replaying the channel.

## Install on Windows

Run `TinyTodo-Setup-<version>.exe`. It installs for the current user (no admin
rights needed) and asks for the two Slack tokens. It checks them with Slack and
stores them in Windows Credential Manager. Leave both fields empty to keep
tokens that are already stored. Setup can also start TinyTodo at sign-in. The
uninstaller (Settings → Apps) asks whether to remove the stored tokens too.

To build the installer yourself, you need
[Inno Setup 6](https://jrsoftware.org/isdl.php). Then run:

```bash
powershell -ExecutionPolicy Bypass -File installer/build.ps1
```

The installer is written to `target\installer\`.

## Setup from source

### 1. Slack app

The app needs a bot token (`xoxb-…`) and an app-level token (`xapp-…`) with
Socket Mode enabled:

- Bot scopes: `chat:write`, `groups:history`, `groups:read`
- Event subscription (bot events): `message.groups`
- Socket Mode on; app-level token with `connections:write`
- Invite the app to the channel (`/invite @TinyTodo` in `#todos-private`)

Replies are posted as the TinyTodo bot. TinyTodo never pops up its own posts.
To silence messages you type into Slack yourself, add your user ID to
`ignore_user_ids`.

### 2. Build

```bash
cargo build --release
```

Fedora (KDE) needs a C compiler for the TLS library: `sudo dnf install gcc`.
Nothing else is needed at build time. The tray uses the StatusNotifierItem
D-Bus protocol that KDE implements natively.

### 3. Store the tokens

```bash
SLACK_BOT_TOKEN=xoxb-… SLACK_APP_TOKEN=xapp-… tinytodo --save-tokens
```

This stores them in Windows Credential Manager or the Secret Service (KWallet
on KDE). If `SLACK_BOT_TOKEN` and `SLACK_APP_TOKEN` are set when the app
starts, those are used instead. `tinytodo --check-tokens` verifies them with
Slack, and `tinytodo --forget-tokens` removes the stored ones.

### 4. Run at login

Turn on *Launch TinyTodo when I sign in* in Settings. On Windows this writes a
`HKCU\…\Run` entry; on Linux it writes `~/.config/autostart/tinytodo.desktop`.

## Configuration

Settings edits `config.toml`, which you can also edit by hand; comments are
kept. It lives in `%APPDATA%\tinytodo\` (Windows) or `~/.config/tinytodo/`
(Linux):

```toml
channel = "todos-private"   # private channel name, or any channel ID
ignore_user_ids = []        # e.g. your own Slack user ID

[notifications]
enabled = true              # false: no popups; read via the tray instead
fade_after_seconds = 15     # fade an untouched popup after N seconds (0 = never)
sound = false               # play the system notification sound

[appearance]
transparency = 0.40         # 0.0 (solid) to 0.80
```

Only one TinyTodo runs at a time. Launching it again opens the conversation in
the running copy.

The delivery cursor lives in `%LOCALAPPDATA%\tinytodo\state.json` or
`~/.local/state/tinytodo/state.json`. Delete it to start fresh.

## Platform notes

- **Windows**: the popup renders with OpenGL, which is the wgpu backend that
  supports per-pixel transparency here. The desktop behind it is blurred by
  DWM. Set `WGPU_BACKEND` to override the backend.
- **Linux**: Wayland doesn't let apps choose where their windows go, so
  TinyTodo runs through XWayland to sit in the top-right corner. Set
  `TINYTODO_WAYLAND=1` to use native Wayland instead (KWin decides placement).
  There is no background blur on Linux, so the glass is drawn more opaque.

## Development

```bash
cargo test
```

Slack, notification policy and UI are separate layers, each testable on its own:

| File | Role |
|---|---|
| `src/slack.rs` | Web API client, Socket Mode framing, reconnecting feed |
| `src/policy.rs` | Which messages notify; the dedup cursor (pure logic) |
| `src/ui.rs` | iced popup / conversation and settings windows |
| `src/style.rs` | Glass and settings styling (light/dark) |
| `src/config.rs` | Config file, keyring tokens, cursor file |
| `src/tray.rs` | Tray icon and menu; the app icon artwork |
| `src/platform.rs` | Per-OS window tweaks, sound, sign-in launch, single instance |
| `installer/` | Inno Setup script, build script, icon |

The installer icon is generated from the tray artwork:
`cargo test -- --ignored write_installer_icon`.

Debug builds have two helpers:

- `TINYTODO_DEBUG_NOTIFY_OWN=1` treats TinyTodo's own posts as the
  assistant's, so the whole loop can be tested with just the app's tokens.
- `TINYTODO_DEBUG_RECONNECTS=1` asks Slack to rotate the socket every few
  minutes, which exercises reconnection.
