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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    Claude,
    Codex,
    External(Uuid),
}

impl AgentKind {
    pub fn is_external(self) -> bool {
        matches!(self, Self::External(_))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalAgentProfile {
    pub id: Uuid,
    pub name: String,
    pub command: PathBuf,
    pub args: Vec<String>,
}

impl ExternalAgentProfile {
    pub fn bundled_presets() -> Vec<Self> {
        vec![Self::copilot_preset(), Self::goose_preset()]
    }

    pub fn copilot_preset() -> Self {
        Self {
            id: Uuid::from_u128(0xa14d73cb951c48eda3053829750c88da),
            name: "GitHub Copilot".into(),
            command: "copilot".into(),
            args: vec!["--acp".into(), "--stdio".into()],
        }
    }
    pub fn goose_preset() -> Self {
        Self {
            id: Uuid::from_u128(0x6b57315e9c134e4abf4ce6bc33b10b21),
            name: "Goose".into(),
            command: "goose".into(),
            args: vec!["acp".into()],
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_nil()
            || self.name.trim().is_empty()
            || self.name.len() > 128
            || self.name.contains('\0')
        {
            return Err("Provide a profile ID and a name of at most 128 bytes.".into());
        }
        if self.command.as_os_str().is_empty()
            || self.command.to_string_lossy().contains('\0')
            || (!self.command.is_absolute()
                && (self.command.components().count() != 1
                    || !matches!(
                        self.command.components().next(),
                        Some(std::path::Component::Normal(_))
                    )
                    || self.command.to_string_lossy().contains('/')))
        {
            return Err(
                "Use an executable name from PATH or an absolute path, without NUL.".into(),
            );
        }
        if self.args.len() > 64
            || self.args.iter().map(String::len).sum::<usize>() > 16384
            || self.args.iter().any(|a| a.contains('\0'))
        {
            return Err("Arguments must contain no NUL, at most 64 values and 16384 bytes.".into());
        }
        Ok(())
    }
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
    /// Last verified, confirmed managed installation, independent of an update candidate.
    #[serde(default)]
    pub current_version: Option<String>,
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
            current_version: None,
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
    /// Provider chosen by history; live authentication/readiness has not been verified.
    HistorySelected,
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
    pub agent_default: Option<String>,
    #[serde(default)]
    pub operation_id: Option<Uuid>,
    pub stage: AgentSelectionStage,
    #[serde(default)]
    pub candidate: Option<AgentKind>,
    #[serde(default)]
    pub auth_methods: Vec<AgentAuthMethod>,
    #[serde(default)]
    pub supports_logout: bool,
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
            agent_default: None,
            candidate: None,
            auth_methods: Vec::new(),
            supports_logout: false,
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
    pub external_agents: Vec<ExternalAgentProfile>,
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
            external_agents: ExternalAgentProfile::bundled_presets(),
            agent_prompt_template: prompt_presets.selected().template.clone(),
            prompt_presets,
        }
    }

    pub fn reset_external_agents(&mut self) {
        self.external_agents = ExternalAgentProfile::bundled_presets();
        self.agent_preferences
            .external
            .retain(|id, _| self.external_agents.iter().any(|profile| profile.id == *id));
        if self.agent.is_external() {
            self.agent = AgentKind::Claude;
        }
    }

    pub fn sync_prompt_template(&mut self) -> Result<(), String> {
        self.prompt_presets = self.prompt_presets.clone().normalize()?;
        self.agent_prompt_template = self.prompt_presets.selected().template.clone();
        Ok(())
    }

    pub fn same_agent_execution(&self, other: &Self, agent: AgentKind) -> bool {
        self.agent_preferences.get(agent) == other.agent_preferences.get(agent)
            && match agent {
                AgentKind::External(id) => {
                    let left = self.external_agents.iter().find(|p| p.id == id);
                    let right = other.external_agents.iter().find(|p| p.id == id);
                    match (left, right) {
                        (Some(a), Some(b)) => a.command == b.command && a.args == b.args,
                        (None, None) => false,
                        _ => false,
                    }
                }
                _ => true,
            }
    }
    pub fn same_execution_config(&self, other: &Self) -> bool {
        self.prompt_presets.execution_revision == other.prompt_presets.execution_revision
            && self.agent == other.agent
            && self.same_agent_execution(other, self.agent)
            && self.working_directory == other.working_directory
            && self.agent_prompt_template == other.agent_prompt_template
    }

    pub fn settings_require_prompt_migration(bytes: &[u8]) -> Result<bool, serde_json::Error> {
        let value: serde_json::Value = serde_json::from_slice(bytes)?;
        Ok(current_catalog_needs_selection(&value["prompt_presets"])
            || catalog_has_legacy_ids(&value["prompt_presets"]))
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
    #[serde(deserialize_with = "present_external_agents")]
    external_agents: Option<Vec<ExternalAgentProfile>>,
    #[serde(deserialize_with = "present_prompt_presets")]
    prompt_presets: Option<PromptPresetCatalog>,
}

fn present_external_agents<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<ExternalAgentProfile>>, D::Error> {
    Vec::<ExternalAgentProfile>::deserialize(deserializer).map(Some)
}

fn catalog_has_legacy_ids(value: &serde_json::Value) -> bool {
    value["schema_version"] == 2
        && value["presets"].as_array().is_some_and(|presets| {
            presets.iter().any(|p| {
                crate::prompt_presets::LEGACY_PRESET_IDS
                    .iter()
                    .any(|(old, _)| p["id"].as_str() == Some(*old))
            })
        })
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
    if current_catalog_needs_selection(&value) {
        value["selected_id"] = value["presets"][0]["id"].clone();
    }
    match value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
    {
        Some(1) => Err(serde::de::Error::custom(
            "Legacy prompt catalog requires recovery; saved content cannot be replaced automatically.",
        )),
        Some(2) => serde_json::from_value::<PromptPresetCatalog>(value)
            .map(|catalog| Some(catalog.migrate_legacy_ids()))
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
            external_agents: None,
            prompt_presets: None,
        }
    }
}

impl AppConfigWire {
    fn into_config(self, default_working_directory: PathBuf) -> Result<AppConfig, String> {
        let external_agents = self.external_agents.unwrap_or_default();
        if external_agents.len() > 16 {
            return Err("At most 16 external Agent profiles are supported.".into());
        }
        let mut ids = std::collections::HashSet::new();
        for profile in &external_agents {
            profile.validate()?;
            if !ids.insert(profile.id) {
                return Err("External Agent profile IDs must be unique.".into());
            }
        }
        if let AgentKind::External(id) = self.agent {
            if !ids.contains(&id) {
                return Err("Selected external Agent profile does not exist.".into());
            }
        }
        if self
            .agent_preferences
            .external
            .keys()
            .any(|id| !ids.contains(id))
        {
            return Err("External Agent defaults reference an unknown profile.".into());
        }
        let prompt_presets = self.prompt_presets.ok_or(
            "Saved settings have no prompt catalog. Restore or explicitly recreate the catalog.",
        )?.normalize()?;
        Ok(AppConfig {
            agent: self.agent,
            external_agents,
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
    fn existing_settings_without_recoverable_prompts_do_not_receive_defaults() {
        for value in [
            serde_json::json!({}),
            serde_json::json!({"response_prompt":"old literal {text}"}),
            serde_json::json!({"prompt_presets":{"schema_version":1,"presets":[{"description":"old"}]}}),
            serde_json::json!({"prompt_presets":{"schema_version":2,"revision":1,"execution_revision":1,"selected_id":"gone","presets":[]}}),
        ] {
            let bytes = serde_json::to_vec(&value).unwrap();
            assert!(!AppConfig::settings_require_prompt_migration(&bytes).unwrap());
            assert!(AppConfig::decode_settings(&bytes, PathBuf::from("/host")).is_err());
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
    fn selection_repair_preserves_every_saved_record_and_revision() {
        let mut original = AppConfig::new("/fixture".into());
        original.prompt_presets.presets.truncate(2);
        original.prompt_presets.presets[0].name = "Saved custom title".into();
        original.prompt_presets.presets[0]
            .template
            .common
            .push_str("\nRetain this instruction.");
        original.prompt_presets.revision = 9;
        original.prompt_presets.execution_revision = 4;
        original.sync_prompt_template().unwrap();
        for missing in [true, false] {
            let mut saved = serde_json::to_value(&original).unwrap();
            if missing {
                saved["prompt_presets"]
                    .as_object_mut()
                    .unwrap()
                    .remove("selected_id");
            } else {
                saved["prompt_presets"]["selected_id"] = "removed".into();
            }
            let bytes = serde_json::to_vec(&saved).unwrap();
            assert!(AppConfig::settings_require_prompt_migration(&bytes).unwrap());
            let repaired = AppConfig::decode_settings(&bytes, "/other".into()).unwrap();
            assert_eq!(repaired, original);
            assert!(!AppConfig::settings_require_prompt_migration(
                &serde_json::to_vec(&repaired).unwrap()
            )
            .unwrap());
        }
    }

    #[test]
    fn legacy_ids_migrate_without_changing_content_and_are_idempotent() {
        for selected in ["conceptual", "practical", "analytical"] {
            let expected = AppConfig::new(PathBuf::from("/host"));
            let mut expected = expected;
            expected.prompt_presets.selected_id = selected.into();
            expected.prompt_presets.presets[0].name = "My edited preset".into();
            expected.prompt_presets.presets[0]
                .template
                .common
                .push_str("\nKeep my instructions.");
            expected.sync_prompt_template().unwrap();
            let mut old = serde_json::to_value(&expected).unwrap();
            for (legacy, current) in crate::prompt_presets::LEGACY_PRESET_IDS {
                for preset in old["prompt_presets"]["presets"].as_array_mut().unwrap() {
                    if preset["id"] == current {
                        preset["id"] = legacy.into();
                        preset["bundled_source"]["id"] = legacy.into();
                    }
                }
                if old["prompt_presets"]["selected_id"] == current {
                    old["prompt_presets"]["selected_id"] = legacy.into();
                }
            }
            let bytes = serde_json::to_vec(&old).unwrap();
            assert!(AppConfig::settings_require_prompt_migration(&bytes).unwrap());
            let actual = AppConfig::decode_settings(&bytes, PathBuf::from("/other")).unwrap();
            assert_eq!(actual, expected);
            let saved = serde_json::to_vec(&actual).unwrap();
            assert!(!AppConfig::settings_require_prompt_migration(&saved).unwrap());
            assert_eq!(
                AppConfig::decode_settings(&saved, PathBuf::from("/other")).unwrap(),
                actual
            );
        }
    }

    #[test]
    fn legacy_id_collision_preserves_both_records_and_selected_content() {
        let mut value = serde_json::to_value(AppConfig::new(PathBuf::from("/host"))).unwrap();
        let mut legacy = value["prompt_presets"]["presets"][0].clone();
        legacy["id"] = "conceptual-learner".into();
        legacy["bundled_source"]["id"] = "conceptual-learner".into();
        legacy["name"] = "Legacy customization".into();
        value["prompt_presets"]["presets"]
            .as_array_mut()
            .unwrap()
            .push(legacy);
        value["prompt_presets"]["selected_id"] = "conceptual-learner".into();
        let config = AppConfig::decode_settings(
            &serde_json::to_vec(&value).unwrap(),
            PathBuf::from("/host"),
        )
        .unwrap();
        assert_eq!(config.prompt_presets.presets.len(), 5);
        assert_eq!(config.prompt_presets.selected_id, "conceptual-migrated-1");
        assert_eq!(
            config.prompt_presets.selected().name,
            "Legacy customization"
        );
        assert_eq!(config.prompt_presets.selected().bundled_source, None);
        assert_eq!(config.prompt_presets.presets[0].id, "conceptual");
    }

    #[test]
    fn older_bundle_contents_are_preserved_until_an_explicit_reset() {
        for version in [1, 2, 3, 4, 5, 6] {
            let mut config = AppConfig::new(PathBuf::from("/host"));
            if version < 6 {
                config
                    .prompt_presets
                    .presets
                    .retain(|preset| preset.id != "evocative");
            }
            if version < 4 {
                let mut retired = config.prompt_presets.presets[0].clone();
                retired.id = "visual-learner".into();
                retired.bundled_source.as_mut().unwrap().id = retired.id.clone();
                config.prompt_presets.selected_id = retired.id.clone();
                config.prompt_presets.presets.insert(0, retired);
            }
            if version == 1 {
                config.prompt_presets.presets.truncate(3);
            }
            let names: &[&str] = if version >= 6 {
                &["Conceptual", "Practical", "Analytical", "Evocative"]
            } else if version >= 4 {
                &["Conceptual", "Practical", "Analytical"]
            } else {
                &[
                    "Visual Learner",
                    "Conceptual Learner",
                    "Practical Learner",
                    "Analytical Learner",
                ]
            };
            for (preset, &name) in config.prompt_presets.presets.iter_mut().zip(names) {
                preset.name = if version >= 3 {
                    if name == "Visual Learner" {
                        "Infographic".into()
                    } else {
                        name.trim_end_matches(" Learner").into()
                    }
                } else {
                    name.into()
                };
                preset.bundled_source.as_mut().unwrap().version = version;
                preset.template.common =
                    "{turn_instruction}\nPreviously saved instructions.".into();
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
            assert_eq!(reset.selected_id, "conceptual");
            assert!(!reset
                .presets
                .iter()
                .any(|preset| preset.id == "visual-learner"));
            for (actual, seed) in reset
                .presets
                .iter()
                .zip(crate::prompt_presets::bundled_presets())
            {
                assert_eq!(actual.name, seed.name);
                assert_eq!(actual.template, seed.template);
                assert_eq!(actual.bundled_source, seed.bundled_source);
            }
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
            AgentSelectionStage::HistorySelected,
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
    fn external_executable_is_part_of_execution_identity_and_legacy_defaults() {
        let mut config = AppConfig::new("/fixture".into());
        config.external_agents.clear();
        let previous = config.clone();
        config.external_agents.push(ExternalAgentProfile {
            id: Uuid::from_u128(1),
            name: "Custom".into(),
            command: "/user/agent".into(),
            args: vec![],
        });
        assert!(!config
            .same_agent_execution(&previous, AgentKind::External(config.external_agents[0].id)));
        let mut saved = serde_json::to_value(AppConfig::new("/fixture".into())).unwrap();
        saved.as_object_mut().unwrap().remove("external_agents");
        let decoded =
            AppConfig::decode_settings(&serde_json::to_vec(&saved).unwrap(), "/fixture".into())
                .unwrap();
        assert!(decoded.external_agents.is_empty());
        assert!(decoded.agent_preferences.external.is_empty());
    }
    #[test]
    fn external_profiles_preserve_managed_wire_and_reject_invalid_catalogs() {
        assert_eq!(serde_json::to_value(AgentKind::Claude).unwrap(), "claude");
        assert_eq!(serde_json::to_value(AgentKind::Codex).unwrap(), "codex");
        let id = Uuid::from_u128(1);
        let profile = ExternalAgentProfile {
            id,
            name: "Custom".into(),
            command: "/bin/agent".into(),
            args: vec!["".into(), "argument with spaces".into()],
        };
        let mut config = AppConfig::new("/fixture".into());
        config.external_agents = vec![profile.clone()];
        config.agent = AgentKind::External(id);
        let wire = serde_json::to_value(&config).unwrap();
        assert_eq!(wire["agent"]["external"], id.to_string());
        assert_eq!(
            AppConfig::decode_settings(&serde_json::to_vec(&wire).unwrap(), "/fixture".into())
                .unwrap(),
            config
        );
        for profiles in [vec![profile.clone(); 2], vec![profile.clone(); 17], vec![]] {
            let mut invalid = wire.clone();
            invalid["external_agents"] = serde_json::to_value(profiles).unwrap();
            assert!(AppConfig::decode_settings(
                &serde_json::to_vec(&invalid).unwrap(),
                "/fixture".into()
            )
            .is_err());
        }
        let mut renamed = config.clone();
        renamed.external_agents[0].name = "Renamed".into();
        assert!(config.same_execution_config(&renamed));
        renamed.external_agents[0].args.push("--changed".into());
        assert!(!config.same_execution_config(&renamed));
    }
    #[test]
    fn initial_external_profiles_have_stable_distinct_ids_and_literal_commands() {
        let goose = ExternalAgentProfile::goose_preset();
        let copilot = ExternalAgentProfile::copilot_preset();
        assert_eq!(goose.id.to_string(), "6b57315e-9c13-4e4a-bf4c-e6bc33b10b21");
        assert_eq!(
            copilot.id.to_string(),
            "a14d73cb-951c-48ed-a305-3829750c88da"
        );
        assert_ne!(goose.id, copilot.id);
        assert_eq!(copilot.name, "GitHub Copilot");
        assert_eq!(copilot.command, PathBuf::from("copilot"));
        assert_eq!(copilot.args, ["--acp", "--stdio"]);
        for profile in [goose, copilot] {
            profile.validate().unwrap();
        }
    }

    #[test]
    fn saved_external_profiles_are_never_augmented_with_initial_presets() {
        let initial = AppConfig::new("/fixture".into());
        let mut customized = ExternalAgentProfile::copilot_preset();
        customized.name = "My connection".into();
        customized.command = "/custom/agent".into();
        customized.args = vec!["custom-acp".into()];
        for profiles in [
            vec![
                ExternalAgentProfile::goose_preset(),
                ExternalAgentProfile::copilot_preset(),
            ],
            vec![ExternalAgentProfile::goose_preset()],
            vec![ExternalAgentProfile::copilot_preset()],
            vec![customized],
            vec![],
        ] {
            let mut saved = initial.clone();
            saved.external_agents = profiles;
            let loaded =
                AppConfig::decode_settings(&serde_json::to_vec(&saved).unwrap(), "/other".into())
                    .unwrap();
            assert_eq!(loaded, saved);
        }
    }

    #[test]
    fn explicit_external_reset_restores_the_declared_preset_order() {
        let mut config = AppConfig::new("/fixture".into());
        config.external_agents.reverse();
        config.reset_external_agents();
        assert_eq!(
            config.external_agents,
            vec![
                ExternalAgentProfile::copilot_preset(),
                ExternalAgentProfile::goose_preset(),
            ]
        );
    }

    #[test]
    fn only_new_settings_seed_external_profiles_and_legacy_markers_are_inert() {
        let initial = AppConfig::new("/fixture".into());
        assert_eq!(
            initial.external_agents,
            vec![
                ExternalAgentProfile::copilot_preset(),
                ExternalAgentProfile::goose_preset(),
            ]
        );
        for marker in [None, Some(0), Some(1), Some(99)] {
            for missing in [true, false] {
                let mut saved = serde_json::to_value(&initial).unwrap();
                if missing {
                    saved.as_object_mut().unwrap().remove("external_agents");
                } else {
                    saved["external_agents"] = serde_json::json!([]);
                }
                if let Some(version) = marker {
                    saved["external_agent_presets_version"] = version.into();
                }
                let loaded = AppConfig::decode_settings(
                    &serde_json::to_vec(&saved).unwrap(),
                    "/other".into(),
                )
                .unwrap();
                assert!(loaded.external_agents.is_empty());
                assert_eq!(loaded.prompt_presets, initial.prompt_presets);
                let saved_again = serde_json::to_value(&loaded).unwrap();
                assert!(saved_again.get("external_agent_presets_version").is_none());
                assert!(AppConfig::decode_settings(
                    &serde_json::to_vec(&saved_again).unwrap(),
                    "/other".into()
                )
                .unwrap()
                .external_agents
                .is_empty());
            }
        }
    }
}
