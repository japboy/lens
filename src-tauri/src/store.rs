use crate::model::AppConfig;
use std::{fs, io, path::PathBuf};

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
            path: base.join("PersonalLens").join("settings.json"),
        }
    }

    pub fn load(&self) -> AppConfig {
        let mut config: AppConfig = fs::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        if !config.working_directory.is_dir() {
            config.working_directory = AppConfig::default().working_directory;
        }
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
        let test_root =
            std::env::temp_dir().join(format!("personal-lens-config-store-{}", Uuid::new_v4()));
        fs::create_dir_all(&test_root).expect("create test settings directory");
        let store = ConfigStore {
            path: test_root.join("settings.json"),
        };
        let config = AppConfig {
            agent: AgentKind::Codex,
            working_directory: test_root.join("unmounted"),
        };
        fs::write(
            &store.path,
            serde_json::to_vec(&config).expect("serialize test settings"),
        )
        .expect("write test settings");

        let loaded = store.load();

        assert_eq!(loaded.agent, AgentKind::Codex);
        assert_eq!(
            loaded.working_directory,
            AppConfig::default().working_directory
        );
        fs::remove_dir_all(test_root).expect("remove test settings directory");
    }
}
