use adapter_platform_macos::presentation;
use port_platform::PlatformError;
use tauri::WebviewWindow;

pub struct MacOsPresentation;

// Tauri's public ContextMenu bound exposes its sealed supertrait's context accessor.
// Keep that dependency-specific access here: Muda's public ns_menu contract borrows
// the pointer for the ContextMenu lifetime. Never recover it from NSStatusItem.menu,
// which tray-icon 0.25 attaches only while a menu is being shown.
// Sources: tauri 2.12.0 src/menu/{mod.rs,menu.rs}; muda 0.20.0 src/context_menu.rs.
fn native_context_menu_ptr<M: tauri::menu::ContextMenu>(menu: &M) -> *mut std::ffi::c_void {
    menu.inner_context().ns_menu()
}

// Wry executes tasks inline when already on the main thread. Elsewhere this waits only
// for handle acquisition and native start, never for an animation callback.
fn on_main_thread<R: tauri::Runtime, T: Send + 'static>(
    window: &WebviewWindow<R>,
    work: impl FnOnce(&WebviewWindow<R>) -> Result<T, PlatformError> + Send + 'static,
) -> Result<T, PlatformError> {
    let owned_window = window.clone();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    window
        .run_on_main_thread(move || {
            let _ = sender.send(work(&owned_window));
        })
        .map_err(|error| {
            PlatformError::Operation(format!("unable to dispatch Preview presentation: {error}"))
        })?;
    receiver
        .recv()
        .map_err(|_| PlatformError::Operation("Preview presentation dispatch was dropped".into()))?
}

impl<R: tauri::Runtime> crate::platform::WindowPresentation<R> for MacOsPresentation {
    fn confirm_destructive_action(
        &self,
        app: &tauri::AppHandle<R>,
        title: &str,
        message: &str,
        confirm_label: &str,
        cancel_label: &str,
    ) -> Result<bool, PlatformError> {
        let (title, message, confirm_label, cancel_label) = (
            title.to_owned(),
            message.to_owned(),
            confirm_label.to_owned(),
            cancel_label.to_owned(),
        );
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        app.run_on_main_thread(move || {
            // SAFETY: Tauri has initialized NSApplication and dispatches to AppKit.
            let result = unsafe {
                presentation::confirm_destructive_action(
                    &title,
                    &message,
                    &confirm_label,
                    &cancel_label,
                )
            };
            let _ = sender.send(result);
        })
        .map_err(|error| {
            PlatformError::Operation(format!("unable to present confirmation: {error}"))
        })?;
        receiver
            .recv()
            .map_err(|_| PlatformError::Operation("confirmation dispatch was dropped".into()))?
    }

    fn format_short_datetime(&self, unix_seconds: f64) -> Result<String, PlatformError> {
        presentation::format_short_datetime(unix_seconds)
    }

    fn menu_presentation(
        &self,
        app: &tauri::AppHandle<R>,
        root: &tauri::menu::Menu<R>,
        history: &tauri::menu::Submenu<R>,
        expected: Vec<port_platform::MenuPresentationItem>,
    ) -> Result<(), PlatformError> {
        let root = root.clone();
        let history = history.clone();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        app.run_on_main_thread(move || {
            let result = (|| {
                let error = |error: tauri::Error| PlatformError::Operation(error.to_string());
                let root_items = root.items().map_err(error)?;
                let index = root_items
                    .iter()
                    .position(|item| item.id() == history.id())
                    .ok_or_else(|| {
                        PlatformError::Operation("target submenu is not attached".into())
                    })?;
                let title = history.text().map_err(error)?;
                // SAFETY: The owned Tauri root keeps the borrowed NSMenu alive through
                // this synchronous main-thread call. No pointer crosses the channel.
                unsafe {
                    presentation::set_menu_presentation(
                        native_context_menu_ptr(&root),
                        index,
                        &title,
                        &expected,
                    )
                }
            })();
            let _ = sender.send(result);
        })
        .map_err(|error| PlatformError::Operation(error.to_string()))?;
        receiver.recv().map_err(|_| {
            PlatformError::Operation("Menu presentation dispatch was dropped".into())
        })?
    }

    fn settings_background(
        &self,
        app: &tauri::AppHandle<R>,
    ) -> Result<tauri::utils::config::Color, PlatformError> {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        app.run_on_main_thread(move || {
            // SAFETY: Tauri has initialized NSApplication and dispatches this closure to AppKit.
            let result = unsafe { presentation::window_background_rgba() }
                .map(|[r, g, b, a]| tauri::utils::config::Color(r, g, b, a));
            let _ = sender.send(result);
        })
        .map_err(|error| {
            PlatformError::Operation(format!("unable to dispatch background resolution: {error}"))
        })?;
        receiver.recv().map_err(|_| {
            PlatformError::Operation("background resolution dispatch was dropped".into())
        })?
    }

    fn control_palette(
        &self,
        app: &tauri::AppHandle<R>,
    ) -> Result<Option<crate::platform::ControlPalette>, PlatformError> {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        app.run_on_main_thread(move || {
            // SAFETY: Tauri initialized NSApplication and dispatches on AppKit's thread.
            let palette = unsafe { presentation::control_palette() }.map(|palette| {
                crate::platform::ControlPalette {
                    colors: palette
                        .colors_available
                        .then_some(crate::platform::ControlColors {
                            control_surface: palette.control_surface,
                            window_surface: palette.window_surface,
                            button_fill: palette.button_fill,
                            button_pressed_fill: palette.button_pressed_fill,
                            separator: palette.separator,
                            primary_button_fill: palette.primary_button_fill,
                            primary_button_foreground: palette.primary_button_foreground,
                        }),
                    increase_contrast: palette.increase_contrast,
                    reduce_transparency: palette.reduce_transparency,
                    window_active: palette.window_active,
                }
            });
            let _ = sender.send(palette);
        })
        .map_err(|error| PlatformError::Operation(error.to_string()))?;
        receiver
            .recv()
            .map_err(|_| PlatformError::Operation("Control palette dispatch was dropped".into()))
    }

    fn observe_control_palette(&self, window: &WebviewWindow<R>) -> Result<(), PlatformError> {
        // with_webview executes on the main thread. Resolve both borrowed handles there;
        // neither pointer leaves this closure or enters a Rust channel.
        let owned_window = window.clone();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        window
            .with_webview(move |webview| {
                let result = owned_window
                    .ns_window()
                    .map_err(|error| PlatformError::Operation(error.to_string()))
                    .and_then(|native_window| {
                        // SAFETY: Live window and its WKWebView are borrowed on AppKit's thread.
                        unsafe {
                            presentation::observe_control_palette(native_window, webview.inner())
                        }
                    });
                let _ = sender.send(result);
            })
            .map_err(|error| PlatformError::Operation(error.to_string()))?;
        receiver.recv().map_err(|_| {
            PlatformError::Operation("Control observation dispatch was dropped".into())
        })?
    }

    fn configure_floating_window_radius(
        &self,
        window: &WebviewWindow<R>,
        radius: f64,
    ) -> Result<(), PlatformError> {
        on_main_thread(window, move |window| {
            let native_window = window.ns_window().map_err(|error| {
                PlatformError::Operation(format!("unable to resolve floating window: {error}"))
            })?;
            // SAFETY: The window is live and borrowed on the AppKit main thread.
            unsafe { presentation::configure_floating_window_radius(native_window, radius) }
        })
    }

    fn present(&self, window: &WebviewWindow<R>) -> Result<(), PlatformError> {
        on_main_thread(window, |window| {
            let native_window = window.ns_window().map_err(|error| {
                PlatformError::Operation(format!(
                    "unable to resolve native Preview window: {error}"
                ))
            })?;
            // SAFETY: Acquisition and synchronous native use share the AppKit main thread
            // and this live Tauri window. No raw handle crosses the channel.
            unsafe { presentation::present_window_from_screen_right(native_window) }
        })
    }

    fn dismiss<'a>(
        &'a self,
        window: &'a WebviewWindow<R>,
    ) -> crate::platform::PresentationFuture<'a> {
        Box::pin(async move {
            let transition = on_main_thread(window, |window| {
                let native_window = window.ns_window().map_err(|error| {
                    PlatformError::Operation(format!(
                        "unable to resolve native Preview window: {error}"
                    ))
                })?;
                // SAFETY: Native code establishes strong retention on the main thread
                // before this live borrowed window leaves the closure.
                Ok(unsafe { presentation::dismiss_window_to_screen_right(native_window) })
            })?;
            transition.wait().await
        })
    }

    fn transition<'a>(
        &'a self,
        window: &'a WebviewWindow<R>,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> crate::platform::PresentationFuture<'a> {
        Box::pin(async move {
            let transition = on_main_thread(window, move |window| {
                let native_window = window.ns_window().map_err(|error| {
                    PlatformError::Operation(format!(
                        "unable to resolve native Preview window: {error}"
                    ))
                })?;
                let scale_factor = window.scale_factor().map_err(|error| {
                    PlatformError::Operation(format!(
                        "unable to resolve Preview scale factor: {error}"
                    ))
                })?;
                let current_position = window
                    .outer_position()
                    .map_err(|error| {
                        PlatformError::Operation(format!(
                            "unable to resolve Preview position: {error}"
                        ))
                    })?
                    .to_logical::<f64>(scale_factor);
                // SAFETY: Main-thread start retains the live borrowed window before returning.
                // Native owns geometry validation, callback context and terminal cleanup.
                Ok(unsafe {
                    presentation::transition_window_frame(
                        native_window,
                        x - current_position.x,
                        y - current_position.y,
                        width,
                        height,
                    )
                })
            })?;
            transition.wait().await
        })
    }
}
