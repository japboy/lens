use crate::{
    authority::TargetReadKey,
    model::{ExtractionResult, LegacyWindow},
    ExtractionLimits, PlatformError,
};

#[derive(Debug, Clone)]
pub enum ExtractionTarget {
    Registered {
        read: TargetReadKey,
    },
    /// The existing explicit one-shot diagnostic path, never a registered fallback.
    Legacy(LegacyWindow),
}

pub trait Accessibility: Send + Sync {
    /// Bounded blocking traversal; the caller owns scheduling and admission.
    fn extract(
        &self,
        target: ExtractionTarget,
        limits: ExtractionLimits,
    ) -> Result<ExtractionResult, PlatformError>;
}
