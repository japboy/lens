#[cfg(target_os = "macos")]
mod macos;

use crate::{
    lens::{LensMediaCapture, LensMediaPlan},
    model::{ExtractionResult, SelectedWindow, WindowPickerReply},
};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Copy)]
pub struct ImageCaptureLimits {
    pub max_long_edge: u32,
    pub max_pixels: u32,
    pub max_attachment_bytes: u32,
    pub max_total_bytes: u32,
}

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

pub fn capture_window_media(
    target: &SelectedWindow,
    context_id: Uuid,
    plan: LensMediaPlan,
    limits: ImageCaptureLimits,
) -> Result<LensMediaCapture, PlatformError> {
    macos::capture_window_media(target, context_id, plan, limits)
}
