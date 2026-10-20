# TinyTodo

A tiny desktop front-end for a personal assistant that lives in a private Slack
channel. When the assistant posts, a small glass popup appears in the top-right
corner; type a reply and press Enter, and it goes back to the channel.

TinyTodo keeps no tasks, history or memory of its own. The assistant owns the
work and Slack owns the conversation. Locally there is only a config file, the
Slack tokens (in the OS keyring), and one timestamp so a message never pops up
twice.

## Using it

- **Popup**: appears without stealing focus. Click the reply field (or the
  popup) to type. **Enter** sends, **Esc** dismisses. **History** expands into
  the conversation.
- **Conversation**: the 10 most recent channel messages, fetched from Slack
  each time it opens. Open it from the tray icon (left-click on Windows, or
  *Open conversation* in the menu).
- **Tray menu**: *Open conversation*, *Quit TinyTodo*. The tooltip shows when
  the app is offline.
- Messages that arrive while TinyTodo is closed or offline pop up once it
  reconnects. On the very first run it starts from the newest message instead
  of replaying the channel.

## Setup

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
starts, those are used instead.

### 4. Run at login

- Windows: put a shortcut to `tinytodo.exe` in `shell:startup`.
- KDE: System Settings → Autostart → Add Application.

## Configuration

`config.toml` is created with comments on first run, in
`%APPDATA%\tinytodo\` (Windows) or `~/.config/tinytodo/` (Linux):

```toml
channel = "todos-private"   # private channel name, or any channel ID
ignore_user_ids = []        # e.g. your own Slack user ID

[notifications]
enabled = true              # false: no popups; read via the tray instead
auto_hide_seconds = 0       # hide an untouched popup after N seconds (0 = never)
```

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

The code has three layers, each testable on its own:

| File | Role |
|---|---|
| `src/slack.rs` | Web API client, Socket Mode framing, reconnecting feed |
| `src/policy.rs` | Which messages notify; the dedup cursor (pure logic) |
| `src/ui.rs` | iced popup / conversation window |
| `src/config.rs` | Config file, keyring tokens, cursor file |
| `src/tray.rs`, `src/platform.rs` | Tray icon; per-OS window tweaks |

Debug builds have two helpers:

- `TINYTODO_DEBUG_NOTIFY_OWN=1` treats TinyTodo's own posts as the
  assistant's, so the whole loop can be tested with just the app's tokens.
- `TINYTODO_DEBUG_RECONNECTS=1` asks Slack to rotate the socket every few
  minutes, which exercises reconnection.
