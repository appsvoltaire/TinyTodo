//! A small log of errors and crashes. Release builds on Windows have no console,
//! so this file is the only trace of what went wrong. It lives next to the
//! delivery cursor and never grows past `MAX_BYTES` (plus one rotated copy).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Plenty for months of disconnects and the odd crash with its backtrace.
const MAX_BYTES: u64 = 256 * 1024;

/// Serializes writers: the UI, the Slack feed and a panicking thread may all log at once.
static LOCK: Mutex<()> = Mutex::new(());

pub fn path() -> PathBuf {
    crate::config::state_dir().join("tinytodo.log")
}

/// Records panics (and on Windows, native crashes) before the process dies.
pub fn install_crash_handlers() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
        let location = info.location().map(ToString::to_string).unwrap_or_default();
        let message = info.payload_as_str().unwrap_or("(no message)");
        let backtrace = std::backtrace::Backtrace::force_capture();
        write_entry(&format!("PANIC in thread '{thread}' at {location}: {}\n{backtrace}", one_line(message)));
        default(info);
    }));
    #[cfg(windows)]
    // SAFETY: installs a plain function as the process-wide last-chance handler.
    unsafe {
        windows_sys::Win32::System::Diagnostics::Debug::SetUnhandledExceptionFilter(Some(on_native_crash));
    }
}

/// Last-chance handler for crashes outside Rust's control (e.g. in a graphics driver).
#[cfg(windows)]
unsafe extern "system" fn on_native_crash(
    info: *const windows_sys::Win32::System::Diagnostics::Debug::EXCEPTION_POINTERS,
) -> i32 {
    // SAFETY: Windows passes valid exception pointers to the filter.
    let record = unsafe { info.as_ref().and_then(|i| i.ExceptionRecord.as_ref()) };
    if let Some(record) = record {
        write_entry(&format!(
            "CRASH: exception {:#010x} at {:?}",
            record.ExceptionCode as u32, record.ExceptionAddress
        ));
    }
    windows_sys::Win32::System::Diagnostics::Debug::EXCEPTION_CONTINUE_SEARCH
}

/// Logs a recoverable error (also to stderr, for debug builds and the CLI).
pub fn error(message: impl std::fmt::Display) {
    let message = one_line(&message.to_string());
    eprintln!("tinytodo: {message}");
    write_entry(&message);
}

/// Appends a timestamped entry. Best effort: logging must never take the app down.
fn write_entry(entry: &str) {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let _ = append_capped(&path(), &format!("{stamp} {entry}\n"), MAX_BYTES);
}

/// Appends `entry`, first moving a full log aside to `<name>.old` (replacing the previous one).
fn append_capped(path: &Path, entry: &str, max_bytes: u64) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if size > 0 && size + entry.len() as u64 > max_bytes {
        std::fs::rename(path, path.with_extension("log.old"))?;
    }
    // A single oversized entry is cut rather than blowing the cap.
    let entry = &entry[..entry.floor_char_boundary(max_bytes as usize)];
    std::fs::OpenOptions::new().create(true).append(true).open(path)?.write_all(entry.as_bytes())
}

/// Keeps text that came from elsewhere (Slack, the network) from forging extra log lines.
fn one_line(text: &str) -> String {
    text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_log(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tinytodo-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("tinytodo.log")
    }

    #[test]
    fn log_rotates_instead_of_growing_past_the_cap() {
        let path = temp_log("rotate");
        for i in 0..50 {
            append_capped(&path, &format!("entry {i:02} {}\n", "x".repeat(20)), 200).unwrap();
        }
        let current = std::fs::read_to_string(&path).unwrap();
        let old = std::fs::read_to_string(path.with_extension("log.old")).unwrap();
        assert!(current.len() <= 200 && old.len() <= 200);
        assert!(current.ends_with(&format!("entry 49 {}\n", "x".repeat(20))));
        assert!(old.contains("entry 4"));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn oversized_entries_are_truncated() {
        let path = temp_log("oversized");
        append_capped(&path, &"é".repeat(500), 100).unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() <= 100);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// Writes to the real log: `cargo test -- --ignored panics_are_logged`
    #[test]
    #[ignore]
    fn panics_are_logged() {
        install_crash_handlers();
        let marker = format!("test panic {}", std::process::id());
        let thread_marker = marker.clone();
        assert!(std::thread::spawn(move || panic!("{thread_marker}")).join().is_err());
        let log = std::fs::read_to_string(path()).unwrap();
        let entry = &log[log.rfind("PANIC").unwrap()..];
        assert!(entry.contains(&marker) && entry.contains("logfile.rs"), "{entry}");
    }

    #[test]
    fn entries_stay_on_one_line() {
        assert_eq!(one_line("bad\nfake 2026-01-01 entry\r\x1b[31m"), "bad fake 2026-01-01 entry  [31m");
    }
}
