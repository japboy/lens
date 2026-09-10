use crate::{model::SelectedWindow, PlatformError, PlatformFuture};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Native transfer outcome; the application validates the single-window product policy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum WindowPickerReply {
    Selected { windows: Vec<SelectedWindow> },
    Cancelled,
    Error { message: String },
}

pub trait TargetSelection: Send + Sync {
    /// Opens an operation before any picker can admit native resources. A picker
    /// must never reopen a released operation as a side effect of completion.
    fn open_operation(&self, operation_id: Uuid) -> Result<(), PlatformError>;
    fn pick(
        &self,
        operation_id: Uuid,
    ) -> PlatformFuture<'_, Result<WindowPickerReply, PlatformError>>;
    fn release_target(
        &self,
        operation_id: Uuid,
        receipt: crate::authority::TargetReceipt,
    ) -> Result<(), PlatformError>;
    fn release_operation(&self, operation_id: Uuid) -> Result<(), PlatformError>;
}
