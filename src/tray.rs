//! System tray icon and menu, plus the app icon artwork.

use futures::Stream;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Open,
    Settings,
    Quit,
}

pub fn create() -> Result<TrayIcon, String> {
    let menu = Menu::new();
    menu.append_items(&[
        &MenuItem::with_id("open", "Open conversation", true, None),
        &MenuItem::with_id("settings", "Settings…", true, None),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id("quit", "Quit TinyTodo", true, None),
    ])
    .map_err(|e| e.to_string())?;
    TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip("TinyTodo")
        .with_icon(Icon::from_rgba(icon_rgba(32), 32, 32).expect("valid icon"))
        .build()
        .map_err(|e| e.to_string())
}

/// Tray clicks and menu picks as a stream. Call once.
pub fn actions() -> impl Stream<Item = Action> {
    let (tx, rx) = futures::channel::mpsc::unbounded();
    let menu_tx = tx.clone();
    TrayIconEvent::set_event_handler(Some(move |event| match event {
        TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }
        | TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } => {
            let _ = tx.unbounded_send(Action::Open);
        }
        _ => {}
    }));
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let action = match event.id.as_ref() {
            "quit" => Action::Quit,
            "settings" => Action::Settings,
            _ => Action::Open,
        };
        let _ = menu_tx.unbounded_send(action);
    }));
    rx
}

/// The app icon, a glassy blue disc with a white check mark, as `size`×`size` RGBA.
pub fn icon_rgba(size: u32) -> Vec<u8> {
    let n = size as f32;
    let unit = n / 32.0; // the artwork is designed on a 32 px grid
    let c = n / 2.0;
    let check = [(9.5, 16.5), (14.0, 21.0), (22.5, 11.5)].map(|(x, y)| (x * unit, y * unit));
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let disc = (c - 0.5 - ((px - c).powi(2) + (py - c).powi(2)).sqrt()).clamp(0.0, 1.0);
            let stroke = check.windows(2).map(|s| segment_distance((px, py), s[0], s[1])).fold(f32::MAX, f32::min);
            let mark = (2.2 * unit - stroke).clamp(0.0, 1.0);
            let shade = py / n;
            let (r, g, b) = (40.0 - 20.0 * shade, 150.0 - 50.0 * shade, 255.0 - 30.0 * shade);
            let mix = |base: f32| (base + (255.0 - base) * mark) as u8;
            rgba.extend([mix(r), mix(g), mix(b), (disc * 255.0) as u8]);
        }
    }
    rgba
}

fn segment_distance(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    ((p.0 - a.0 - t * dx).powi(2) + (p.1 - a.1 - t * dy).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_has_opaque_center_and_clear_corners() {
        let rgba = icon_rgba(32);
        assert_eq!(rgba.len(), 32 * 32 * 4);
        assert_eq!(rgba[3], 0, "corner is transparent");
        assert_eq!(rgba[(16 * 32 + 16) * 4 + 3], 255, "center is opaque");
    }

    /// Regenerates `installer/tinytodo.ico`: `cargo test -- --ignored write_installer_icon`
    #[test]
    #[ignore]
    fn write_installer_icon() {
        let sizes = [16u32, 24, 32, 48, 64, 256];
        let mut header = vec![0, 0, 1, 0];
        header.extend((sizes.len() as u16).to_le_bytes());
        let mut images = Vec::new();
        for &size in &sizes {
            // A 32-bit DIB: header, BGRA rows bottom-up, then an all-zero AND mask.
            let mut dib = Vec::new();
            for v in [40u32, size, size * 2] {
                dib.extend(v.to_le_bytes());
            }
            dib.extend(1u16.to_le_bytes());
            dib.extend(32u16.to_le_bytes());
            dib.extend([0u8; 24]);
            let rgba = icon_rgba(size);
            for row in (0..size as usize).rev() {
                for px in rgba[row * size as usize * 4..][..size as usize * 4].chunks(4) {
                    dib.extend([px[2], px[1], px[0], px[3]]);
                }
            }
            dib.extend(vec![0u8; (size as usize).div_ceil(32) * 4 * size as usize]);
            let offset = 6 + 16 * sizes.len() + images.len();
            let dim = if size >= 256 { 0 } else { size as u8 };
            header.extend([dim, dim, 0, 0]);
            header.extend(1u16.to_le_bytes());
            header.extend(32u16.to_le_bytes());
            header.extend((dib.len() as u32).to_le_bytes());
            header.extend((offset as u32).to_le_bytes());
            images.extend(dib);
        }
        header.extend(images);
        std::fs::write(concat!(env!("CARGO_MANIFEST_DIR"), "/installer/tinytodo.ico"), header).unwrap();
    }

    /// Regenerates `installer/tinytodo.png` (the Linux menu icon):
    /// `cargo test -- --ignored write_linux_icon`
    #[test]
    #[ignore]
    fn write_linux_icon() {
        let size = 256;
        let file = std::fs::File::create(concat!(env!("CARGO_MANIFEST_DIR"), "/installer/tinytodo.png")).unwrap();
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), size, size);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::High);
        encoder.write_header().unwrap().write_image_data(&icon_rgba(size)).unwrap();
    }
}
