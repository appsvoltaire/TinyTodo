// No console window for release builds on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod config;
mod platform;
mod policy;
mod slack;
mod tray;
mod ui;

use config::{Config, Tokens};

const USAGE: &str = "\
tinytodo            run the app (tray + popups)
tinytodo --save-tokens
                    store SLACK_BOT_TOKEN and SLACK_APP_TOKEN from the
                    environment in the OS keyring, then exit";

fn main() -> iced::Result {
    match std::env::args().nth(1).as_deref() {
        None => {}
        Some("--save-tokens") => {
            attach_console();
            let result = Tokens::from_env()
                .ok_or_else(|| "set SLACK_BOT_TOKEN and SLACK_APP_TOKEN first".to_string())
                .and_then(|t| t.save_to_keyring());
            match result {
                Ok(()) => println!("Tokens saved to the OS keyring."),
                Err(e) => {
                    eprintln!("tinytodo: {e}");
                    std::process::exit(1);
                }
            }
            return Ok(());
        }
        Some(_) => {
            attach_console();
            println!("{USAGE}");
            return Ok(());
        }
    }

    platform::prepare_env();
    let cfg = Config::load();
    let feed = cfg.clone().and_then(|cfg| {
        Ok(slack::Feed { tokens: Tokens::load()?, channel: cfg.channel, ignore_user_ids: cfg.ignore_user_ids })
    });
    ui::run(cfg.unwrap_or_default(), feed)
}

/// Release builds have no console; borrow the parent's for CLI output.
fn attach_console() {
    #[cfg(windows)]
    // SAFETY: plain Win32 call with a constant argument.
    unsafe {
        windows_sys::Win32::System::Console::AttachConsole(windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS);
    }
}
