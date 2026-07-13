use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub scheduler_tick_seconds: u64,
    pub subscription_sync_seconds: u64,
    pub platform_usage_sync_seconds: u64,
    pub manual_refresh_min_seconds: u64,
    pub stale_after_seconds: i64,
    pub retention: RetentionConfig,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RetentionConfig {
    pub raw_days: i64,
    pub hourly_days: i64,
    pub provider_events_days: i64,
}
impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            raw_days: 30,
            hourly_days: 180,
            provider_events_days: 30,
        }
    }
}
impl Default for Config {
    fn default() -> Self {
        Self {
            scheduler_tick_seconds: 30,
            subscription_sync_seconds: 300,
            platform_usage_sync_seconds: 600,
            manual_refresh_min_seconds: 30,
            stale_after_seconds: 1800,
            retention: RetentionConfig::default(),
        }
    }
}
impl Config {
    pub fn path() -> Result<PathBuf, io::Error> {
        Ok(crate::app_home()?.join("config.toml"))
    }
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(value) => {
                let parsed: Self = toml::from_str(&value)?;
                parsed.validate()?;
                Ok(parsed)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let config = Self::default();
                let encoded = toml::to_string_pretty(&config)?;
                std::fs::write(path, encoded)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
                }
                Ok(config)
            }
            Err(e) => Err(e.into()),
        }
    }
    fn validate(&self) -> Result<(), ConfigError> {
        if self.scheduler_tick_seconds == 0
            || self.subscription_sync_seconds < 30
            || self.platform_usage_sync_seconds < 30
            || self.retention.raw_days < 1
            || self.retention.hourly_days < self.retention.raw_days
        {
            return Err(ConfigError::Invalid);
        }
        Ok(())
    }
    pub fn interval_for(&self, connection_type: &str) -> u64 {
        match connection_type {
            "chatgpt_subscription" => self.subscription_sync_seconds,
            "platform_admin" => self.platform_usage_sync_seconds,
            _ => self.platform_usage_sync_seconds,
        }
    }
}
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("configuration I/O: {0}")]
    Io(#[from] io::Error),
    #[error("configuration syntax: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("configuration serialization: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("invalid configuration intervals or retention windows")]
    Invalid,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unknown_or_unsafe_values() {
        assert!(toml::from_str::<Config>("mystery=1").is_err());
        let mut c = Config::default();
        c.retention.hourly_days = 1;
        assert!(c.validate().is_err());
    }
}
