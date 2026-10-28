//! macOS native effects. Domain conversion and application state do not belong here.

#[cfg(not(target_os = "macos"))]
compile_error!("adapter-platform-macos requires a macOS target");

mod accessibility;
mod capture;
mod capture_stream;
pub mod presentation;
mod source;

#[derive(Debug, Clone, Copy, Default)]
pub struct MacOsPlatform;

/// Enables metadata-only release diagnostics for the explicit debug validation composition.
pub fn enable_source_validation_metadata() {
    #[cfg(debug_assertions)]
    source::enable_source_validation_metadata();
}

pub use capture_stream::SourceValidationCapture;

/// Configures one purpose-bound experiment for the explicit debug composition.
pub fn configure_source_validation_capture(_action: Option<SourceValidationCapture>) {
    #[cfg(debug_assertions)]
    capture_stream::configure_validation(_action);
}

/// Arms the configured experiment only after this operation starts Watching.
pub fn arm_source_validation_capture(_operation: uuid::Uuid) {
    #[cfg(debug_assertions)]
    capture_stream::arm_validation(_operation);
}
