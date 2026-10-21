//! Windows and app state: the glass popup / conversation panel in the top-right
//! corner, and a small settings window, both reachable from the tray.

use crate::checklist::Checklist;
use crate::config::{self, Config};
use crate::platform;
use crate::policy;
use crate::slack::{self, Feed, Message, Session};
use crate::style::{Palette, SEMIBOLD};
use crate::tray;
use iced::widget::{
    checkbox, column, container, mouse_area, operation, row, slider, space, stack, text, text_input, toggler,
};
use iced::{Center, Color, Element, Fill, Point, Size, Subscription, Task, Theme, event, keyboard, mouse, window};
use std::collections::HashMap;
use std::time::{Duration, Instant};

const WIDTH: f32 = 400.0;
/// Transparent margin around the panel so its shadow isn't clipped.
const GUTTER: f32 = 10.0;
const SCREEN_MARGIN: f32 = 6.0;
const CONVERSATION_HEIGHT: f32 = 560.0;
const MIN_HEIGHT: f32 = 170.0;
const MAX_POPUP_BODY: f32 = 300.0;
const MESSAGE_GAP: f32 = 20.0;
/// Most messages kept in memory, both in the conversation view and in the popup.
const MAX_MESSAGES: usize = 10;
const FADE_DURATION: Duration = Duration::from_millis(700);
const INPUT: &str = "reply";

pub fn run(cfg: Config, feed: Result<Feed, String>) -> iced::Result {
    iced::daemon(move || App::boot(cfg.clone(), feed.clone()), App::update, App::view)
        .title(App::title)
        .theme(|app: &App, _| app.theme.clone())
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
    popup: Option<window::Id>,
    settings: Option<window::Id>,
    activate_on_open: bool,
    mode: Mode,
    unread: Vec<Message>,
    history: Vec<Message>,
    draft: String,
    /// Checklist boxes ticked but not sent yet, by message timestamp.
    ticks: HashMap<String, Vec<bool>>,
    sending: bool,
    error: Option<String>,
    /// The pointer is over the popup; it never fades while hovered.
    hovered: bool,
    /// The user resized the popup; keep their size until it closes.
    user_sized: bool,
    fade_generation: u64,
    fading_since: Option<Instant>,
    /// Popup opacity while fading out, 1.0 otherwise.
    fade: f32,
    dark: bool,
    theme: Theme,
    launch_at_sign_in: bool,
    settings_error: Option<String>,
}

#[derive(Debug, Clone)]
enum Msg {
    Slack(slack::Event),
    Tray(tray::Action),
    /// Another launch asked us to show ourselves.
    Activate,
    ThemeMode(iced::theme::Mode),
    PopupOpened(window::Id),
    SettingsOpened(window::Id),
    WindowClosed(window::Id),
    Close(window::Id),
    Resized(window::Id),
    Draft(String),
    Hover(bool),
    /// A click, scroll or key press in a window.
    Interacted(window::Id),
    StartResize,
    Send,
    Sent(Result<Message, String>),
    /// Tick or untick item `.1` of the checklist in message `.0`.
    Check(String, usize, bool),
    SendChecklist(String),
    ChecklistSent(String, Result<Message, String>),
    ShowHistory,
    History(Result<Vec<Message>, String>),
    Dismiss,
    FadeDue(u64),
    FadeFrame(Instant),
    SetEnabled(bool),
    SetFadeSeconds(f32),
    SetSound(bool),
    SetTransparency(f32),
    SetLaunchAtSignIn(bool),
    SaveSettings,
    TestNotification,
}

impl App {
    fn boot(cfg: Config, feed: Result<Feed, String>) -> (Self, Task<Msg>) {
        let tray = tray::create().map_err(|e| crate::logfile::error(format!("no tray icon: {e}"))).ok();
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
            popup: None,
            settings: None,
            activate_on_open: false,
            mode: Mode::Popup,
            unread: Vec::new(),
            history: Vec::new(),
            draft: String::new(),
            ticks: HashMap::new(),
            sending: false,
            error,
            hovered: false,
            user_sized: false,
            fade_generation: 0,
            fading_since: None,
            fade: 1.0,
            dark: true,
            theme: crate::style::theme(true),
            launch_at_sign_in: false,
            settings_error: None,
        };
        let theme = iced::system::theme().map(Msg::ThemeMode);
        let startup = if app.error.is_some() { app.show(Mode::Popup, false) } else { Task::none() };
        (app, Task::batch([theme, startup]))
    }

    fn title(&self, window: window::Id) -> String {
        if self.settings == Some(window) { "TinyTodo Settings".into() } else { "TinyTodo".into() }
    }

    fn update(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::Slack(event) => return self.on_slack(event),
            Msg::Tray(tray::Action::Open) | Msg::Activate | Msg::ShowHistory => return self.open_conversation(),
            Msg::Tray(tray::Action::Settings) => return self.open_settings(),
            Msg::Tray(tray::Action::Quit) => return iced::exit(),
            Msg::ThemeMode(mode) => {
                self.dark = mode != iced::theme::Mode::Light;
                self.theme = crate::style::theme(self.dark);
            }
            Msg::PopupOpened(id) => {
                let activate = self.activate_on_open;
                let prepare = window::run(id, move |w| {
                    platform::apply_glass(w, GUTTER, crate::style::RADIUS);
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
            Msg::SettingsOpened(id) => return window::gain_focus(id),
            Msg::WindowClosed(id) => {
                if self.popup == Some(id) {
                    self.popup = None;
                }
                if self.settings == Some(id) {
                    self.settings = None;
                }
            }
            Msg::Close(id) => {
                if self.settings == Some(id) {
                    self.settings = None;
                    return window::close(id);
                }
                return self.hide();
            }
            Msg::Resized(id) => {
                // The rounded blur region has to follow the window's size.
                if self.popup == Some(id) && self.fading_since.is_none() {
                    return window::run(id, |w| platform::apply_glass(w, GUTTER, crate::style::RADIUS)).discard();
                }
            }
            Msg::Draft(draft) => {
                self.draft = draft;
                return self.restart_fade();
            }
            Msg::Hover(hovered) => {
                self.hovered = hovered;
                return if hovered { self.cancel_fade() } else { self.restart_fade() };
            }
            Msg::Interacted(id) => {
                if self.popup == Some(id) {
                    return self.restart_fade();
                }
            }
            Msg::StartResize => {
                if let Some(id) = self.popup {
                    self.user_sized = true;
                    return window::drag_resize(id, window::Direction::South);
                }
            }
            Msg::Send => return self.send(),
            Msg::Sent(Ok(sent)) => {
                self.sending = false;
                self.draft.clear();
                if self.mode == Mode::Conversation {
                    keep_recent(&mut self.history, sent);
                } else {
                    return self.hide();
                }
            }
            Msg::Check(ts, index, done) => {
                let original = self.find(&ts).and_then(|m| Checklist::parse(&m.text));
                if let Some(list) = original {
                    let ticks = self.ticks.entry(ts).or_insert_with(|| list.items.iter().map(|i| i.done).collect());
                    if let Some(tick) = ticks.get_mut(index) {
                        *tick = done;
                    }
                }
                return self.restart_fade();
            }
            Msg::SendChecklist(ts) => return self.send_checklist(ts),
            Msg::ChecklistSent(ts, Ok(sent)) => {
                self.sending = false;
                self.ticks.remove(&ts);
                if self.mode == Mode::Conversation {
                    keep_recent(&mut self.history, sent);
                } else {
                    return self.hide();
                }
            }
            Msg::Sent(Err(e)) | Msg::History(Err(e)) | Msg::ChecklistSent(_, Err(e)) => {
                self.sending = false;
                self.error = Some(e);
                return self.resize();
            }
            Msg::History(Ok(history)) => {
                self.history.clear();
                for message in history {
                    keep_recent(&mut self.history, message);
                }
            }
            Msg::Dismiss => return self.hide(),
            Msg::FadeDue(generation) => {
                if generation == self.fade_generation && self.may_fade() {
                    self.fading_since = Some(Instant::now());
                    if let Some(id) = self.popup {
                        return window::run(id, platform::clear_blur).discard();
                    }
                }
            }
            Msg::FadeFrame(now) => {
                if let Some(since) = self.fading_since {
                    self.fade = 1.0 - now.duration_since(since).as_secs_f32() / FADE_DURATION.as_secs_f32();
                    if self.fade <= 0.0 {
                        return self.hide();
                    }
                }
            }
            Msg::SetEnabled(on) => {
                self.cfg.notifications.enabled = on;
                self.save_settings();
            }
            Msg::SetFadeSeconds(secs) => self.cfg.notifications.fade_after_seconds = secs.round() as u64,
            Msg::SetSound(on) => {
                self.cfg.notifications.sound = on;
                self.save_settings();
                if on {
                    platform::play_notification_sound();
                }
            }
            Msg::SetTransparency(t) => self.cfg.appearance.transparency = t,
            Msg::SaveSettings => self.save_settings(),
            Msg::SetLaunchAtSignIn(on) => match platform::set_launch_at_sign_in(on) {
                Ok(()) => self.launch_at_sign_in = on,
                Err(e) => self.settings_error = Some(format!("Couldn't change startup: {e}")),
            },
            Msg::TestNotification => {
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
                return self.present(Message {
                    ts: format!("{}.{:06}", now.as_secs(), now.subsec_micros()),
                    user: None,
                    bot_id: None,
                    text: "This is a test notification. Replies typed here go to Slack.".into(),
                    subtype: None,
                    hidden: false,
                });
            }
        }
        Task::none()
    }

    fn on_slack(&mut self, event: slack::Event) -> Task<Msg> {
        match event {
            slack::Event::Ready(session) => self.session = Some(session),
            slack::Event::Connected => self.set_status(Status::Online),
            slack::Event::Disconnected(reason) => {
                crate::logfile::error(format!("disconnected: {reason}"));
                self.set_status(Status::Offline(reason));
            }
            slack::Event::Notify(message) => {
                if self.popup.is_some() && self.mode == Mode::Conversation {
                    keep_recent(&mut self.history, message);
                } else if self.cfg.notifications.enabled {
                    return self.present(message);
                }
            }
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

    /// Shows `message` in the popup (with sound, if enabled) and restarts the fade timer.
    fn present(&mut self, message: Message) -> Task<Msg> {
        keep_recent(&mut self.unread, message);
        if self.cfg.notifications.sound {
            platform::play_notification_sound();
        }
        let restore = self.cancel_fade();
        let show = self.show(Mode::Popup, false);
        Task::batch([restore, show, self.schedule_fade()])
    }

    fn may_fade(&self) -> bool {
        self.popup.is_some()
            && self.mode == Mode::Popup
            && self.cfg.notifications.fade_after_seconds > 0
            && !self.hovered
            && !self.sending
            && self.error.is_none()
            && !self.unread.iter().any(|m| self.checklist(m).is_some_and(|(_, edited)| edited))
    }

    fn schedule_fade(&mut self) -> Task<Msg> {
        self.fade_generation += 1;
        if !self.may_fade() {
            return Task::none();
        }
        let generation = self.fade_generation;
        let delay = Duration::from_secs(self.cfg.notifications.fade_after_seconds);
        Task::perform(tokio::time::sleep(delay), move |()| Msg::FadeDue(generation))
    }

    /// Any interaction starts the countdown over (and stops a fade in progress).
    fn restart_fade(&mut self) -> Task<Msg> {
        let restore = self.cancel_fade();
        Task::batch([restore, self.schedule_fade()])
    }

    /// Stops a fade in progress and brings the glass back.
    fn cancel_fade(&mut self) -> Task<Msg> {
        self.fade_generation += 1;
        self.fade = 1.0;
        match (self.fading_since.take(), self.popup) {
            (Some(_), Some(id)) => {
                window::run(id, |w| platform::apply_glass(w, GUTTER, crate::style::RADIUS)).discard()
            }
            _ => Task::none(),
        }
    }

    fn open_conversation(&mut self) -> Task<Msg> {
        self.unread.clear();
        self.error = None;
        self.user_sized = false;
        let restore = self.cancel_fade();
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
        Task::batch([restore, self.show(Mode::Conversation, true), fetch])
    }

    fn open_settings(&mut self) -> Task<Msg> {
        if let Some(id) = self.settings {
            return window::gain_focus(id);
        }
        self.launch_at_sign_in = platform::launches_at_sign_in();
        self.settings_error = None;
        let (id, open) = window::open(window::Settings {
            size: Size::new(400.0, 450.0),
            position: window::Position::Centered,
            resizable: false,
            minimizable: false,
            exit_on_close_request: false,
            icon: window::icon::from_rgba(tray::icon_rgba(64), 64, 64).ok(),
            ..Default::default()
        });
        self.settings = Some(id);
        open.map(Msg::SettingsOpened)
    }

    fn save_settings(&mut self) {
        self.settings_error = self.cfg.save().err().map(|e| format!("Couldn't save settings: {e}"));
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
        self.error = None;
        let (client, channel) = (session.client.clone(), session.channel.clone());
        Task::perform(async move { client.post(&channel, &text).await }, Msg::Sent)
    }

    /// Posts the edited checklist in message `ts` back to the channel.
    fn send_checklist(&mut self, ts: String) -> Task<Msg> {
        let Some((list, true)) = self.find(&ts).and_then(|m| self.checklist(m)) else {
            return Task::none();
        };
        if self.sending {
            return Task::none();
        }
        let Some(session) = &self.session else {
            self.error = Some("Not connected to Slack yet.".into());
            return self.resize();
        };
        self.sending = true;
        self.error = None;
        let (client, channel, text) = (session.client.clone(), session.channel.clone(), list.to_slack());
        Task::perform(async move { client.post(&channel, &text).await }, move |r| Msg::ChecklistSent(ts.clone(), r))
    }

    fn find(&self, ts: &str) -> Option<&Message> {
        self.unread.iter().chain(&self.history).find(|m| m.ts == ts)
    }

    /// The checklist in `message`, if it has one, with unsent ticks applied; and whether there are any.
    fn checklist(&self, message: &Message) -> Option<(Checklist, bool)> {
        let mut list = Checklist::parse(&message.text)?;
        let mut edited = false;
        for (item, &done) in list.items.iter_mut().zip(self.ticks.get(&message.ts).into_iter().flatten()) {
            edited |= item.done != done;
            item.done = done;
        }
        Some((list, edited))
    }

    /// Opens the popup window (or adapts the open one) for `mode`.
    fn show(&mut self, mode: Mode, activate: bool) -> Task<Msg> {
        if self.mode != mode {
            self.user_sized = false;
        }
        self.mode = mode;
        if let Some(id) = self.popup {
            let resize = self.resize();
            return if activate {
                Task::batch([resize, window::gain_focus(id), operation::focus(INPUT)])
            } else {
                resize
            };
        }
        self.activate_on_open = activate;
        self.hovered = false;
        let (id, open) = window::open(window::Settings {
            size: Size::new(WIDTH, self.height()),
            min_size: Some(Size::new(WIDTH, MIN_HEIGHT)),
            max_size: Some(Size::new(WIDTH, 2000.0)),
            position: window::Position::SpecificWith(|win, screen| {
                Point::new(screen.width - win.width - SCREEN_MARGIN, SCREEN_MARGIN)
            }),
            // On Windows a popup is shown later without taking focus; see `show_without_activating`.
            visible: activate || cfg!(not(windows)),
            // Needed for the resize grip; the width is pinned by min/max size.
            resizable: true,
            decorations: false,
            transparent: true,
            level: window::Level::AlwaysOnTop,
            exit_on_close_request: false,
            platform_specific: platform_settings(),
            ..Default::default()
        });
        self.popup = Some(id);
        open.map(Msg::PopupOpened)
    }

    fn resize(&self) -> Task<Msg> {
        match self.popup {
            Some(id) if !self.user_sized => window::resize(id, Size::new(WIDTH, self.height())),
            _ => Task::none(),
        }
    }

    fn hide(&mut self) -> Task<Msg> {
        self.unread.clear();
        self.error = None;
        self.mode = Mode::Popup;
        self.user_sized = false;
        self.hovered = false;
        self.fade_generation += 1;
        self.fading_since = None;
        self.fade = 1.0;
        // Ticks on dismissed popups go; those in the conversation stay until it's refreshed.
        self.ticks.retain(|ts, _| self.history.iter().any(|m| &m.ts == ts));
        match self.popup.take() {
            Some(id) => window::close(id),
            None => Task::none(),
        }
    }

    /// Popup height: just enough for the newest message; older ones are a scroll away.
    fn height(&self) -> f32 {
        match self.mode {
            Mode::Conversation => CONVERSATION_HEIGHT,
            Mode::Popup => {
                let latest = self.unread.last().map(|m| estimate_message_height(&m.text));
                let heights: Vec<f32> =
                    latest.into_iter().chain(self.error.as_deref().map(estimate_text_height)).collect();
                let body = heights.iter().sum::<f32>() + MESSAGE_GAP * heights.len().saturating_sub(1) as f32;
                // Gutters, panel padding, header, input, buttons and the gaps between them.
                let chrome = 2.0 * GUTTER + 32.0 + 20.0 + 40.0 + 32.0 + 3.0 * 12.0;
                (chrome + body.clamp(20.0, MAX_POPUP_BODY)).max(MIN_HEIGHT)
            }
        }
    }

    fn subscription(&self) -> Subscription<Msg> {
        let mut subscriptions = vec![
            Subscription::run(tray::actions).map(Msg::Tray),
            Subscription::run(platform::activations).map(|()| Msg::Activate),
            window::close_requests().map(Msg::Close),
            window::close_events().map(Msg::WindowClosed),
            window::resize_events().map(|(id, _)| Msg::Resized(id)),
            iced::system::theme_changes().map(Msg::ThemeMode),
            event::listen_with(|event, _, id| match event {
                iced::Event::Keyboard(keyboard::Event::KeyPressed {
                    key: keyboard::Key::Named(keyboard::key::Named::Escape),
                    ..
                }) => Some(Msg::Close(id)),
                iced::Event::Keyboard(keyboard::Event::KeyPressed { .. })
                | iced::Event::Mouse(mouse::Event::ButtonPressed(_) | mouse::Event::WheelScrolled { .. }) => {
                    Some(Msg::Interacted(id))
                }
                _ => None,
            }),
        ];
        if self.fading_since.is_some() {
            subscriptions.push(window::frames().map(Msg::FadeFrame));
        }
        if let Some(feed) = &self.feed {
            subscriptions.push(Subscription::run_with(feed.clone(), Feed::stream).map(Msg::Slack));
        }
        Subscription::batch(subscriptions)
    }

    fn view(&self, window: window::Id) -> Element<'_, Msg> {
        if self.settings == Some(window) {
            return self.settings_view();
        }
        let p = Palette::new(self.dark, self.cfg.appearance.transparency, self.fade.clamp(0.0, 1.0));
        let content = match self.mode {
            Mode::Popup => self.popup_view(p),
            Mode::Conversation => self.conversation_view(p),
        };
        let grip = mouse_area(container(container(space()).width(36).height(4).style(move |_| p.grip())).padding(6))
            .on_press(Msg::StartResize)
            .interaction(mouse::Interaction::ResizingVertically);
        let glass = stack![
            container(space()).width(Fill).height(Fill).style(move |_| p.glass()),
            container(space()).width(Fill).height(Fill).style(move |_| p.sheen()),
            container(content).padding(16).width(Fill).height(Fill),
            container(grip).center_x(Fill).align_bottom(Fill),
        ];
        container(mouse_area(glass).on_enter(Msg::Hover(true)).on_exit(Msg::Hover(false))).padding(GUTTER).into()
    }

    fn online(&self) -> Option<bool> {
        match self.status {
            Status::Connecting => None,
            Status::Online => Some(true),
            Status::Offline(_) => Some(false),
        }
    }

    fn popup_view(&self, p: Palette) -> Element<'_, Msg> {
        let (title, time) = match self.unread.last() {
            Some(latest) => ("Assistant", time_label(&latest.ts)),
            None => ("TinyTodo", String::new()),
        };
        // Older unread messages sit above the newest one, a scroll away.
        let count = (self.unread.len() > 1).then(|| text(format!("{} new ↑", self.unread.len())).size(11).color(p.dim));
        let header = row![p.status_dot(self.online()), text(title).size(13).font(SEMIBOLD).color(p.text)]
            .push(text(time).size(12).color(p.dim))
            .push(count)
            .spacing(8)
            .align_y(Center);
        let messages = column(self.unread.iter().map(|m| self.message_body(m, p, p.text, true)))
            .spacing(MESSAGE_GAP)
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

    fn conversation_view(&self, p: Palette) -> Element<'_, Msg> {
        let status = match &self.status {
            Status::Connecting => "Connecting…",
            Status::Online => "Connected",
            Status::Offline(_) => "Reconnecting…",
        };
        let header = row![
            p.status_dot(self.online()),
            text("Conversation").size(13).font(SEMIBOLD).color(p.text),
            text(status).size(12).color(p.dim),
        ]
        .spacing(8)
        .align_y(Center);
        let identity = self.session.as_ref().map(|s| &s.identity);
        let bubbles =
            self.history.iter().filter(|m| policy::is_conversational(m) && !m.text.trim().is_empty()).map(|m| {
                let mine = identity.is_some_and(|i| i.is_mine(m));
                // Our own posts (sent checklists included) are a record, not something to tick.
                let body = |color| self.message_body(m, p, color, !mine);
                // Checklists get the full width so their items don't wrap into a tall column.
                let max_width = if Checklist::parse(&m.text).is_some() { WIDTH } else { WIDTH * 0.75 };
                p.bubble(body, mine, max_width, time_label(&m.ts))
            });
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

    /// A message's text; a checklist becomes checkboxes and, if `interactive`, a send button.
    fn message_body(&self, m: &Message, p: Palette, color: Color, interactive: bool) -> Element<'_, Msg> {
        let Some((list, edited)) = self.checklist(m) else {
            return text(slack::plain_text(&m.text)).size(14).color(color).into();
        };
        let boxes = list.items.into_iter().enumerate().map(|(index, item)| {
            let ts = m.ts.clone();
            checkbox(item.done)
                .label(item.label)
                .size(16)
                .spacing(10)
                .text_size(14)
                .on_toggle_maybe(interactive.then_some(move |done| Msg::Check(ts.clone(), index, done)))
                .style(move |_, status| p.checkbox(status, color))
                .into()
        });
        let send_ready = edited && !self.sending && self.session.is_some();
        let send = interactive.then(|| {
            let label = if self.sending && edited { "Sending…" } else { "Send checklist" };
            p.button(label, send_ready.then(|| Msg::SendChecklist(m.ts.clone())), true)
        });
        let prose = |t: String| (!t.is_empty()).then(|| text(t).size(14).color(color));
        let list =
            column![].push(prose(list.before)).push(column(boxes).spacing(8)).push(prose(list.after)).spacing(10);
        // Right-aligned by the column rather than a spacer, which would stretch a bubble to fill its row.
        column![list].push(send).spacing(10).align_x(iced::Alignment::End).into()
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

    fn settings_view(&self) -> Element<'_, Msg> {
        let p = Palette::new(self.dark, 0.0, 1.0);
        let n = &self.cfg.notifications;
        let section = |title: &'static str, body: Element<'static, Msg>| {
            column![
                text(title).size(12).font(SEMIBOLD).color(p.dim),
                container(body).padding(14).width(Fill).style(move |_| p.card()),
            ]
            .spacing(6)
        };
        let labelled = |label: &'static str, value: String| {
            row![text(label).size(14), space::horizontal(), text(value).size(13).color(p.dim)]
        };
        let fade = match n.fade_after_seconds {
            0 => "Never".to_string(),
            secs => format!("{secs} s"),
        };
        let notifications = column![
            toggler(n.enabled).label("Show popups for new messages").text_size(14).on_toggle(Msg::SetEnabled),
            toggler(n.sound).label("Play a sound").text_size(14).on_toggle(Msg::SetSound),
            labelled("Fade out after", fade),
            slider(0.0..=config::MAX_FADE_SECONDS as f32, n.fade_after_seconds as f32, Msg::SetFadeSeconds)
                .step(1.0_f32)
                .on_release(Msg::SaveSettings),
        ]
        .spacing(12);
        let transparency = self.cfg.appearance.transparency;
        let appearance = column![
            labelled("Transparency", format!("{:.0}%", transparency * 100.0)),
            slider(0.0..=config::MAX_TRANSPARENCY, transparency, Msg::SetTransparency)
                .step(0.01_f32)
                .on_release(Msg::SaveSettings),
        ]
        .spacing(12);
        let system = toggler(self.launch_at_sign_in)
            .label("Launch TinyTodo when I sign in")
            .text_size(14)
            .on_toggle(Msg::SetLaunchAtSignIn);
        let actions = row![
            p.button("Show test notification", Some(Msg::TestNotification), false),
            space::horizontal(),
            p.button("Done", self.settings.map(Msg::Close), true),
        ];
        let body = column![
            section("NOTIFICATIONS", notifications.into()),
            section("APPEARANCE", appearance.into()),
            section("SYSTEM", system.into()),
            space::vertical(),
        ]
        .push(self.settings_error.as_ref().map(|e| text(e).size(12).color(p.error)))
        .push(actions)
        .spacing(16);
        container(body).padding(20).width(Fill).height(Fill).style(move |_| p.window_background()).into()
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

/// Rough wrapped height of message text at size 14 in the popup's width.
fn estimate_text_height(text: &str) -> f32 {
    wrapped_height(text, 46)
}

fn wrapped_height(text: &str, chars_per_line: usize) -> f32 {
    let lines: usize = text.lines().map(|l| l.chars().count().div_ceil(chars_per_line).max(1)).sum();
    lines.max(1) as f32 * 20.0
}

/// Like `estimate_text_height`, but a checklist is a column of checkboxes plus its send button.
fn estimate_message_height(text: &str) -> f32 {
    let Some(list) = Checklist::parse(text) else {
        return estimate_text_height(&slack::plain_text(text));
    };
    // Labels sit beside the box, so they wrap a little sooner.
    let boxes = list.items.iter().map(|i| wrapped_height(&i.label, 41) + 8.0).sum::<f32>() - 8.0;
    let prose: f32 =
        [list.before, list.after].iter().filter(|t| !t.is_empty()).map(|t| estimate_text_height(t) + 10.0).sum();
    boxes + prose + 10.0 + 32.0
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

#[cfg(test)]
mod tests {
    use super::*;

    fn message(ts: u32, text: &str) -> Message {
        Message {
            ts: format!("{ts}.000000"),
            user: None,
            bot_id: None,
            text: text.into(),
            subtype: None,
            hidden: false,
        }
    }

    #[test]
    fn keeps_only_the_newest_ten_messages() {
        let mut list = Vec::new();
        for ts in (1..=15).rev() {
            keep_recent(&mut list, message(ts, &ts.to_string()));
        }
        keep_recent(&mut list, message(15, "15"));
        let kept: Vec<_> = list.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(kept, ["6", "7", "8", "9", "10", "11", "12", "13", "14", "15"]);
    }

    #[test]
    fn text_height_grows_with_wrapping_and_newlines() {
        assert_eq!(estimate_text_height(""), 20.0);
        assert_eq!(estimate_text_height("short"), 20.0);
        assert_eq!(estimate_text_height(&"x".repeat(47)), 40.0);
        assert_eq!(estimate_text_height("a\nb\nc"), 60.0);
    }

    #[test]
    fn time_label_handles_bad_input() {
        assert_eq!(time_label("garbage"), time_label("0"));
        assert!(!time_label("1792534645.644179").is_empty());
    }
}
