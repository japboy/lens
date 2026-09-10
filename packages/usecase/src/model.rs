use crate::live_sync::ProjectionRef;
use crate::prompt_presets::PromptPresetCatalog;
pub use domain::model::{
    Bounds, ExtractionMetrics, ExtractionQuality, ExtractionResult, LensOutputBlock,
    ResourceReference, SelectedWindow, WindowIdentity, WindowObservableFacts,
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
    pub prompt_presets: PromptPresetCatalog,
}

impl AppConfig {
    /// Application defaults require the host to supply its resolved default directory.
    pub fn new(default_working_directory: PathBuf) -> Self {
        let prompt_presets = PromptPresetCatalog::default();
        Self {
            agent: AgentKind::Claude,
            agent_preferences: Default::default(),
            working_directory: default_working_directory,
            agent_prompt_template: prompt_presets.selected().template.clone(),
            prompt_presets,
        }
    }

    pub fn sync_prompt_template(&mut self) -> Result<(), String> {
        self.prompt_presets = self.prompt_presets.clone().normalize()?;
        self.agent_prompt_template = self.prompt_presets.selected().template.clone();
        Ok(())
    }

    pub fn same_execution_config(&self, other: &Self) -> bool {
        self.prompt_presets.execution_revision == other.prompt_presets.execution_revision
            && self.agent == other.agent
            && self.agent_preferences == other.agent_preferences
            && self.working_directory == other.working_directory
            && self.agent_prompt_template == other.agent_prompt_template
    }

    pub fn settings_require_prompt_migration(bytes: &[u8]) -> Result<bool, serde_json::Error> {
        let value: serde_json::Value = serde_json::from_slice(bytes)?;
        Ok(value.get("prompt_presets").is_none()
            || value["prompt_presets"]["schema_version"] == 1
            || current_catalog_has_no_records(&value["prompt_presets"])
            || current_catalog_needs_selection(&value["prompt_presets"]))
    }

    /// Persisted settings may omit fields. Their host-dependent default is explicit.
    pub fn decode_settings(
        bytes: &[u8],
        default_working_directory: PathBuf,
    ) -> Result<Self, serde_json::Error> {
        let wire: AppConfigWire = serde_json::from_slice(bytes)?;
        wire.into_config(default_working_directory)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Deserialize)]
#[serde(default)]
struct AppConfigWire {
    agent_preferences: crate::agent_preferences::AgentPreferences,
    agent: AgentKind,
    #[serde(deserialize_with = "present_directory")]
    working_directory: Option<PathBuf>,
    #[serde(deserialize_with = "present_prompt_presets")]
    prompt_presets: Option<PromptPresetCatalog>,
}

fn current_catalog_has_no_records(value: &serde_json::Value) -> bool {
    value["schema_version"] == 2
        && (value.get("presets").is_none()
            || value
                .get("presets")
                .and_then(serde_json::Value::as_array)
                .is_some_and(Vec::is_empty))
}

fn current_catalog_needs_selection(value: &serde_json::Value) -> bool {
    value["schema_version"] == 2
        && value
            .get("presets")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|presets| {
                !presets.is_empty()
                    && (value.get("selected_id").is_none()
                        || value["selected_id"].as_str().is_some_and(|selected| {
                            !presets
                                .iter()
                                .any(|preset| preset["id"].as_str() == Some(selected))
                        }))
            })
}

fn present_prompt_presets<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<PromptPresetCatalog>, D::Error> {
    let mut value = serde_json::Value::deserialize(deserializer)?;
    if current_catalog_has_no_records(&value) {
        let defaults = PromptPresetCatalog::default();
        value["presets"] =
            serde_json::to_value(&defaults.presets).map_err(serde::de::Error::custom)?;
        value["selected_id"] = serde_json::Value::String(defaults.selected_id.clone());
        serde_json::from_value::<PromptPresetCatalog>(value)
            .map_err(serde::de::Error::custom)?
            .normalize()
            .map_err(serde::de::Error::custom)?;
        return Ok(Some(defaults));
    }
    if current_catalog_needs_selection(&value) {
        value["selected_id"] = value["presets"][0]["id"].clone();
    }
    match value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
    {
        Some(1) => Ok(Some(PromptPresetCatalog::default())),
        Some(2) => serde_json::from_value::<PromptPresetCatalog>(value)
            .map(Some)
            .map_err(serde::de::Error::custom),
        _ => Err(serde::de::Error::custom(
            "unsupported prompt preset catalog schema",
        )),
    }
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
            prompt_presets: None,
        }
    }
}

impl AppConfigWire {
    fn into_config(self, default_working_directory: PathBuf) -> Result<AppConfig, String> {
        let prompt_presets = self.prompt_presets.unwrap_or_default().normalize()?;
        Ok(AppConfig {
            agent: self.agent,
            agent_preferences: self.agent_preferences,
            working_directory: self.working_directory.unwrap_or(default_working_directory),
            agent_prompt_template: prompt_presets.selected().template.clone(),
            prompt_presets,
        })
    }
}

impl TryFrom<AppConfigWire> for AppConfig {
    type Error = String;

    fn try_from(wire: AppConfigWire) -> Result<Self, Self::Error> {
        // A current snapshot carries its directory. Legacy persisted input uses decode_settings,
        // where its host fallback is explicit instead of an ambient deserialization side effect.
        let directory = wire
            .working_directory
            .clone()
            .ok_or("missing field `working_directory`")?;
        wire.into_config(directory)
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
    #[serde(default = "initial_prompt_execution_revision")]
    pub prompt_execution_revision: u32,
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
    #[serde(default = "initial_prompt_execution_revision")]
    pub prompt_execution_revision: u32,
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

fn initial_prompt_execution_revision() -> u32 {
    1
}

impl Default for LensState {
    fn default() -> Self {
        Self {
            prompt_execution_revision: initial_prompt_execution_revision(),
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
    fn settings_upgrade_prompt_schema_once_and_preserve_other_settings() {
        for legacy in [
            serde_json::json!({}),
            serde_json::json!({"response_prompt":"old literal {text}"}),
            serde_json::json!({"prompt_presets":{"schema_version":1,"presets":[{"description":"old"}]}}),
        ] {
            let mut value = legacy;
            value["agent"] = "codex".into();
            value["working_directory"] = "/explicit".into();
            let bytes = serde_json::to_vec(&value).unwrap();
            assert!(AppConfig::settings_require_prompt_migration(&bytes).unwrap());
            let config = AppConfig::decode_settings(&bytes, PathBuf::from("/host")).unwrap();
            assert_eq!(config.agent, AgentKind::Codex);
            assert_eq!(config.working_directory, PathBuf::from("/explicit"));
            assert_eq!(config.prompt_presets, PromptPresetCatalog::default());
            assert_eq!(
                config.agent_prompt_template,
                config.prompt_presets.presets[0].template
            );
        }
        let mut config = AppConfig::new(PathBuf::from("/host"));
        config.prompt_presets.presets[0].name = "My visual".into();
        let bytes = serde_json::to_vec(&config).unwrap();
        assert!(!AppConfig::settings_require_prompt_migration(&bytes).unwrap());
        assert_eq!(
            AppConfig::decode_settings(&bytes, PathBuf::from("/other")).unwrap(),
            config
        );
    }

    #[test]
    fn older_bundle_contents_are_preserved_until_an_explicit_reset() {
        let mut config = AppConfig::new(PathBuf::from("/host"));
        config.prompt_presets.presets.truncate(3);
        for preset in &mut config.prompt_presets.presets {
            preset.bundled_source.as_mut().unwrap().version = 1;
            preset
                .template
                .common
                .push_str("\nPreviously saved instructions.");
        }
        config.sync_prompt_template().unwrap();
        let bytes = serde_json::to_vec(&config).unwrap();
        assert!(!AppConfig::settings_require_prompt_migration(&bytes).unwrap());
        let loaded = AppConfig::decode_settings(&bytes, PathBuf::from("/other")).unwrap();
        assert_eq!(loaded, config);
        let reset = loaded
            .prompt_presets
            .apply(crate::prompt_presets::PromptPresetMutation::ResetAll {
                expected_catalog_revision: loaded.prompt_presets.revision,
            })
            .unwrap();
        assert_eq!(reset.presets.len(), 4);
        for (actual, seed) in reset
            .presets
            .iter()
            .zip(crate::prompt_presets::bundled_presets())
        {
            assert_eq!(actual.template, seed.template);
            assert_eq!(actual.bundled_source, seed.bundled_source);
        }
    }

    #[test]
    fn settings_reject_invalid_current_catalog_and_nonprompt_fields() {
        for value in [
            serde_json::json!({"working_directory":null}),
            serde_json::json!({"agent":"unknown"}),
            serde_json::json!({"prompt_presets":null}),
            serde_json::json!({"prompt_presets":{"schema_version":3}}),
            serde_json::json!({"prompt_presets":{"schema_version":2}}),
            serde_json::json!({"prompt_presets":{"schema_version":2,"revision":0,"execution_revision":1,"presets":[]}}),
            serde_json::json!({"prompt_presets":{"schema_version":2,"revision":1,"execution_revision":1,"presets":null}}),
        ] {
            assert!(AppConfig::decode_settings(
                &serde_json::to_vec(&value).unwrap(),
                PathBuf::from("/host")
            )
            .is_err());
        }
        assert!(serde_json::from_str::<AppConfig>("{}").is_err());
        let snapshot = AppSnapshot::new(AppConfig::new(PathBuf::from("/snapshot")));
        let decoded: AppSnapshot =
            serde_json::from_slice(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
        assert_eq!(decoded, snapshot);
    }

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
}
