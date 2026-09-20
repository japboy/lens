use adapter_platform_macos::presentation;
use port_platform::PlatformError;
use tauri::WebviewWindow;

pub struct MacOsPresentation;

// Keep every top-level row for native title/position validation, but only
// the leading session MenuItems may receive the supplied tooltip metadata.
fn history_tooltip_rows(
    items: Vec<(String, bool)>,
    tooltips: Vec<String>,
) -> Result<Vec<(String, Option<String>)>, PlatformError> {
    if tooltips.len() > items.len() {
        return Err(PlatformError::Operation(
            "history tooltip count exceeds menu items".into(),
        ));
    }
    items
        .into_iter()
        .enumerate()
        .map(|(index, (text, is_session_item))| {
            let tooltip = tooltips.get(index).cloned();
            if tooltip.is_some() && !is_session_item {
                return Err(PlatformError::Operation(
                    "session tooltip points to a non-session menu item".into(),
                ));
            }
            Ok((text, tooltip))
        })
        .collect()
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
    fn format_short_datetime(&self, unix_seconds: f64) -> Result<String, PlatformError> {
        presentation::format_short_datetime(unix_seconds)
    }

    fn history_tooltips(
        &self,
        app: &tauri::AppHandle<R>,
        root: &tauri::menu::Menu<R>,
        history: &tauri::menu::Submenu<R>,
        tooltips: Vec<String>,
    ) -> Result<(), PlatformError> {
        let error = |error: tauri::Error| PlatformError::Operation(error.to_string());
        let root_items = root.items().map_err(error)?;
        let index = root_items
            .iter()
            .position(|item| item.id() == history.id())
            .ok_or_else(|| PlatformError::Operation("history submenu is not attached".into()))?;
        let title = history.text().map_err(error)?;
        let items = history.items().map_err(error)?;
        let expected = history_tooltip_rows(
            items
                .iter()
                .map(|item| match item {
                    tauri::menu::MenuItemKind::MenuItem(item) => {
                        item.text().map(|text| (text, true)).map_err(error)
                    }
                    tauri::menu::MenuItemKind::Predefined(item) => {
                        item.text().map(|text| (text, false)).map_err(error)
                    }
                    tauri::menu::MenuItemKind::Submenu(item) => {
                        item.text().map(|text| (text, false)).map_err(error)
                    }
                    _ => Err(PlatformError::Operation(
                        "unexpected history menu item kind".into(),
                    )),
                })
                .collect::<Result<Vec<_>, _>>()?,
            tooltips,
        )?;
        app.tray_by_id("lens")
            .ok_or_else(|| PlatformError::Operation("Lens tray icon is unavailable".into()))?
            .with_inner_tray_icon(move |tray| {
                let item = tray.ns_status_item().ok_or_else(|| {
                    PlatformError::Operation("native status item is unavailable".into())
                })?;
                // SAFETY: Tauri runs this closure on AppKit's main thread. The retained
                // status item remains alive throughout this synchronous borrowed call.
                unsafe {
                    presentation::set_menu_tooltips(
                        &*item as *const _ as *mut std::ffi::c_void,
                        index,
                        &title,
                        &expected,
                    )
                }
            })
            .map_err(error)?
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trailing_history_controls_keep_positions_without_session_tooltips() {
        let rows = history_tooltip_rows(
            vec![
                ("Session".into(), true),
                ("".into(), false),
                ("Reload Saved Sessions".into(), true),
                ("Filter by Agent".into(), false),
                ("Update from Agent…".into(), false),
            ],
            vec!["date · agent".into()],
        )
        .unwrap();
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[0].1.as_deref(), Some("date · agent"));
        assert_eq!(rows[3].0, "Filter by Agent");
        assert_eq!(rows[4].0, "Update from Agent…");
        assert!(rows[1..].iter().all(|(_, tooltip)| tooltip.is_none()));
    }

    #[test]
    fn misplaced_or_excess_session_tooltips_are_rejected() {
        assert!(
            history_tooltip_rows(vec![("Filter".into(), false)], vec!["wrong".into()]).is_err()
        );
        assert!(history_tooltip_rows(Vec::new(), vec!["orphan".into()]).is_err());
    }
}
