//! Per-platform pieces: window tweaks for the popup, launch at sign-in,
//! notification sound and the single-instance guard. Best effort throughout.

use futures::Stream;
use iced::window::Window;

/// Whether the desktop behind the panel is blurred (otherwise the panel is
/// drawn more opaque to stay readable).
pub const HAS_BLUR: bool = cfg!(windows);

/// Blurs what's behind the window and rounds its corners to `radius`, so clicks
/// outside the panel's corners fall through. Call again after resizes.
///
/// The blur always covers the whole window rectangle: on Windows 10 a window
/// region clips what the app draws, but not the blur. So the window has to be
/// exactly the size of the panel, or the blur shows around it.
#[cfg(windows)]
pub fn apply_glass(window: &dyn Window, radius: f32) {
    use windows_sys::Win32::Graphics::Gdi::{CreateRoundRectRgn, SetWindowRgn};
    use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;
    let Some(hwnd) = hwnd(window) else { return };
    let _ = window_vibrancy::apply_blur(window, Some((0, 0, 0, 0)));
    // SAFETY: `hwnd` is a live window owned by this process; the region is
    // handed over to the system by SetWindowRgn.
    unsafe {
        let scale = GetDpiForWindow(hwnd) as f32 / 96.0;
        let mut rect = std::mem::zeroed();
        GetClientRect(hwnd, &mut rect);
        let diameter = (2.0 * radius * scale) as i32;
        let region = CreateRoundRectRgn(0, 0, rect.right + 1, rect.bottom + 1, diameter, diameter);
        SetWindowRgn(hwnd, region, 1);
    }
}

#[cfg(not(windows))]
pub fn apply_glass(_: &dyn Window, _: f32) {}

/// Drops the background blur, e.g. while the popup fades out.
pub fn clear_blur(window: &dyn Window) {
    #[cfg(windows)]
    let _ = window_vibrancy::clear_blur(window);
    #[cfg(not(windows))]
    let _ = window;
}

/// Shows a hidden window without taking keyboard focus from the user's current app.
#[cfg(windows)]
pub fn show_without_activating(window: &dyn Window) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SW_SHOWNOACTIVATE, ShowWindow};
    if let Some(hwnd) = hwnd(window) {
        // SAFETY: `hwnd` is a live window owned by this process.
        unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
    }
}

#[cfg(not(windows))]
pub fn show_without_activating(_: &dyn Window) {}

#[cfg(windows)]
fn hwnd(window: &dyn Window) -> Option<windows_sys::Win32::Foundation::HWND> {
    match window.window_handle().ok()?.as_raw() {
        raw_window_handle::RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as _),
        _ => None,
    }
}

/// Process-wide settings that must be in place before the GUI starts.
pub fn prepare_env() {
    // SAFETY: called first thing in main, before any other thread exists.
    unsafe {
        // Of wgpu's Windows backends, only OpenGL gives the window real per-pixel transparency.
        #[cfg(windows)]
        if std::env::var_os("WGPU_BACKEND").is_none() {
            std::env::set_var("WGPU_BACKEND", "gl");
        }
        // Wayland doesn't let apps position their own windows, so the popup
        // couldn't sit in the top-right corner. XWayland can.
        #[cfg(target_os = "linux")]
        if std::env::var_os("TINYTODO_WAYLAND").is_none() {
            std::env::remove_var("WAYLAND_DISPLAY");
        }
    }
}

/// The notification sound: a Pixabay MP3, converted to WAV because
/// that's what every OS sound API plays without extra codecs.
const DROPLET: &[u8] = include_bytes!("../assets/droplet.wav");

/// Plays the notification sound without blocking.
pub fn play_notification_sound() {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
        // SAFETY: with SND_MEMORY the "name" is a pointer to an in-memory WAV image,
        // which is static so it outlives the asynchronous playback.
        unsafe { PlaySoundW(DROPLET.as_ptr().cast(), std::ptr::null_mut(), SND_MEMORY | SND_ASYNC | SND_NODEFAULT) };
    }
    #[cfg(not(windows))]
    {
        // Players take a file, so keep a copy in the cache directory.
        let Some(path) = dirs::cache_dir().map(|d| d.join("tinytodo").join("droplet.wav")) else { return };
        if std::fs::metadata(&path).map(|m| m.len()).ok() != Some(DROPLET.len() as u64) {
            let _ = std::fs::create_dir_all(path.parent().unwrap());
            let _ = std::fs::write(&path, DROPLET);
        }
        // PulseAudio/PipeWire first, then plain ALSA.
        for player in ["paplay", "pw-play", "aplay"] {
            if std::process::Command::new(player).arg(&path).spawn().is_ok() {
                break;
            }
        }
    }
}

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

#[cfg(windows)]
mod startup {
    use super::wide;
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
    };

    // The installer's "start at sign-in" option writes the same value.
    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE: &str = "TinyTodo";

    pub fn enabled() -> bool {
        let (key, value) = (wide(RUN_KEY), wide(VALUE));
        // SAFETY: valid NUL-terminated strings; null output pointers just test for existence.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        status == ERROR_SUCCESS
    }

    pub fn set(on: bool) -> Result<(), String> {
        let (key, value) = (wide(RUN_KEY), wide(VALUE));
        let status = if on {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let command = wide(&format!("\"{}\"", exe.display()));
            // SAFETY: valid NUL-terminated strings; the size covers `command` including its NUL.
            unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    key.as_ptr(),
                    value.as_ptr(),
                    REG_SZ,
                    command.as_ptr().cast(),
                    (command.len() * 2) as u32,
                )
            }
        } else {
            // SAFETY: valid NUL-terminated strings.
            match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr()) } {
                windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND => ERROR_SUCCESS,
                status => status,
            }
        };
        if status == ERROR_SUCCESS { Ok(()) } else { Err(format!("registry error {status}")) }
    }
}

#[cfg(not(windows))]
mod startup {
    use std::path::PathBuf;

    fn entry() -> Option<PathBuf> {
        Some(dirs::config_dir()?.join("autostart").join("tinytodo.desktop"))
    }

    pub fn enabled() -> bool {
        entry().is_some_and(|p| p.exists())
    }

    pub fn set(on: bool) -> Result<(), String> {
        let path = entry().ok_or("no config directory")?;
        if !on {
            return match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            };
        }
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let desktop = format!(
            "[Desktop Entry]\nType=Application\nName=TinyTodo\nExec=\"{}\"\nX-GNOME-Autostart-enabled=true\n",
            exe.display()
        );
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(path, desktop).map_err(|e| e.to_string())
    }
}

/// Whether TinyTodo starts when the user signs in.
pub fn launches_at_sign_in() -> bool {
    startup::enabled()
}

pub fn set_launch_at_sign_in(on: bool) -> Result<(), String> {
    startup::set(on)
}

/// Makes this the only running TinyTodo. Returns false if one is already
/// running, after asking it to show itself.
#[cfg(windows)]
pub fn claim_single_instance() -> bool {
    use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
    use windows_sys::Win32::System::Threading::{CreateEventW, CreateMutexW, EVENT_MODIFY_STATE, OpenEventW, SetEvent};
    let (mutex, event) = (wide(instance::MUTEX), wide(instance::EVENT));
    // SAFETY: valid NUL-terminated names; handles are intentionally kept for the process lifetime.
    unsafe {
        CreateMutexW(std::ptr::null(), 0, mutex.as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let other = OpenEventW(EVENT_MODIFY_STATE, 0, event.as_ptr());
            if !other.is_null() {
                SetEvent(other);
            }
            return false;
        }
        let handle = CreateEventW(std::ptr::null(), 0, 0, event.as_ptr());
        instance::EVENT_HANDLE.store(handle as usize, std::sync::atomic::Ordering::Relaxed);
    }
    true
}

#[cfg(not(windows))]
pub fn claim_single_instance() -> bool {
    true
}

#[cfg(windows)]
mod instance {
    pub const MUTEX: &str = "TinyTodo.Instance";
    pub const EVENT: &str = "TinyTodo.Activate";
    pub static EVENT_HANDLE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
}

/// Requests from later launches (e.g. the Start Menu shortcut) to show the app.
pub fn activations() -> impl Stream<Item = ()> {
    let (tx, rx) = futures::channel::mpsc::unbounded();
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::{INFINITE, WaitForSingleObject};
        let handle = instance::EVENT_HANDLE.load(std::sync::atomic::Ordering::Relaxed);
        if handle != 0 {
            std::thread::spawn(move || {
                // SAFETY: the event handle stays open for the life of the process.
                while unsafe { WaitForSingleObject(handle as _, INFINITE) } == 0 {
                    if tx.unbounded_send(()).is_err() {
                        break;
                    }
                }
            });
        }
    }
    #[cfg(not(windows))]
    drop(tx);
    rx
}

#[cfg(test)]
mod tests {
    use super::DROPLET;

    #[test]
    fn droplet_is_a_pcm_wav() {
        assert_eq!(&DROPLET[0..4], b"RIFF");
        assert_eq!(&DROPLET[8..16], b"WAVEfmt ");
        assert_eq!(u16::from_le_bytes([DROPLET[20], DROPLET[21]]), 1, "PCM");
        assert_eq!(u32::from_le_bytes(DROPLET[4..8].try_into().unwrap()) as usize, DROPLET.len() - 8);
    }

    /// Audible: `cargo test -- --ignored plays_droplet`
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn plays_droplet() {
        use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_MEMORY, SND_NODEFAULT, SND_SYNC};
        // SAFETY: as in `play_notification_sound`; synchronous here.
        let ok =
            unsafe { PlaySoundW(DROPLET.as_ptr().cast(), std::ptr::null_mut(), SND_MEMORY | SND_SYNC | SND_NODEFAULT) };
        assert_ne!(ok, 0, "Windows rejected the sound");
    }
}
