use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub hotkey: HotkeyConfig,
    pub whisper: WhisperConfig,
    pub audio: AudioConfig,
    pub clipboard: ClipboardConfig,
    pub history: HistoryConfig,
    pub logging: LoggingConfig,
    #[serde(default = "default_notifications")]
    pub notifications: NotificationsConfig,
}

fn default_notifications() -> NotificationsConfig {
    NotificationsConfig { enabled: true }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotkeyConfig {
    pub key: String,
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhisperConfig {
    pub model: String,
    pub language: String,
    pub min_duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    pub device: String,
    pub mute_output_during_recording: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardConfig {
    pub restore_previous: bool,
    pub paste_delay_ms: u64,
    pub restore_delay_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryConfig {
    pub database: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
    pub max_file_size_mb: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationsConfig {
    pub enabled: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: HotkeyConfig {
                key: "fn".to_string(),
                mode: "hold".to_string(),
            },
            whisper: WhisperConfig {
                model: "large-v3-turbo".to_string(),
                language: "auto".to_string(),
                min_duration_ms: 500,
            },
            audio: AudioConfig {
                device: "default".to_string(),
                mute_output_during_recording: true,
            },
            clipboard: ClipboardConfig {
                restore_previous: false,
                paste_delay_ms: 100,
                restore_delay_ms: 200,
            },
            history: HistoryConfig {
                database: "~/.whisper-ptt/history.db".to_string(),
            },
            logging: LoggingConfig {
                level: "info".to_string(),
                max_file_size_mb: 10,
            },
            notifications: NotificationsConfig { enabled: true },
        }
    }
}

const VALID_MODES: &[&str] = &["hold", "toggle"];
const VALID_MODELS: &[&str] = &["tiny", "base", "small", "medium", "large", "large-v3-turbo"];
const VALID_LOG_LEVELS: &[&str] = &["trace", "debug", "info", "warn", "error"];
const VALID_KEYS: &[&str] = &[
    "fn",
    "function",
    "f1",
    "f2",
    "f3",
    "f4",
    "f5",
    "f6",
    "f7",
    "f8",
    "f9",
    "f10",
    "f11",
    "f12",
    "f13",
    "f14",
    "f15",
    "f16",
    "f17",
    "f18",
    "f19",
    "f20",
    "leftalt",
    "leftoption",
    "rightalt",
    "rightoption",
    "leftcontrol",
    "leftctrl",
    "rightcontrol",
    "rightctrl",
    "leftshift",
    "rightshift",
    "leftmeta",
    "leftcmd",
    "leftcommand",
    "rightmeta",
    "rightcmd",
    "rightcommand",
    "space",
    "capslock",
    "escape",
    "esc",
];

/// Resolve ~ to the user's home directory.
pub fn resolve_path(path: &str) -> Result<PathBuf> {
    if path == "~" {
        dirs::home_dir().context("cannot determine home directory")
    } else if let Some(rest) = path.strip_prefix("~/") {
        let home = dirs::home_dir().context("cannot determine home directory")?;
        Ok(home.join(rest))
    } else {
        Ok(PathBuf::from(path))
    }
}

/// Return the base directory for all whisper-ptt data (~/.whisper-ptt).
pub fn data_dir() -> Result<PathBuf> {
    resolve_path("~/.whisper-ptt")
}

/// Return the path to the config file.
pub fn config_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("config.toml"))
}

impl Config {
    /// Load config from file, or create default if missing.
    pub fn load() -> Result<Self> {
        let path = config_path()?;

        if !path.exists() {
            let config = Config::default();
            config.save(&path)?;
            return Ok(config);
        }

        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read config: {}", path.display()))?;
        let config: Config = toml::from_str(&content)
            .with_context(|| format!("failed to parse config: {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    /// Write config to the given path, creating parent dirs.
    fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Validate all config values.
    pub fn validate(&self) -> Result<()> {
        if !VALID_KEYS.contains(&self.hotkey.key.to_lowercase().as_str()) {
            bail!(
                "invalid hotkey key '{}', expected one of: fn, F18, RightAlt, LeftControl, etc.",
                self.hotkey.key
            );
        }
        if !VALID_MODES.contains(&self.hotkey.mode.as_str()) {
            bail!(
                "invalid hotkey mode '{}', expected one of: {}",
                self.hotkey.mode,
                VALID_MODES.join(", ")
            );
        }
        if !VALID_MODELS.contains(&self.whisper.model.as_str()) {
            bail!(
                "invalid whisper model '{}', expected one of: {}",
                self.whisper.model,
                VALID_MODELS.join(", ")
            );
        }
        if self.whisper.language != "auto" && self.whisper.language.len() != 2 {
            bail!(
                "invalid language '{}', expected 'auto' or a 2-letter code like 'fr', 'en'",
                self.whisper.language
            );
        }
        if !VALID_LOG_LEVELS.contains(&self.logging.level.as_str()) {
            bail!(
                "invalid log level '{}', expected one of: {}",
                self.logging.level,
                VALID_LOG_LEVELS.join(", ")
            );
        }
        Ok(())
    }

    /// Resolved path to the history database.
    pub fn database_path(&self) -> Result<PathBuf> {
        resolve_path(&self.history.database)
    }

    /// Resolved path to the models directory.
    pub fn models_dir(&self) -> Result<PathBuf> {
        Ok(data_dir()?.join("models"))
    }

    /// Resolved path to the log file.
    pub fn log_path(&self) -> Result<PathBuf> {
        Ok(data_dir()?.join("whisper-ptt.log"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_is_valid() {
        let config = Config::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_invalid_key() {
        let mut config = Config::default();
        config.hotkey.key = "FooBar".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_invalid_mode() {
        let mut config = Config::default();
        config.hotkey.mode = "push".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_invalid_model() {
        let mut config = Config::default();
        config.whisper.model = "huge".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_invalid_log_level() {
        let mut config = Config::default();
        config.logging.level = "verbose".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_resolve_path_with_tilde() {
        let path = resolve_path("~/test").unwrap();
        assert!(path.is_absolute());
        assert!(path.ends_with("test"));
    }

    #[test]
    fn test_resolve_path_without_tilde() {
        let path = resolve_path("/tmp/test").unwrap();
        assert_eq!(path, PathBuf::from("/tmp/test"));
    }

    #[test]
    fn test_roundtrip_toml() {
        let config = Config::default();
        let serialized = toml::to_string_pretty(&config).unwrap();
        let deserialized: Config = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.hotkey.key, "fn");
        assert_eq!(deserialized.whisper.model, "large-v3-turbo");
    }

    #[test]
    fn test_load_creates_default_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        assert!(!path.exists());
        let config = Config::default();
        config.save(&path).unwrap();
        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        let loaded: Config = toml::from_str(&content).unwrap();
        assert_eq!(loaded.hotkey.mode, "hold");
    }

    #[test]
    fn test_load_from_toml_string() {
        let toml_str = r#"
[hotkey]
key = "F18"
mode = "toggle"

[whisper]
model = "tiny"
language = "fr"
min_duration_ms = 300

[audio]
device = "default"
mute_output_during_recording = false

[clipboard]
restore_previous = false
paste_delay_ms = 50
restore_delay_ms = 100

[history]
database = "~/.whisper-ptt/history.db"

[logging]
level = "debug"
max_file_size_mb = 5
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.hotkey.key, "F18");
        assert_eq!(config.hotkey.mode, "toggle");
        assert_eq!(config.whisper.model, "tiny");
        assert_eq!(config.whisper.language, "fr");
        assert!(!config.audio.mute_output_during_recording);
        assert!(config.validate().is_ok());
    }
}
