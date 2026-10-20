//! Configuration, credentials and the tiny bit of persisted state.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const KEYRING_SERVICE: &str = "tinytodo";
const BOT_TOKEN_KEY: &str = "slack_bot_token";
const APP_TOKEN_KEY: &str = "slack_app_token";

const TEMPLATE: &str = r#"# TinyTodo configuration.
# Slack tokens are not stored here: they are read from the OS keyring
# (run `tinytodo --save-tokens` once with SLACK_BOT_TOKEN and SLACK_APP_TOKEN set)
# or from those environment variables.

# Private channel name (or any channel ID) the assistant talks in.
channel = "todos-private"

# Messages from these Slack user IDs never pop up (e.g. your own user ID, so
# messages you type in Slack itself don't notify you).
ignore_user_ids = []

[notifications]
# Show a popup when the assistant posts. When false, messages are still marked
# as seen; open the conversation from the tray to read them.
enabled = true
# Hide an untouched popup after this many seconds (0 = keep until dismissed).
auto_hide_seconds = 0
"#;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Config {
    pub channel: String,
    pub ignore_user_ids: Vec<String>,
    pub notifications: Notifications,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Notifications {
    pub enabled: bool,
    pub auto_hide_seconds: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self { channel: "todos-private".into(), ignore_user_ids: Vec::new(), notifications: Notifications::default() }
    }
}

impl Default for Notifications {
    fn default() -> Self {
        Self { enabled: true, auto_hide_seconds: 0 }
    }
}

impl Config {
    pub fn parse(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| format!("invalid config: {e}"))
    }

    /// Loads the config file, writing the commented template on first run.
    pub fn load() -> Result<Self, String> {
        let path = config_dir().join("config.toml");
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let _ = std::fs::create_dir_all(config_dir());
                let _ = std::fs::write(&path, TEMPLATE);
                Self::parse(TEMPLATE)
            }
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("tinytodo")
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
                .map_err(|e| format!("Slack tokens not found in keyring ({e}); run `tinytodo --save-tokens`"))
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
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        // Write-then-rename so a crash can't leave a truncated file behind.
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_string(self).unwrap()).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_parses_to_defaults() {
        assert_eq!(Config::parse(TEMPLATE).unwrap(), Config::default());
    }

    #[test]
    fn partial_config_falls_back_to_defaults() {
        let cfg = Config::parse("[notifications]\nauto_hide_seconds = 10\n").unwrap();
        assert_eq!(cfg.notifications.auto_hide_seconds, 10);
        assert!(cfg.notifications.enabled);
        assert_eq!(cfg.channel, "todos-private");
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
