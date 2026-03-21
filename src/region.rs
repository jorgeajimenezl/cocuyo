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

/// Precomputed aspect-ratio-correct layout (ContentFit::Contain).
pub struct ContainLayout {
    pub scale_x: f32,
    pub scale_y: f32,
    pub rendered_w: f32,
    pub rendered_h: f32,
    pub offset_x: f32,
    pub offset_y: f32,
}

impl ContainLayout {
    pub fn compute(frame_w: u32, frame_h: u32, bounds: Rectangle) -> Self {
        if frame_w == 0 || frame_h == 0 || bounds.width <= 0.0 || bounds.height <= 0.0 {
            return Self {
                scale_x: 1.0,
                scale_y: 1.0,
                rendered_w: 0.0,
                rendered_h: 0.0,
                offset_x: 0.0,
                offset_y: 0.0,
            };
        }

        let frame_aspect = frame_w as f32 / frame_h as f32;
        let bounds_aspect = bounds.width / bounds.height;

        let (scale_x, scale_y) = if frame_aspect > bounds_aspect {
            (1.0, bounds_aspect / frame_aspect)
        } else {
            (frame_aspect / bounds_aspect, 1.0)
        };

        let rendered_w = bounds.width * scale_x;
        let rendered_h = bounds.height * scale_y;
        let offset_x = (bounds.width - rendered_w) * 0.5;
        let offset_y = (bounds.height - rendered_h) * 0.5;

        Self {
            scale_x,
            scale_y,
            rendered_w,
            rendered_h,
            offset_x,
            offset_y,
        }
    }
}

/// Convert widget-space point to frame-space coordinates.
pub fn widget_to_frame(
    widget_x: f32,
    widget_y: f32,
    widget_bounds: Rectangle,
    frame_w: u32,
    frame_h: u32,
) -> Option<(f32, f32)> {
    let layout = ContainLayout::compute(frame_w, frame_h, widget_bounds);

    let local_x = widget_x - layout.offset_x;
    let local_y = widget_y - layout.offset_y;

    if local_x < 0.0 || local_y < 0.0 || local_x > layout.rendered_w || local_y > layout.rendered_h
    {
        return None;
    }

    let frame_x = (local_x / layout.rendered_w) * frame_w as f32;
    let frame_y = (local_y / layout.rendered_h) * frame_h as f32;

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
    let layout = ContainLayout::compute(frame_w, frame_h, widget_bounds);

    let local_x = widget_x - layout.offset_x;
    let local_y = widget_y - layout.offset_y;

    let frame_x = (local_x / layout.rendered_w) * frame_w as f32;
    let frame_y = (local_y / layout.rendered_h) * frame_h as f32;

    (frame_x, frame_y)
}

/// Convert a frame-space region to widget-space rectangle for drawing.
pub fn frame_to_widget(
    region: &Region,
    widget_bounds: Rectangle,
    frame_w: u32,
    frame_h: u32,
) -> Rectangle {
    let layout = ContainLayout::compute(frame_w, frame_h, widget_bounds);

    let x = layout.offset_x + (region.x / frame_w as f32) * layout.rendered_w;
    let y = layout.offset_y + (region.y / frame_h as f32) * layout.rendered_h;
    let w = (region.width / frame_w as f32) * layout.rendered_w;
    let h = (region.height / frame_h as f32) * layout.rendered_h;

    Rectangle::new(iced::Point::new(x, y), iced::Size::new(w, h))
}
