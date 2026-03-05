use iced::Rectangle;

use crate::sampling::BoxedStrategy;

#[derive(Debug, Clone)]
pub struct Region {
    pub id: usize,
    /// Frame-space coordinates (pixels)
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub bulb_mac: String,
    pub sampled_color: Option<(u8, u8, u8)>,
    pub strategy: BoxedStrategy,
}

/// Convert widget-space point to frame-space coordinates.
///
/// Uses the same aspect-ratio math as video_shader.rs (ContentFit::Contain):
/// the frame is scaled to fit within the widget bounds while preserving aspect ratio.
pub fn widget_to_frame(
    widget_x: f32,
    widget_y: f32,
    widget_bounds: Rectangle,
    frame_w: u32,
    frame_h: u32,
) -> Option<(f32, f32)> {
    let frame_aspect = frame_w as f32 / frame_h as f32;
    let bounds_aspect = widget_bounds.width / widget_bounds.height;

    let (scale_x, scale_y) = if frame_aspect > bounds_aspect {
        (1.0, bounds_aspect / frame_aspect)
    } else {
        (frame_aspect / bounds_aspect, 1.0)
    };

    // The rendered frame area within widget bounds
    let rendered_w = widget_bounds.width * scale_x;
    let rendered_h = widget_bounds.height * scale_y;
    let offset_x = (widget_bounds.width - rendered_w) * 0.5;
    let offset_y = (widget_bounds.height - rendered_h) * 0.5;

    // Convert widget-local position to position within rendered frame area
    let local_x = widget_x - offset_x;
    let local_y = widget_y - offset_y;

    // Check if inside the rendered frame area
    if local_x < 0.0 || local_y < 0.0 || local_x > rendered_w || local_y > rendered_h {
        return None;
    }

    // Map to frame pixel coordinates
    let frame_x = (local_x / rendered_w) * frame_w as f32;
    let frame_y = (local_y / rendered_h) * frame_h as f32;

    Some((frame_x, frame_y))
}

/// Like `widget_to_frame`, but without the bounds check.
/// Useful during dragging where the mouse may leave the frame area
/// but the result will be clamped afterwards.
pub fn widget_to_frame_unclamped(
    widget_x: f32,
    widget_y: f32,
    widget_bounds: Rectangle,
    frame_w: u32,
    frame_h: u32,
) -> (f32, f32) {
    let frame_aspect = frame_w as f32 / frame_h as f32;
    let bounds_aspect = widget_bounds.width / widget_bounds.height;

    let (scale_x, scale_y) = if frame_aspect > bounds_aspect {
        (1.0, bounds_aspect / frame_aspect)
    } else {
        (frame_aspect / bounds_aspect, 1.0)
    };

    let rendered_w = widget_bounds.width * scale_x;
    let rendered_h = widget_bounds.height * scale_y;
    let offset_x = (widget_bounds.width - rendered_w) * 0.5;
    let offset_y = (widget_bounds.height - rendered_h) * 0.5;

    let local_x = widget_x - offset_x;
    let local_y = widget_y - offset_y;

    let frame_x = (local_x / rendered_w) * frame_w as f32;
    let frame_y = (local_y / rendered_h) * frame_h as f32;

    (frame_x, frame_y)
}

/// Convert a frame-space region to widget-space rectangle for drawing.
pub fn frame_to_widget(
    region: &Region,
    widget_bounds: Rectangle,
    frame_w: u32,
    frame_h: u32,
) -> Rectangle {
    let frame_aspect = frame_w as f32 / frame_h as f32;
    let bounds_aspect = widget_bounds.width / widget_bounds.height;

    let (scale_x, scale_y) = if frame_aspect > bounds_aspect {
        (1.0, bounds_aspect / frame_aspect)
    } else {
        (frame_aspect / bounds_aspect, 1.0)
    };

    let rendered_w = widget_bounds.width * scale_x;
    let rendered_h = widget_bounds.height * scale_y;
    let offset_x = (widget_bounds.width - rendered_w) * 0.5;
    let offset_y = (widget_bounds.height - rendered_h) * 0.5;

    let x = offset_x + (region.x / frame_w as f32) * rendered_w;
    let y = offset_y + (region.y / frame_h as f32) * rendered_h;
    let w = (region.width / frame_w as f32) * rendered_w;
    let h = (region.height / frame_h as f32) * rendered_h;

    Rectangle::new(iced::Point::new(x, y), iced::Size::new(w, h))
}
