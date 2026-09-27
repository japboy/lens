//! Persisted Agent defaults contain exact choice IDs and permission-request response policies, never credentials.
use crate::model::AgentKind;
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
    pub other: ToolPolicy,
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
            other: ToolPolicy::Ask,
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
    pub antigravity: AgentDefaults,
    pub external: std::collections::BTreeMap<uuid::Uuid, AgentDefaults>,
}
impl AgentPreferences {
    pub fn get(&self, kind: AgentKind) -> &AgentDefaults {
        match kind {
            AgentKind::Claude => &self.claude,
            AgentKind::Codex => &self.codex,
            AgentKind::Antigravity => &self.antigravity,
            AgentKind::External(id) => self.external.get(&id).unwrap_or_else(|| {
                static DEFAULT: std::sync::LazyLock<AgentDefaults> =
                    std::sync::LazyLock::new(AgentDefaults::default);
                &DEFAULT
            }),
        }
    }
    pub fn set(&mut self, kind: AgentKind, defaults: AgentDefaults) {
        match kind {
            AgentKind::Claude => self.claude = defaults,
            AgentKind::Codex => self.codex = defaults,
            AgentKind::Antigravity => self.antigravity = defaults,
            AgentKind::External(id) => {
                self.external.insert(id, defaults);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_preferences_gain_isolated_antigravity_defaults() {
        let external_id = uuid::Uuid::from_u128(1);
        let legacy = serde_json::json!({
            "claude": {"choices":[{"config_id":"model","value":"claude-model"}]},
            "codex": {"tools":{"execute":"allow"}},
            "external": {external_id.to_string(): {"tools":{"edit":"allow"}}}
        });
        let mut preferences: AgentPreferences = serde_json::from_value(legacy).unwrap();
        assert_eq!(preferences.antigravity, AgentDefaults::default());
        let before = preferences.clone();
        preferences.set(
            AgentKind::Antigravity,
            AgentDefaults {
                choices: vec![SavedChoice {
                    config_id: "model".into(),
                    value: "gemini-model".into(),
                }],
                ..Default::default()
            },
        );
        let restored: AgentPreferences =
            serde_json::from_value(serde_json::to_value(&preferences).unwrap()).unwrap();
        assert_eq!(restored, preferences);
        assert_eq!(restored.claude, before.claude);
        assert_eq!(restored.codex, before.codex);
        assert_eq!(restored.external, before.external);
        assert_eq!(
            restored.get(AgentKind::Antigravity).choices[0].value,
            "gemini-model"
        );
    }
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
        assert_eq!(restored.codex.tools.execute, ToolPolicy::Deny);
        assert!(serde_json::from_str::<ToolPolicies>(r#"{"read":"allow_always"}"#).is_err());
    }
    #[test]
    fn existing_tool_policies_without_other_default_to_ask() {
        let policies: ToolPolicies =
            serde_json::from_value(serde_json::json!({"execute":"deny"})).unwrap();
        assert_eq!(policies.other, ToolPolicy::Ask);
        assert_eq!(policies.execute, ToolPolicy::Deny);
    }
}
