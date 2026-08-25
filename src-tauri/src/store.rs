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
        fs::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .filter(|config: &AppConfig| config.working_directory.is_dir())
            .unwrap_or_default()
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
