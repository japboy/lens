/// Application-level accessibility availability, not authority to read any target.
/// Target-specific restrictions must still be checked by the native source adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessibilityAccess {
    Ready,
    PermissionRequired,
    AccessRestricted,
    Unsupported,
    Failed { message: String },
}

/// Inspection never prompts. Request is a separately authorized effect; adapters
/// without a user-grant mechanism return their explicit unavailable state.
pub trait AccessibilityTrust: Send + Sync {
    fn inspect(&self) -> AccessibilityAccess;
    fn request(&self) -> AccessibilityAccess;
}
