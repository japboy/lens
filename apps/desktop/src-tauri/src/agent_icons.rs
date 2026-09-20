//! Shared name matching for native menu and frontend Agent icons.

use serde::Deserialize;
use std::sync::LazyLock;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum AgentIcon {
    Claude,
    Openai,
    Copilot,
    Robot,
}

#[derive(Deserialize)]
struct Rule {
    #[serde(rename = "containsAny")]
    contains_any: Vec<String>,
    icon: AgentIcon,
}

#[derive(Deserialize)]
struct Catalog {
    rules: Vec<Rule>,
    fallback: AgentIcon,
}

static CATALOG: LazyLock<Catalog> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../agent-icons/catalog.json"))
        .expect("bundled Agent icon catalog must be valid")
});

fn icon_name(name: &str) -> AgentIcon {
    let normalized = name.to_ascii_lowercase();
    CATALOG
        .rules
        .iter()
        .find(|rule| {
            rule.contains_any
                .iter()
                .any(|part| normalized.contains(part))
        })
        .map_or(CATALOG.fallback, |rule| rule.icon)
}

pub(crate) fn icon_png(name: &str) -> &'static [u8] {
    match icon_name(name) {
        AgentIcon::Claude => include_bytes!("../../agent-icons/claude.png"),
        AgentIcon::Openai => include_bytes!("../../agent-icons/openai.png"),
        AgentIcon::Copilot => include_bytes!("../../agent-icons/copilot.png"),
        AgentIcon::Robot => include_bytes!("../../agent-icons/robot.png"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct Fixture {
        name: String,
        icon: AgentIcon,
    }

    #[test]
    fn shared_name_matching_contract() {
        let fixtures: Vec<Fixture> =
            serde_json::from_str(include_str!("../../agent-icons/fixtures.json")).unwrap();
        for fixture in fixtures {
            assert_eq!(icon_name(&fixture.name), fixture.icon, "{}", fixture.name);
            assert!(icon_png(&fixture.name).starts_with(b"\x89PNG\r\n\x1a\n"));
        }
    }
}
