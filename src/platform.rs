//! Per-platform window tweaks for the popup. Everything here is best effort.

use iced::window::Window;

/// Whether the desktop behind the panel is blurred (otherwise the panel is
/// drawn more opaque to stay readable).
pub const HAS_BLUR: bool = cfg!(windows);

/// Blurs what's behind the window and clips it to the rounded panel, which is
/// inset by `inset` logical pixels and has corner `radius`. Call again after resizes.
#[cfg(windows)]
pub fn apply_glass(window: &dyn Window, inset: f32, radius: f32) {
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
        let (inset, diameter) = ((inset * scale) as i32, (2.0 * radius * scale) as i32);
        let region =
            CreateRoundRectRgn(inset, inset, rect.right - inset + 1, rect.bottom - inset + 1, diameter, diameter);
        SetWindowRgn(hwnd, region, 1);
    }
}

#[cfg(not(windows))]
pub fn apply_glass(_: &dyn Window, _: f32, _: f32) {}

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
