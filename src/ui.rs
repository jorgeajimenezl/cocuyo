use eframe::egui::{
    self, Align2, Button, CentralPanel, FontId, Id, PointerButton, RichText, Sense, UiBuilder, vec2,
};

/// Renders a custom window frame with title bar and returns the content area rect.
pub fn custom_window_frame(
    ctx: &egui::Context,
    title: &str,
    screen_window_open: &mut bool,
    info_window_open: &mut bool,
    add_contents: impl FnOnce(&mut egui::Ui),
) -> egui::Rect {
    let panel_frame = egui::Frame::new()
        .fill(ctx.style().visuals.window_fill())
        .corner_radius(10)
        .stroke(ctx.style().visuals.widgets.noninteractive.fg_stroke)
        .outer_margin(1);

    let mut content_rect = egui::Rect::NOTHING;

    CentralPanel::default().frame(panel_frame).show(ctx, |ui| {
        let app_rect = ui.max_rect();

        let title_bar_height = 32.0;
        let title_bar_rect = {
            let mut rect = app_rect;
            rect.max.y = rect.min.y + title_bar_height;
            rect
        };
        title_bar_ui(
            ui,
            title_bar_rect,
            title,
            screen_window_open,
            info_window_open,
        );

        let inner_rect = {
            let mut rect = app_rect;
            rect.min.y = title_bar_rect.max.y;
            rect
        }
        .shrink(4.0);

        content_rect = inner_rect;
        let mut content_ui = ui.new_child(UiBuilder::new().max_rect(inner_rect));
        add_contents(&mut content_ui);
    });

    content_rect
}

fn title_bar_ui(
    ui: &mut egui::Ui,
    title_bar_rect: egui::Rect,
    title: &str,
    screen_window_open: &mut bool,
    info_window_open: &mut bool,
) {
    let painter = ui.painter();

    painter.text(
        title_bar_rect.center(),
        Align2::CENTER_CENTER,
        title,
        FontId::proportional(20.0),
        ui.style().visuals.text_color(),
    );

    painter.line_segment(
        [
            title_bar_rect.left_bottom() + vec2(1.0, 0.0),
            title_bar_rect.right_bottom() + vec2(-1.0, 0.0),
        ],
        ui.visuals().widgets.noninteractive.bg_stroke,
    );

    let title_bar_response = ui.interact(
        title_bar_rect,
        Id::new("title_bar_drag"),
        Sense::click_and_drag(),
    );

    if title_bar_response.double_clicked() {
        let is_maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::Maximized(!is_maximized));
    }

    if title_bar_response.drag_started_by(PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }

    ui.scope_builder(
        UiBuilder::new()
            .max_rect(title_bar_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.add_space(8.0);
            ui.menu_button("View", |ui| {
                if ui.button("Screen Preview").clicked() {
                    *screen_window_open = true;
                    ui.close();
                }
                if ui.button("Stream Information").clicked() {
                    *info_window_open = true;
                    ui.close();
                }
            });
        },
    );

    ui.scope_builder(
        UiBuilder::new()
            .max_rect(title_bar_rect)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.visuals_mut().button_frame = false;
            ui.add_space(8.0);
            window_controls(ui);
        },
    );
}

fn window_controls(ui: &mut egui::Ui) {
    let button_height = 24.0;

    if ui
        .add(Button::new(RichText::new("❌").size(button_height)))
        .on_hover_text("Close the window")
        .clicked()
    {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
    }

    let is_maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
    if ui
        .add(Button::new(RichText::new("🗗").size(button_height)))
        .on_hover_text(if is_maximized {
            "Restore window"
        } else {
            "Maximize window"
        })
        .clicked()
    {
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::Maximized(!is_maximized));
    }

    if ui
        .add(Button::new(RichText::new("🗕").size(button_height)))
        .on_hover_text("Minimize the window")
        .clicked()
    {
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
    }
}
