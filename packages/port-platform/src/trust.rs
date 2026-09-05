/// Preserves the implemented permission inspection/request behavior without inventing
/// a new cross-platform permission schema or prompting during inspection.
pub trait AccessibilityTrust: Send + Sync {
    fn inspect(&self) -> bool;
    fn request(&self) -> bool;
}
