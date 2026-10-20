//! The popup / conversation window: one small always-on-top glass panel in the
//! top-right corner, opened when the assistant writes or from the tray.

use crate::config::Config;
use crate::platform;
use crate::policy;
use crate::slack::{self, Feed, Message, Session};
use crate::tray;
use iced::widget::{button, column, container, mouse_area, operation, row, scrollable, space, stack, text, text_input};
use iced::{
    Background, Border, Center, Color, Element, Fill, Font, Point, Radians, Shadow, Size, Subscription, Task, Vector,
    event, font, gradient, keyboard, window,
};
use std::time::Duration;

const WIDTH: f32 = 400.0;
/// Transparent margin around the panel so its shadow isn't clipped.
const GUTTER: f32 = 10.0;
const RADIUS: f32 = 22.0;
const SCREEN_MARGIN: f32 = 6.0;
const CONVERSATION_HEIGHT: f32 = 560.0;
const MAX_POPUP_BODY: f32 = 300.0;
/// Most messages kept in memory, both in the conversation view and in the popup.
const MAX_MESSAGES: usize = 10;
const INPUT: &str = "reply";

pub fn run(cfg: Config, feed: Result<Feed, String>) -> iced::Result {
    iced::daemon(move || App::boot(cfg.clone(), feed.clone()), App::update, App::view)
        .title(|_: &App, _| "TinyTodo".to_string())
        .subscription(App::subscription)
        .style(|_, _| iced::theme::Style { background_color: Color::TRANSPARENT, text_color: Color::BLACK })
        .run()
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Popup,
    Conversation,
}

#[derive(Debug, Clone)]
enum Status {
    Connecting,
    Online,
    Offline(String),
}

struct App {
    cfg: Config,
    feed: Option<Feed>,
    session: Option<Session>,
    status: Status,
    tray: Option<tray_icon::TrayIcon>,
    window: Option<window::Id>,
    activate_on_open: bool,
    mode: Mode,
    unread: Vec<Message>,
    history: Vec<Message>,
    draft: String,
    sending: bool,
    error: Option<String>,
    /// The user has touched the popup, so it won't auto-hide.
    engaged: bool,
    hide_generation: u64,
    dark: bool,
}

#[derive(Debug, Clone)]
enum Msg {
    Slack(slack::Event),
    Tray(tray::Action),
    ThemeMode(iced::theme::Mode),
    WindowOpened(window::Id),
    WindowClosed(window::Id),
    Draft(String),
    Engage,
    Send,
    Sent(Result<Message, String>),
    ShowHistory,
    History(Result<Vec<Message>, String>),
    Dismiss,
    AutoHide(u64),
}

impl App {
    fn boot(cfg: Config, feed: Result<Feed, String>) -> (Self, Task<Msg>) {
        let tray = tray::create().map_err(|e| eprintln!("tinytodo: no tray icon: {e}")).ok();
        let (feed, error) = match feed {
            Ok(feed) => (Some(feed), None),
            Err(e) => (None, Some(e)),
        };
        let mut app = Self {
            cfg,
            feed,
            session: None,
            status: Status::Connecting,
            tray,
            window: None,
            activate_on_open: false,
            mode: Mode::Popup,
            unread: Vec::new(),
            history: Vec::new(),
            draft: String::new(),
            sending: false,
            error,
            engaged: false,
            hide_generation: 0,
            dark: true,
        };
        let theme = iced::system::theme().map(Msg::ThemeMode);
        let startup = if app.error.is_some() { app.show(Mode::Popup, false) } else { Task::none() };
        (app, Task::batch([theme, startup]))
    }

    fn update(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::Slack(event) => return self.on_slack(event),
            Msg::Tray(tray::Action::Open) => return self.open_conversation(),
            Msg::Tray(tray::Action::Quit) => return iced::exit(),
            Msg::ThemeMode(mode) => self.dark = mode != iced::theme::Mode::Light,
            Msg::WindowOpened(id) => {
                let activate = self.activate_on_open;
                let prepare = window::run(id, move |w| {
                    platform::apply_glass(w, GUTTER, RADIUS);
                    if !activate {
                        platform::show_without_activating(w);
                    }
                });
                let focus = if activate {
                    Task::batch([window::gain_focus(id), operation::focus(INPUT)])
                } else {
                    Task::none()
                };
                return Task::batch([prepare.discard(), focus]);
            }
            Msg::WindowClosed(id) => {
                if self.window == Some(id) {
                    self.window = None;
                }
            }
            Msg::Draft(draft) => {
                self.draft = draft;
                self.engaged = true;
            }
            Msg::Engage => self.engaged = true,
            Msg::Send => return self.send(),
            Msg::Sent(Ok(sent)) => {
                self.sending = false;
                self.draft.clear();
                if self.mode == Mode::Conversation {
                    self.push_history(sent);
                } else {
                    return self.hide();
                }
            }
            Msg::Sent(Err(e)) | Msg::History(Err(e)) => {
                self.sending = false;
                self.error = Some(e);
                return self.resize();
            }
            Msg::ShowHistory => return self.open_conversation(),
            Msg::History(Ok(history)) => {
                self.history.clear();
                for message in history {
                    keep_recent(&mut self.history, message);
                }
            }
            Msg::Dismiss => return self.hide(),
            Msg::AutoHide(generation) => {
                if generation == self.hide_generation && !self.engaged && self.mode == Mode::Popup {
                    return self.hide();
                }
            }
        }
        Task::none()
    }

    fn on_slack(&mut self, event: slack::Event) -> Task<Msg> {
        match event {
            slack::Event::Ready(session) => self.session = Some(session),
            slack::Event::Connected => self.set_status(Status::Online),
            slack::Event::Disconnected(reason) => {
                eprintln!("tinytodo: disconnected: {reason}");
                self.set_status(Status::Offline(reason));
            }
            slack::Event::Notify(message) => return self.notify(message),
        }
        Task::none()
    }

    fn set_status(&mut self, status: Status) {
        if let Some(tray) = &self.tray {
            let tip = match &status {
                Status::Online => "TinyTodo".to_string(),
                Status::Connecting => "TinyTodo — connecting…".to_string(),
                Status::Offline(reason) => format!("TinyTodo — offline ({reason})"),
            };
            let _ = tray.set_tooltip(Some(tip));
        }
        self.status = status;
    }

    fn notify(&mut self, message: Message) -> Task<Msg> {
        if self.window.is_some() && self.mode == Mode::Conversation {
            self.push_history(message);
            return Task::none();
        }
        if !self.cfg.notifications.enabled {
            return Task::none();
        }
        keep_recent(&mut self.unread, message);
        let show = self.show(Mode::Popup, false);
        let secs = self.cfg.notifications.auto_hide_seconds;
        if secs == 0 {
            return show;
        }
        self.hide_generation += 1;
        let generation = self.hide_generation;
        let timer = Task::perform(tokio::time::sleep(Duration::from_secs(secs)), move |()| Msg::AutoHide(generation));
        Task::batch([show, timer])
    }

    fn open_conversation(&mut self) -> Task<Msg> {
        self.unread.clear();
        self.error = None;
        let fetch = match &self.session {
            Some(s) => {
                let (client, channel) = (s.client.clone(), s.channel.clone());
                Task::perform(async move { client.history(&channel, None, MAX_MESSAGES as u32).await }, Msg::History)
            }
            None => {
                self.error = Some("Not connected to Slack yet.".into());
                Task::none()
            }
        };
        Task::batch([self.show(Mode::Conversation, true), fetch])
    }

    fn send(&mut self) -> Task<Msg> {
        let text = self.draft.trim().to_string();
        if text.is_empty() || self.sending {
            return Task::none();
        }
        let Some(session) = &self.session else {
            self.error = Some("Not connected to Slack yet.".into());
            return self.resize();
        };
        self.sending = true;
        self.engaged = true;
        self.error = None;
        let (client, channel) = (session.client.clone(), session.channel.clone());
        Task::perform(async move { client.post(&channel, &text).await }, Msg::Sent)
    }

    fn push_history(&mut self, message: Message) {
        keep_recent(&mut self.history, message);
    }

    /// Opens the window (or adapts the open one) for `mode`.
    fn show(&mut self, mode: Mode, activate: bool) -> Task<Msg> {
        self.mode = mode;
        if let Some(id) = self.window {
            let resize = self.resize();
            return if activate {
                Task::batch([resize, window::gain_focus(id), operation::focus(INPUT)])
            } else {
                resize
            };
        }
        self.engaged = activate;
        self.activate_on_open = activate;
        let (id, open) = window::open(window::Settings {
            size: Size::new(WIDTH, self.height()),
            position: window::Position::SpecificWith(|win, screen| {
                Point::new(screen.width - win.width - SCREEN_MARGIN, SCREEN_MARGIN)
            }),
            // On Windows a popup is shown later without taking focus; see `show_without_activating`.
            visible: activate || cfg!(not(windows)),
            resizable: false,
            decorations: false,
            transparent: true,
            level: window::Level::AlwaysOnTop,
            exit_on_close_request: false,
            platform_specific: platform_settings(),
            ..Default::default()
        });
        self.window = Some(id);
        open.map(Msg::WindowOpened)
    }

    fn resize(&self) -> Task<Msg> {
        match self.window {
            Some(id) => window::resize(id, Size::new(WIDTH, self.height()))
                .chain(window::run(id, |w| platform::apply_glass(w, GUTTER, RADIUS)).discard()),
            None => Task::none(),
        }
    }

    fn hide(&mut self) -> Task<Msg> {
        self.unread.clear();
        self.error = None;
        self.mode = Mode::Popup;
        self.hide_generation += 1;
        match self.window.take() {
            Some(id) => window::close(id),
            None => Task::none(),
        }
    }

    fn height(&self) -> f32 {
        match self.mode {
            Mode::Conversation => CONVERSATION_HEIGHT,
            Mode::Popup => {
                let texts = self.unread.iter().map(|m| slack::plain_text(&m.text));
                let body = texts.chain(self.error.clone()).map(|t| estimate_text_height(&t) + 12.0).sum::<f32>();
                // Gutters, panel padding, header, input, buttons and the gaps between them.
                2.0 * GUTTER + 32.0 + 20.0 + 40.0 + 32.0 + 3.0 * 12.0 + body.clamp(20.0, MAX_POPUP_BODY)
            }
        }
    }

    fn subscription(&self) -> Subscription<Msg> {
        let mut subscriptions = vec![
            Subscription::run(tray::actions).map(Msg::Tray),
            window::close_requests().map(|_| Msg::Dismiss),
            window::close_events().map(Msg::WindowClosed),
            iced::system::theme_changes().map(Msg::ThemeMode),
            event::listen_with(|event, _, _| match event {
                iced::Event::Keyboard(keyboard::Event::KeyPressed {
                    key: keyboard::Key::Named(keyboard::key::Named::Escape),
                    ..
                }) => Some(Msg::Dismiss),
                _ => None,
            }),
        ];
        if let Some(feed) = &self.feed {
            subscriptions.push(Subscription::run_with(feed.clone(), Feed::stream).map(Msg::Slack));
        }
        Subscription::batch(subscriptions)
    }

    fn view(&self, _window: window::Id) -> Element<'_, Msg> {
        let p = Palette::new(self.dark);
        let content = match self.mode {
            Mode::Popup => self.popup(p),
            Mode::Conversation => self.conversation(p),
        };
        let glass = stack![
            container(space()).width(Fill).height(Fill).style(move |_| p.glass()),
            container(space()).width(Fill).height(Fill).style(move |_| p.sheen()),
            container(content).padding(16).width(Fill).height(Fill),
        ];
        container(mouse_area(glass).on_enter(Msg::Engage)).padding(GUTTER).into()
    }

    fn popup(&self, p: Palette) -> Element<'_, Msg> {
        let (title, time) = match self.unread.last() {
            Some(latest) => ("Assistant", time_label(&latest.ts)),
            None => ("TinyTodo", String::new()),
        };
        let count = (self.unread.len() > 1).then(|| text(format!("{} new", self.unread.len())).size(11).color(p.dim));
        let header = row![p.status_dot(&self.status), text(title).size(13).font(SEMIBOLD).color(p.text)]
            .push(text(time).size(12).color(p.dim))
            .push(count)
            .push(space::horizontal())
            .push(p.close_button())
            .spacing(8)
            .align_y(Center);
        let messages =
            column(self.unread.iter().map(|m| text(slack::plain_text(&m.text)).size(14).color(p.text).into()))
                .spacing(12)
                .push(self.error.as_ref().map(|e| text(e).size(12).color(p.error)));
        let reply_ready = !self.sending && !self.draft.trim().is_empty() && self.session.is_some();
        let actions = row![
            p.button("History", Some(Msg::ShowHistory), false),
            space::horizontal(),
            p.button("Dismiss", Some(Msg::Dismiss), false),
            p.button(if self.sending { "Sending…" } else { "Reply" }, reply_ready.then_some(Msg::Send), true),
        ]
        .spacing(8);
        column![header, p.scroll(messages), self.input(p), actions].spacing(12).into()
    }

    fn conversation(&self, p: Palette) -> Element<'_, Msg> {
        let status = match &self.status {
            Status::Connecting => "Connecting…",
            Status::Online => "Connected",
            Status::Offline(_) => "Reconnecting…",
        };
        let header = row![
            p.status_dot(&self.status),
            text("Conversation").size(13).font(SEMIBOLD).color(p.text),
            text(status).size(12).color(p.dim),
            space::horizontal(),
            p.close_button(),
        ]
        .spacing(8)
        .align_y(Center);
        let identity = self.session.as_ref().map(|s| &s.identity);
        let bubbles = self
            .history
            .iter()
            .filter(|m| policy::is_conversational(m) && !m.text.trim().is_empty())
            .map(|m| p.bubble(m, identity.is_some_and(|i| i.is_mine(m))));
        let list = column(bubbles).spacing(8).push(self.error.as_ref().map(|e| text(e).size(12).color(p.error)));
        let send_ready = !self.sending && !self.draft.trim().is_empty() && self.session.is_some();
        let actions = row![
            space::horizontal(),
            p.button("Close", Some(Msg::Dismiss), false),
            p.button(if self.sending { "Sending…" } else { "Send" }, send_ready.then_some(Msg::Send), true),
        ]
        .spacing(8);
        column![header, p.scroll(list), self.input(p), actions].spacing(12).into()
    }

    fn input(&self, p: Palette) -> Element<'_, Msg> {
        text_input("Reply…", &self.draft)
            .id(INPUT)
            .on_input(Msg::Draft)
            .on_submit(Msg::Send)
            .padding([9, 12])
            .size(14)
            .style(move |_, status| p.input(status))
            .into()
    }
}

/// Adds `message` in timestamp order, skipping duplicates and keeping only the newest `MAX_MESSAGES`.
fn keep_recent(list: &mut Vec<Message>, message: Message) {
    if list.iter().any(|m| m.ts == message.ts) {
        return;
    }
    list.push(message);
    list.sort_by(|a, b| policy::ts_cmp(&a.ts, &b.ts));
    let excess = list.len().saturating_sub(MAX_MESSAGES);
    list.drain(..excess);
}

const SEMIBOLD: Font = Font { weight: font::Weight::Semibold, ..Font::DEFAULT };

/// Rough wrapped height of message text at size 14 in the popup's width.
fn estimate_text_height(text: &str) -> f32 {
    const CHARS_PER_LINE: usize = 44;
    let lines: usize = text.lines().map(|l| l.chars().count().div_ceil(CHARS_PER_LINE).max(1)).sum();
    lines.max(1) as f32 * 20.0
}

fn time_label(ts: &str) -> String {
    use chrono::{DateTime, Local};
    let secs = ts.split('.').next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let Some(time) = DateTime::from_timestamp(secs, 0).map(|t| t.with_timezone(&Local)) else {
        return String::new();
    };
    let format = if time.date_naive() == Local::now().date_naive() { "%-I:%M %p" } else { "%b %-d, %-I:%M %p" };
    time.format(format).to_string()
}

#[cfg(windows)]
fn platform_settings() -> window::settings::PlatformSpecific {
    window::settings::PlatformSpecific { skip_taskbar: true, ..Default::default() }
}

#[cfg(target_os = "linux")]
fn platform_settings() -> window::settings::PlatformSpecific {
    window::settings::PlatformSpecific { application_id: "tinytodo".into(), ..Default::default() }
}

#[cfg(not(any(windows, target_os = "linux")))]
fn platform_settings() -> window::settings::PlatformSpecific {
    Default::default()
}

/// Colors and styles for the glass look, in a light and a dark variant.
#[derive(Debug, Clone, Copy)]
struct Palette {
    dark: bool,
    text: Color,
    dim: Color,
    error: Color,
}

const ACCENT: Color = Color::from_rgb(0.04, 0.52, 1.0);

impl Palette {
    fn new(dark: bool) -> Self {
        if dark {
            Self {
                dark,
                text: Color::from_rgb(0.96, 0.96, 0.98),
                dim: Color::from_rgba(0.92, 0.92, 0.96, 0.6),
                error: Color::from_rgb(1.0, 0.45, 0.42),
            }
        } else {
            Self {
                dark,
                text: Color::from_rgb(0.08, 0.08, 0.1),
                dim: Color::from_rgba(0.1, 0.1, 0.14, 0.55),
                error: Color::from_rgb(0.8, 0.15, 0.12),
            }
        }
    }

    /// A white overlay at `alpha` in dark mode, a black one in light mode.
    fn wash(&self, alpha: f32) -> Color {
        if self.dark { Color::from_rgba(1.0, 1.0, 1.0, alpha) } else { Color::from_rgba(0.0, 0.0, 0.0, alpha * 0.6) }
    }

    fn rim(&self) -> Color {
        if self.dark { Color::from_rgba(1.0, 1.0, 1.0, 0.22) } else { Color::from_rgba(1.0, 1.0, 1.0, 0.9) }
    }

    fn glass(&self) -> container::Style {
        // With the desktop blurred behind us the tint can be lighter, like real glass.
        let opacity = if platform::HAS_BLUR { 0.72 } else { 1.0 };
        let (top, bottom) = if self.dark {
            (Color::from_rgba(0.24, 0.24, 0.28, 0.80), Color::from_rgba(0.11, 0.11, 0.14, 0.88))
        } else {
            (Color::from_rgba(1.0, 1.0, 1.0, 0.86), Color::from_rgba(0.92, 0.93, 0.96, 0.80))
        };
        container::Style {
            background: Some(vertical_gradient(top.scale_alpha(opacity), bottom.scale_alpha(opacity), 1.0)),
            border: Border { color: self.rim(), width: 1.0, radius: RADIUS.into() },
            shadow: Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.28),
                offset: Vector::new(0.0, 3.0),
                blur_radius: 10.0,
            },
            ..Default::default()
        }
    }

    /// Specular highlight across the top of the panel.
    fn sheen(&self) -> container::Style {
        let alpha = if self.dark { 0.10 } else { 0.5 };
        container::Style {
            background: Some(vertical_gradient(Color::from_rgba(1.0, 1.0, 1.0, alpha), Color::TRANSPARENT, 0.4)),
            border: Border { radius: RADIUS.into(), ..Default::default() },
            ..Default::default()
        }
    }

    fn status_dot<'a>(&self, status: &Status) -> Element<'a, Msg> {
        let color = match status {
            Status::Online => Color::from_rgb(0.2, 0.8, 0.4),
            Status::Connecting => Color::from_rgb(1.0, 0.75, 0.2),
            Status::Offline(_) => Color::from_rgb(1.0, 0.35, 0.3),
        };
        container(space())
            .width(8)
            .height(8)
            .style(move |_| container::Style {
                background: Some(color.into()),
                border: Border { radius: 4.0.into(), ..Default::default() },
                ..Default::default()
            })
            .into()
    }

    fn close_button<'a>(self) -> Element<'a, Msg> {
        button(text("×").size(16).color(self.dim).center())
            .width(24)
            .height(24)
            .padding(0)
            .on_press(Msg::Dismiss)
            .style(move |_, status| button::Style {
                background: matches!(status, button::Status::Hovered | button::Status::Pressed)
                    .then(|| self.wash(0.12).into()),
                border: Border { radius: 12.0.into(), ..Default::default() },
                ..Default::default()
            })
            .into()
    }

    fn button<'a>(self, label: &'a str, on_press: Option<Msg>, primary: bool) -> Element<'a, Msg> {
        button(text(label).size(13))
            .padding([6, 16])
            .on_press_maybe(on_press)
            .style(move |_, status| {
                let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
                let disabled = status == button::Status::Disabled;
                let (background, text_color, border) = if primary {
                    let alpha = if disabled {
                        0.35
                    } else if hovered {
                        0.85
                    } else {
                        1.0
                    };
                    (ACCENT.scale_alpha(alpha), Color::WHITE, Color::from_rgba(1.0, 1.0, 1.0, 0.25))
                } else {
                    (self.wash(if hovered { 0.18 } else { 0.10 }), self.text, self.rim())
                };
                button::Style {
                    background: Some(background.into()),
                    text_color: if disabled { text_color.scale_alpha(0.7) } else { text_color },
                    border: Border { color: border, width: 1.0, radius: 999.0.into() },
                    ..Default::default()
                }
            })
            .into()
    }

    fn input(&self, status: text_input::Status) -> text_input::Style {
        let focused = matches!(status, text_input::Status::Focused { .. });
        text_input::Style {
            background: self.wash(0.08).into(),
            border: Border {
                color: if focused { ACCENT.scale_alpha(0.8) } else { self.rim() },
                width: 1.0,
                radius: 12.0.into(),
            },
            icon: self.dim,
            placeholder: self.dim,
            value: self.text,
            selection: ACCENT.scale_alpha(0.35),
        }
    }

    /// A bottom-anchored list with a slim scrollbar kept clear of the content.
    fn scroll<'a>(self, content: impl Into<Element<'a, Msg>>) -> Element<'a, Msg> {
        let bar = scrollable::Scrollbar::new().width(4).scroller_width(4).spacing(6);
        scrollable(content)
            .direction(scrollable::Direction::Vertical(bar))
            .anchor_bottom()
            .height(Fill)
            .style(move |theme, status| scrollable::Style {
                vertical_rail: scrollable::Rail {
                    background: None,
                    border: Border::default(),
                    scroller: scrollable::Scroller {
                        background: self.wash(0.3).into(),
                        border: Border { radius: 2.0.into(), ..Default::default() },
                    },
                },
                ..scrollable::default(theme, status)
            })
            .into()
    }

    fn bubble<'a>(self, message: &'a Message, mine: bool) -> Element<'a, Msg> {
        let who = if mine { "You" } else { "Assistant" };
        let body = column![
            text(slack::plain_text(&message.text)).size(14).color(if mine { Color::WHITE } else { self.text }),
            text(format!("{who} · {}", time_label(&message.ts))).size(11).color(if mine {
                Color::from_rgba(1.0, 1.0, 1.0, 0.7)
            } else {
                self.dim
            }),
        ]
        .spacing(3);
        let bubble = container(body).padding([8, 12]).max_width(WIDTH * 0.75).style(move |_| container::Style {
            background: Some(if mine { ACCENT.scale_alpha(0.9).into() } else { self.wash(0.10).into() }),
            border: Border { color: self.rim(), width: if mine { 0.0 } else { 1.0 }, radius: 16.0.into() },
            ..Default::default()
        });
        if mine { row![space::horizontal(), bubble] } else { row![bubble, space::horizontal()] }.into()
    }
}

fn vertical_gradient(top: Color, bottom: Color, end: f32) -> Background {
    gradient::Linear::new(Radians::PI).add_stop(0.0, top).add_stop(end, bottom).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_the_newest_ten_messages() {
        let message = |ts: u32| Message {
            ts: format!("{ts}.000000"),
            user: None,
            bot_id: None,
            text: ts.to_string(),
            subtype: None,
            hidden: false,
        };
        let mut list = Vec::new();
        for ts in (1..=15).rev() {
            keep_recent(&mut list, message(ts));
        }
        keep_recent(&mut list, message(15));
        let kept: Vec<_> = list.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(kept, ["6", "7", "8", "9", "10", "11", "12", "13", "14", "15"]);
    }

    #[test]
    fn text_height_grows_with_wrapping_and_newlines() {
        assert_eq!(estimate_text_height(""), 20.0);
        assert_eq!(estimate_text_height("short"), 20.0);
        assert_eq!(estimate_text_height(&"x".repeat(45)), 40.0);
        assert_eq!(estimate_text_height("a\nb\nc"), 60.0);
    }

    #[test]
    fn time_label_handles_bad_input() {
        assert_eq!(time_label("garbage"), time_label("0"));
        assert!(!time_label("1792534645.644179").is_empty());
    }
}
