//! Platform capabilities expose owned facts, never domain state or native objects.

pub mod accessibility;
pub mod capture;
pub mod model;
pub mod observation;
pub mod selection;
pub mod trust;

pub type PlatformFuture<'a, T> =
    std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

use thiserror::Error;

#[derive(Debug, Clone, Copy)]
pub struct ImageCaptureLimits {
    pub max_long_edge: u32,
    pub max_pixels: u32,
    pub max_attachment_bytes: u32,
    pub max_total_bytes: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct ExtractionLimits {
    pub max_nodes: u32,
    pub max_text_bytes: u32,
    pub max_resource_refs: u32,
    pub max_resource_uri_bytes: u32,
    pub max_total_resource_uri_bytes: u32,
}

#[derive(Debug, Error)]
pub enum PlatformError {
    #[error("native picker is already active")]
    PickerBusy,
    #[error("native picker callback was dropped")]
    PickerCallbackDropped,
    #[error("invalid native response: {0}")]
    InvalidResponse(String),
    #[error("platform operation failed: {0}")]
    Operation(String),
}
