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
    model::{ExtractionResult, SelectedWindow, WindowIdentity, WindowPickerReply},
};
use std::num::NonZeroU64;
use uuid::Uuid;

use port_platform::capture::CaptureTarget;
pub use port_platform::{ExtractionLimits, ImageCaptureLimits, PlatformError};
use use_case::media::{capture_media, MediaCaptureContext};

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
    identity: &WindowIdentity,
    limits: ExtractionLimits,
) -> Result<ExtractionResult, PlatformError> {
    macos::extract_registered_window(operation_id, identity, limits)
}

pub fn start_window_observation(
    operation_id: Uuid,
    context_id: Uuid,
    source_registration_id: Uuid,
    observer_epoch: NonZeroU64,
    identity: &WindowIdentity,
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
        identity,
    )
}

pub fn capture_window_media(
    target: &SelectedWindow,
    context_id: Uuid,
    plan: LensMediaPlan,
    limits: ImageCaptureLimits,
) -> Result<LensMediaCapture, PlatformError> {
    capture_media(
        &adapter_platform_macos::MacOsCapture,
        MediaCaptureContext {
            target: CaptureTarget::Legacy {
                window_id: target.identity.window_id,
            },
            target_id: crate::lens::target_id(target),
            context_id,
            context_revision: NonZeroU64::MIN,
        },
        plan,
        limits,
    )
}

pub fn capture_registered_window_media(
    operation_id: Uuid,
    target_id: &str,
    identity: &WindowIdentity,
    context_id: Uuid,
    context_revision: u64,
    plan: LensMediaPlan,
    limits: ImageCaptureLimits,
) -> Result<LensMediaCapture, PlatformError> {
    let context_revision = NonZeroU64::new(context_revision).ok_or_else(|| {
        PlatformError::Operation(
            "registered image capture context revision must be non-zero".into(),
        )
    })?;
    capture_media(
        &adapter_platform_macos::MacOsCapture,
        MediaCaptureContext {
            target: CaptureTarget::Registered {
                operation_id,
                window_id: identity.window_id,
            },
            target_id: target_id.to_owned(),
            context_id,
            context_revision,
        },
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
