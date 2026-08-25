#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
pub use macos::{WindowObserver, WindowObserverEvent};

use crate::model::{Bounds, ExtractionResult, SelectedWindow, WindowPickerReply};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PlatformError {
    #[error("native picker is already active")]
    PickerBusy,
    #[error("native picker callback was dropped")]
    PickerCallbackDropped,
    #[error("invalid native response: {0}")]
    InvalidResponse(String),
    #[error("platform operation failed: {0}")]
    Operation(String),
}

pub fn accessibility_is_trusted() -> bool {
    macos::accessibility_is_trusted()
}

pub fn request_accessibility_trust() -> bool {
    macos::request_accessibility_trust()
}

pub async fn present_window_picker() -> Result<WindowPickerReply, PlatformError> {
    macos::present_window_picker().await
}

pub fn extract_window(target: &SelectedWindow) -> Result<ExtractionResult, PlatformError> {
    macos::extract_window(target)
}

pub fn current_window_frame(window_id: u32) -> Result<Option<Bounds>, PlatformError> {
    macos::current_window_frame(window_id)
}

pub fn observe_window(target: &SelectedWindow) -> Result<WindowObserver, PlatformError> {
    macos::observe_window(target)
}
