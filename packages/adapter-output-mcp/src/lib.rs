//! Source-bound MCP Apps transport and isolated HTML display services.
pub mod apps;
pub const MAX_HTML_BYTES: usize = 512 * 1024;
pub type ServerError = Box<dyn std::error::Error + Send + Sync>;
