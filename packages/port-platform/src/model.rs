use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourceReference {
    pub uri: String,
    pub source_attribute: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct WindowIdentity {
    pub operation_id: uuid::Uuid,
    pub receipt: crate::authority::TargetReceipt,
    pub selection_ordinal: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WindowObservableFacts {
    pub application_id: String,
    pub title: String,
    pub application_name: String,
    pub frame: Bounds,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SelectedWindow {
    #[serde(flatten)]
    pub identity: WindowIdentity,
    #[serde(flatten)]
    pub facts: WindowObservableFacts,
}

/// Explicit one-shot diagnostic address. Never serialized as a selected target or
/// used as a fallback for a revoked registered receipt.
#[derive(Debug, Clone, PartialEq)]
pub struct LegacyWindow {
    pub window_id: u32,
    pub pid: i32,
    pub facts: WindowObservableFacts,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionQuality {
    Full,
    Partial,
    Unavailable,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceApi {
    MacosAx,
    WindowsUia,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SemanticKind {
    Heading,
    Text,
    List,
    ListItem,
    Table,
    Row,
    Cell,
    Link,
    Control,
    Dialog,
    Region,
    Paragraph,
    Image,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodePurpose {
    Content,
    WindowChrome,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtractedNode {
    pub id: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    pub order: usize,
    pub depth: usize,
    #[serde(default)]
    pub native_role: Option<String>,
    #[serde(default)]
    pub native_subrole: Option<String>,
    pub source_api: SourceApi,
    pub semantic_kind: SemanticKind,
    pub node_purpose: NodePurpose,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub bounds: Option<crate::geometry::NodeBounds>,
    #[serde(default)]
    pub resource_refs: Vec<ResourceReference>,
    #[serde(default)]
    pub children: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolvedWindow {
    pub facts: WindowObservableFacts,
    pub resolution_score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ExtractionMetrics {
    pub visited_nodes: usize,
    pub text_bytes: usize,
    pub offscreen_text_nodes: usize,
    pub virtualization_signals: usize,
    pub truncated_nodes: bool,
    pub truncated_text: bool,
    pub children_read_errors: usize,
    pub resource_ref_count: usize,
    pub resource_uri_bytes: usize,
    pub omitted_resource_refs: usize,
    pub resource_read_errors: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtractionResult {
    /// Registered calls must return their exact authority. None belongs only to
    /// the explicit legacy diagnostic path; consumers must reject it otherwise.
    pub read: Option<crate::authority::TargetReadKey>,
    /// Validated native acquisition geometry, absent for unavailable or legacy reads.
    pub geometry: Option<crate::geometry::ReadGeometryDescriptor>,
    pub quality: ExtractionQuality,
    #[serde(default)]
    pub resolved_window: Option<ResolvedWindow>,
    #[serde(default)]
    pub nodes: Vec<ExtractedNode>,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub metrics: ExtractionMetrics,
    #[serde(default)]
    pub diagnostics: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_semantics_are_required_and_raw_native_fields_are_optional() {
        let complete = serde_json::json!({
            "id": "node", "order": 0, "depth": 0,
            "source_api": "windows_uia", "semantic_kind": "unknown", "node_purpose": "content"
        });
        let node: ExtractedNode = serde_json::from_value(complete.clone()).unwrap();
        assert_eq!(node.source_api, SourceApi::WindowsUia);
        assert_eq!(node.semantic_kind, SemanticKind::Unknown);
        assert_eq!(node.node_purpose, NodePurpose::Content);
        assert_eq!(node.native_role, None);
        assert_eq!(node.native_subrole, None);
        for key in ["source_api", "semantic_kind", "node_purpose"] {
            let mut missing = complete.clone();
            missing.as_object_mut().unwrap().remove(key);
            assert!(
                serde_json::from_value::<ExtractedNode>(missing).is_err(),
                "{key}"
            );
            let mut unknown = complete.clone();
            unknown[key] = serde_json::json!("future_value");
            assert!(
                serde_json::from_value::<ExtractedNode>(unknown).is_err(),
                "{key}"
            );
        }
    }
}
