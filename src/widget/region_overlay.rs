use std::cell::Cell;
use std::hash::{Hash, Hasher};

use iced::mouse;
use iced::widget::canvas;
use iced::widget::canvas::{Action, Cache, Canvas, Event, Geometry, Path, Stroke, Text};
use iced::{Color, Point, Rectangle, Size, Theme};

use crate::app::Message;
use crate::region::{self, Region};

const HANDLE_SIZE: f32 = 8.0;
const MIN_REGION_SIZE: f32 = 10.0;

/// Region geometry update: (x, y, width, height)
#[derive(Debug, Clone)]
pub enum RegionMessage {
    Updated(usize, f32, f32, f32, f32),
    Selected(Option<usize>),
}

#[derive(Debug, Clone)]
enum Interaction {
    None,
    Dragging { region_id: usize, offset: Point },
    Resizing { region_id: usize, handle: Handle },
}

#[derive(Debug, Clone, Copy)]
enum Handle {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

pub struct OverlayState {
    interaction: Interaction,
    cache: Cache,
    /// Fingerprint of the last drawn region data; used to invalidate cache on
    /// external state changes (region add/remove, selection sync, etc.).
    last_fingerprint: Cell<u64>,
}

impl Default for OverlayState {
    fn default() -> Self {
        Self {
            interaction: Interaction::None,
            cache: Cache::new(),
            last_fingerprint: Cell::new(0),
        }
    }
}

/// Compute a cheap fingerprint over the fields that affect drawing.
fn region_fingerprint(regions: &[Region], selected: Option<usize>) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    regions.len().hash(&mut h);
    selected.hash(&mut h);
    for r in regions {
        r.id.hash(&mut h);
        r.x.to_bits().hash(&mut h);
        r.y.to_bits().hash(&mut h);
        r.width.to_bits().hash(&mut h);
        r.height.to_bits().hash(&mut h);
        r.bulb_mac.hash(&mut h);
    }
    h.finish()
}

pub struct RegionOverlay<'a> {
    regions: &'a [Region],
    frame_width: u32,
    frame_height: u32,
    selected_region: Option<usize>,
}

impl<'a> RegionOverlay<'a> {
    pub fn new(
        regions: &'a [Region],
        frame_width: u32,
        frame_height: u32,
        selected_region: Option<usize>,
    ) -> Self {
        Self {
            regions,
            frame_width,
            frame_height,
            selected_region,
        }
    }

    pub fn view(self) -> Canvas<Self, Message, Theme> {
        Canvas::new(self).width(iced::Fill).height(iced::Fill)
    }
}

fn corner_handles(rect: Rectangle) -> [(Handle, Rectangle); 4] {
    let hs = HANDLE_SIZE;
    let half = hs / 2.0;
    [
        (
            Handle::TopLeft,
            Rectangle::new(Point::new(rect.x - half, rect.y - half), Size::new(hs, hs)),
        ),
        (
            Handle::TopRight,
            Rectangle::new(
                Point::new(rect.x + rect.width - half, rect.y - half),
                Size::new(hs, hs),
            ),
        ),
        (
            Handle::BottomLeft,
            Rectangle::new(
                Point::new(rect.x - half, rect.y + rect.height - half),
                Size::new(hs, hs),
            ),
        ),
        (
            Handle::BottomRight,
            Rectangle::new(
                Point::new(rect.x + rect.width - half, rect.y + rect.height - half),
                Size::new(hs, hs),
            ),
        ),
    ]
}

fn hit_test_handle(rect: Rectangle, pos: Point) -> Option<Handle> {
    for (handle, hr) in corner_handles(rect) {
        if hr.contains(pos) {
            return Some(handle);
        }
    }
    None
}

impl canvas::Program<Message, Theme> for RegionOverlay<'_> {
    type State = OverlayState;

    fn update(
        &self,
        state: &mut Self::State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<Action<Message>> {
        // Handle ButtonReleased regardless of cursor position to avoid
        // interaction getting stuck in Dragging/Resizing when the cursor
        // leaves the widget bounds.
        if let Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) = event {
            let interaction = std::mem::replace(&mut state.interaction, Interaction::None);

            return match interaction {
                Interaction::Dragging { .. } | Interaction::Resizing { .. } => {
                    state.cache.clear();
                    Some(Action::capture())
                }
                Interaction::None => None,
            };
        }

        let Some(pos) = cursor.position_in(bounds) else {
            return None;
        };

        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                // Check if clicking on a handle of the selected region
                if let Some(sel_id) = self.selected_region {
                    if let Some(region) = self.regions.iter().find(|r| r.id == sel_id) {
                        let wrect = region::frame_to_widget(
                            region,
                            bounds,
                            self.frame_width,
                            self.frame_height,
                        );
                        if let Some(handle) = hit_test_handle(wrect, pos) {
                            state.interaction = Interaction::Resizing {
                                region_id: sel_id,
                                handle,
                            };
                            state.cache.clear();
                            return Some(Action::capture());
                        }
                    }
                }

                // Check if clicking inside any region (drag or select)
                for region in self.regions.iter().rev() {
                    let wrect = region::frame_to_widget(
                        region,
                        bounds,
                        self.frame_width,
                        self.frame_height,
                    );
                    if wrect.contains(pos) {
                        let offset = Point::new(pos.x - wrect.x, pos.y - wrect.y);
                        state.interaction = Interaction::Dragging {
                            region_id: region.id,
                            offset,
                        };
                        state.cache.clear();
                        return Some(
                            Action::publish(Message::RegionUpdate(RegionMessage::Selected(Some(
                                region.id,
                            ))))
                            .and_capture(),
                        );
                    }
                }

                // Click on empty area deselects
                Some(
                    Action::publish(Message::RegionUpdate(RegionMessage::Selected(None)))
                        .and_capture(),
                )
            }

            Event::Mouse(mouse::Event::CursorMoved { .. }) => match &state.interaction {
                Interaction::Dragging { region_id, offset } => {
                    let region_id = *region_id;
                    let offset = *offset;
                    state.cache.clear();

                    let Some(region) = self.regions.iter().find(|r| r.id == region_id) else {
                        return Some(Action::capture());
                    };

                    let new_x = pos.x - offset.x;
                    let new_y = pos.y - offset.y;

                    let (fx, fy) = region::widget_to_frame_unclamped(
                        new_x,
                        new_y,
                        bounds,
                        self.frame_width,
                        self.frame_height,
                    );

                    let new_x = fx
                        .max(0.0)
                        .min((self.frame_width as f32 - region.width).max(0.0));
                    let new_y = fy
                        .max(0.0)
                        .min((self.frame_height as f32 - region.height).max(0.0));

                    Some(
                        Action::publish(Message::RegionUpdate(RegionMessage::Updated(
                            region_id,
                            new_x,
                            new_y,
                            region.width,
                            region.height,
                        )))
                        .and_capture(),
                    )
                }
                Interaction::Resizing { region_id, handle } => {
                    let region_id = *region_id;
                    let handle = *handle;
                    state.cache.clear();

                    let Some(region) = self.regions.iter().find(|r| r.id == region_id) else {
                        return Some(Action::capture());
                    };

                    let wrect = region::frame_to_widget(
                        region,
                        bounds,
                        self.frame_width,
                        self.frame_height,
                    );

                    let (new_x, new_y, new_w, new_h) = match handle {
                        Handle::TopLeft => {
                            let nx = pos.x.min(wrect.x + wrect.width - MIN_REGION_SIZE);
                            let ny = pos.y.min(wrect.y + wrect.height - MIN_REGION_SIZE);
                            (
                                nx,
                                ny,
                                wrect.x + wrect.width - nx,
                                wrect.y + wrect.height - ny,
                            )
                        }
                        Handle::TopRight => {
                            let nw = (pos.x - wrect.x).max(MIN_REGION_SIZE);
                            let ny = pos.y.min(wrect.y + wrect.height - MIN_REGION_SIZE);
                            (wrect.x, ny, nw, wrect.y + wrect.height - ny)
                        }
                        Handle::BottomLeft => {
                            let nx = pos.x.min(wrect.x + wrect.width - MIN_REGION_SIZE);
                            let nh = (pos.y - wrect.y).max(MIN_REGION_SIZE);
                            (nx, wrect.y, wrect.x + wrect.width - nx, nh)
                        }
                        Handle::BottomRight => {
                            let nw = (pos.x - wrect.x).max(MIN_REGION_SIZE);
                            let nh = (pos.y - wrect.y).max(MIN_REGION_SIZE);
                            (wrect.x, wrect.y, nw, nh)
                        }
                    };

                    let Some((fx0, fy0)) = region::widget_to_frame(
                        new_x,
                        new_y,
                        bounds,
                        self.frame_width,
                        self.frame_height,
                    ) else {
                        return Some(Action::capture());
                    };
                    let Some((fx1, fy1)) = region::widget_to_frame(
                        new_x + new_w,
                        new_y + new_h,
                        bounds,
                        self.frame_width,
                        self.frame_height,
                    ) else {
                        return Some(Action::capture());
                    };

                    Some(
                        Action::publish(Message::RegionUpdate(RegionMessage::Updated(
                            region_id,
                            fx0.max(0.0),
                            fy0.max(0.0),
                            (fx1 - fx0).max(1.0),
                            (fy1 - fy0).max(1.0),
                        )))
                        .and_capture(),
                    )
                }
                Interaction::None => None,
            },

            _ => None,
        }
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let _ = cursor;

        // Invalidate cache when region data changes externally (add/remove,
        // selection sync, etc.) — not only on mouse events.
        let fp = region_fingerprint(self.regions, self.selected_region);
        if fp != state.last_fingerprint.get() {
            state.cache.clear();
            state.last_fingerprint.set(fp);
        }

        let geom = state.cache.draw(renderer, bounds.size(), |frame| {
            let region_colors = [
                Color::from_rgba8(246, 182, 201, 0.3),
                Color::from_rgba8(177, 182, 149, 0.3),
                Color::from_rgba8(255, 160, 122, 0.3),
                Color::from_rgba8(254, 205, 178, 0.3),
            ];
            let border_colors = [
                Color::from_rgb8(246, 182, 201),
                Color::from_rgb8(177, 182, 149),
                Color::from_rgb8(255, 160, 122),
                Color::from_rgb8(254, 205, 178),
            ];

            for (i, region) in self.regions.iter().enumerate() {
                let wrect =
                    region::frame_to_widget(region, bounds, self.frame_width, self.frame_height);

                let fill_color = region_colors[i % region_colors.len()];
                let border_color = border_colors[i % border_colors.len()];

                let is_selected = self.selected_region == Some(region.id);
                let stroke_width = if is_selected { 2.5 } else { 1.5 };

                // Fill
                frame.fill_rectangle(
                    Point::new(wrect.x, wrect.y),
                    Size::new(wrect.width, wrect.height),
                    fill_color,
                );

                // Border
                frame.stroke(
                    &Path::rectangle(
                        Point::new(wrect.x, wrect.y),
                        Size::new(wrect.width, wrect.height),
                    ),
                    Stroke::default()
                        .with_color(border_color)
                        .with_width(stroke_width),
                );

                // Label: show bulb name or MAC suffix
                let label = format!(
                    "R{} ({})",
                    i + 1,
                    &region.bulb_mac[region.bulb_mac.len().saturating_sub(5)..]
                );

                frame.fill_text(Text {
                    content: label,
                    position: Point::new(wrect.x + 4.0, wrect.y + 2.0),
                    color: Color::WHITE,
                    size: 12.0.into(),
                    ..Text::default()
                });

                // Corner handles for selected region
                if is_selected {
                    for (_handle, hr) in corner_handles(wrect) {
                        frame.fill_rectangle(
                            Point::new(hr.x, hr.y),
                            Size::new(hr.width, hr.height),
                            Color::WHITE,
                        );
                        frame.stroke(
                            &Path::rectangle(
                                Point::new(hr.x, hr.y),
                                Size::new(hr.width, hr.height),
                            ),
                            Stroke::default().with_color(border_color).with_width(1.0),
                        );
                    }
                }
            }
        });

        vec![geom]
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        let Some(pos) = cursor.position_in(bounds) else {
            return mouse::Interaction::default();
        };

        match &state.interaction {
            Interaction::Dragging { .. } => mouse::Interaction::Grabbing,
            Interaction::Resizing { handle, .. } => match handle {
                Handle::TopLeft | Handle::BottomRight => mouse::Interaction::ResizingDiagonallyDown,
                Handle::TopRight | Handle::BottomLeft => mouse::Interaction::ResizingDiagonallyUp,
            },
            Interaction::None => {
                // Check handles of selected region
                if let Some(sel_id) = self.selected_region {
                    if let Some(region) = self.regions.iter().find(|r| r.id == sel_id) {
                        let wrect = region::frame_to_widget(
                            region,
                            bounds,
                            self.frame_width,
                            self.frame_height,
                        );
                        if let Some(handle) = hit_test_handle(wrect, pos) {
                            return match handle {
                                Handle::TopLeft | Handle::BottomRight => {
                                    mouse::Interaction::ResizingDiagonallyDown
                                }
                                Handle::TopRight | Handle::BottomLeft => {
                                    mouse::Interaction::ResizingDiagonallyUp
                                }
                            };
                        }
                    }
                }

                // Check if hovering any region
                for region in self.regions.iter().rev() {
                    let wrect = region::frame_to_widget(
                        region,
                        bounds,
                        self.frame_width,
                        self.frame_height,
                    );
                    if wrect.contains(pos) {
                        return mouse::Interaction::Grab;
                    }
                }

                mouse::Interaction::default()
            }
        }
    }
}
