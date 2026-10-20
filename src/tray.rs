//! System tray icon and menu.

use futures::Stream;
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Open,
    Quit,
}

pub fn create() -> Result<TrayIcon, String> {
    let menu = Menu::new();
    menu.append(&MenuItem::with_id("open", "Open conversation", true, None))
        .and_then(|()| menu.append(&MenuItem::with_id("quit", "Quit TinyTodo", true, None)))
        .map_err(|e| e.to_string())?;
    TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip("TinyTodo")
        .with_icon(icon())
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
        let action = if event.id == "quit" { Action::Quit } else { Action::Open };
        let _ = menu_tx.unbounded_send(action);
    }));
    rx
}

/// A small glassy blue disc with a white check mark, drawn in code.
fn icon() -> Icon {
    const N: usize = 32;
    let mut rgba = Vec::with_capacity(N * N * 4);
    let c = N as f32 / 2.0;
    let check = [(9.5, 16.5), (14.0, 21.0), (22.5, 11.5)];
    for y in 0..N {
        for x in 0..N {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let disc = (c - 0.5 - ((px - c).powi(2) + (py - c).powi(2)).sqrt()).clamp(0.0, 1.0);
            let stroke = check.windows(2).map(|s| segment_distance((px, py), s[0], s[1])).fold(f32::MAX, f32::min);
            let mark = (2.2 - stroke).clamp(0.0, 1.0);
            let shade = py / N as f32;
            let (r, g, b) = (40.0 - 20.0 * shade, 150.0 - 50.0 * shade, 255.0 - 30.0 * shade);
            let mix = |base: f32| (base + (255.0 - base) * mark) as u8;
            rgba.extend([mix(r), mix(g), mix(b), (disc * 255.0) as u8]);
        }
    }
    Icon::from_rgba(rgba, N as u32, N as u32).expect("valid icon")
}

fn segment_distance(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    ((p.0 - a.0 - t * dx).powi(2) + (p.1 - a.1 - t * dy).powi(2)).sqrt()
}
