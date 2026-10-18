//! macOS native effects. Domain conversion and application state do not belong here.

#[cfg(not(target_os = "macos"))]
compile_error!("adapter-platform-macos requires a macOS target");

mod accessibility;
mod capture;
pub mod presentation;
mod source;

#[derive(Debug, Clone, Copy, Default)]
pub struct MacOsPlatform;

/// Enables metadata-only release diagnostics for the explicit debug validation composition.
pub fn enable_source_validation_metadata() {
    #[cfg(debug_assertions)]
    source::enable_source_validation_metadata();
}
