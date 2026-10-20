// No console window for release builds on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod config;
mod platform;
mod policy;
mod slack;
mod style;
mod tray;
mod ui;

use config::{Config, Tokens};

const USAGE: &str = "\
tinytodo                run the app (tray + popups)
tinytodo --save-tokens  store SLACK_BOT_TOKEN and SLACK_APP_TOKEN from the
                        environment in the OS credential store
tinytodo --check-tokens verify SLACK_BOT_TOKEN and SLACK_APP_TOKEN with Slack
tinytodo --forget-tokens
                        remove the stored tokens";

fn main() -> iced::Result {
    let command = std::env::args().nth(1);
    if let Some(command) = command {
        attach_console();
        let result = match command.as_str() {
            "--save-tokens" => env_tokens().and_then(|t| t.save_to_keyring()).map(|()| "Tokens saved."),
            "--check-tokens" => env_tokens().and_then(check_tokens).map(|()| "Tokens OK."),
            "--forget-tokens" => Tokens::forget().map(|()| "Tokens removed."),
            _ => Ok(USAGE),
        };
        match result {
            Ok(message) => println!("{message}"),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }

    if !platform::claim_single_instance() {
        return Ok(());
    }
    platform::prepare_env();
    let cfg = Config::load();
    let feed = cfg.clone().and_then(|cfg| {
        Ok(slack::Feed { tokens: Tokens::load()?, channel: cfg.channel, ignore_user_ids: cfg.ignore_user_ids })
    });
    ui::run(cfg.unwrap_or_default(), feed)
}

fn env_tokens() -> Result<Tokens, String> {
    Tokens::from_env().ok_or_else(|| "set SLACK_BOT_TOKEN and SLACK_APP_TOKEN first".into())
}

fn check_tokens(tokens: Tokens) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    runtime.block_on(slack::Client::new(tokens).check())
}

/// Release builds have no console; borrow the parent's for CLI output.
fn attach_console() {
    #[cfg(windows)]
    // SAFETY: plain Win32 call with a constant argument.
    unsafe {
        windows_sys::Win32::System::Console::AttachConsole(windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS);
    }
}
