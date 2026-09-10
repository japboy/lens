#![forbid(unsafe_code)]

//! Application operations compose domain rules with explicitly supplied capabilities.

// Canonical contracts exposed to common application consumers through the usecase boundary.
pub use domain::{lens, prompt_template};

pub mod agent_preferences;
pub mod confirm_targets;
pub mod context;
pub mod elicitation;
pub mod live_sync;
pub mod media;
pub mod model;
pub mod observation;
pub mod platform;
pub mod prompt_presets;
pub mod session_controls;
pub mod state;
