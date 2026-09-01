#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
#[allow(unused_imports)]
// Public source-observation boundary; consumers live above this module.
pub use macos::{
    WindowObservationEvent, WindowObservationNotification, WindowObservationReceiver,
    WindowObservationRegistration, WindowObservationStart,
};

use crate::{
    lens::{LensMediaCapture, LensMediaPlan},
    model::{ExtractionResult, SelectedWindow, WindowPickerReply},
};
use std::num::NonZeroU64;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Copy)]
pub struct ImageCaptureLimits {
    pub max_long_edge: u32,
    pub max_pixels: u32,
    pub max_attachment_bytes: u32,
    pub max_total_bytes: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct ExtractionLimits {
    pub max_nodes: u32,
    pub max_text_bytes: u32,
    pub max_resource_refs: u32,
    pub max_resource_uri_bytes: u32,
    pub max_total_resource_uri_bytes: u32,
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

pub fn present_window_from_screen_right(
    window: &tauri::WebviewWindow,
) -> Result<(), PlatformError> {
    macos::present_window_from_screen_right(window)
}

pub async fn dismiss_window_to_screen_right(
    window: &tauri::WebviewWindow,
) -> Result<(), PlatformError> {
    macos::dismiss_window_to_screen_right(window).await
}

pub async fn transition_window_frame(
    window: &tauri::WebviewWindow,
    target_x: f64,
    target_y: f64,
    target_content_width: f64,
    target_content_height: f64,
) -> Result<(), PlatformError> {
    macos::transition_window_frame(
        window,
        target_x,
        target_y,
        target_content_width,
        target_content_height,
    )
    .await
}

pub async fn present_window_picker_for_operation(
    operation_id: Uuid,
) -> Result<WindowPickerReply, PlatformError> {
    macos::present_window_picker_for_operation(operation_id).await
}

pub fn extract_window(
    target: &SelectedWindow,
    limits: ExtractionLimits,
) -> Result<ExtractionResult, PlatformError> {
    macos::extract_window(target, limits)
}

pub fn extract_registered_window(
    operation_id: Uuid,
    target: &SelectedWindow,
    limits: ExtractionLimits,
) -> Result<ExtractionResult, PlatformError> {
    macos::extract_registered_window(operation_id, target, limits)
}

pub fn start_window_observation(
    operation_id: Uuid,
    context_id: Uuid,
    source_registration_id: Uuid,
    observer_epoch: NonZeroU64,
    target: &SelectedWindow,
) -> Result<
    (
        WindowObservationRegistration,
        WindowObservationReceiver,
        WindowObservationStart,
    ),
    PlatformError,
> {
    macos::start_window_observation(
        operation_id,
        context_id,
        source_registration_id,
        observer_epoch,
        target,
    )
}

pub fn capture_window_media(
    target: &SelectedWindow,
    context_id: Uuid,
    plan: LensMediaPlan,
    limits: ImageCaptureLimits,
) -> Result<LensMediaCapture, PlatformError> {
    macos::capture_window_media(target, context_id, plan, limits)
}

pub fn capture_registered_window_media(
    operation_id: Uuid,
    target: &SelectedWindow,
    context_id: Uuid,
    context_revision: u64,
    plan: LensMediaPlan,
    limits: ImageCaptureLimits,
) -> Result<LensMediaCapture, PlatformError> {
    macos::capture_registered_window_media(
        operation_id,
        target,
        context_id,
        context_revision,
        plan,
        limits,
    )
}

pub fn release_registered_window(operation_id: Uuid, window_id: u32) -> Result<(), PlatformError> {
    macos::release_registered_window(operation_id, window_id)
}

pub fn release_window_operation(operation_id: Uuid) -> Result<(), PlatformError> {
    macos::release_window_operation(operation_id)
}
