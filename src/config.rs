//! Configuration, credentials and the tiny bit of persisted state.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const KEYRING_SERVICE: &str = "tinytodo";
const BOT_TOKEN_KEY: &str = "slack_bot_token";
const APP_TOKEN_KEY: &str = "slack_app_token";

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Config {
    pub channel: String,
    pub ignore_user_ids: Vec<String>,
    pub notifications: Notifications,
    pub appearance: Appearance,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Notifications {
    pub enabled: bool,
    pub fade_after_seconds: u64,
    pub sound: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub transparency: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            channel: "todos-private".into(),
            ignore_user_ids: Vec::new(),
            notifications: Notifications::default(),
            appearance: Appearance::default(),
        }
    }
}

impl Default for Notifications {
    fn default() -> Self {
        Self { enabled: true, fade_after_seconds: 15, sound: false }
    }
}

impl Default for Appearance {
    fn default() -> Self {
        // Without a blurred backdrop the panel needs to be more solid to stay readable.
        Self { transparency: if crate::platform::HAS_BLUR { 0.4 } else { 0.15 } }
    }
}

pub const MAX_TRANSPARENCY: f32 = 0.8;
pub const MAX_FADE_SECONDS: u64 = 60;

impl Config {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut cfg: Self = toml::from_str(text).map_err(|e| format!("invalid config: {e}"))?;
        cfg.appearance.transparency = cfg.appearance.transparency.clamp(0.0, MAX_TRANSPARENCY);
        Ok(cfg)
    }

    fn path() -> PathBuf {
        config_dir().join("config.toml")
    }

    /// Loads the config file, writing the commented defaults on first run.
    pub fn load() -> Result<Self, String> {
        let path = Self::path();
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let cfg = Self::default();
                let _ = cfg.save();
                Ok(cfg)
            }
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// Writes the config back out, comments included, so it stays hand-editable.
    pub fn save(&self) -> Result<(), String> {
        write_atomic(&Self::path(), &self.render())
    }

    fn render(&self) -> String {
        let channel = toml::Value::from(self.channel.as_str());
        let ignore = toml::Value::from(self.ignore_user_ids.clone());
        let n = &self.notifications;
        format!(
            r#"# TinyTodo configuration. Also editable from the tray: Settings.
# Slack tokens are not stored here: they live in the OS credential store
# (entered in the installer, or `tinytodo --save-tokens` with SLACK_BOT_TOKEN
# and SLACK_APP_TOKEN set). Those environment variables also override it.

# Private channel name (or any channel ID) the assistant talks in.
channel = {channel}

# Messages from these Slack user IDs never pop up (e.g. your own user ID, so
# messages you type in Slack itself don't notify you).
ignore_user_ids = {ignore}

[notifications]
# Show a popup when the assistant posts. When false, messages are still marked
# as seen; open the conversation from the tray to read them.
enabled = {}
# Fade out an untouched popup after this many seconds (0 = keep until dismissed).
fade_after_seconds = {}
# Play the system notification sound for new messages.
sound = {}

[appearance]
# How see-through the popup glass is, from 0.0 (solid) to {MAX_TRANSPARENCY}.
transparency = {:.2}
"#,
            n.enabled, n.fade_after_seconds, n.sound, self.appearance.transparency
        )
    }
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("tinytodo")
}

/// Write-then-rename so a crash can't leave a truncated file behind.
fn write_atomic(path: &Path, contents: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, contents).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// Slack tokens. `Debug` is redacted so they can't end up in logs by accident.
#[derive(Clone)]
pub struct Tokens {
    pub bot: String,
    pub app: String,
}

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Tokens(<redacted>)")
    }
}

impl Tokens {
    /// Environment variables win; otherwise the OS keyring.
    pub fn load() -> Result<Self, String> {
        if let Some(tokens) = Self::from_env() {
            return Ok(tokens);
        }
        let get = |key| {
            keyring::Entry::new(KEYRING_SERVICE, key)
                .and_then(|e| e.get_password())
                .map_err(|e| format!("Slack tokens not found ({e}). Run the TinyTodo installer to enter them."))
        };
        Ok(Self { bot: get(BOT_TOKEN_KEY)?, app: get(APP_TOKEN_KEY)? })
    }

    pub fn from_env() -> Option<Self> {
        let bot = std::env::var("SLACK_BOT_TOKEN").ok().filter(|t| !t.is_empty())?;
        let app = std::env::var("SLACK_APP_TOKEN").ok().filter(|t| !t.is_empty())?;
        Some(Self { bot, app })
    }

    pub fn save_to_keyring(&self) -> Result<(), String> {
        for (key, value) in [(BOT_TOKEN_KEY, &self.bot), (APP_TOKEN_KEY, &self.app)] {
            keyring::Entry::new(KEYRING_SERVICE, key)
                .and_then(|e| e.set_password(value))
                .map_err(|e| format!("could not store {key} in keyring: {e}"))?;
        }
        Ok(())
    }

    /// Removes stored tokens (on uninstall). Missing entries are fine.
    pub fn forget() -> Result<(), String> {
        for key in [BOT_TOKEN_KEY, APP_TOKEN_KEY] {
            match keyring::Entry::new(KEYRING_SERVICE, key).and_then(|e| e.delete_credential()) {
                Ok(()) | Err(keyring::Error::NoEntry) => {}
                Err(e) => return Err(format!("could not remove {key}: {e}")),
            }
        }
        Ok(())
    }
}

/// The delivery cursor: the newest Slack message timestamp already handled.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub cursor: Option<String>,
}

impl State {
    fn path() -> PathBuf {
        dirs::state_dir()
            .or_else(dirs::data_local_dir)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("tinytodo")
            .join("state.json")
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        write_atomic(&Self::path(), &serde_json::to_string(self).unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendered_config_round_trips() {
        assert_eq!(Config::parse(&Config::default().render()).unwrap(), Config::default());
        let cfg = Config {
            channel: "a \"quoted\" name".into(),
            ignore_user_ids: vec!["U1".into(), "U2".into()],
            notifications: Notifications { enabled: false, fade_after_seconds: 42, sound: true },
            appearance: Appearance { transparency: 0.25 },
        };
        assert_eq!(Config::parse(&cfg.render()).unwrap(), cfg);
    }

    #[test]
    fn partial_config_falls_back_to_defaults() {
        // Also: keys from older versions (`auto_hide_seconds`) are ignored.
        let cfg = Config::parse(
            "[notifications]
sound = true
auto_hide_seconds = 0
",
        )
        .unwrap();
        assert!(cfg.notifications.sound);
        assert!(cfg.notifications.enabled);
        assert_eq!(cfg.notifications.fade_after_seconds, 15);
        assert_eq!(cfg.channel, "todos-private");
    }

    #[test]
    fn transparency_is_clamped() {
        let cfg = Config::parse(
            "[appearance]
transparency = 3.0
",
        )
        .unwrap();
        assert_eq!(cfg.appearance.transparency, MAX_TRANSPARENCY);
    }

    #[test]
    fn bad_config_is_an_error() {
        assert!(Config::parse("channel = 5").is_err());
    }

    #[test]
    fn tokens_debug_is_redacted() {
        let t = Tokens { bot: "xoxb-secret".into(), app: "xapp-secret".into() };
        assert!(!format!("{t:?}").contains("secret"));
    }
}
