#![forbid(unsafe_code)]

//! Deterministic observation models, normalization, projection and prompt rules.
//! This package has no native-service, Tauri, transport or persistence dependency.

pub mod geometry;
pub mod lens;
pub mod model;
pub mod projection;
pub mod prompt_template;
