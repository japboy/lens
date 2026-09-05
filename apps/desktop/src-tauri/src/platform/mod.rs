#[cfg(target_os = "macos")]
mod presentation_macos;

#[allow(unused_imports)]
pub use port_platform::observation::{
    WindowObservationEvent, WindowObservationNotification, WindowObservationReceiver,
    WindowObservationRegistration, WindowObservationStart,
};
use port_platform::{
    accessibility::Accessibility, capture::Capture, observation::Observation,
    selection::TargetSelection, trust::AccessibilityTrust,
};
pub use port_platform::{ImageCaptureLimits, PlatformError};
use std::sync::Arc;

/// One explicitly supplied instance per capability; no ambient platform selection in consumers.
#[derive(Clone)]
pub struct Services {
    pub selection: Arc<dyn TargetSelection>,
    pub accessibility: Arc<dyn Accessibility>,
    pub capture: Arc<dyn Capture>,
    pub observation: Arc<dyn Observation>,
    pub trust: Arc<dyn AccessibilityTrust>,
}

#[cfg(target_os = "macos")]
pub fn macos_services() -> Services {
    let native = Arc::new(adapter_platform_macos::MacOsPlatform);
    Services {
        selection: native.clone(),
        accessibility: native.clone(),
        capture: native.clone(),
        observation: native.clone(),
        trust: native,
    }
}

/// Tauri-owned presentation effects, separate from portable source capabilities.
pub trait WindowPresentation<R: tauri::Runtime>: Send + Sync {
    fn present(&self, window: &tauri::WebviewWindow<R>) -> Result<(), PlatformError>;
    fn dismiss<'a>(&'a self, window: &'a tauri::WebviewWindow<R>) -> PresentationFuture<'a>;
    fn transition<'a>(
        &'a self,
        window: &'a tauri::WebviewWindow<R>,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> PresentationFuture<'a>;
}

pub type PresentationFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), PlatformError>> + Send + 'a>>;

pub struct Presentation<R: tauri::Runtime>(pub Arc<dyn WindowPresentation<R>>);

#[cfg(target_os = "macos")]
pub fn macos_presentation() -> Presentation<tauri::Wry> {
    Presentation(Arc::new(presentation_macos::MacOsPresentation))
}

pub fn present_window_from_screen_right<R: tauri::Runtime>(
    window: &tauri::WebviewWindow<R>,
) -> Result<(), PlatformError> {
    use tauri::Manager;
    window.state::<Presentation<R>>().0.present(window)
}

pub async fn dismiss_window_to_screen_right<R: tauri::Runtime>(
    window: &tauri::WebviewWindow<R>,
) -> Result<(), PlatformError> {
    use tauri::Manager;
    window.state::<Presentation<R>>().0.dismiss(window).await
}

pub async fn transition_window_frame<R: tauri::Runtime>(
    window: &tauri::WebviewWindow<R>,
    target_x: f64,
    target_y: f64,
    target_content_width: f64,
    target_content_height: f64,
) -> Result<(), PlatformError> {
    use tauri::Manager;
    window
        .state::<Presentation<R>>()
        .0
        .transition(
            window,
            target_x,
            target_y,
            target_content_width,
            target_content_height,
        )
        .await
}
