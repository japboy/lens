//! ACP tool-kind interpretation stays at the desktop protocol boundary.
use agent_client_protocol::schema::v1::ToolKind;
pub use use_case::agent_preferences::*;

pub fn policy_for_tool(policies: &ToolPolicies, kind: ToolKind) -> ToolPolicy {
    match kind {
        ToolKind::Read => policies.read,
        ToolKind::Search => policies.search,
        ToolKind::Fetch => policies.fetch,
        ToolKind::Edit => policies.edit,
        ToolKind::Delete => policies.delete,
        ToolKind::Move => policies.r#move,
        ToolKind::Execute => policies.execute,
        _ => ToolPolicy::Ask,
    }
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
        };
        for (kind, expected) in [
            (ToolKind::Read, policies.read),
            (ToolKind::Search, policies.search),
            (ToolKind::Fetch, policies.fetch),
            (ToolKind::Edit, policies.edit),
            (ToolKind::Delete, policies.delete),
            (ToolKind::Move, policies.r#move),
            (ToolKind::Execute, policies.execute),
            (ToolKind::Other, ToolPolicy::Ask),
        ] {
            assert_eq!(policy_for_tool(&policies, kind), expected);
        }
    }
}
