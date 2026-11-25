//! Configuration persistence with validation and atomic writes.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use crate::config::{SystemConfig, CONFIG_SCHEMA_VERSION};
use crate::error::{CoreError, CoreResult};
use crate::state::fsutil;

/// File name of the persisted configuration inside the data directory.
pub const CONFIG_FILE: &str = "config.json";

/// Thread-safe configuration holder.
///
/// Reads are cheap (clone of a small struct); writes take the lock, validate
/// and persist atomically before publishing the new value.
#[derive(Debug)]
pub struct ConfigManager {
    path: PathBuf,
    config: RwLock<SystemConfig>,
}

impl ConfigManager {
    /// Load the configuration from `data_dir`, creating a default one when the
    /// file does not exist yet.
    ///
    /// # Errors
    /// Returns [`CoreError::Config`] for malformed or unsupported files.
    pub fn load(data_dir: &Path) -> CoreResult<Self> {
        fsutil::ensure_dir(data_dir)?;
        let path = data_dir.join(CONFIG_FILE);

        let mut config = if path.exists() {
            let raw = fsutil::read_to_string(&path)?;
            serde_json::from_str::<SystemConfig>(&raw).map_err(|err| {
                CoreError::Config(format!("failed to parse {}: {err}", path.display()))
            })?
        } else {
            SystemConfig::bootstrap()
        };

        if config.schema_version > CONFIG_SCHEMA_VERSION {
            return Err(CoreError::Config(format!(
                "configuration schema {} is newer than supported schema {CONFIG_SCHEMA_VERSION}",
                config.schema_version
            )));
        }
        config.schema_version = CONFIG_SCHEMA_VERSION;
        config.validate()?;

        let manager = Self {
            path,
            config: RwLock::new(config),
        };

        if !manager.path.exists() {
            manager.persist(&manager.snapshot())?;
        }
        Ok(manager)
    }

    /// Path of the persisted configuration file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Current configuration snapshot.
    #[must_use]
    pub fn snapshot(&self) -> SystemConfig {
        self.config
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    /// Replace the whole configuration after validating and persisting it.
    ///
    /// # Errors
    /// Returns an error when validation or persistence fails.
    pub fn replace(&self, config: SystemConfig) -> CoreResult<SystemConfig> {
        config.validate()?;
        self.persist(&config)?;
        let mut guard = self
            .config
            .write()
            .map_err(|_| CoreError::Internal("configuration lock poisoned".to_owned()))?;
        *guard = config.clone();
        Ok(config)
    }

    /// Mutate the configuration transactionally.
    ///
    /// The closure runs on a copy; if it returns an error or the resulting
    /// configuration is invalid, nothing is persisted and the current value is
    /// preserved.
    ///
    /// # Errors
    /// Returns an error when the closure, validation or persistence fails.
    pub fn update<F>(&self, mutate: F) -> CoreResult<SystemConfig>
    where
        F: FnOnce(&mut SystemConfig) -> CoreResult<()>,
    {
        let mut candidate = self.snapshot();
        mutate(&mut candidate)?;
        candidate.schema_version = CONFIG_SCHEMA_VERSION;
        candidate.validate()?;
        self.persist(&candidate)?;
        let mut guard = self
            .config
            .write()
            .map_err(|_| CoreError::Internal("configuration lock poisoned".to_owned()))?;
        *guard = candidate.clone();
        Ok(candidate)
    }

    fn persist(&self, config: &SystemConfig) -> CoreResult<()> {
        let json = serde_json::to_vec_pretty(config)?;
        fsutil::write_private(&self.path, &json).map_err(|err| {
            CoreError::Io(std::io::Error::other(format!(
                "failed to persist configuration to {}: {err}",
                self.path.display()
            )))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DnsProvider;

    #[test]
    fn creates_default_config_on_first_load() {
        let dir = tempfile::tempdir().unwrap();
        let manager = ConfigManager::load(dir.path()).unwrap();
        assert!(!manager.snapshot().initialized);
        assert!(manager.path().exists());
        assert_eq!(manager.snapshot().schema_version, CONFIG_SCHEMA_VERSION);
    }

    #[test]
    fn updates_are_persisted_and_reloaded() {
        let dir = tempfile::tempdir().unwrap();
        {
            let manager = ConfigManager::load(dir.path()).unwrap();
            manager
                .update(|config| {
                    config.device.name = "Salón".to_owned();
                    config.initialized = true;
                    config.network.dns.provider = DnsProvider::quad9();
                    Ok(())
                })
                .unwrap();
        }
        let manager = ConfigManager::load(dir.path()).unwrap();
        assert_eq!(manager.snapshot().device.name, "Salón");
        assert!(manager.snapshot().initialized);
        assert_eq!(
            manager.snapshot().network.dns.provider,
            DnsProvider::quad9()
        );
    }

    #[test]
    fn invalid_update_is_not_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let manager = ConfigManager::load(dir.path()).unwrap();
        let result = manager.update(|config| {
            config.device.language = "xx".to_owned();
            config.initialized = true;
            Ok(())
        });
        assert!(result.is_err());
        assert!(!manager.snapshot().initialized);
        let reloaded = ConfigManager::load(dir.path()).unwrap();
        assert!(!reloaded.snapshot().initialized);
    }

    #[test]
    fn closure_error_keeps_previous_state() {
        let dir = tempfile::tempdir().unwrap();
        let manager = ConfigManager::load(dir.path()).unwrap();
        let err = manager
            .update(|_| Err(CoreError::Config("nope".to_owned())))
            .unwrap_err();
        assert!(matches!(err, CoreError::Config(_)));
        assert_eq!(manager.snapshot().device.name, "HomeServer Assistant");
    }
}
