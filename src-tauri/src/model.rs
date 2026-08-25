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
    pub kind: AgentKind,
    pub adapter_name: String,
    pub adapter_version: String,
    #[serde(default)]
    pub session_id: Option<String>,
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
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            agent: AgentKind::Claude,
            working_directory: dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")),
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
    pub transformed_text: Option<String>,
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
            transformed_text: None,
            agent: None,
            error: None,
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
}
