use iced::widget::{button, container, pick_list, rule};
use iced::{color, Background, Border, Color, Shadow, Theme};

// ── Color palette ──────────────────────────────────────────────

pub const BG: Color = color!(0x0a0a0a);
pub const BG_SECONDARY: Color = color!(0x141414);
pub const GREEN: Color = color!(0x00ff41);
pub const GREEN_DIM: Color = color!(0x00aa2a);
pub const GREEN_DARK: Color = color!(0x004d1a);
pub const RED: Color = color!(0xff3333);
pub const YELLOW: Color = color!(0xcccc00);
pub const TEXT_DIM: Color = color!(0x00aa2a);

// ── Theme constructor ──────────────────────────────────────────

pub fn create_theme() -> Theme {
    Theme::custom(
        "Cocuyo".to_string(),
        iced::theme::Palette {
            background: BG,
            text: GREEN,
            primary: GREEN,
            success: GREEN,
            warning: YELLOW,
            danger: RED,
        },
    )
}

// ── Pixel border helper ────────────────────────────────────────

fn pixel_border(color: Color) -> Border {
    Border {
        radius: 0.0.into(),
        width: 1.0,
        color,
    }
}

// ── Button ─────────────────────────────────────────────────────

pub fn pixel_button(_theme: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Active => button::Style {
            background: Some(Background::Color(BG)),
            text_color: GREEN,
            border: pixel_border(GREEN_DIM),
            shadow: Shadow::default(),
            snap: true,
        },
        button::Status::Hovered => button::Style {
            background: Some(Background::Color(GREEN)),
            text_color: BG,
            border: pixel_border(GREEN),
            shadow: Shadow::default(),
            snap: true,
        },
        button::Status::Pressed => button::Style {
            background: Some(Background::Color(GREEN_DIM)),
            text_color: BG,
            border: pixel_border(GREEN),
            shadow: Shadow::default(),
            snap: true,
        },
        button::Status::Disabled => button::Style {
            background: Some(Background::Color(BG)),
            text_color: GREEN_DARK,
            border: pixel_border(GREEN_DARK),
            shadow: Shadow::default(),
            snap: true,
        },
    }
}

// ── Containers ─────────────────────────────────────────────────

pub fn pixel_container(_theme: &Theme) -> container::Style {
    container::Style {
        text_color: Some(GREEN),
        background: Some(Background::Color(BG)),
        border: Border::default(),
        shadow: Shadow::default(),
        snap: true,
    }
}

pub fn menu_bar_container(_theme: &Theme) -> container::Style {
    container::Style {
        text_color: Some(GREEN),
        background: Some(Background::Color(BG_SECONDARY)),
        border: Border {
            radius: 0.0.into(),
            width: 0.0,
            color: Color::TRANSPARENT,
        },
        shadow: Shadow::default(),
        snap: true,
    }
}

pub fn status_bar_container(_theme: &Theme) -> container::Style {
    container::Style {
        text_color: Some(GREEN),
        background: Some(Background::Color(BG_SECONDARY)),
        border: Border {
            radius: 0.0.into(),
            width: 0.0,
            color: Color::TRANSPARENT,
        },
        shadow: Shadow::default(),
        snap: true,
    }
}

// ── Rule ───────────────────────────────────────────────────────

pub fn pixel_rule(_theme: &Theme) -> rule::Style {
    rule::Style {
        color: GREEN_DIM,
        radius: 0.0.into(),
        fill_mode: rule::FillMode::Full,
        snap: true,
    }
}

// ── PickList ───────────────────────────────────────────────────

pub fn pixel_pick_list(_theme: &Theme, status: pick_list::Status) -> pick_list::Style {
    let border_color = match status {
        pick_list::Status::Active => GREEN_DIM,
        pick_list::Status::Hovered | pick_list::Status::Opened { .. } => GREEN,
    };

    pick_list::Style {
        text_color: GREEN,
        placeholder_color: GREEN_DARK,
        handle_color: GREEN,
        background: Background::Color(BG_SECONDARY),
        border: pixel_border(border_color),
    }
}
