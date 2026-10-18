use crate::{authority::TargetAuthority, model::SelectedWindow, PlatformError, PlatformFuture};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Native transfer outcome; the application validates the single-window product policy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum WindowPickerReply {
    Selected { windows: Vec<RegisteredWindow> },
    Cancelled,
    Error { message: String },
}

/// Display facts are not lookup authority; the adapter owns the receipt binding.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegisteredWindow {
    pub target: TargetAuthority,
    pub window: SelectedWindow,
}

pub trait TargetSelection: Send + Sync {
    fn pick(
        &self,
        operation_id: Uuid,
    ) -> PlatformFuture<'_, Result<WindowPickerReply, PlatformError>>;
    fn release_target(&self, target: TargetAuthority) -> Result<(), PlatformError>;
    fn release_operation(&self, operation_id: Uuid) -> Result<(), PlatformError>;
}
