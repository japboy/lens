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
    pub window_id: u32,
    pub bundle_id: String,
    pub pid: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WindowObservableFacts {
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionQuality {
    Full,
    Partial,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtractedNode {
    pub id: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    pub order: usize,
    pub depth: usize,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub subrole: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub bounds: Option<Bounds>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LensOutputBlock {
    Markdown {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
        text: String,
    },
    Image {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
        mime_type: String,
        data: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        uri: Option<String>,
    },
    Unsupported {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
        content_type: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_block_serialization_is_tagged_and_self_describing() {
        let block = LensOutputBlock::Image {
            message_id: None,
            mime_type: "image/webp".into(),
            data: "aW1hZ2U=".into(),
            uri: Some("urn:fixture:image".into()),
        };

        assert_eq!(
            serde_json::to_value(block).expect("serialize output block"),
            serde_json::json!({
                "type": "image",
                "mime_type": "image/webp",
                "data": "aW1hZ2U=",
                "uri": "urn:fixture:image"
            })
        );
    }
}
