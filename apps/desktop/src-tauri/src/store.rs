use crate::model::AppConfig;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Write},
    path::PathBuf,
};

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
        Self::at_path(base.join("Lens").join("settings.json"))
    }

    pub(crate) fn at_path(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn try_load(&self) -> Result<AppConfig, String> {
        let defaults = default_config();
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(defaults),
            Err(error) => return Err(format!("Unable to read Lens settings: {error}")),
        };
        let migrate = AppConfig::settings_require_prompt_migration(&bytes)
            .map_err(|error| error.to_string())?;
        let mut config = AppConfig::decode_settings(&bytes, defaults.working_directory.clone())
            .map_err(|error| format!("Unable to decode Lens settings: {error}"))?;
        if migrate {
            self.save(&config)
                .map_err(|error| format!("Unable to back up and migrate Lens settings: {error}"))?;
        }
        // Host path recovery affects this runtime only; a prompt migration must not
        // rewrite an unavailable saved working directory as a side effect.
        if !config.working_directory.is_dir() {
            config.working_directory = defaults.working_directory;
        }
        Ok(config)
    }

    #[cfg(test)]
    pub fn load(&self) -> AppConfig {
        self.try_load().expect("load valid test settings")
    }

    /// Moves a settings file that `try_load` refused aside, preserving its bytes at a
    /// discoverable path, so startup can continue from defaults instead of aborting
    /// before any window, tray or dialog exists. `try_load` itself stays strict.
    pub(crate) fn quarantine_unreadable(&self) -> io::Result<Option<PathBuf>> {
        match fs::metadata(&self.path) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        }
        let quarantined = self
            .path
            .with_file_name(format!("settings.unreadable.{}.json", uuid::Uuid::new_v4()));
        fs::rename(&self.path, &quarantined)?;
        Ok(Some(quarantined))
    }

    fn backup_legacy(&self, bytes: &[u8]) -> io::Result<()> {
        if !AppConfig::settings_require_prompt_migration(bytes).map_err(io::Error::other)? {
            return Ok(());
        }
        let digest = Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let backup = self
            .path
            .with_file_name(format!("settings.before-prompt-presets-v2.{digest}.json"));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&backup)
        {
            Ok(mut file) => {
                if let Err(error) = fs::metadata(&self.path)
                    .and_then(|metadata| file.set_permissions(metadata.permissions()))
                    .and_then(|_| file.write_all(bytes))
                    .and_then(|_| file.sync_all())
                {
                    drop(file);
                    let _ = fs::remove_file(&backup);
                    return Err(error);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                if fs::read(&backup)? != bytes {
                    return Err(io::Error::other(
                        "Existing settings backup does not match the original bytes",
                    ));
                }
            }
            Err(error) => return Err(error),
        }
        Ok(())
    }

    pub fn save(&self, config: &AppConfig) -> io::Result<()> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid config path"))?;
        fs::create_dir_all(parent)?;
        config.prompt_presets.validate().map_err(io::Error::other)?;
        let mut value = match fs::read(&self.path) {
            Ok(bytes) => {
                self.backup_legacy(&bytes)?;
                serde_json::from_slice::<serde_json::Value>(&bytes)?
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => serde_json::json!({}),
            Err(error) => return Err(error),
        };
        let object = value
            .as_object_mut()
            .ok_or_else(|| io::Error::other("Settings must be an object"))?;
        object.remove("response_prompt");
        object.extend(
            serde_json::to_value(config)?
                .as_object()
                .expect("AppConfig serializes as an object")
                .clone(),
        );
        let temporary = self
            .path
            .with_extension(format!("json.tmp.{}", uuid::Uuid::new_v4()));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let result = (|| {
            match fs::metadata(&self.path) {
                Ok(metadata) => file.set_permissions(metadata.permissions())?,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            file.write_all(&serde_json::to_vec_pretty(&value)?)?;
            file.sync_all()?;
            fs::rename(&temporary, &self.path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn store() -> (PathBuf, ConfigStore) {
        let root = std::env::temp_dir().join(format!("lens-config-store-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let store = ConfigStore::at_path(root.join("settings.json"));
        (root, store)
    }
    fn backup_path(store: &ConfigStore, bytes: &[u8]) -> PathBuf {
        store.path.with_file_name(format!(
            "settings.before-prompt-presets-v2.{}.json",
            Sha256::digest(bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        ))
    }

    #[test]
    fn migration_backs_up_exact_bytes_once_preserves_other_settings_and_keeps_v2_edits() {
        let mut empty = serde_json::to_value(default_config().prompt_presets).unwrap();
        empty["presets"] = serde_json::json!([]);
        let mut missing = empty.clone();
        missing.as_object_mut().unwrap().remove("presets");
        for catalog in [
            Some(empty),
            Some(missing),
            None,
            Some(
                serde_json::json!({"schema_version":1,"presets":[{"name":"Custom","description":"old"}]}),
            ),
        ] {
            let (root, store) = store();
            let mut legacy = serde_json::json!({"agent":"codex", "working_directory":root, "response_prompt":"My exact {legacy} prompt", "unrecognized_setting":{"keep":true}});
            if let Some(catalog) = catalog {
                legacy["prompt_presets"] = catalog;
            }
            let bytes =
                format!(" \n{}\n\n", serde_json::to_string_pretty(&legacy).unwrap()).into_bytes();
            fs::write(&store.path, &bytes).unwrap();
            let original_permissions = fs::metadata(&store.path).unwrap().permissions();
            let config = store.try_load().unwrap();
            assert_eq!(
                fs::metadata(backup_path(&store, &bytes))
                    .unwrap()
                    .permissions(),
                original_permissions
            );
            assert_eq!(fs::read(backup_path(&store, &bytes)).unwrap(), bytes);
            assert_eq!(config.agent, crate::model::AgentKind::Codex);
            assert_eq!(config.working_directory, root);
            assert_eq!(config.prompt_presets, Default::default());
            let persisted: serde_json::Value =
                serde_json::from_slice(&fs::read(&store.path).unwrap()).unwrap();
            assert_eq!(
                persisted["unrecognized_setting"],
                legacy["unrecognized_setting"]
            );
            assert!(persisted.get("response_prompt").is_none());
            let migrated_bytes = fs::read(&store.path).unwrap();
            assert_eq!(store.try_load().unwrap(), config);
            assert_eq!(fs::read(&store.path).unwrap(), migrated_bytes);
            let mut edited = config;
            edited.prompt_presets.presets[0].name = "My visual".into();
            store.save(&edited).unwrap();
            assert_eq!(store.try_load().unwrap(), edited);
            assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
            assert_eq!(fs::read(backup_path(&store, &bytes)).unwrap(), bytes);
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn missing_selected_record_uses_first_survivor_once_without_resetting_edits() {
        for missing_id in [false, true] {
            let (root, store) = store();
            let mut config = default_config();
            config.working_directory = root.clone();
            config.prompt_presets.presets.remove(0);
            config.prompt_presets.presets[0].name = "Kept custom name".into();
            let mut value = serde_json::to_value(&config).unwrap();
            if missing_id {
                value["prompt_presets"]
                    .as_object_mut()
                    .unwrap()
                    .remove("selected_id");
            }
            let bytes = serde_json::to_vec(&value).unwrap();
            fs::write(&store.path, &bytes).unwrap();
            config.prompt_presets.selected_id = config.prompt_presets.presets[0].id.clone();
            config.agent_prompt_template = config.prompt_presets.selected().template.clone();
            assert_eq!(store.try_load().unwrap(), config);
            let normalized = fs::read(&store.path).unwrap();
            assert_eq!(store.try_load().unwrap(), config);
            assert_eq!(fs::read(&store.path).unwrap(), normalized);
            assert_eq!(fs::read(backup_path(&store, &bytes)).unwrap(), bytes);
            assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn backup_failure_aborts_load_and_later_save_without_overwriting_original() {
        let (root, store) = store();
        let bytes = b"{\"agent\":\"codex\",\"response_prompt\":\"precious old text\"}";
        fs::write(&store.path, bytes).unwrap();
        fs::create_dir(backup_path(&store, bytes)).unwrap();
        assert!(store.try_load().is_err());
        assert!(store.save(&default_config()).is_err());
        assert_eq!(fs::read(&store.path).unwrap(), bytes);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mismatched_existing_backup_never_gets_overwritten() {
        let (root, store) = store();
        let bytes = b"{}";
        fs::write(&store.path, bytes).unwrap();
        let backup = backup_path(&store, bytes);
        fs::write(&backup, b"different content").unwrap();
        assert!(store.try_load().is_err());
        assert_eq!(fs::read(&backup).unwrap(), b"different content");
        assert_eq!(fs::read(&store.path).unwrap(), bytes);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_file_uses_defaults_and_invalid_v2_never_resets_silently() {
        let (root, store) = store();
        assert_eq!(store.try_load().unwrap(), default_config());
        let bytes = b"{\"prompt_presets\":{\"schema_version\":2}}";
        fs::write(&store.path, bytes).unwrap();
        assert!(store.try_load().is_err());
        assert_eq!(fs::read(&store.path).unwrap(), bytes);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }
}
