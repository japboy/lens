use crate::{live_sync::ProjectionRef, prompt_template::AgentPromptTemplate};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum WindowPickerReply {
    Selected { windows: Vec<SelectedWindow> },
    Cancelled,
    Error { message: String },
}

impl WindowPickerReply {
    pub fn into_selected(self) -> Result<Option<Vec<SelectedWindow>>, String> {
        match self {
            Self::Selected { windows } => Ok(Some(windows)),
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
    pub config_options: Option<Vec<agent_client_protocol::schema::v1::SessionConfigOption>>,
    #[serde(default)]
    pub modes: Vec<agent_client_protocol::schema::v1::SessionMode>,
    #[serde(default)]
    pub policy_default: Option<String>,
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
            config_options: None,
            modes: Vec::new(),
            policy_default: None,
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
    #[serde(default)]
    pub input_projection: Option<ProjectionRef>,
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
#[serde(from = "AppConfigWire")]
pub struct AppConfig {
    pub agent_preferences: crate::agent_preferences::AgentPreferences,
    pub agent: AgentKind,
    pub working_directory: PathBuf,
    pub agent_prompt_template: AgentPromptTemplate,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            agent: AgentKind::Claude,
            agent_preferences: Default::default(),
            working_directory: dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")),
            agent_prompt_template: AgentPromptTemplate::default(),
        }
    }
}

#[derive(Deserialize)]
#[serde(default)]
struct AppConfigWire {
    agent_preferences: crate::agent_preferences::AgentPreferences,
    agent: AgentKind,
    working_directory: PathBuf,
    agent_prompt_template: Option<AgentPromptTemplate>,
    response_prompt: Option<String>,
}

impl Default for AppConfigWire {
    fn default() -> Self {
        let config = AppConfig::default();
        Self {
            agent: config.agent,
            agent_preferences: config.agent_preferences,
            working_directory: config.working_directory,
            agent_prompt_template: None,
            response_prompt: None,
        }
    }
}

impl From<AppConfigWire> for AppConfig {
    fn from(wire: AppConfigWire) -> Self {
        let agent_prompt_template = wire.agent_prompt_template.unwrap_or_else(|| {
            wire.response_prompt
                .as_deref()
                .map(AgentPromptTemplate::from_legacy_response_prompt)
                .unwrap_or_default()
        });
        Self {
            agent: wire.agent,
            agent_preferences: wire.agent_preferences,
            working_directory: wire.working_directory,
            agent_prompt_template,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LensMonitoringLifecycle {
    Watching,
    Paused,
    Stopped,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LensSourceHealth {
    Healthy,
    Degraded,
    Unavailable,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LensFreshness {
    None,
    Current,
    Checking,
    Stale,
    Unverified,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LensRefreshOutcome {
    Unchanged,
    Updated,
    Failed,
}

pub const LIVE_AGENT_REFRESH_INTERVAL_SECONDS: u64 = 3 * 60;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LensLiveState {
    pub lifecycle: LensMonitoringLifecycle,
    pub health: LensSourceHealth,
    pub freshness: LensFreshness,
    pub agent_refresh_interval_seconds: u64,
    #[serde(default)]
    pub last_outcome: Option<LensRefreshOutcome>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LensRepresentation {
    pub representation_id: Uuid,
    pub context_id: Uuid,
    pub context_revision: u64,
    pub projection: ProjectionRef,
    pub run_id: Uuid,
    pub output_blocks: Vec<LensOutputBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LensPendingRepresentation {
    pub turn_id: Uuid,
    pub target_projection: ProjectionRef,
    #[serde(default)]
    pub base_representation_id: Option<Uuid>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LensTargetSelectionStage {
    Picking,
    Reviewing,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LensTargetSelectionItem {
    pub id: String,
    pub window: SelectedWindow,
    #[serde(default)]
    pub preview_uri: Option<String>,
    #[serde(default)]
    pub preview_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LensTargetSelection {
    pub selection_id: Uuid,
    pub stage: LensTargetSelectionStage,
    pub maximum_targets: usize,
    #[serde(default)]
    pub anchor: Option<Bounds>,
    #[serde(default)]
    pub items: Vec<LensTargetSelectionItem>,
    #[serde(default)]
    pub notice: Option<String>,
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
    pub session_controls: Option<crate::session_controls::AgentSessionControlState>,
    #[serde(default)]
    pub operation_id: Option<Uuid>,
    pub stage: LensStage,
    #[serde(default)]
    pub selection: Option<LensTargetSelection>,
    #[serde(default)]
    pub target_set: Option<crate::lens::LensTargetSet>,
    #[serde(default)]
    pub context: Option<crate::lens::LensContext>,
    #[serde(default)]
    pub input: Option<crate::lens::LensInput>,
    #[serde(default)]
    pub projection: Option<ProjectionRef>,
    #[serde(default)]
    pub output_blocks: Vec<LensOutputBlock>,
    #[serde(default)]
    pub representation: Option<LensRepresentation>,
    #[serde(default)]
    pub pending_representation: Option<LensPendingRepresentation>,
    #[serde(default)]
    pub live: Option<LensLiveState>,
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
            session_controls: None,
            selection: None,
            target_set: None,
            context: None,
            input: None,
            projection: None,
            output_blocks: Vec::new(),
            representation: None,
            pending_representation: None,
            live: None,
            agent: None,
            error: None,
        }
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
    fn picker_reply_preserves_every_serialized_window() {
        let reply: WindowPickerReply = serde_json::from_str(
            r#"{
                "status":"selected",
                "windows":[
                    {"window_id":9,"title":"Nine","application_name":"App Z","bundle_id":"z.example","pid":90,"frame":{"x":0.0,"y":0.0,"width":900.0,"height":700.0}},
                    {"window_id":7,"title":"Seven","application_name":"App A","bundle_id":"a.example","pid":70,"frame":{"x":10.0,"y":20.0,"width":800.0,"height":600.0}}
                ]
            }"#,
        )
        .expect("valid native multi-window reply");
        let windows = reply
            .into_selected()
            .expect("selected reply")
            .expect("selected windows");

        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].identity.window_id, 9);
        assert_eq!(windows[1].identity.window_id, 7);
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
