use iced::widget::{button, container, pick_list, rule};
use iced::{color, Background, Border, Color, Font, Shadow, Theme};

// ── Heading font ─────────────────────────────────────────────

pub const HEADING_FONT: Font = Font::with_name("Geist Pixel Circle");

// ── Color palette ────────────────────────────────────────────

pub const BG: Color = color!(0x2b292d);
pub const BG_SECONDARY: Color = color!(0x242226);
pub const TEXT: Color = color!(0xfecdb2);
pub const TEXT_DIM: Color = color!(0xab8a79);
pub const ACCENT: Color = color!(0xf6b6c9);
pub const ACCENT_DIM: Color = color!(0x7d6e76);
pub const BORDER: Color = color!(0x4f474d);
pub const DANGER: Color = color!(0xe06b75);
pub const WARNING: Color = color!(0xffa07a);
pub const SUCCESS: Color = color!(0xb1b695);

// ── Theme constructor ────────────────────────────────────────

pub fn create_theme() -> Theme {
    Theme::custom(
        "Cocuyo".to_string(),
        iced::theme::Palette {
            background: BG,
            text: TEXT,
            primary: ACCENT,
            success: SUCCESS,
            warning: WARNING,
            danger: DANGER,
        },
    )
}

// ── Rounded border helper ────────────────────────────────────

fn rounded_border(color: Color, radius: f32) -> Border {
    Border {
        radius: radius.into(),
        width: 1.0,
        color,
    }
}

// ── Button ───────────────────────────────────────────────────

pub fn styled_button(_theme: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Active => button::Style {
            background: Some(Background::Color(BG_SECONDARY)),
            text_color: TEXT,
            border: rounded_border(BORDER, 6.0),
            shadow: Shadow::default(),
            snap: false,
        },
        button::Status::Hovered => button::Style {
            background: Some(Background::Color(ACCENT)),
            text_color: BG,
            border: rounded_border(ACCENT, 6.0),
            shadow: Shadow::default(),
            snap: false,
        },
        button::Status::Pressed => button::Style {
            background: Some(Background::Color(ACCENT_DIM)),
            text_color: BG,
            border: rounded_border(ACCENT, 6.0),
            shadow: Shadow::default(),
            snap: false,
        },
        button::Status::Disabled => button::Style {
            background: Some(Background::Color(BG_SECONDARY)),
            text_color: ACCENT_DIM,
            border: rounded_border(BORDER, 6.0),
            shadow: Shadow::default(),
            snap: false,
        },
    }
}

pub fn close_button(_theme: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Active => button::Style {
            background: Some(Background::Color(BG_SECONDARY)),
            text_color: TEXT,
            border: rounded_border(BORDER, 6.0),
            shadow: Shadow::default(),
            snap: false,
        },
        button::Status::Hovered => button::Style {
            background: Some(Background::Color(DANGER)),
            text_color: BG,
            border: rounded_border(DANGER, 6.0),
            shadow: Shadow::default(),
            snap: false,
        },
        button::Status::Pressed => button::Style {
            background: Some(Background::Color(color!(0xaa4444))),
            text_color: BG,
            border: rounded_border(DANGER, 6.0),
            shadow: Shadow::default(),
            snap: false,
        },
        button::Status::Disabled => button::Style {
            background: Some(Background::Color(BG_SECONDARY)),
            text_color: ACCENT_DIM,
            border: rounded_border(BORDER, 6.0),
            shadow: Shadow::default(),
            snap: false,
        },
    }
}

pub fn title_bar_container(_theme: &Theme) -> container::Style {
    container::Style {
        text_color: Some(TEXT),
        background: Some(Background::Color(BG_SECONDARY)),
        border: Border::default(),
        shadow: Shadow::default(),
        snap: false,
    }
}

pub fn window_border_container(_theme: &Theme) -> container::Style {
    container::Style {
        text_color: Some(TEXT),
        background: Some(Background::Color(BG)),
        border: rounded_border(BORDER, 8.0),
        shadow: Shadow::default(),
        snap: false,
    }
}

// ── Containers ───────────────────────────────────────────────

pub fn styled_container(_theme: &Theme) -> container::Style {
    container::Style {
        text_color: Some(TEXT),
        background: Some(Background::Color(BG)),
        border: Border::default(),
        shadow: Shadow::default(),
        snap: false,
    }
}

pub fn menu_bar_container(_theme: &Theme) -> container::Style {
    container::Style {
        text_color: Some(TEXT),
        background: Some(Background::Color(BG_SECONDARY)),
        border: Border {
            radius: 0.0.into(),
            width: 0.0,
            color: Color::TRANSPARENT,
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

pub fn status_bar_container(_theme: &Theme) -> container::Style {
    container::Style {
        text_color: Some(TEXT),
        background: Some(Background::Color(BG_SECONDARY)),
        border: Border {
            radius: 0.0.into(),
            width: 0.0,
            color: Color::TRANSPARENT,
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

// ── Rule ─────────────────────────────────────────────────────

pub fn styled_rule(_theme: &Theme) -> rule::Style {
    rule::Style {
        color: BORDER,
        radius: 0.0.into(),
        fill_mode: rule::FillMode::Full,
        snap: false,
    }
}

// ── PickList ─────────────────────────────────────────────────

pub fn styled_pick_list(_theme: &Theme, status: pick_list::Status) -> pick_list::Style {
    let border_color = match status {
        pick_list::Status::Active => BORDER,
        pick_list::Status::Hovered | pick_list::Status::Opened { .. } => ACCENT,
    };

    pick_list::Style {
        text_color: TEXT,
        placeholder_color: ACCENT_DIM,
        handle_color: TEXT,
        background: Background::Color(BG_SECONDARY),
        border: rounded_border(border_color, 6.0),
    }
}
