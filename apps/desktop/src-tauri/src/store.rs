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

/// Runtime directory selection never mutates the persisted configuration.
pub(crate) fn effective_working_directory(config: &AppConfig) -> PathBuf {
    if config.working_directory.is_dir() {
        config.working_directory.clone()
    } else {
        default_config().working_directory
    }
}

#[derive(Debug)]
pub struct ConfigStore {
    path: PathBuf,
}

#[derive(serde::Serialize)]
pub(crate) struct RecoveryInfo {
    pub message: String,
    pub settings_path: String,
    pub can_restore_prompt_presets: bool,
    pub digest: Option<String>,
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn prompt_recovery_value(bytes: &[u8]) -> Result<(serde_json::Value, AppConfig), String> {
    let defaults = default_config();
    if AppConfig::decode_settings(bytes, defaults.working_directory.clone()).is_ok() {
        return Err("The settings no longer require prompt recovery. Reload them.".into());
    }
    let mut value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| "The settings file is not valid JSON.")?;
    let object = value
        .as_object_mut()
        .ok_or("Settings must be a JSON object.")?;
    object.insert(
        "prompt_presets".into(),
        serde_json::to_value(defaults.prompt_presets)
            .map_err(|_| "Unable to prepare prompt presets")?,
    );
    let config = AppConfig::decode_settings(
        &serde_json::to_vec(&value).map_err(|_| "Unable to prepare settings")?,
        defaults.working_directory,
    )
    .map_err(|_| {
        "Other settings are invalid. Repair the settings file before restoring prompts."
    })?;
    Ok((value, config))
}

impl ConfigStore {
    pub fn new<R: tauri::Runtime>(app: &impl tauri::Manager<R>) -> Result<Self, tauri::Error> {
        let directory = app.path().app_config_dir()?;
        Ok(Self::at_path(directory.join("settings.json")))
    }

    pub(crate) fn at_path(path: PathBuf) -> Self {
        Self { path }
    }

    pub(crate) fn inspect_recovery(&self, message: String) -> RecoveryInfo {
        let bytes = fs::read(&self.path).ok();
        RecoveryInfo {
            message,
            settings_path: self.path.to_string_lossy().into_owned(),
            can_restore_prompt_presets: bytes
                .as_deref()
                .is_some_and(|bytes| prompt_recovery_value(bytes).is_ok()),
            digest: bytes.as_deref().map(digest),
        }
    }

    pub(crate) fn restore_prompt_presets(
        &self,
        expected_digest: &str,
    ) -> Result<AppConfig, String> {
        let bytes = fs::read(&self.path).map_err(|_| "Unable to read the settings file.")?;
        if digest(&bytes) != expected_digest {
            return Err("The settings file changed. Reload before restoring prompts.".into());
        }
        let (value, config) = prompt_recovery_value(&bytes)?;
        let backup = self.path.with_file_name(format!(
            "settings.before-prompt-recovery.{expected_digest}.json"
        ));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&backup)
        {
            Ok(mut file) => {
                let result = (|| -> io::Result<()> {
                    file.set_permissions(fs::metadata(&self.path)?.permissions())?;
                    file.write_all(&bytes)?;
                    file.sync_all()
                })();
                if result.is_err() {
                    drop(file);
                    let _ = fs::remove_file(&backup);
                }
                result.map_err(|_| "Unable to back up the original settings.")?;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                if fs::read(&backup).map_err(|_| "Unable to verify the settings backup.")? != bytes
                {
                    return Err("The existing backup differs from the original settings.".into());
                }
            }
            Err(_) => return Err("Unable to back up the original settings.".into()),
        }
        let temporary = self
            .path
            .with_extension(format!("json.recovery.{}", uuid::Uuid::new_v4()));
        let result = (|| -> Result<(), String> {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|_| "Unable to create recovery file")?;
            file.set_permissions(
                fs::metadata(&self.path)
                    .map_err(|_| "Unable to inspect settings")?
                    .permissions(),
            )
            .map_err(|_| "Unable to preserve settings permissions")?;
            file.write_all(
                &serde_json::to_vec_pretty(&value).map_err(|_| "Unable to encode settings")?,
            )
            .map_err(|_| "Unable to write settings")?;
            file.sync_all().map_err(|_| "Unable to sync settings")?;
            if fs::read(&self.path).map_err(|_| "Unable to recheck settings")? != bytes {
                return Err("The settings file changed. Reload before restoring prompts.".into());
            }
            fs::rename(&temporary, &self.path).map_err(|_| "Unable to replace settings")?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result?;
        Ok(config)
    }

    pub fn try_load(&self) -> Result<AppConfig, String> {
        let defaults = default_config();
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.initialize_missing(&defaults)
                    .map_err(|error| format!("Unable to initialize Lens settings: {error}"))?;
                fs::read(&self.path)
                    .map_err(|error| format!("Unable to read initialized Lens settings: {error}"))?
            }
            Err(error) => return Err(format!("Unable to read Lens settings: {error}")),
        };
        let migrate = AppConfig::settings_require_prompt_migration(&bytes)
            .map_err(|error| error.to_string())?;
        let config = AppConfig::decode_settings(&bytes, defaults.working_directory.clone())
            .map_err(|error| format!("Unable to decode Lens settings: {error}"))?;
        if migrate {
            self.save(&config)
                .map_err(|error| format!("Unable to back up and migrate Lens settings: {error}"))?;
        }
        Ok(config)
    }

    /// Publish initial defaults only if no other writer has created the settings.
    fn initialize_missing(&self, config: &AppConfig) -> io::Result<()> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| io::Error::other("invalid config path"))?;
        fs::create_dir_all(parent)?;
        let temporary = self
            .path
            .with_extension(format!("json.initial.{}", uuid::Uuid::new_v4()));
        let result = (|| {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(&serde_json::to_vec_pretty(config)?)?;
            file.sync_all()?;
            match fs::hard_link(&temporary, &self.path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
                Err(error) => Err(error),
            }
        })();
        let _ = fs::remove_file(&temporary);
        result
    }

    #[cfg(test)]
    pub fn load(&self) -> AppConfig {
        self.try_load().expect("load valid test settings")
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
                AppConfig::decode_settings(&bytes, default_config().working_directory)
                    .map_err(io::Error::other)?;
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

    #[test]
    fn settings_path_uses_app_identifier_instead_of_product_name() {
        for identifier in ["com.github.japboy.lens", "com.example.another-lens"] {
            let mut context = crate::product_context();
            context.config_mut().identifier = identifier.into();
            let app = tauri::test::mock_builder().build(context).unwrap();
            let store = ConfigStore::new(&app).unwrap();
            assert_eq!(
                store.path,
                dirs::config_dir()
                    .unwrap()
                    .join(identifier)
                    .join("settings.json")
            );
        }
    }

    fn store() -> (PathBuf, ConfigStore) {
        let root = std::env::temp_dir().join(format!("lens-config-store-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let store = ConfigStore::at_path(root.join("settings.json"));
        (root, store)
    }

    #[test]
    fn missing_file_is_persisted_once_and_existing_file_wins_creation_race() {
        let (root, store) = store();
        let config = store.try_load().unwrap();
        assert!(store.path.is_file());
        let bytes = fs::read(&store.path).unwrap();
        assert_eq!(store.try_load().unwrap(), config);
        assert_eq!(fs::read(&store.path).unwrap(), bytes);
        let mut different = config;
        different.agent = crate::model::AgentKind::Codex;
        store.initialize_missing(&different).unwrap();
        assert_eq!(fs::read(&store.path).unwrap(), bytes);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_existing_settings_are_never_overwritten_by_load_or_save() {
        for bytes in [
            b"{broken".as_slice(),
            b"{}",
            br#"{"prompt_presets":{"schema_version":2,"presets":[]}}"#,
        ] {
            let (root, store) = store();
            fs::write(&store.path, bytes).unwrap();
            assert!(store.try_load().is_err());
            assert!(store.save(&default_config()).is_err());
            assert_eq!(fs::read(&store.path).unwrap(), bytes);
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn unavailable_saved_directory_survives_unrelated_settings_save() {
        let (root, store) = store();
        let mut config = store.try_load().unwrap();
        config.working_directory = root.join("not-present");
        store.save(&config).unwrap();
        let mut loaded = store.try_load().unwrap();
        assert_eq!(loaded.working_directory, config.working_directory);
        assert_eq!(
            effective_working_directory(&loaded),
            default_config().working_directory
        );
        loaded.agent = crate::model::AgentKind::Codex;
        store.save(&loaded).unwrap();
        assert_eq!(
            store.try_load().unwrap().working_directory,
            config.working_directory
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn explicit_prompt_recovery_preserves_original_and_unrelated_fields_and_rejects_stale_digest() {
        let (root, store) = store();
        let bytes = br#"{"agent":"codex","working_directory":"/unavailable/saved","custom":{"preserve":true},"response_prompt":"precious old prompt","prompt_presets":{"schema_version":1}}"#;
        fs::write(&store.path, bytes).unwrap();
        let info = store.inspect_recovery("Invalid prompts".into());
        assert!(info.can_restore_prompt_presets);
        assert!(store.restore_prompt_presets("stale").is_err());
        assert_eq!(fs::read(&store.path).unwrap(), bytes);
        let digest = info.digest.unwrap();
        let recovered = store.restore_prompt_presets(&digest).unwrap();
        assert_eq!(recovered.agent, crate::model::AgentKind::Codex);
        assert_eq!(
            recovered.working_directory,
            PathBuf::from("/unavailable/saved")
        );
        let actual: serde_json::Value =
            serde_json::from_slice(&fs::read(&store.path).unwrap()).unwrap();
        let expected: serde_json::Value = serde_json::from_slice(bytes).unwrap();
        for field in ["agent", "working_directory", "custom", "response_prompt"] {
            assert_eq!(actual[field], expected[field]);
        }
        assert_eq!(
            fs::read(root.join(format!("settings.before-prompt-recovery.{digest}.json"))).unwrap(),
            bytes
        );
        assert!(!store.inspect_recovery("".into()).can_restore_prompt_presets);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn prompt_recovery_cannot_repair_invalid_json_or_nonprompt_settings() {
        for bytes in [
            b"{broken".as_slice(),
            br#"{"agent":"unknown","prompt_presets":null}"#,
        ] {
            let (root, store) = store();
            fs::write(&store.path, bytes).unwrap();
            let info = store.inspect_recovery("Invalid settings".into());
            assert!(!info.can_restore_prompt_presets);
            assert!(store.restore_prompt_presets(&info.digest.unwrap()).is_err());
            assert_eq!(fs::read(&store.path).unwrap(), bytes);
            fs::remove_dir_all(root).unwrap();
        }
    }
}
