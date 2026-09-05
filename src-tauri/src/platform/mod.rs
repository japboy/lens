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

pub fn present_window_from_screen_right(
    window: &tauri::WebviewWindow,
) -> Result<(), PlatformError> {
    presentation_macos::present_window_from_screen_right(window)
}

pub async fn dismiss_window_to_screen_right(
    window: &tauri::WebviewWindow,
) -> Result<(), PlatformError> {
    presentation_macos::dismiss_window_to_screen_right(window).await
}

pub async fn transition_window_frame(
    window: &tauri::WebviewWindow,
    target_x: f64,
    target_y: f64,
    target_content_width: f64,
    target_content_height: f64,
) -> Result<(), PlatformError> {
    presentation_macos::transition_window_frame(
        window,
        target_x,
        target_y,
        target_content_width,
        target_content_height,
    )
    .await
}
