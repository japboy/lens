//! Session values and bounded selector decisions; no responders, channels or transport.
use agent_client_protocol_schema::v1::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

const MAX_CONFIG_BYTES: usize = 128 * 1024;

/// Saved response policy selects only an unambiguous one-shot option, never a persistent grant.
pub fn permission_response(
    policy: crate::agent_preferences::ToolPolicy,
    options: &[PermissionOption],
) -> Option<RequestPermissionOutcome> {
    use crate::agent_preferences::ToolPolicy;
    let desired = match policy {
        ToolPolicy::Ask => return None,
        ToolPolicy::Allow => PermissionOptionKind::AllowOnce,
        ToolPolicy::Deny => PermissionOptionKind::RejectOnce,
    };
    let mut matching = options.iter().filter(|option| option.kind == desired);
    match (matching.next(), matching.next()) {
        (Some(option), None) => Some(RequestPermissionOutcome::Selected(
            SelectedPermissionOutcome::new(option.option_id.clone()),
        )),
        _ if policy == ToolPolicy::Deny => Some(RequestPermissionOutcome::Cancelled),
        _ => None,
    }
}

pub fn confirm_choice(options: &[SessionConfigOption], id: &str, value: &str) -> Result<(), Error> {
    let option = options
        .iter()
        .find(|o| o.id.to_string() == id)
        .ok_or_else(|| invalid("Agent removed the requested selector"))?;
    if current_value(option)? != value {
        return Err(invalid("Agent did not confirm the requested value"));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InteractionStatus {
    Pending,
    Accepted,
    Declined,
    Cancelled,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InteractionDetails {
    ModeTransition {
        from: String,
        to: String,
    },
    Form {
        message: String,
        schema: serde_json::Value,
    },
    Url {
        message: String,
        elicitation_id: String,
        url: String,
    },
    Permission {
        tool_call_id: String,
        title: String,
        effect: String,
        arguments: serde_json::Value,
        options: Vec<PermissionOption>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentInteraction {
    pub id: Uuid,
    pub run_id: Option<Uuid>,
    pub sequence: u32,
    pub status: InteractionStatus,
    pub details: Option<InteractionDetails>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChangeStatus {
    Pending,
    Succeeded,
    Rejected,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConfigChange {
    pub config_id: String,
    pub value: String,
    pub status: ChangeStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModeOrigin {
    Policy,
    User,
    Agent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentSessionControlState {
    pub instance_id: Uuid,
    pub operation_id: Uuid,
    pub session_id: String,
    pub agent_name: String,
    pub active: bool,
    pub config_revision: u32,
    pub config_options: Option<Vec<SessionConfigOption>>,
    pub modes: Vec<SessionMode>,
    pub effective_mode: String,
    pub configured_mode: String,
    pub configured_origin: ModeOrigin,
    pub last_mode_origin: ModeOrigin,
    pub policy_default: String,
    pub change: Option<ConfigChange>,
    pub notice: Option<String>,
    pub interactions: Vec<AgentInteraction>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum InteractionResponse {
    Accept,
    Decline,
    Cancel,
    Select { option_id: String },
    Submit { content: serde_json::Value },
}

fn invalid(message: &str) -> Error {
    Error::invalid_params().data(message)
}

pub fn mode_option(options: &[SessionConfigOption]) -> Result<Option<&SessionConfigOption>, Error> {
    let mut modes = options
        .iter()
        .filter(|o| o.category == Some(SessionConfigOptionCategory::Mode));
    let result = modes.next();
    if modes.next().is_some() {
        return Err(invalid("Agent supplied ambiguous mode selectors"));
    }
    Ok(result)
}
pub fn values(option: &SessionConfigOption) -> Result<Vec<&SessionConfigSelectOption>, Error> {
    match &option.kind {
        SessionConfigKind::Select(select) => Ok(match &select.options {
            SessionConfigSelectOptions::Ungrouped(options) => options.iter().collect(),
            SessionConfigSelectOptions::Grouped(groups) => {
                groups.iter().flat_map(|g| g.options.iter()).collect()
            }
            _ => return Err(invalid("Unsupported Agent selector choices")),
        }),
        _ => Err(invalid("Unsupported Agent selector type")),
    }
}
pub fn current_value(option: &SessionConfigOption) -> Result<String, Error> {
    match &option.kind {
        SessionConfigKind::Select(select) => Ok(select.current_value.to_string()),
        _ => Err(invalid("Unsupported Agent selector type")),
    }
}
pub fn validate_options(options: &[SessionConfigOption]) -> Result<(), Error> {
    if options.len() > 32
        || serde_json::to_vec(options)
            .map_err(|_| invalid("Invalid Agent options"))?
            .len()
            > MAX_CONFIG_BYTES
    {
        return Err(invalid("Agent options exceed the session control budget"));
    }
    let mut ids = BTreeSet::new();
    for option in options {
        if option.id.to_string().is_empty() || !ids.insert(option.id.to_string()) {
            return Err(invalid("Ambiguous Agent option IDs"));
        }
        match &option.kind {
            SessionConfigKind::Select(_) => {
                let mut choices = BTreeSet::new();
                for choice in values(option)? {
                    if choice.value.to_string().is_empty()
                        || !choices.insert(choice.value.to_string())
                    {
                        return Err(invalid("Ambiguous Agent option values"));
                    }
                }
                if !choices.contains(&current_value(option)?) {
                    return Err(invalid("Agent current value is not an advertised choice"));
                }
            }
            SessionConfigKind::Boolean(_) => {}
            _ => return Err(invalid("Unknown Agent option type")),
        }
    }
    if let Some(mode) = mode_option(options)? {
        let _ = values(mode)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_permission_policy_never_infers_persistent_or_ambiguous_grants() {
        use crate::agent_preferences::ToolPolicy;
        let choices = [
            PermissionOption::new("allow", "Allow", PermissionOptionKind::AllowOnce),
            PermissionOption::new("deny", "Deny", PermissionOptionKind::RejectOnce),
            PermissionOption::new("always", "Always", PermissionOptionKind::AllowAlways),
        ];
        assert!(permission_response(ToolPolicy::Ask, &choices).is_none());
        for (policy, expected) in [(ToolPolicy::Allow, "allow"), (ToolPolicy::Deny, "deny")] {
            let result = permission_response(policy, &choices).unwrap();
            assert_eq!(serde_json::to_value(result).unwrap()["optionId"], expected);
        }
        for choices in [
            vec![],
            vec![choices[2].clone()],
            vec![choices[0].clone(), choices[0].clone()],
        ] {
            assert!(permission_response(ToolPolicy::Allow, &choices).is_none());
            assert!(matches!(
                permission_response(ToolPolicy::Deny, &choices),
                Some(RequestPermissionOutcome::Cancelled)
            ));
        }
        let ambiguous_denials = [choices[1].clone(), choices[1].clone()];
        assert!(matches!(
            permission_response(ToolPolicy::Deny, &ambiguous_denials),
            Some(RequestPermissionOutcome::Cancelled)
        ));
    }

    #[test]
    fn confirmed_selector_requires_the_returned_id_and_current_value() {
        let options = [selector()];
        assert!(confirm_choice(&options, "mode", "safe").is_ok());
        assert!(confirm_choice(&options, "removed", "safe").is_err());
        assert!(confirm_choice(&options, "mode", "different").is_err());
    }
    use serde_json::json;

    fn selector() -> SessionConfigOption {
        SessionConfigOption::select(
            "mode",
            "Mode",
            "safe",
            vec![SessionConfigSelectOption::new("safe", "Safe")],
        )
        .category(SessionConfigOptionCategory::Mode)
    }

    #[test]
    fn selector_admission_rejects_ambiguous_ids_values_and_modes() {
        let good = selector();
        validate_options(std::slice::from_ref(&good)).unwrap();
        assert_eq!(current_value(&good).unwrap(), "safe");
        assert_eq!(values(&good).unwrap()[0].value.to_string(), "safe");
        assert!(validate_options(&[good.clone(), good.clone()]).is_err());
        let mut second = good.clone();
        second.id = "second-mode".into();
        assert!(validate_options(&[good, second]).is_err());
        for (id, current, choices) in [
            ("", "safe", vec!["safe"]),
            ("option", "missing", vec!["safe"]),
            ("option", "safe", vec!["safe", "safe"]),
            ("option", "", vec![""]),
        ] {
            let option = SessionConfigOption::select(
                id,
                "Option",
                current,
                choices
                    .into_iter()
                    .map(|value| SessionConfigSelectOption::new(value, value))
                    .collect::<Vec<_>>(),
            );
            assert!(validate_options(&[option]).is_err());
        }
    }

    #[test]
    fn selector_admission_enforces_count_and_serialized_byte_budgets() {
        let options: Vec<_> = (0..32)
            .map(|index| {
                SessionConfigOption::select(
                    format!("option-{index}"),
                    "Option",
                    "value",
                    vec![SessionConfigSelectOption::new("value", "Value")],
                )
            })
            .collect();
        validate_options(&options).unwrap();
        let mut over = options;
        over.push(selector());
        assert!(validate_options(&over).is_err());
        let mut oversized = selector();
        oversized.name = "x".repeat(MAX_CONFIG_BYTES);
        assert!(validate_options(&[oversized]).is_err());
    }

    #[test]
    fn session_values_preserve_schema_metadata_and_interaction_vocabulary() {
        let value = json!({
            "instance_id": Uuid::from_u128(1),
            "operation_id": Uuid::from_u128(2),
            "session_id": "session", "agent_name": "Agent", "active": true,
            "config_revision": 7,
            "config_options": [{
                "id":"mode", "name":"Mode", "type":"select", "category":"mode",
                "currentValue":"safe", "options":[{"value":"safe","name":"Safe"}],
                "_meta":{"vendor":{"revision":2}}
            }],
            "modes":[{"id":"safe","name":"Safe","_meta":{"vendor":true}}],
            "effective_mode":"safe", "configured_mode":"safe",
            "configured_origin":"user", "last_mode_origin":"agent", "policy_default":"safe",
            "change":{"config_id":"mode","value":"safe","status":"succeeded"},
            "notice":null,
            "interactions":[{
                "id":Uuid::from_u128(3), "run_id":null, "sequence":1,
                "status":"pending", "details":{
                    "kind":"permission", "tool_call_id":"tool", "title":"Read",
                    "effect":"read", "arguments":{"path":"fixture"},
                    "options":[{"optionId":"once","name":"Once","kind":"allow_once"}]
                }
            }]
        });
        let state: AgentSessionControlState = serde_json::from_value(value.clone()).unwrap();
        validate_options(state.config_options.as_ref().unwrap()).unwrap();
        assert_eq!(serde_json::to_value(state).unwrap(), value);
        for action in ["accept", "decline", "cancel"] {
            serde_json::from_value::<InteractionResponse>(json!({"action":action})).unwrap();
        }
        assert!(matches!(
            serde_json::from_value::<InteractionResponse>(json!({"action":"select","option_id":"once"})).unwrap(),
            InteractionResponse::Select { option_id } if option_id == "once"
        ));
        assert!(matches!(
            serde_json::from_value::<InteractionResponse>(json!({"action":"submit","content":{"count":2}})).unwrap(),
            InteractionResponse::Submit { content } if content == json!({"count":2})
        ));
        assert!(
            serde_json::from_value::<InteractionResponse>(json!({"action":"unknown"})).is_err()
        );
    }
}
