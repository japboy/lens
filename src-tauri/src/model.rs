use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

pub const BUILT_IN_RESPONSE_PROMPT: &str = "Transform the information currently being viewed by the user into the form that is easiest for this user to consume. Use the user's existing instructions, memory, and preferences available to you.";

fn built_in_response_prompt() -> String {
    BUILT_IN_RESPONSE_PROMPT.into()
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SelectedWindow {
    pub window_id: u32,
    pub title: String,
    pub application_name: String,
    pub bundle_id: String,
    pub pid: i32,
    pub frame: Bounds,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum WindowPickerReply {
    Selected {
        window_id: u32,
        title: String,
        application_name: String,
        bundle_id: String,
        pid: i32,
        frame: Bounds,
    },
    Cancelled,
    Error {
        message: String,
    },
}

impl WindowPickerReply {
    pub fn into_selected(self) -> Result<Option<SelectedWindow>, String> {
        match self {
            Self::Selected {
                window_id,
                title,
                application_name,
                bundle_id,
                pid,
                frame,
            } => Ok(Some(SelectedWindow {
                window_id,
                title,
                application_name,
                bundle_id,
                pid,
                frame,
            })),
            Self::Cancelled => Ok(None),
            Self::Error { message } => Err(message),
        }
    }
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolvedWindow {
    pub title: String,
    pub bounds: Bounds,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LensSource {
    pub application: String,
    pub window_title: String,
    pub bundle_id: String,
    pub window_id: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LensInput {
    pub source: LensSource,
    pub text: String,
    pub extraction_quality: ExtractionQuality,
}

impl LensInput {
    pub fn from_extraction(target: &SelectedWindow, extraction: &ExtractionResult) -> Option<Self> {
        if extraction.quality == ExtractionQuality::Unavailable || extraction.text.is_empty() {
            return None;
        }
        Some(Self {
            source: LensSource {
                application: target.application_name.clone(),
                window_title: target.title.clone(),
                bundle_id: target.bundle_id.clone(),
                window_id: target.window_id,
            },
            text: extraction.text.clone(),
            extraction_quality: extraction.quality,
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    Claude,
    Codex,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentRuntimeStage {
    NotInstalled,
    Resolving,
    Downloading,
    Verifying,
    Installing,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentRuntimeState {
    #[serde(default)]
    pub operation_id: Option<Uuid>,
    pub stage: AgentRuntimeStage,
    #[serde(default)]
    pub agent: Option<AgentKind>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub downloaded_bytes: u64,
    #[serde(default)]
    pub total_bytes: Option<u64>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

impl Default for AgentRuntimeState {
    fn default() -> Self {
        Self {
            operation_id: None,
            stage: AgentRuntimeStage::NotInstalled,
            agent: None,
            version: None,
            downloaded_bytes: 0,
            total_bytes: None,
            message: None,
            error: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentAuthMethodKind {
    Agent,
    Terminal,
    EnvironmentVariable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentAuthMethod {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub kind: AgentAuthMethodKind,
    pub supported: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentSelectionStage {
    Unselected,
    Checking,
    AuthenticationRequired,
    Authenticating,
    SigningOut,
    Selected,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentSelectionState {
    #[serde(default)]
    pub operation_id: Option<Uuid>,
    pub stage: AgentSelectionStage,
    #[serde(default)]
    pub candidate: Option<AgentKind>,
    #[serde(default)]
    pub auth_methods: Vec<AgentAuthMethod>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

impl AgentSelectionState {
    pub fn selected_agent(&self) -> Option<AgentKind> {
        (self.stage == AgentSelectionStage::Selected)
            .then_some(self.candidate)
            .flatten()
    }

    pub fn can_select_lens_target(&self) -> bool {
        self.selected_agent().is_some()
    }
}

impl Default for AgentSelectionState {
    fn default() -> Self {
        Self {
            operation_id: None,
            stage: AgentSelectionStage::Unselected,
            candidate: None,
            auth_methods: Vec::new(),
            message: None,
            error: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentRunState {
    pub run_id: Uuid,
    pub kind: AgentKind,
    pub adapter_name: String,
    pub adapter_version: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub session_mode_id: Option<String>,
    #[serde(default)]
    pub auth_methods: Vec<AgentAuthMethod>,
    pub received_updates: usize,
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub authentication_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppConfig {
    pub agent: AgentKind,
    pub working_directory: PathBuf,
    #[serde(default = "built_in_response_prompt")]
    pub response_prompt: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            agent: AgentKind::Claude,
            working_directory: dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")),
            response_prompt: built_in_response_prompt(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LensStage {
    Idle,
    Selecting,
    Extracting,
    Ready,
    Connecting,
    AuthenticationRequired,
    Transforming,
    Completed,
    Cancelled,
    Failed,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LensState {
    #[serde(default)]
    pub operation_id: Option<Uuid>,
    pub stage: LensStage,
    #[serde(default)]
    pub target: Option<SelectedWindow>,
    #[serde(default)]
    pub extraction: Option<ExtractionResult>,
    #[serde(default)]
    pub input: Option<LensInput>,
    #[serde(default)]
    pub output_blocks: Vec<LensOutputBlock>,
    #[serde(default)]
    pub agent: Option<AgentRunState>,
    #[serde(default)]
    pub error: Option<String>,
}

impl Default for LensState {
    fn default() -> Self {
        Self {
            operation_id: None,
            stage: LensStage::Idle,
            target: None,
            extraction: None,
            input: None,
            output_blocks: Vec::new(),
            agent: None,
            error: None,
        }
    }
}

impl LensState {
    pub fn push_output_block(&mut self, block: LensOutputBlock) {
        if let LensOutputBlock::Markdown { message_id, text } = &block {
            if text.is_empty() {
                return;
            }
            if let Some(LensOutputBlock::Markdown {
                message_id: previous_message_id,
                text: previous_text,
            }) = self.output_blocks.last_mut()
            {
                if previous_message_id == message_id {
                    previous_text.push_str(text);
                    return;
                }
            }
        }
        self.output_blocks.push(block);
    }

    pub fn has_output(&self) -> bool {
        !self.output_blocks.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppSnapshot {
    pub revision: u32,
    pub config: AppConfig,
    pub agent_runtime: AgentRuntimeState,
    pub agent_selection: AgentSelectionState,
    pub lens: LensState,
}

impl AppSnapshot {
    pub fn new(config: AppConfig) -> Self {
        Self {
            revision: 0,
            config,
            agent_runtime: AgentRuntimeState::default(),
            agent_selection: AgentSelectionState::default(),
            lens: LensState::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_extraction_never_becomes_lens_input() {
        let target = SelectedWindow {
            window_id: 1,
            title: "Document".into(),
            application_name: "Browser".into(),
            bundle_id: "example.browser".into(),
            pid: 42,
            frame: Bounds {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            },
        };
        let extraction = ExtractionResult {
            quality: ExtractionQuality::Unavailable,
            resolved_window: None,
            nodes: vec![],
            text: String::new(),
            metrics: ExtractionMetrics::default(),
            diagnostics: vec![],
        };
        assert_eq!(LensInput::from_extraction(&target, &extraction), None);
    }

    #[test]
    fn partial_extraction_remains_usable_and_explicit() {
        let target = SelectedWindow {
            window_id: 1,
            title: "Virtualized list".into(),
            application_name: "Application".into(),
            bundle_id: "example.application".into(),
            pid: 42,
            frame: Bounds {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            },
        };
        let extraction = ExtractionResult {
            quality: ExtractionQuality::Partial,
            resolved_window: None,
            nodes: vec![],
            text: "Visible rows".into(),
            metrics: ExtractionMetrics {
                virtualization_signals: 1,
                ..ExtractionMetrics::default()
            },
            diagnostics: vec!["Virtualized rows are partial by design.".into()],
        };

        let input = LensInput::from_extraction(&target, &extraction).expect("partial LensInput");

        assert_eq!(input.extraction_quality, ExtractionQuality::Partial);
        assert_eq!(input.text, "Visible rows");
    }

    #[test]
    fn only_an_authenticated_selection_enables_lens_target_selection() {
        for stage in [
            AgentSelectionStage::Unselected,
            AgentSelectionStage::Checking,
            AgentSelectionStage::AuthenticationRequired,
            AgentSelectionStage::Authenticating,
            AgentSelectionStage::SigningOut,
            AgentSelectionStage::Failed,
        ] {
            let state = AgentSelectionState {
                stage,
                candidate: Some(AgentKind::Codex),
                ..AgentSelectionState::default()
            };
            assert_eq!(state.selected_agent(), None);
            assert!(!state.can_select_lens_target());
        }

        let selected = AgentSelectionState {
            stage: AgentSelectionStage::Selected,
            candidate: Some(AgentKind::Codex),
            ..AgentSelectionState::default()
        };
        assert_eq!(selected.selected_agent(), Some(AgentKind::Codex));
        assert!(selected.can_select_lens_target());
    }

    #[test]
    fn output_blocks_merge_only_adjacent_markdown_from_the_same_message() {
        let mut lens = LensState::default();
        lens.push_output_block(LensOutputBlock::Markdown {
            message_id: Some("message-1".into()),
            text: "First ".into(),
        });
        lens.push_output_block(LensOutputBlock::Markdown {
            message_id: Some("message-1".into()),
            text: "message".into(),
        });
        lens.push_output_block(LensOutputBlock::Image {
            message_id: Some("message-1".into()),
            mime_type: "image/png".into(),
            data: "aW1hZ2U=".into(),
            uri: None,
        });
        lens.push_output_block(LensOutputBlock::Markdown {
            message_id: Some("message-1".into()),
            text: "After image".into(),
        });
        lens.push_output_block(LensOutputBlock::Markdown {
            message_id: Some("message-2".into()),
            text: "Second message".into(),
        });

        assert_eq!(
            lens.output_blocks,
            vec![
                LensOutputBlock::Markdown {
                    message_id: Some("message-1".into()),
                    text: "First message".into(),
                },
                LensOutputBlock::Image {
                    message_id: Some("message-1".into()),
                    mime_type: "image/png".into(),
                    data: "aW1hZ2U=".into(),
                    uri: None,
                },
                LensOutputBlock::Markdown {
                    message_id: Some("message-1".into()),
                    text: "After image".into(),
                },
                LensOutputBlock::Markdown {
                    message_id: Some("message-2".into()),
                    text: "Second message".into(),
                },
            ]
        );
        assert!(lens.has_output());
    }

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
