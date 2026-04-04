#[cfg(feature = "tray")]
mod imp {
    use std::pin::Pin;
    use std::sync::OnceLock;

    use futures::Stream;
    use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};

    #[derive(Debug, Clone)]
    pub enum TrayAction {
        ToggleWindow,
        ToggleAmbient,
        Exit,
    }

    /// Menu IDs stored globally so the subscription fn pointer can access them.
    #[derive(Debug)]
    struct TrayIds {
        toggle_window_id: MenuId,
        toggle_ambient_id: MenuId,
        exit_id: MenuId,
    }

    static TRAY_IDS: OnceLock<TrayIds> = OnceLock::new();

    /// Holds the tray icon and menu items. Must stay alive on the main thread.
    /// Leaked as `&'static` so Cocuyo (which is Send) can hold a reference.
    pub struct TrayState {
        _tray_icon: TrayIcon,
        toggle_window_item: MenuItem,
        toggle_ambient_item: MenuItem,
    }

    fn generate_icon() -> Icon {
        let size: u32 = 32;
        let mut rgba = vec![0u8; (size * size * 4) as usize];
        let center = size as f32 / 2.0;
        let radius = center - 1.0;

        for y in 0..size {
            for x in 0..size {
                let dx = x as f32 - center;
                let dy = y as f32 - center;
                let dist = (dx * dx + dy * dy).sqrt();
                let idx = ((y * size + x) * 4) as usize;

                if dist <= radius {
                    // Amber/yellow color for firefly theme
                    rgba[idx] = 255; // R
                    rgba[idx + 1] = 191; // G
                    rgba[idx + 2] = 0; // B
                    rgba[idx + 3] = 255; // A
                }
            }
        }

        Icon::from_rgba(rgba, size, size).expect("Failed to create tray icon")
    }

    /// Creates the tray icon on the main thread and leaks it as `&'static`.
    /// Must be called before `iced::daemon()`.
    pub fn create_tray() -> &'static TrayState {
        let menu = Menu::new();

        let toggle_window_item = MenuItem::new("Hide", true, None);
        let toggle_ambient_item = MenuItem::new("Start Ambient", true, None);
        let exit_item = MenuItem::new("Exit", true, None);

        let toggle_window_id = toggle_window_item.id().clone();
        let toggle_ambient_id = toggle_ambient_item.id().clone();
        let exit_id = exit_item.id().clone();

        TRAY_IDS
            .set(TrayIds {
                toggle_window_id,
                toggle_ambient_id,
                exit_id,
            })
            .expect("create_tray called more than once");

        menu.append_items(&[
            &toggle_window_item,
            &toggle_ambient_item,
            &PredefinedMenuItem::separator(),
            &exit_item,
        ])
        .expect("Failed to build tray menu");

        let icon = generate_icon();

        let tray_icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Cocuyo")
            .with_icon(icon)
            .with_menu_on_left_click(false)
            .build()
            .expect("Failed to create tray icon");

        let state = TrayState {
            _tray_icon: tray_icon,
            toggle_window_item,
            toggle_ambient_item,
        };

        // Leak so it lives for the entire process and we get a &'static ref.
        // The tray icon must remain alive anyway.
        Box::leak(Box::new(state))
    }

    // SAFETY: MenuItem (from muda) uses Arc internally and is safe to send references across threads.
    // TrayIcon is !Send but we only hold it alive via the leaked &'static; we never move it.
    unsafe impl Sync for TrayState {}

    impl TrayState {
        pub fn update_menu_text(&self, main_visible: bool, ambient_active: bool) {
            self.toggle_window_item
                .set_text(if main_visible { "Hide" } else { "Show" });
            self.toggle_ambient_item.set_text(if ambient_active {
                "Stop Ambient"
            } else {
                "Start Ambient"
            });
        }
    }

    /// Subscription builder compatible with `Subscription::run_with((), tray_subscription)`.
    pub fn tray_subscription(_input: &()) -> Pin<Box<dyn Stream<Item = TrayAction> + Send>> {
        let ids = TRAY_IDS.get().expect("TRAY_IDS not initialized");
        let toggle_window_id = ids.toggle_window_id.clone();
        let toggle_ambient_id = ids.toggle_ambient_id.clone();
        let exit_id = ids.exit_id.clone();

        Box::pin(iced::stream::channel(4, async move |mut output| {
            use futures::SinkExt;

            let menu_receiver = MenuEvent::receiver();
            let tray_receiver = TrayIconEvent::receiver();

            loop {
                let mut had_event = false;

                if let Ok(event) = menu_receiver.try_recv() {
                    had_event = true;
                    let action = if event.id == toggle_window_id {
                        Some(TrayAction::ToggleWindow)
                    } else if event.id == toggle_ambient_id {
                        Some(TrayAction::ToggleAmbient)
                    } else if event.id == exit_id {
                        Some(TrayAction::Exit)
                    } else {
                        None
                    };
                    if let Some(action) = action {
                        let _ = output.send(action).await;
                    }
                }

                if let Ok(event) = tray_receiver.try_recv() {
                    if let TrayIconEvent::Click {
                        button: tray_icon::MouseButton::Left,
                        ..
                    } = event
                    {
                        had_event = true;
                        let _ = output.send(TrayAction::ToggleWindow).await;
                    }
                }

                if !had_event {
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
            }
        }))
    }
}

#[cfg(not(feature = "tray"))]
mod imp {
    use std::pin::Pin;

    use futures::Stream;

    #[derive(Debug, Clone)]
    #[allow(dead_code)]
    pub enum TrayAction {
        ToggleWindow,
        ToggleAmbient,
        Exit,
    }

    pub struct TrayState;

    pub fn create_tray() -> &'static TrayState {
        static INSTANCE: TrayState = TrayState;
        &INSTANCE
    }

    impl TrayState {
        pub fn update_menu_text(&self, _main_visible: bool, _ambient_active: bool) {}
    }

    #[allow(dead_code)]
    pub fn tray_subscription(_input: &()) -> Pin<Box<dyn Stream<Item = TrayAction> + Send>> {
        Box::pin(futures::stream::empty())
    }
}

pub use imp::*;
