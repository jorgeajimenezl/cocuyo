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

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rectangle {
        Rectangle::new(iced::Point::new(x, y), iced::Size::new(w, h))
    }

    fn make_region(x: f32, y: f32, w: f32, h: f32) -> Region {
        Region {
            id: 0,
            x,
            y,
            width: w,
            height: h,
            bulb_mac: String::new(),
            sampled_color: None,
            strategy: BoxedStrategy::default(),
        }
    }

    // -- ContainLayout::compute --

    #[test]
    fn wider_frame_letterboxed() {
        // 16:9 frame in a square widget → letterboxed (offset_y > 0, offset_x == 0)
        let layout = ContainLayout::compute(1920, 1080, rect(0.0, 0.0, 800.0, 800.0));
        assert!(layout.offset_x.abs() < 1e-3, "no horizontal offset for wider frame");
        assert!(layout.offset_y > 1.0, "should have vertical offset (letterbox)");
        assert!((layout.rendered_w - 800.0).abs() < 1e-3, "full width used");
        assert!(layout.rendered_h < 800.0, "height smaller than bounds");
    }

    #[test]
    fn taller_frame_pillarboxed() {
        // 9:16 frame in a square widget → pillarboxed (offset_x > 0, offset_y == 0)
        let layout = ContainLayout::compute(1080, 1920, rect(0.0, 0.0, 800.0, 800.0));
        assert!(layout.offset_x > 1.0, "should have horizontal offset (pillarbox)");
        assert!(layout.offset_y.abs() < 1e-3, "no vertical offset for taller frame");
        assert!(layout.rendered_w < 800.0, "width smaller than bounds");
        assert!((layout.rendered_h - 800.0).abs() < 1e-3, "full height used");
    }

    #[test]
    fn zero_frame_returns_zero_rendered() {
        let layout = ContainLayout::compute(0, 1080, rect(0.0, 0.0, 800.0, 600.0));
        assert_eq!(layout.rendered_w, 0.0);
        assert_eq!(layout.rendered_h, 0.0);
    }

    // -- Coordinate transforms --

    #[test]
    fn widget_to_frame_center() {
        let bounds = rect(0.0, 0.0, 800.0, 600.0);
        // 800x600 frame in 800x600 widget → exact fit, no offset
        let result = widget_to_frame(400.0, 300.0, bounds, 800, 600);
        let (fx, fy) = result.expect("center should be inside frame");
        assert!((fx - 400.0).abs() < 1.0);
        assert!((fy - 300.0).abs() < 1.0);
    }

    #[test]
    fn widget_to_frame_in_letterbox_returns_none() {
        // 16:9 frame in square widget → top/bottom are letterbox
        let bounds = rect(0.0, 0.0, 800.0, 800.0);
        let layout = ContainLayout::compute(1920, 1080, bounds);
        // Click in the top letterbox area (y < offset_y)
        let result = widget_to_frame(400.0, layout.offset_y - 5.0, bounds, 1920, 1080);
        assert!(result.is_none(), "point in letterbox should return None");
    }

    #[test]
    fn widget_to_frame_unclamped_allows_out_of_bounds() {
        let bounds = rect(0.0, 0.0, 800.0, 800.0);
        // Point clearly outside the rendered area
        let (fx, fy) = widget_to_frame_unclamped(-100.0, -100.0, bounds, 1920, 1080);
        // Should return values (negative) without returning None
        assert!(fx < 0.0 || fy < 0.0, "unclamped should allow out-of-bounds coords");
    }

    #[test]
    fn frame_to_widget_round_trip() {
        let bounds = rect(0.0, 0.0, 800.0, 600.0);
        let fw: u32 = 1920;
        let fh: u32 = 1080;

        // Region covering center quarter of the frame
        let region = make_region(480.0, 270.0, 960.0, 540.0);
        let widget_rect = frame_to_widget(&region, bounds, fw, fh);

        // Convert the top-left corner of the widget rect back to frame space
        let (fx, fy) = widget_to_frame(
            widget_rect.x,
            widget_rect.y,
            bounds,
            fw,
            fh,
        )
        .expect("widget rect corner should map back to frame");

        assert!((fx - region.x).abs() < 2.0, "x round-trip: got {fx}, expected {}", region.x);
        assert!((fy - region.y).abs() < 2.0, "y round-trip: got {fy}, expected {}", region.y);
    }
}
