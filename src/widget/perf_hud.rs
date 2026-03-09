use std::cell::Cell;

use iced::mouse;
use iced::widget::canvas;
use iced::widget::canvas::{Cache, Canvas, Geometry, Path, Text};
use iced::{Point, Rectangle, Size, Theme};

use crate::app::Message;
use crate::perf_stats::PerfStats;
use crate::theme;

const HUD_PADDING: f32 = 8.0;
const LINE_HEIGHT: f32 = 16.0;
const FONT_SIZE: f32 = 12.0;
/// Approximate monospace character width at FONT_SIZE (12px).
const CHAR_WIDTH: f32 = 7.2;
const HUD_CORNER_RADIUS: f32 = 6.0;
const HUD_MARGIN: f32 = 8.0;

pub struct HudState {
    cache: Cache,
    last_fingerprint: Cell<u64>,
}

impl Default for HudState {
    fn default() -> Self {
        Self {
            cache: Cache::new(),
            last_fingerprint: Cell::new(0),
        }
    }
}

pub struct PerfHud<'a> {
    stats: &'a PerfStats,
}

impl<'a> PerfHud<'a> {
    pub fn new(stats: &'a PerfStats) -> Self {
        Self { stats }
    }

    pub fn view(self) -> Canvas<Self, Message, Theme> {
        Canvas::new(self).width(iced::Fill).height(iced::Fill)
    }
}

impl canvas::Program<Message, Theme> for PerfHud<'_> {
    type State = HudState;

    fn update(
        &self,
        _state: &mut Self::State,
        _event: &canvas::Event,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        // Read-only overlay, no interaction
        None
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let fp = self.stats.fingerprint();
        if fp != state.last_fingerprint.get() {
            state.cache.clear();
            state.last_fingerprint.set(fp);
        }

        let stats = self.stats;

        let geom = state.cache.draw(renderer, bounds.size(), |frame| {
            let mut lines: Vec<String> = Vec::new();

            if stats.has_frame_data() {
                lines.push(format!(
                    "FPS: {:.1}  ({:.1}ms)",
                    stats.effective_fps(),
                    stats.frame_interval_ms()
                ));
            }
            if stats.has_sampling_data() {
                lines.push(format!("Sample: {:.1}ms", stats.sampling_time_ms()));
            }
            if stats.has_light_data() {
                lines.push(format!("Lights: {:.0}ms", stats.light_dispatch_ms()));
            }

            if lines.is_empty() {
                return;
            }

            // Compute HUD dimensions
            // Approximate width: longest line * ~7px per char (monospace)
            let max_chars = lines.iter().map(|l| l.len()).max().unwrap_or(0);
            let hud_width = (max_chars as f32) * CHAR_WIDTH + HUD_PADDING * 2.0;
            let hud_height = (lines.len() as f32) * LINE_HEIGHT + HUD_PADDING * 2.0;

            let hud_x = HUD_MARGIN;
            let hud_y = HUD_MARGIN;

            // Background
            let bg_path = Path::rounded_rectangle(
                Point::new(hud_x, hud_y),
                Size::new(hud_width, hud_height),
                HUD_CORNER_RADIUS.into(),
            );
            frame.fill(&bg_path, theme::HUD_BG);
            frame.stroke(
                &bg_path,
                canvas::Stroke::default()
                    .with_color(theme::HUD_BORDER)
                    .with_width(1.0),
            );

            // Text lines
            for (i, line) in lines.iter().enumerate() {
                let y = hud_y + HUD_PADDING + (i as f32) * LINE_HEIGHT;
                frame.fill_text(Text {
                    content: line.clone(),
                    position: Point::new(hud_x + HUD_PADDING, y),
                    color: theme::HUD_TEXT,
                    size: FONT_SIZE.into(),
                    font: iced::Font::MONOSPACE,
                    ..Text::default()
                });
            }
        });

        vec![geom]
    }
}
