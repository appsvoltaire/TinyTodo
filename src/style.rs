//! Colors and widget styles: the glass popup and the plain settings window,
//! each in a light and a dark variant.

use iced::widget::{button, checkbox, column, container, row, scrollable, space, text, text_input};
use iced::{Background, Border, Color, Element, Fill, Font, Radians, Shadow, Theme, Vector, font, gradient};

pub const ACCENT: Color = Color::from_rgb(0.04, 0.52, 1.0);
pub const RADIUS: f32 = 22.0;
pub const SEMIBOLD: Font = Font { weight: font::Weight::Semibold, ..Font::DEFAULT };

/// The iced theme behind stock widgets (sliders, togglers), using our accent.
pub fn theme(dark: bool) -> Theme {
    let base = if dark { iced::theme::Palette::DARK } else { iced::theme::Palette::LIGHT };
    let background = if dark { Color::from_rgb(0.12, 0.12, 0.14) } else { Color::from_rgb(0.97, 0.97, 0.98) };
    Theme::custom("TinyTodo", iced::theme::Palette { primary: ACCENT, background, ..base })
}

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    dark: bool,
    /// 0.0 = solid glass, up to `config::MAX_TRANSPARENCY`.
    transparency: f32,
    /// Overall opacity while fading out, 1.0 normally.
    fade: f32,
    pub text: Color,
    pub dim: Color,
    pub error: Color,
}

impl Palette {
    pub fn new(dark: bool, transparency: f32, fade: f32) -> Self {
        let (text, dim, error) = if dark {
            (
                Color::from_rgb(0.96, 0.96, 0.98),
                Color::from_rgba(0.92, 0.92, 0.96, 0.6),
                Color::from_rgb(1.0, 0.45, 0.42),
            )
        } else {
            (Color::from_rgb(0.08, 0.08, 0.1), Color::from_rgba(0.1, 0.1, 0.14, 0.55), Color::from_rgb(0.8, 0.15, 0.12))
        };
        Self {
            dark,
            transparency,
            fade,
            text: text.scale_alpha(fade),
            dim: dim.scale_alpha(fade),
            error: error.scale_alpha(fade),
        }
    }

    fn faded(&self, color: Color) -> Color {
        color.scale_alpha(self.fade)
    }

    /// A white overlay at `alpha` in dark mode, a black one in light mode.
    fn wash(&self, alpha: f32) -> Color {
        let wash = if self.dark {
            Color::from_rgba(1.0, 1.0, 1.0, alpha)
        } else {
            Color::from_rgba(0.0, 0.0, 0.0, alpha * 0.6)
        };
        self.faded(wash)
    }

    fn rim(&self) -> Color {
        self.faded(if self.dark { Color::from_rgba(1.0, 1.0, 1.0, 0.22) } else { Color::from_rgba(1.0, 1.0, 1.0, 0.9) })
    }

    pub fn glass(&self) -> container::Style {
        let opacity = 1.0 - self.transparency;
        let (top, bottom) = if self.dark {
            (Color::from_rgb(0.24, 0.24, 0.28), Color::from_rgb(0.11, 0.11, 0.14))
        } else {
            (Color::from_rgb(1.0, 1.0, 1.0), Color::from_rgb(0.92, 0.93, 0.96))
        };
        // The bottom is a touch denser, where the buttons sit.
        let top = self.faded(top.scale_alpha(opacity));
        let bottom = self.faded(bottom.scale_alpha((opacity + 0.08).min(1.0)));
        container::Style {
            background: Some(vertical_gradient(top, bottom, 1.0)),
            border: Border { color: self.rim(), width: 1.0, radius: RADIUS.into() },
            shadow: Shadow {
                color: self.faded(Color::from_rgba(0.0, 0.0, 0.0, 0.28)),
                offset: Vector::new(0.0, 3.0),
                blur_radius: 10.0,
            },
            ..Default::default()
        }
    }

    /// Specular highlight across the top of the panel.
    pub fn sheen(&self) -> container::Style {
        let alpha = if self.dark { 0.10 } else { 0.5 };
        container::Style {
            background: Some(vertical_gradient(
                self.faded(Color::from_rgba(1.0, 1.0, 1.0, alpha)),
                Color::TRANSPARENT,
                0.4,
            )),
            border: Border { radius: RADIUS.into(), ..Default::default() },
            ..Default::default()
        }
    }

    pub fn status_dot<'a, M: 'a>(&self, online: Option<bool>) -> Element<'a, M> {
        let color = match online {
            Some(true) => Color::from_rgb(0.2, 0.8, 0.4),
            None => Color::from_rgb(1.0, 0.75, 0.2),
            Some(false) => Color::from_rgb(1.0, 0.35, 0.3),
        };
        let color = self.faded(color);
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

    pub fn button<'a, M: Clone + 'a>(self, label: &'a str, on_press: Option<M>, primary: bool) -> Element<'a, M> {
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
                    background: Some(self.faded(background).into()),
                    text_color: self.faded(if disabled { text_color.scale_alpha(0.7) } else { text_color }),
                    border: Border { color: self.faded(border), width: 1.0, radius: 999.0.into() },
                    ..Default::default()
                }
            })
            .into()
    }

    pub fn input(&self, status: text_input::Status) -> text_input::Style {
        let focused = matches!(status, text_input::Status::Focused { .. });
        text_input::Style {
            background: self.wash(0.08).into(),
            border: Border {
                color: if focused { self.faded(ACCENT.scale_alpha(0.8)) } else { self.rim() },
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
    pub fn scroll<'a, M: 'a>(self, content: impl Into<Element<'a, M>>) -> Element<'a, M> {
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

    /// The small pill at the bottom edge that resizes the popup.
    pub fn grip(&self) -> container::Style {
        container::Style {
            background: Some(self.wash(0.25).into()),
            border: Border { radius: 2.0.into(), ..Default::default() },
            ..Default::default()
        }
    }

    /// A chat bubble; `body` renders the message in the text color it is given.
    pub fn bubble<'a, M: 'a>(
        self,
        body: impl FnOnce(Color) -> Element<'a, M>,
        mine: bool,
        max_width: f32,
        time: String,
    ) -> Element<'a, M> {
        let who = if mine { "You" } else { "Assistant" };
        let (body_color, meta_color) =
            if mine { (Color::WHITE, Color::from_rgba(1.0, 1.0, 1.0, 0.7)) } else { (self.text, self.dim) };
        let body = column![
            body(self.faded(body_color)),
            text(format!("{who} · {time}")).size(11).color(self.faded(meta_color))
        ]
        .spacing(3);
        let bubble = container(body).padding([8, 12]).max_width(max_width).style(move |_| container::Style {
            background: Some(if mine { self.faded(ACCENT.scale_alpha(0.9)).into() } else { self.wash(0.10).into() }),
            border: Border { color: self.rim(), width: if mine { 0.0 } else { 1.0 }, radius: 16.0.into() },
            ..Default::default()
        });
        if mine { row![space::horizontal(), bubble] } else { row![bubble, space::horizontal()] }.into()
    }

    /// A checklist item; ticked ones fade back so what's left stands out.
    pub fn checkbox(&self, status: checkbox::Status, text_color: Color) -> checkbox::Style {
        let (checked, hovered) = match status {
            checkbox::Status::Active { is_checked } | checkbox::Status::Disabled { is_checked } => (is_checked, false),
            checkbox::Status::Hovered { is_checked } => (is_checked, true),
        };
        checkbox::Style {
            background: if checked {
                self.faded(ACCENT).into()
            } else {
                self.wash(if hovered { 0.18 } else { 0.08 }).into()
            },
            icon_color: self.faded(Color::WHITE),
            border: Border {
                color: if checked { self.faded(ACCENT) } else { self.wash(0.4) },
                width: 1.0,
                radius: 5.0.into(),
            },
            text_color: Some(if checked { text_color.scale_alpha(0.55) } else { text_color }),
        }
    }

    /// Solid background for the settings window.
    pub fn window_background(&self) -> container::Style {
        let background = if self.dark { Color::from_rgb(0.12, 0.12, 0.14) } else { Color::from_rgb(0.97, 0.97, 0.98) };
        container::Style { background: Some(background.into()), text_color: Some(self.text), ..Default::default() }
    }

    /// A grouped section card in the settings window.
    pub fn card(&self) -> container::Style {
        container::Style {
            background: Some(self.wash(0.05).into()),
            border: Border { color: self.wash(0.10), width: 1.0, radius: 12.0.into() },
            ..Default::default()
        }
    }
}

fn vertical_gradient(top: Color, bottom: Color, end: f32) -> Background {
    gradient::Linear::new(Radians::PI).add_stop(0.0, top).add_stop(end, bottom).into()
}
