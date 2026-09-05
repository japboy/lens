use crate::{
    model::{ExtractionResult, SelectedWindow, WindowIdentity},
    ExtractionLimits, PlatformError,
};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub enum ExtractionTarget {
    Registered {
        operation_id: Uuid,
        identity: WindowIdentity,
    },
    /// The existing explicit one-shot diagnostic path, never a registered fallback.
    Legacy(SelectedWindow),
}

pub trait Accessibility: Send + Sync {
    /// Bounded blocking traversal; the caller owns scheduling and admission.
    fn extract(
        &self,
        target: ExtractionTarget,
        limits: ExtractionLimits,
    ) -> Result<ExtractionResult, PlatformError>;
}
