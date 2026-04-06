use std::sync::LazyLock;

use iced::widget::{button, container, pick_list, rule, text_input, theme};
use iced::{Background, Border, Color, Shadow, Theme, border::Radius, color};

use crate::app::Cocuyo;

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
pub const HUD_BG: Color = Color::from_rgba8(0x24, 0x22, 0x26, 0.75);
pub const HUD_BORDER: Color = Color::from_rgba8(0x4f, 0x47, 0x4d, 0.5);
pub const HUD_TEXT: Color = Color::from_rgba8(0xfe, 0xcd, 0xb2, 0.9);

// ── Theme constructor ────────────────────────────────────────

static THEME: LazyLock<Theme> = LazyLock::new(|| {
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
});

pub fn create_theme() -> Theme {
    THEME.clone()
}

// ── Rounded border helper ────────────────────────────────────

fn rounded_border(color: Color, radius: f32) -> Border {
    Border {
        radius: radius.into(),
        width: 1.0,
        color,
    }
}

// ── App Style ───────────────────────────────────────────────────
pub fn app_style(_state: &Cocuyo, _theme: &Theme) -> theme::Style {
    theme::Style {
        background_color: iced::Color::TRANSPARENT,
        text_color: iced::Color::WHITE,
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
        border: Border {
            radius: Radius::default().top(7.0),
            ..Border::default()
        },
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
        border: Border {
            radius: Radius::default().bottom(7.0),
            ..Border::default()
        },
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
            radius: Radius::default().bottom(7.0),
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

// ── TextInput ───────────────────────────────────────────────

pub fn styled_text_input(_theme: &Theme, status: text_input::Status) -> text_input::Style {
    let border_color = match status {
        text_input::Status::Active => BORDER,
        text_input::Status::Hovered | text_input::Status::Focused { .. } => ACCENT,
        text_input::Status::Disabled => BORDER,
    };

    text_input::Style {
        background: Background::Color(BG_SECONDARY),
        border: rounded_border(border_color, 6.0),
        icon: TEXT_DIM,
        placeholder: TEXT_DIM,
        value: TEXT,
        selection: ACCENT_DIM,
    }
}

// ── Tooltip ─────────────────────────────────────────────────
pub fn styled_tooltip(_theme: &Theme) -> container::Style {
    container::Style {
        text_color: Some(TEXT),
        background: Some(Background::Color(BG_SECONDARY)),
        border: rounded_border(BORDER, 6.0),
        shadow: Shadow::default(),
        snap: false,
    }
}

// ── Picker ──────────────────────────────────────────────────

#[cfg(target_os = "windows")]
pub fn picker_item(_theme: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Active => button::Style {
            background: Some(Background::Color(Color::TRANSPARENT)),
            text_color: TEXT,
            border: rounded_border(Color::TRANSPARENT, 4.0),
            shadow: Shadow::default(),
            snap: false,
        },
        button::Status::Hovered => button::Style {
            background: Some(Background::Color(BG_SECONDARY)),
            text_color: TEXT,
            border: rounded_border(BORDER, 4.0),
            shadow: Shadow::default(),
            snap: false,
        },
        button::Status::Pressed => button::Style {
            background: Some(Background::Color(ACCENT_DIM)),
            text_color: BG,
            border: rounded_border(ACCENT, 4.0),
            shadow: Shadow::default(),
            snap: false,
        },
        button::Status::Disabled => button::Style {
            background: Some(Background::Color(Color::TRANSPARENT)),
            text_color: ACCENT_DIM,
            border: rounded_border(Color::TRANSPARENT, 4.0),
            shadow: Shadow::default(),
            snap: false,
        },
    }
}

#[cfg(target_os = "windows")]
pub fn picker_item_selected(_theme: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Active | button::Status::Hovered | button::Status::Pressed => {
            button::Style {
                background: Some(Background::Color(ACCENT_DIM)),
                text_color: TEXT,
                border: rounded_border(ACCENT, 4.0),
                shadow: Shadow::default(),
                snap: false,
            }
        }
        button::Status::Disabled => button::Style {
            background: Some(Background::Color(ACCENT_DIM)),
            text_color: TEXT,
            border: rounded_border(ACCENT, 4.0),
            shadow: Shadow::default(),
            snap: false,
        },
    }
}

#[cfg(target_os = "windows")]
pub fn picker_tab(_theme: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Active => button::Style {
            background: Some(Background::Color(BG_SECONDARY)),
            text_color: TEXT_DIM,
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

#[cfg(target_os = "windows")]
pub fn picker_tab_active(_theme: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Active | button::Status::Hovered | button::Status::Pressed => {
            button::Style {
                background: Some(Background::Color(ACCENT)),
                text_color: BG,
                border: rounded_border(ACCENT, 6.0),
                shadow: Shadow::default(),
                snap: false,
            }
        }
        button::Status::Disabled => button::Style {
            background: Some(Background::Color(BG_SECONDARY)),
            text_color: ACCENT_DIM,
            border: rounded_border(BORDER, 6.0),
            shadow: Shadow::default(),
            snap: false,
        },
    }
}
