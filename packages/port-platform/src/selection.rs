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
    fn pick(
        &self,
        operation_id: Uuid,
    ) -> PlatformFuture<'_, Result<WindowPickerReply, PlatformError>>;
    fn release_target(&self, operation_id: Uuid, window_id: u32) -> Result<(), PlatformError>;
    fn release_operation(&self, operation_id: Uuid) -> Result<(), PlatformError>;
}
