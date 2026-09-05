use crate::{ImageCaptureLimits, PlatformError};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use crate::model::Bounds;

/// Registered operations resolve only their retained target; they never re-enumerate it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureTarget {
    Registered {
        operation_id: Uuid,
        window_id: u32,
    },
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
}

/// Native capture needs geometry and correlation only, not domain node IDs or media URIs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CaptureRequest {
    pub id: String,
    pub scope: CaptureScope,
    pub bounds: Option<Bounds>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CapturedImage {
    pub attachment_id: String,
    pub source_bounds: Bounds,
    pub captured_bounds: Bounds,
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
    pub window_bounds: Option<Bounds>,
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
