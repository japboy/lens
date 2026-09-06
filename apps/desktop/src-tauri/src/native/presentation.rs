use adapter_platform_macos::presentation;
use port_platform::PlatformError;
use tauri::WebviewWindow;

pub struct MacOsPresentation;

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
