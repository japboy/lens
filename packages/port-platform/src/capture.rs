use crate::geometry::{AcquiredFrame, DesktopSpace, FrameId, PixelRect};
use crate::{authority::TargetReadKey, ImageCaptureLimits, PlatformError};
use serde::{Deserialize, Serialize};

pub use crate::model::Bounds;

/// Registered operations resolve only their retained target; they never re-enumerate it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureTarget {
    Registered(TargetReadKey),
    /// Existing explicit one-shot validation path, never a registered-operation fallback.
    Legacy {
        window_id: u32,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureScope {
    AxElementRegion,
    WindowFallback,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureCoverage {
    FullRegion,
    VisibleSubregion,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureOmissionReason {
    MissingBounds,
    InvalidBounds,
    OutsideWindow,
    AttachmentLimit,
    ByteBudget,
    CaptureFailed,
    GeometryUnavailable,
    ObservationChanged,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CaptureRegion {
    FrameContent,
    DesktopRegion {
        space: DesktopSpace,
        bounds: Bounds,
        observed_window_frame: Bounds,
    },
}

/// Native capture needs geometry and correlation only, not domain node IDs or media URIs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CaptureRequest {
    pub id: String,
    pub scope: CaptureScope,
    pub region: CaptureRegion,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CapturedImage {
    pub attachment_id: String,
    pub frame_id: FrameId,
    /// Requested and captured regions are frame_pixels, never desktop geometry.
    pub source_bounds: Bounds,
    pub captured_bounds: Bounds,
    pub crop: PixelRect,
    pub coverage: CaptureCoverage,
    pub pixel_width: usize,
    pub pixel_height: usize,
    /// Decoded PNG bytes; native transfer encoding is not an application contract.
    pub png: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureOmission {
    pub attachment_id: String,
    pub reason: CaptureOmissionReason,
    pub detail: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CaptureBatch {
    /// Required whenever captures exist; absent only when no image was acquired.
    pub frame: Option<AcquiredFrame>,
    pub captures: Vec<CapturedImage>,
    pub omissions: Vec<CaptureOmission>,
    pub diagnostics: Vec<String>,
}

/// A bounded blocking capability. The caller owns scheduling and operation admission.
/// Implementations validate correlation, PNG transfer metadata, geometry and byte limits
/// before returning. An error is never silently converted into an empty successful batch.
pub trait Capture: Send + Sync {
    fn capture(
        &self,
        target: CaptureTarget,
        requests: &[CaptureRequest],
        limits: ImageCaptureLimits,
    ) -> Result<CaptureBatch, PlatformError>;
}
