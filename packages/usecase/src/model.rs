use crate::live_sync::ProjectionRef;
pub use domain::model::{
    Bounds, ExtractionMetrics, ExtractionQuality, ExtractionResult, LensOutputBlock, NodePurpose,
    ResourceReference, SelectedWindow, SemanticKind, SourceApi, WindowIdentity,
    WindowObservableFacts,
};
use domain::prompt_template::AgentPromptTemplate;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[cfg(test)]
use crate::platform::WindowPickerReply;

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
    pub config_options: Option<Vec<agent_client_protocol_schema::v1::SessionConfigOption>>,
    #[serde(default)]
    pub modes: Vec<agent_client_protocol_schema::v1::SessionMode>,
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
    /// Latest bounded public Agent progress, scoped to this run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress_text: Option<String>,
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub authentication_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(try_from = "AppConfigWire")]
pub struct AppConfig {
    pub agent_preferences: crate::agent_preferences::AgentPreferences,
    pub agent: AgentKind,
    pub working_directory: PathBuf,
    pub agent_prompt_template: AgentPromptTemplate,
}

impl AppConfig {
    /// Application defaults require the host to supply its resolved default directory.
    pub fn new(default_working_directory: PathBuf) -> Self {
        Self {
            agent: AgentKind::Claude,
            agent_preferences: Default::default(),
            working_directory: default_working_directory,
            agent_prompt_template: AgentPromptTemplate::default(),
        }
    }

    /// Persisted settings may omit fields. Their host-dependent default is explicit.
    pub fn decode_settings(
        bytes: &[u8],
        default_working_directory: PathBuf,
    ) -> Result<Self, serde_json::Error> {
        let wire: AppConfigWire = serde_json::from_slice(bytes)?;
        Ok(wire.into_config(default_working_directory))
    }
}

#[derive(Deserialize)]
#[serde(default)]
struct AppConfigWire {
    agent_preferences: crate::agent_preferences::AgentPreferences,
    agent: AgentKind,
    #[serde(deserialize_with = "present_directory")]
    working_directory: Option<PathBuf>,
    agent_prompt_template: Option<AgentPromptTemplate>,
    response_prompt: Option<String>,
}

// A missing setting can use the supplied host default; an explicit null was never a path.
fn present_directory<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<PathBuf>, D::Error> {
    PathBuf::deserialize(deserializer).map(Some)
}

impl Default for AppConfigWire {
    fn default() -> Self {
        Self {
            agent: AgentKind::Claude,
            agent_preferences: Default::default(),
            working_directory: None,
            agent_prompt_template: None,
            response_prompt: None,
        }
    }
}

impl AppConfigWire {
    fn into_config(self, default_working_directory: PathBuf) -> AppConfig {
        let agent_prompt_template = self.agent_prompt_template.unwrap_or_else(|| {
            self.response_prompt
                .as_deref()
                .map(AgentPromptTemplate::from_legacy_response_prompt)
                .unwrap_or_default()
        });
        AppConfig {
            agent: self.agent,
            agent_preferences: self.agent_preferences,
            working_directory: self.working_directory.unwrap_or(default_working_directory),
            agent_prompt_template,
        }
    }
}

impl TryFrom<AppConfigWire> for AppConfig {
    type Error = &'static str;

    fn try_from(wire: AppConfigWire) -> Result<Self, Self::Error> {
        // A current snapshot carries its directory. Legacy persisted input uses decode_settings,
        // where its host fallback is explicit instead of an ambient deserialization side effect.
        let directory = wire
            .working_directory
            .clone()
            .ok_or("missing field `working_directory`")?;
        Ok(wire.into_config(directory))
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
    pub target_set: Option<domain::lens::LensTargetSet>,
    #[serde(default)]
    pub context: Option<domain::lens::LensContext>,
    #[serde(default)]
    pub input: Option<domain::lens::LensInput>,
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
    fn missing_settings_use_only_the_explicit_host_defaults() {
        for directory in ["/host-a", "/host-b"] {
            let default = PathBuf::from(directory);
            assert_eq!(
                AppConfig::decode_settings(b"{}", default.clone()).unwrap(),
                AppConfig::new(default.clone())
            );
            // Serde's existing defaulted preference struct also accepts an empty sequence.
            assert_eq!(
                AppConfig::decode_settings(br#"{"agent_preferences":[]}"#, default.clone())
                    .unwrap(),
                AppConfig::new(default)
            );
        }
        let config = AppConfig::decode_settings(
            br#"{"agent":"codex","working_directory":""}"#,
            PathBuf::from("/host"),
        )
        .unwrap();
        assert_eq!(config.agent, AgentKind::Codex);
        assert_eq!(config.working_directory, PathBuf::new());
        // Existence checks belong to the desktop store, not shared decoding.
        assert_eq!(
            AppConfig::decode_settings(
                br#"{"working_directory":"/explicit/nonexistent"}"#,
                PathBuf::from("/host"),
            )
            .unwrap()
            .working_directory,
            PathBuf::from("/explicit/nonexistent")
        );
    }

    #[test]
    fn settings_preserve_legacy_prompt_precedence_and_literal_braces() {
        let legacy = "Explain {literal} and {{braces}}.";
        let expected = AgentPromptTemplate::from_legacy_response_prompt(legacy);
        for template in [serde_json::Value::Null, serde_json::json!(expected)] {
            let bytes = serde_json::to_vec(&serde_json::json!({
                "response_prompt": legacy,
                "agent_prompt_template": template,
            }))
            .unwrap();
            let config = AppConfig::decode_settings(&bytes, PathBuf::from("/host")).unwrap();
            assert_eq!(config.agent_prompt_template, expected);
            assert!(config.agent_prompt_template.common.contains("{{literal}}"));
        }
        let explicit = AgentPromptTemplate::default();
        let bytes = serde_json::to_vec(&serde_json::json!({
            "response_prompt": legacy,
            "agent_prompt_template": explicit,
        }))
        .unwrap();
        assert_eq!(
            AppConfig::decode_settings(&bytes, PathBuf::from("/host"))
                .unwrap()
                .agent_prompt_template,
            explicit
        );
        assert_eq!(
            AppConfig::decode_settings(
                br#"{"response_prompt":null,"agent_prompt_template":null}"#,
                PathBuf::from("/host"),
            )
            .unwrap(),
            AppConfig::new(PathBuf::from("/host"))
        );
    }

    #[test]
    fn settings_reject_explicit_null_and_invalid_field_types() {
        for bytes in [
            r#"{"working_directory":null}"#,
            r#"{"working_directory":42}"#,
            r#"{"agent":null}"#,
            r#"{"agent":"unknown"}"#,
            r#"{"agent_preferences":null}"#,
            r#"{"agent_preferences":42}"#,
            r#"{"response_prompt":42}"#,
            r#"{"agent_prompt_template":"invalid"}"#,
        ] {
            assert!(
                AppConfig::decode_settings(bytes.as_bytes(), PathBuf::from("/host")).is_err(),
                "unexpectedly accepted {bytes}"
            );
        }
    }

    #[test]
    fn snapshots_round_trip_without_resolving_a_host_directory() {
        let snapshot = AppSnapshot::new(AppConfig::new(PathBuf::from("/snapshot")));
        let value = serde_json::to_value(&snapshot).unwrap();
        let decoded: AppSnapshot = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), value);
        assert!(value["config"].get("response_prompt").is_none());
        assert!(serde_json::from_str::<AppConfig>("{}").is_err());
        let legacy: AppConfig = serde_json::from_value(serde_json::json!({
            "working_directory": "/snapshot",
            "response_prompt": "Legacy instruction",
        }))
        .unwrap();
        assert_eq!(
            legacy.agent_prompt_template,
            AgentPromptTemplate::from_legacy_response_prompt("Legacy instruction")
        );
    }

    #[test]
    fn picker_reply_preserves_every_serialized_window() {
        let reply: WindowPickerReply = serde_json::from_str(
            r#"{
                "status":"selected",
                "windows":[
                    {"operation_id":"00000000-0000-0000-0000-000000000001","receipt":"00000000-0000-0000-0000-000000000009","selection_ordinal":9,"title":"Nine","application_name":"App Z","application_id":"z.example","frame":{"x":0.0,"y":0.0,"width":900.0,"height":700.0}},
                    {"operation_id":"00000000-0000-0000-0000-000000000001","receipt":"00000000-0000-0000-0000-000000000007","selection_ordinal":7,"title":"Seven","application_name":"App A","application_id":"a.example","frame":{"x":10.0,"y":20.0,"width":800.0,"height":600.0}}
                ]
            }"#,
        )
        .expect("valid native multi-window reply");
        let windows = reply
            .into_selected()
            .expect("selected reply")
            .expect("selected windows");

        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].identity.receipt, Uuid::from_u128(9));
        assert_eq!(windows[1].identity.receipt, Uuid::from_u128(7));
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
}
