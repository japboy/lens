//! Persisted Agent defaults contain exact choice IDs and permission-request response policies, never credentials.
use crate::model::AgentKind;
use agent_client_protocol::schema::v1::ToolKind;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolPolicy {
    Ask,
    Allow,
    #[default]
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ToolPolicies {
    pub read: ToolPolicy,
    pub search: ToolPolicy,
    pub fetch: ToolPolicy,
    pub edit: ToolPolicy,
    pub delete: ToolPolicy,
    pub r#move: ToolPolicy,
    pub execute: ToolPolicy,
}
impl Default for ToolPolicies {
    fn default() -> Self {
        Self {
            read: ToolPolicy::Ask,
            search: ToolPolicy::Ask,
            fetch: ToolPolicy::Ask,
            edit: ToolPolicy::Deny,
            delete: ToolPolicy::Deny,
            r#move: ToolPolicy::Deny,
            execute: ToolPolicy::Deny,
        }
    }
}
impl ToolPolicies {
    pub fn for_kind(&self, kind: ToolKind) -> ToolPolicy {
        match kind {
            ToolKind::Read => self.read,
            ToolKind::Search => self.search,
            ToolKind::Fetch => self.fetch,
            ToolKind::Edit => self.edit,
            ToolKind::Delete => self.delete,
            ToolKind::Move => self.r#move,
            ToolKind::Execute => self.execute,
            _ => ToolPolicy::Ask,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SavedChoice {
    pub config_id: String,
    pub value: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct AgentDefaults {
    pub choices: Vec<SavedChoice>,
    pub tools: ToolPolicies,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct AgentPreferences {
    pub claude: AgentDefaults,
    pub codex: AgentDefaults,
}
impl AgentPreferences {
    pub fn get(&self, kind: AgentKind) -> &AgentDefaults {
        match kind {
            AgentKind::Claude => &self.claude,
            AgentKind::Codex => &self.codex,
        }
    }
    pub fn set(&mut self, kind: AgentKind, defaults: AgentDefaults) {
        match kind {
            AgentKind::Claude => self.claude = defaults,
            AgentKind::Codex => self.codex = defaults,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_round_trip_without_changing_other_agents_or_legacy_policy() {
        let mut preferences: AgentPreferences = serde_json::from_str("{}").unwrap();
        let legacy = preferences.codex.clone();
        preferences.set(
            AgentKind::Claude,
            AgentDefaults {
                choices: vec![SavedChoice {
                    config_id: "effort".into(),
                    value: "high".into(),
                }],
                tools: ToolPolicies {
                    execute: ToolPolicy::Allow,
                    ..ToolPolicies::default()
                },
            },
        );
        let encoded = serde_json::to_string(&preferences).unwrap();
        let restored: AgentPreferences = serde_json::from_str(&encoded).unwrap();
        assert_eq!(restored, preferences);
        assert_eq!(restored.codex, legacy);
        assert_eq!(restored.claude.tools.execute, ToolPolicy::Allow);
        assert_eq!(
            restored.claude.tools.for_kind(ToolKind::Other),
            ToolPolicy::Ask
        );
        assert_eq!(restored.codex.tools.execute, ToolPolicy::Deny);
        assert!(serde_json::from_str::<ToolPolicies>(r#"{"read":"allow_always"}"#).is_err());
    }
}
