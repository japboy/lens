//! ACP tool-kind interpretation stays at the desktop protocol boundary.
use agent_client_protocol::schema::v1::ToolKind;
pub use usecase::agent_preferences::*;

pub fn policy_for_tool(policies: &ToolPolicies, kind: ToolKind) -> ToolPolicy {
    match kind {
        ToolKind::Read => policies.read,
        ToolKind::Search => policies.search,
        ToolKind::Fetch => policies.fetch,
        ToolKind::Edit => policies.edit,
        ToolKind::Delete => policies.delete,
        ToolKind::Move => policies.r#move,
        ToolKind::Execute => policies.execute,
        _ => policies.other,
    }
}

/// External profiles leave provider and advanced configuration with the owning CLI.
pub(crate) fn validate_config_choice(
    kind: crate::model::AgentKind,
    config_id: &str,
    options: Option<&[agent_client_protocol::schema::v1::SessionConfigOption]>,
) -> Result<(), String> {
    use agent_client_protocol::schema::v1::SessionConfigOptionCategory as Category;
    if !kind.is_external() {
        return Ok(());
    }
    let allowed = match options {
        Some(options) => options.iter().any(|option| {
            option.id.to_string() == config_id
                && matches!(
                    option.category,
                    Some(Category::Mode | Category::Model | Category::ThoughtLevel)
                )
        }),
        None => config_id == "mode",
    };
    if allowed {
        Ok(())
    } else {
        Err("Configure provider and advanced options in the external Agent CLI, then verify the connection again.".into())
    }
}
pub(crate) fn validate_defaults(
    kind: crate::model::AgentKind,
    defaults: &AgentDefaults,
    options: Option<&[agent_client_protocol::schema::v1::SessionConfigOption]>,
) -> Result<(), String> {
    for choice in &defaults.choices {
        validate_config_choice(kind, &choice.config_id, options)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acp_tool_kinds_keep_their_exact_persisted_policy_and_unknown_tools_ask() {
        let policies = ToolPolicies {
            read: ToolPolicy::Allow,
            search: ToolPolicy::Deny,
            fetch: ToolPolicy::Ask,
            edit: ToolPolicy::Allow,
            delete: ToolPolicy::Deny,
            r#move: ToolPolicy::Ask,
            execute: ToolPolicy::Allow,
            other: ToolPolicy::Deny,
        };
        for (kind, expected) in [
            (ToolKind::Read, policies.read),
            (ToolKind::Search, policies.search),
            (ToolKind::Fetch, policies.fetch),
            (ToolKind::Edit, policies.edit),
            (ToolKind::Delete, policies.delete),
            (ToolKind::Move, policies.r#move),
            (ToolKind::Execute, policies.execute),
            (ToolKind::Other, policies.other),
        ] {
            assert_eq!(policy_for_tool(&policies, kind), expected);
        }
    }
    #[test]
    fn external_advanced_options_are_owned_by_cli() {
        use crate::model::AgentKind;
        let external = AgentKind::External(uuid::Uuid::from_u128(1));
        assert!(validate_config_choice(external, "provider", None).is_err());
        assert!(validate_config_choice(external, "mode", None).is_ok());
        assert!(validate_config_choice(external, "mode", Some(&[])).is_err());
        assert!(validate_config_choice(AgentKind::Codex, "provider", None).is_ok());
    }
}
