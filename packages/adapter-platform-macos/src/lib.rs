//! macOS native effects. Domain conversion and application state do not belong here.

#[cfg(not(target_os = "macos"))]
compile_error!("adapter-platform-macos requires a macOS target");

mod capture;
pub mod presentation;
mod source;

#[derive(Debug, Clone, Copy, Default)]
pub struct MacOsPlatform;
