use crate::{
    authority::TargetReadKey,
    model::{ExtractionResult, SelectedWindow},
    ExtractionLimits, PlatformError,
};

#[derive(Debug, Clone)]
pub enum ExtractionTarget {
    Registered(TargetReadKey),
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
