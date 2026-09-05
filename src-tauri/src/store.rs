use crate::model::AppConfig;
use std::{fs, io, path::PathBuf};

pub(crate) fn default_config() -> AppConfig {
    AppConfig::new(dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")))
}

#[derive(Debug)]
pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    pub fn new() -> Self {
        let base = dirs::config_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            path: base.join("Lens").join("settings.json"),
        }
    }

    pub fn load(&self) -> AppConfig {
        let defaults = default_config();
        let default_directory = defaults.working_directory.clone();
        let mut config: AppConfig = fs::read(&self.path)
            .ok()
            .and_then(|bytes| AppConfig::decode_settings(&bytes, default_directory.clone()).ok())
            .unwrap_or(defaults);
        if !config.working_directory.is_dir() {
            config.working_directory = default_directory;
        }
        config.agent_prompt_template = std::mem::take(&mut config.agent_prompt_template)
            .normalize()
            .unwrap_or_default();
        config
    }

    pub fn save(&self, config: &AppConfig) -> io::Result<()> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid config path"))?;
        fs::create_dir_all(parent)?;
        let temporary = self.path.with_extension("json.tmp");
        fs::write(&temporary, serde_json::to_vec_pretty(config)?)?;
        fs::rename(temporary, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AgentKind;
    use uuid::Uuid;

    #[test]
    fn load_normalizes_an_unavailable_directory_without_resetting_the_agent() {
        let test_root = std::env::temp_dir().join(format!("lens-config-store-{}", Uuid::new_v4()));
        fs::create_dir_all(&test_root).expect("create test settings directory");
        let store = ConfigStore {
            path: test_root.join("settings.json"),
        };
        let mut agent_prompt_template = crate::store::default_config().agent_prompt_template;
        agent_prompt_template.common =
            "Summarize the source.\n\n{turn_instruction}\n\nUse only attached observations.".into();
        let config = AppConfig {
            agent: AgentKind::Codex,
            working_directory: test_root.join("unmounted"),
            agent_prompt_template: agent_prompt_template.clone(),
            agent_preferences: Default::default(),
        };
        fs::write(
            &store.path,
            serde_json::to_vec(&config).expect("serialize test settings"),
        )
        .expect("write test settings");

        let loaded = store.load();

        assert_eq!(loaded.agent, AgentKind::Codex);
        assert_eq!(loaded.agent_prompt_template, agent_prompt_template);
        assert_eq!(
            loaded.working_directory,
            crate::store::default_config().working_directory
        );
        fs::remove_dir_all(test_root).expect("remove test settings directory");
    }

    #[test]
    fn load_migrates_settings_without_a_prompt_to_the_built_in_template() {
        let test_root = std::env::temp_dir().join(format!("lens-config-store-{}", Uuid::new_v4()));
        fs::create_dir_all(&test_root).expect("create test settings directory");
        let store = ConfigStore {
            path: test_root.join("settings.json"),
        };
        fs::write(
            &store.path,
            serde_json::to_vec(&serde_json::json!({
                "agent": "codex",
                "working_directory": test_root,
            }))
            .expect("serialize legacy settings"),
        )
        .expect("write legacy settings");

        let loaded = store.load();

        assert_eq!(
            loaded.agent_prompt_template,
            crate::store::default_config().agent_prompt_template
        );
        fs::remove_dir_all(test_root).expect("remove test settings directory");
    }

    #[test]
    fn load_migrates_a_legacy_response_prompt_into_the_complete_template() {
        let test_root = std::env::temp_dir().join(format!("lens-config-store-{}", Uuid::new_v4()));
        fs::create_dir_all(&test_root).expect("create test settings directory");
        let store = ConfigStore {
            path: test_root.join("settings.json"),
        };
        fs::write(
            &store.path,
            serde_json::to_vec(&serde_json::json!({
                "agent": "codex",
                "working_directory": test_root,
                "response_prompt": "Explain this for a beginner."
            }))
            .expect("serialize legacy settings"),
        )
        .expect("write legacy settings");

        let loaded = store.load();

        assert!(loaded
            .agent_prompt_template
            .common
            .starts_with("Explain this for a beginner."));
        assert!(loaded
            .agent_prompt_template
            .common
            .contains("{turn_instruction}"));
        fs::remove_dir_all(test_root).expect("remove test settings directory");
    }

    #[test]
    fn save_persists_the_complete_template_without_the_legacy_prompt_field() {
        let test_root = std::env::temp_dir().join(format!("lens-config-store-{}", Uuid::new_v4()));
        fs::create_dir_all(&test_root).expect("create test settings directory");
        let store = ConfigStore {
            path: test_root.join("settings.json"),
        };
        let config = AppConfig {
            working_directory: test_root.clone(),
            ..crate::store::default_config()
        };

        store.save(&config).expect("save settings");

        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(&store.path).expect("read persisted settings"))
                .expect("parse persisted settings");
        assert_eq!(
            persisted["agent_prompt_template"],
            serde_json::to_value(&config.agent_prompt_template).expect("serialize prompt template")
        );
        assert!(persisted.get("response_prompt").is_none());
        assert!(!store.path.with_extension("json.tmp").exists());
        fs::remove_dir_all(test_root).expect("remove test settings directory");
    }
}
