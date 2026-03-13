# Whisper Push-to-Talk Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a lightweight Rust binary that listens for a push-to-talk hotkey, captures audio, transcribes it locally via whisper.cpp, and pastes the result at the cursor position.

**Architecture:** Single Rust binary with 7 modules: config, hotkey, audio, transcriber, clipboard, history, main. Modules communicate via `mpsc` channels. The whisper model stays resident in memory. No UI — TOML config file, SQLite history, log file.

**Tech Stack:** Rust, whisper-rs (whisper.cpp bindings from Codeberg), cpal (audio capture), rdev (global hotkeys + key simulation), arboard (clipboard), rodio (sound playback), coreaudio-sys (output muting), rusqlite (SQLite), rubato (resampling), serde + toml (config).

**Spec:** `docs/superpowers/specs/2026-03-13-whisper-push-to-talk-design.md`

---

## File Structure

```
whisper-ptt/
├── Cargo.toml
├── src/
│   ├── main.rs           # Entry point, event loop, shutdown handling
│   ├── config.rs         # TOML config loading, validation, defaults
│   ├── hotkey.rs         # Global keyboard listener, hold/toggle modes
│   ├── audio/
│   │   ├── mod.rs        # Re-exports
│   │   ├── capture.rs    # Microphone capture via cpal + resampling
│   │   ├── feedback.rs   # Embedded sound playback via rodio
│   │   └── mute.rs       # macOS output muting via CoreAudio
│   ├── transcriber.rs    # Whisper model download, loading, transcription
│   ├── clipboard.rs      # Clipboard save/set/paste/restore
│   └── history.rs        # SQLite schema, insert, WAL mode
├── assets/
│   ├── start.wav         # Short beep for recording start
│   └── stop.wav          # Short beep for recording stop
└── README.md
```

---

## Chunk 1: Project Scaffolding + Config Module

### Task 1: Initialize Cargo Project

**Files:**
- Create: `Cargo.toml`
- Create: `src/main.rs`

- [ ] **Step 1: Create the Rust project**

Run:
```bash
cd /Users/along/Workspace/WhisperPushToTalk
cargo init --name whisper-ptt
```
Expected: `Created binary (application) package`

- [ ] **Step 2: Set up Cargo.toml with all dependencies**

Replace `Cargo.toml` with:

```toml
[package]
name = "whisper-ptt"
version = "0.1.0"
edition = "2021"
description = "Lightweight push-to-talk speech recognition using Whisper"

[dependencies]
# Whisper bindings
whisper-rs = { git = "https://codeberg.org/tazz4843/whisper-rs.git", tag = "v0.15.1" }

# Audio
cpal = "0.15"
rodio = "0.20"
rubato = "0.16"

# Keyboard
rdev = "0.5"

# Clipboard
arboard = "3"

# macOS audio muting
coreaudio-sys = "0.2"

# Database
rusqlite = { version = "0.32", features = ["bundled"] }

# Config
serde = { version = "1", features = ["derive"] }
toml = "0.8"

# Networking (model download)
reqwest = { version = "0.12", features = ["blocking"] }
indicatif = "0.17"

# Utilities
dirs = "6"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
tracing-appender = "0.2"
ctrlc = "3"
chrono = { version = "0.4", features = ["serde"] }
anyhow = "1"
sha2 = "0.10"

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 3: Write minimal main.rs to verify compilation**

```rust
fn main() {
    println!("whisper-ptt starting...");
}
```

- [ ] **Step 4: Verify the project compiles**

Run: `cargo check`
Expected: compiles with no errors (warnings OK)

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/main.rs
git commit -m "feat: initialize cargo project with dependencies"
```

---

### Task 2: Config Module

**Files:**
- Create: `src/config.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Write config unit tests**

Create `src/config.rs` with tests at the bottom:

```rust
use anyhow::{Context, Result, bail};
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
                restore_previous: true,
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
        }
    }
}

const VALID_MODES: &[&str] = &["hold", "toggle"];
const VALID_MODELS: &[&str] = &["tiny", "base", "small", "medium", "large", "large-v3-turbo"];
const VALID_LOG_LEVELS: &[&str] = &["trace", "debug", "info", "warn", "error"];
const VALID_KEYS: &[&str] = &[
    "fn", "function", "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9", "f10",
    "f11", "f12", "f13", "f14", "f15", "f16", "f17", "f18", "f19", "f20",
    "leftalt", "leftoption", "rightalt", "rightoption",
    "leftcontrol", "leftctrl", "rightcontrol", "rightctrl",
    "leftshift", "rightshift",
    "leftmeta", "leftcmd", "leftcommand", "rightmeta", "rightcmd", "rightcommand",
    "space", "capslock", "escape", "esc",
];

/// Resolve ~ to the user's home directory.
pub fn resolve_path(path: &str) -> Result<PathBuf> {
    if let Some(rest) = path.strip_prefix("~/") {
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
            bail!("invalid hotkey key '{}', expected one of: fn, F18, RightAlt, LeftControl, etc.", self.hotkey.key);
        }
        if !VALID_MODES.contains(&self.hotkey.mode.as_str()) {
            bail!("invalid hotkey mode '{}', expected one of: {}", self.hotkey.mode, VALID_MODES.join(", "));
        }
        if !VALID_MODELS.contains(&self.whisper.model.as_str()) {
            bail!("invalid whisper model '{}', expected one of: {}", self.whisper.model, VALID_MODELS.join(", "));
        }
        if self.whisper.language != "auto" && self.whisper.language.len() != 2 {
            bail!("invalid language '{}', expected 'auto' or a 2-letter code like 'fr', 'en'", self.whisper.language);
        }
        if !VALID_LOG_LEVELS.contains(&self.logging.level.as_str()) {
            bail!("invalid log level '{}', expected one of: {}", self.logging.level, VALID_LOG_LEVELS.join(", "));
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
    use std::io::Write;

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
        // File doesn't exist yet
        assert!(!path.exists());
        let config = Config::default();
        config.save(&path).unwrap();
        assert!(path.exists());
        // Can re-read it
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
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test --lib config`
Expected: all tests pass

- [ ] **Step 3: Wire config into main.rs**

Update `src/main.rs`:

```rust
mod config;

use anyhow::Result;

fn main() -> Result<()> {
    let config = config::Config::load()?;
    println!("whisper-ptt loaded config: mode={}, model={}", config.hotkey.mode, config.whisper.model);
    Ok(())
}
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check`
Expected: compiles

- [ ] **Step 5: Commit**

```bash
git add src/config.rs src/main.rs Cargo.toml Cargo.lock
git commit -m "feat: add config module with TOML loading and validation"
```

---

## Chunk 2: History Module (SQLite)

### Task 3: History Module

**Files:**
- Create: `src/history.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Write history module with tests**

Create `src/history.rs`:

```rust
use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;

const SCHEMA_VERSION: i32 = 1;

pub struct History {
    conn: Connection,
}

impl History {
    /// Open (or create) the database at the given path.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("failed to open database: {}", path.display()))?;

        conn.execute_batch("PRAGMA journal_mode=WAL;")?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS transcriptions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                text TEXT NOT NULL,
                language TEXT,
                model TEXT NOT NULL,
                duration_ms INTEGER,
                created_at TEXT NOT NULL
            );"
        )?;

        // Set schema version if empty
        let count: i32 = conn.query_row(
            "SELECT COUNT(*) FROM schema_version", [], |row| row.get(0)
        )?;
        if count == 0 {
            conn.execute("INSERT INTO schema_version (version) VALUES (?1)", [SCHEMA_VERSION])?;
        }

        Ok(Self { conn })
    }

    /// Insert a transcription record.
    pub fn insert(
        &self,
        text: &str,
        language: Option<&str>,
        model: &str,
        duration_ms: u64,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO transcriptions (text, language, model, duration_ms, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![text, language, model, duration_ms as i64, now],
        )?;
        Ok(())
    }

    /// Get the count of transcriptions (for testing).
    pub fn count(&self) -> Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM transcriptions", [], |row| row.get(0)
        )?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_creates_tables() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let history = History::open(&db_path).unwrap();
        assert_eq!(history.count().unwrap(), 0);
    }

    #[test]
    fn test_insert_and_count() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let history = History::open(&db_path).unwrap();

        history.insert("hello world", Some("en"), "tiny", 1500).unwrap();
        assert_eq!(history.count().unwrap(), 1);

        history.insert("bonjour le monde", Some("fr"), "large-v3-turbo", 2300).unwrap();
        assert_eq!(history.count().unwrap(), 2);
    }

    #[test]
    fn test_insert_with_no_language() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let history = History::open(&db_path).unwrap();

        history.insert("test", None, "base", 800).unwrap();
        assert_eq!(history.count().unwrap(), 1);
    }

    #[test]
    fn test_schema_version() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let history = History::open(&db_path).unwrap();

        let version: i32 = history.conn.query_row(
            "SELECT version FROM schema_version", [], |row| row.get(0)
        ).unwrap();
        assert_eq!(version, 1);
    }

    #[test]
    fn test_wal_mode() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let history = History::open(&db_path).unwrap();

        let mode: String = history.conn.query_row(
            "PRAGMA journal_mode", [], |row| row.get(0)
        ).unwrap();
        assert_eq!(mode, "wal");
    }

    #[test]
    fn test_reopen_existing_db() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        {
            let history = History::open(&db_path).unwrap();
            history.insert("first", Some("en"), "tiny", 1000).unwrap();
        }
        {
            let history = History::open(&db_path).unwrap();
            assert_eq!(history.count().unwrap(), 1);
            history.insert("second", Some("fr"), "tiny", 1200).unwrap();
            assert_eq!(history.count().unwrap(), 2);
        }
    }
}
```

- [ ] **Step 2: Add module to main.rs**

Add `mod history;` to `src/main.rs`.

- [ ] **Step 3: Run tests**

Run: `cargo test --lib history`
Expected: all tests pass

- [ ] **Step 4: Commit**

```bash
git add src/history.rs src/main.rs
git commit -m "feat: add history module with SQLite storage and WAL mode"
```

---

## Chunk 3: Hotkey Module

### Task 4: Hotkey Module

**Files:**
- Create: `src/hotkey.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Write hotkey module**

Create `src/hotkey.rs`:

```rust
use anyhow::{Result, bail};
use rdev::{Event, EventType, Key};
use std::sync::mpsc;
use std::thread;

#[derive(Debug, Clone, PartialEq)]
pub enum HotkeyEvent {
    StartRecording,
    StopRecording,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HotkeyMode {
    Hold,
    Toggle,
}

impl HotkeyMode {
    pub fn from_str(s: &str) -> Result<Self> {
        match s {
            "hold" => Ok(Self::Hold),
            "toggle" => Ok(Self::Toggle),
            other => bail!("invalid hotkey mode: '{}'", other),
        }
    }
}

/// Map a config key name to an rdev::Key.
pub fn parse_key(name: &str) -> Result<Key> {
    match name.to_lowercase().as_str() {
        "fn" | "function" => Ok(Key::Function),
        "f1" => Ok(Key::F1),
        "f2" => Ok(Key::F2),
        "f3" => Ok(Key::F3),
        "f4" => Ok(Key::F4),
        "f5" => Ok(Key::F5),
        "f6" => Ok(Key::F6),
        "f7" => Ok(Key::F7),
        "f8" => Ok(Key::F8),
        "f9" => Ok(Key::F9),
        "f10" => Ok(Key::F10),
        "f11" => Ok(Key::F11),
        "f12" => Ok(Key::F12),
        "f13" => Ok(Key::F13),
        "f14" => Ok(Key::F14),
        "f15" => Ok(Key::F15),
        "f16" => Ok(Key::F16),
        "f17" => Ok(Key::F17),
        "f18" => Ok(Key::F18),
        "f19" => Ok(Key::F19),
        "f20" => Ok(Key::F20),
        "leftalt" | "leftoption" => Ok(Key::Alt),
        "rightalt" | "rightoption" => Ok(Key::AltGr),
        "leftcontrol" | "leftctrl" => Ok(Key::ControlLeft),
        "rightcontrol" | "rightctrl" => Ok(Key::ControlRight),
        "leftshift" => Ok(Key::ShiftLeft),
        "rightshift" => Ok(Key::ShiftRight),
        "leftmeta" | "leftcmd" | "leftcommand" => Ok(Key::MetaLeft),
        "rightmeta" | "rightcmd" | "rightcommand" => Ok(Key::MetaRight),
        "space" => Ok(Key::Space),
        "capslock" => Ok(Key::CapsLock),
        "escape" | "esc" => Ok(Key::Escape),
        other => bail!("unknown key name: '{}'. Use key names like 'fn', 'F18', 'RightAlt', 'LeftControl', etc.", other),
    }
}

/// State machine for processing key events into HotkeyEvents.
pub struct HotkeyState {
    target_key: Key,
    mode: HotkeyMode,
    is_pressed: bool,
    is_recording: bool,
}

impl HotkeyState {
    pub fn new(target_key: Key, mode: HotkeyMode) -> Self {
        Self {
            target_key,
            mode,
            is_pressed: false,
            is_recording: false,
        }
    }

    /// Process a raw key event and return an optional HotkeyEvent.
    pub fn process(&mut self, event_type: &EventType) -> Option<HotkeyEvent> {
        match event_type {
            EventType::KeyPress(key) if *key == self.target_key => {
                match self.mode {
                    HotkeyMode::Hold => {
                        if self.is_pressed {
                            // OS key repeat — ignore
                            return None;
                        }
                        self.is_pressed = true;
                        self.is_recording = true;
                        Some(HotkeyEvent::StartRecording)
                    }
                    HotkeyMode::Toggle => {
                        if self.is_pressed {
                            // Key repeat — ignore
                            return None;
                        }
                        self.is_pressed = true;
                        if self.is_recording {
                            self.is_recording = false;
                            Some(HotkeyEvent::StopRecording)
                        } else {
                            self.is_recording = true;
                            Some(HotkeyEvent::StartRecording)
                        }
                    }
                }
            }
            EventType::KeyRelease(key) if *key == self.target_key => {
                self.is_pressed = false;
                match self.mode {
                    HotkeyMode::Hold => {
                        if self.is_recording {
                            self.is_recording = false;
                            Some(HotkeyEvent::StopRecording)
                        } else {
                            None
                        }
                    }
                    HotkeyMode::Toggle => None, // Ignore releases in toggle mode
                }
            }
            _ => None,
        }
    }
}

/// Spawn a background thread that listens for global key events.
/// Returns a receiver for HotkeyEvents.
pub fn start_listener(target_key: Key, mode: HotkeyMode) -> Result<mpsc::Receiver<HotkeyEvent>> {
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let mut state = HotkeyState::new(target_key, mode);
        let callback = move |event: Event| {
            if let Some(hotkey_event) = state.process(&event.event_type) {
                // Ignore send errors — receiver might have been dropped during shutdown
                let _ = tx.send(hotkey_event);
            }
        };
        if let Err(e) = rdev::listen(callback) {
            tracing::error!("hotkey listener failed: {:?}", e);
        }
    });

    Ok(rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_key_fn() {
        assert_eq!(parse_key("fn").unwrap(), Key::Function);
        assert_eq!(parse_key("Fn").unwrap(), Key::Function);
        assert_eq!(parse_key("function").unwrap(), Key::Function);
    }

    #[test]
    fn test_parse_key_f18() {
        assert_eq!(parse_key("F18").unwrap(), Key::F18);
        assert_eq!(parse_key("f18").unwrap(), Key::F18);
    }

    #[test]
    fn test_parse_key_modifiers() {
        assert_eq!(parse_key("RightAlt").unwrap(), Key::AltGr);
        assert_eq!(parse_key("LeftControl").unwrap(), Key::ControlLeft);
        assert_eq!(parse_key("LeftCmd").unwrap(), Key::MetaLeft);
    }

    #[test]
    fn test_parse_key_unknown() {
        assert!(parse_key("FooBar").is_err());
    }

    #[test]
    fn test_hold_mode_press_release() {
        let mut state = HotkeyState::new(Key::Function, HotkeyMode::Hold);

        // Press → StartRecording
        let event = state.process(&EventType::KeyPress(Key::Function));
        assert_eq!(event, Some(HotkeyEvent::StartRecording));

        // Release → StopRecording
        let event = state.process(&EventType::KeyRelease(Key::Function));
        assert_eq!(event, Some(HotkeyEvent::StopRecording));
    }

    #[test]
    fn test_hold_mode_ignores_repeat() {
        let mut state = HotkeyState::new(Key::Function, HotkeyMode::Hold);

        state.process(&EventType::KeyPress(Key::Function));

        // Second press (repeat) → None
        let event = state.process(&EventType::KeyPress(Key::Function));
        assert_eq!(event, None);

        // Release still works
        let event = state.process(&EventType::KeyRelease(Key::Function));
        assert_eq!(event, Some(HotkeyEvent::StopRecording));
    }

    #[test]
    fn test_toggle_mode() {
        let mut state = HotkeyState::new(Key::F18, HotkeyMode::Toggle);

        // First press → Start
        let event = state.process(&EventType::KeyPress(Key::F18));
        assert_eq!(event, Some(HotkeyEvent::StartRecording));

        // Release → ignored
        let event = state.process(&EventType::KeyRelease(Key::F18));
        assert_eq!(event, None);

        // Second press → Stop
        let event = state.process(&EventType::KeyPress(Key::F18));
        assert_eq!(event, Some(HotkeyEvent::StopRecording));

        // Release → ignored
        let event = state.process(&EventType::KeyRelease(Key::F18));
        assert_eq!(event, None);
    }

    #[test]
    fn test_ignores_other_keys() {
        let mut state = HotkeyState::new(Key::Function, HotkeyMode::Hold);

        let event = state.process(&EventType::KeyPress(Key::Space));
        assert_eq!(event, None);
    }

    #[test]
    fn test_hotkey_mode_from_str() {
        assert_eq!(HotkeyMode::from_str("hold").unwrap(), HotkeyMode::Hold);
        assert_eq!(HotkeyMode::from_str("toggle").unwrap(), HotkeyMode::Toggle);
        assert!(HotkeyMode::from_str("push").is_err());
    }
}
```

- [ ] **Step 2: Add module to main.rs**

Add `mod hotkey;` to `src/main.rs`.

- [ ] **Step 3: Run tests**

Run: `cargo test --lib hotkey`
Expected: all tests pass

- [ ] **Step 4: Commit**

```bash
git add src/hotkey.rs src/main.rs
git commit -m "feat: add hotkey module with hold/toggle modes and key parsing"
```

---

## Chunk 4: Audio Module (Capture + Feedback + Muting)

### Task 5: Audio Feedback (Embedded Sounds)

**Files:**
- Create: `src/audio/mod.rs`
- Create: `src/audio/feedback.rs`
- Create: `assets/start.wav`
- Create: `assets/stop.wav`
- Modify: `src/main.rs`

- [ ] **Step 1: Generate placeholder WAV sounds**

We need two short beep sounds. Generate them programmatically using a small script:

Run:
```bash
cd /Users/along/Workspace/WhisperPushToTalk
mkdir -p assets
```

Then create a small Rust build script or use `sox` if available. Alternatively, create minimal WAVs via a quick Python script:

```bash
python3 -c "
import struct, math, wave

def make_beep(filename, freq, duration_ms=150, sample_rate=44100, volume=0.5):
    n_samples = int(sample_rate * duration_ms / 1000)
    with wave.open(filename, 'w') as f:
        f.setnchannels(1)
        f.setsampwidth(2)
        f.setframerate(sample_rate)
        for i in range(n_samples):
            t = i / sample_rate
            # Apply fade in/out envelope
            env = min(i / 500, 1.0) * min((n_samples - i) / 500, 1.0)
            sample = int(volume * env * 32767 * math.sin(2 * math.pi * freq * t))
            f.writeframes(struct.pack('<h', sample))

make_beep('assets/start.wav', 880, 120)  # A5 - higher pitch
make_beep('assets/stop.wav', 440, 120)   # A4 - lower pitch
"
```

- [ ] **Step 2: Write feedback module**

Create `src/audio/mod.rs`:

```rust
pub mod feedback;
pub mod capture;
pub mod mute;
```

Create `src/audio/feedback.rs`:

```rust
use anyhow::Result;
use rodio::{Decoder, OutputStream, Sink};
use std::io::Cursor;

const START_WAV: &[u8] = include_bytes!("../../assets/start.wav");
const STOP_WAV: &[u8] = include_bytes!("../../assets/stop.wav");

/// Play the start recording sound. Blocks until playback is complete.
pub fn play_start_sound_blocking() -> Result<()> {
    play_embedded_wav_blocking(START_WAV)
}

/// Play the stop recording sound. Non-blocking.
pub fn play_stop_sound() -> Result<()> {
    play_embedded_wav_nonblocking(STOP_WAV)
}

fn play_embedded_wav_blocking(wav_data: &[u8]) -> Result<()> {
    let (_stream, stream_handle) = OutputStream::try_default()?;
    let cursor = Cursor::new(wav_data);
    let source = Decoder::new(cursor)?;
    let sink = Sink::try_new(&stream_handle)?;
    sink.append(source);
    sink.sleep_until_end();
    Ok(())
}

fn play_embedded_wav_nonblocking(wav_data: &'static [u8]) -> Result<()> {
    std::thread::spawn(move || {
        let _ = play_embedded_wav_blocking(wav_data);
    });
    Ok(())
}
```

- [ ] **Step 3: Add module to main.rs**

Add `mod audio;` to `src/main.rs`.

- [ ] **Step 4: Verify compilation**

Run: `cargo check`
Expected: compiles (audio/capture.rs and audio/mute.rs will be created in next tasks, so create empty placeholder files for now)

Create placeholder files:
```rust
// src/audio/capture.rs
// Audio capture — implemented in Task 6

// src/audio/mute.rs
// Output muting — implemented in Task 7
```

Run: `cargo check`
Expected: compiles

- [ ] **Step 5: Commit**

```bash
git add src/audio/ assets/
git commit -m "feat: add audio feedback module with embedded start/stop sounds"
```

---

### Task 6: Audio Capture

**Files:**
- Create: `src/audio/capture.rs` (replace placeholder)

- [ ] **Step 1: Write audio capture module**

Replace `src/audio/capture.rs`:

```rust
use anyhow::{Context, Result, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, StreamConfig};
use rubato::{SincFixedIn, SincInterpolationParameters, SincInterpolationType, Resampler, WindowFunction};
use std::sync::{Arc, Mutex};

const TARGET_SAMPLE_RATE: u32 = 16_000;

/// Holds a recording session.
pub struct AudioCapture {
    stream: Stream,
    buffer: Arc<Mutex<Vec<f32>>>,
    device_sample_rate: u32,
    device_channels: u16,
}

/// Get the input device by name, or default.
fn get_input_device(name: &str) -> Result<Device> {
    let host = cpal::default_host();
    if name == "default" {
        host.default_input_device()
            .context("no default input device available")
    } else {
        let devices = host.input_devices().context("cannot list input devices")?;
        for device in devices {
            if let Ok(n) = device.name() {
                if n.contains(name) {
                    return Ok(device);
                }
            }
        }
        bail!("input device '{}' not found", name)
    }
}

impl AudioCapture {
    /// Start capturing audio from the given device name.
    pub fn start(device_name: &str) -> Result<Self> {
        let device = get_input_device(device_name)?;
        let config = device.default_input_config()
            .context("failed to get default input config")?;

        let sample_rate = config.sample_rate().0;
        let channels = config.channels();
        let sample_format = config.sample_format();

        let buffer: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
        let buffer_clone = Arc::clone(&buffer);

        let stream_config: StreamConfig = config.into();

        let stream = match sample_format {
            SampleFormat::F32 => device.build_input_stream(
                &stream_config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    let mut buf = buffer_clone.lock().unwrap();
                    buf.extend_from_slice(data);
                },
                |err| tracing::error!("audio capture error: {}", err),
                None,
            )?,
            SampleFormat::I16 => {
                let buffer_clone = Arc::clone(&buffer);
                device.build_input_stream(
                    &stream_config,
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        let mut buf = buffer_clone.lock().unwrap();
                        buf.extend(data.iter().map(|&s| s as f32 / i16::MAX as f32));
                    },
                    |err| tracing::error!("audio capture error: {}", err),
                    None,
                )?
            }
            SampleFormat::U16 => {
                let buffer_clone_u16 = Arc::clone(&buffer);
                device.build_input_stream(
                    &stream_config,
                    move |data: &[u16], _: &cpal::InputCallbackInfo| {
                        let mut buf = buffer_clone_u16.lock().unwrap();
                        buf.extend(data.iter().map(|&s| (s as f32 / u16::MAX as f32) * 2.0 - 1.0));
                    },
                    |err| tracing::error!("audio capture error: {}", err),
                    None,
                )?
            }
            _ => bail!("unsupported sample format: {:?}", sample_format),
        };

        stream.play().context("failed to start audio stream")?;

        Ok(Self {
            stream,
            buffer,
            device_sample_rate: sample_rate,
            device_channels: channels,
        })
    }

    /// Stop capturing and return the audio as 16kHz mono f32 samples.
    pub fn stop(self) -> Result<Vec<f32>> {
        drop(self.stream); // Stop the stream

        let raw = self.buffer.lock().unwrap().clone();

        if raw.is_empty() {
            return Ok(Vec::new());
        }

        // Convert to mono if stereo or multi-channel
        let mono = if self.device_channels > 1 {
            raw.chunks(self.device_channels as usize)
                .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
                .collect()
        } else {
            raw
        };

        // Resample to 16kHz if needed
        if self.device_sample_rate == TARGET_SAMPLE_RATE {
            return Ok(mono);
        }

        let params = SincInterpolationParameters {
            sinc_len: 256,
            f_cutoff: 0.95,
            interpolation: SincInterpolationType::Linear,
            oversampling_factor: 256,
            window: WindowFunction::BlackmanHarris2,
        };

        let chunk_size = 1024;
        let mut resampler = SincFixedIn::<f32>::new(
            TARGET_SAMPLE_RATE as f64 / self.device_sample_rate as f64,
            2.0,
            params,
            chunk_size,
            1, // mono
        )?;

        // Process in chunks
        let mut output = Vec::new();
        for chunk in mono.chunks(chunk_size) {
            // Pad last chunk if needed
            let input = if chunk.len() < chunk_size {
                let mut padded = chunk.to_vec();
                padded.resize(chunk_size, 0.0);
                padded
            } else {
                chunk.to_vec()
            };
            let resampled = resampler.process(&[&input], None)?;
            if let Some(channel) = resampled.into_iter().next() {
                output.extend(channel);
            }
        }
        Ok(output)
    }
}
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check`
Expected: compiles

- [ ] **Step 3: Commit**

```bash
git add src/audio/capture.rs
git commit -m "feat: add audio capture module with cpal and rubato resampling"
```

---

### Task 7: macOS Output Muting

**Files:**
- Create: `src/audio/mute.rs` (replace placeholder)

- [ ] **Step 1: Write mute module**

Replace `src/audio/mute.rs`:

```rust
use anyhow::{Context, Result};

#[cfg(target_os = "macos")]
mod macos {
    use anyhow::{Context, Result};
    use coreaudio_sys::*;
    use std::mem;
    // NOTE: verify constant names against coreaudio-sys 0.2 at build time.
    // Apple renamed kAudioObjectPropertyElementMaster to
    // kAudioObjectPropertyElementMain in newer SDKs. If compilation fails,
    // swap to the available variant.

    /// Get the default output audio device ID.
    fn default_output_device() -> Result<AudioDeviceID> {
        let mut device_id: AudioDeviceID = 0;
        let mut size = mem::size_of::<AudioDeviceID>() as u32;
        let address = AudioObjectPropertyAddress {
            mSelector: kAudioHardwarePropertyDefaultOutputDevice,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain,
        };

        let status = unsafe {
            AudioObjectGetPropertyData(
                kAudioObjectSystemObject,
                &address,
                0,
                std::ptr::null(),
                &mut size,
                &mut device_id as *mut _ as *mut _,
            )
        };

        if status != 0 {
            anyhow::bail!("failed to get default output device (status: {})", status);
        }
        Ok(device_id)
    }

    /// Get the current mute state of the default output device.
    pub fn is_muted() -> Result<bool> {
        let device_id = default_output_device()?;
        let mut muted: u32 = 0;
        let mut size = mem::size_of::<u32>() as u32;
        let address = AudioObjectPropertyAddress {
            mSelector: kAudioDevicePropertyMute,
            mScope: kAudioDevicePropertyScopeOutput,
            mElement: kAudioObjectPropertyElementMain,
        };

        let status = unsafe {
            AudioObjectGetPropertyData(
                device_id,
                &address,
                0,
                std::ptr::null(),
                &mut size,
                &mut muted as *mut _ as *mut _,
            )
        };

        if status != 0 {
            anyhow::bail!("failed to get mute state (status: {})", status);
        }
        Ok(muted != 0)
    }

    /// Set the mute state of the default output device.
    pub fn set_muted(mute: bool) -> Result<()> {
        let device_id = default_output_device()?;
        let muted: u32 = if mute { 1 } else { 0 };
        let size = mem::size_of::<u32>() as u32;
        let address = AudioObjectPropertyAddress {
            mSelector: kAudioDevicePropertyMute,
            mScope: kAudioDevicePropertyScopeOutput,
            mElement: kAudioObjectPropertyElementMain,
        };

        let status = unsafe {
            AudioObjectSetPropertyData(
                device_id,
                &address,
                0,
                std::ptr::null(),
                size,
                &muted as *const _ as *const _,
            )
        };

        if status != 0 {
            anyhow::bail!("failed to set mute state (status: {})", status);
        }
        Ok(())
    }
}

/// Mute the system output. Returns the previous mute state.
pub fn mute_output() -> Result<bool> {
    #[cfg(target_os = "macos")]
    {
        let was_muted = macos::is_muted()?;
        macos::set_muted(true)?;
        Ok(was_muted)
    }
    #[cfg(not(target_os = "macos"))]
    {
        tracing::warn!("output muting is only supported on macOS");
        Ok(false)
    }
}

/// Unmute the system output (or restore to a given state).
pub fn unmute_output(was_muted: bool) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        if !was_muted {
            macos::set_muted(false)?;
        }
        // If it was already muted before we started, leave it muted
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = was_muted;
        Ok(())
    }
}
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check`
Expected: compiles

- [ ] **Step 3: Commit**

```bash
git add src/audio/mute.rs
git commit -m "feat: add macOS output muting via CoreAudio"
```

---

## Chunk 5: Transcriber Module

### Task 8: Model Download + Transcription

**Files:**
- Create: `src/transcriber.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Write transcriber module**

Create `src/transcriber.rs`:

```rust
use anyhow::{Context, Result, bail};
use indicatif::{ProgressBar, ProgressStyle};
use sha2::{Sha256, Digest};
use std::io::Write;
use std::path::{Path, PathBuf};
// NOTE: whisper-rs API may differ between versions. The code below targets v0.15.1.
// If method signatures differ at build time, check docs.rs/whisper-rs for the pinned version.
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

struct ModelInfoOwned {
    filename: String,
    url: String,
    sha256: String,
}

fn get_model_info(name: &str) -> Result<ModelInfoOwned> {
    let base = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";
    let (filename, sha256) = match name {
        "tiny" => ("ggml-tiny.bin", "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21"),
        "base" => ("ggml-base.bin", "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b7f0291dbddd5c0b24"),
        "small" => ("ggml-small.bin", "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1571c4b527"),
        "medium" => ("ggml-medium.bin", "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208"),
        "large" => ("ggml-large-v3.bin", "ad82bf6a9043ceed055076d0fd39f5f186ff25b81e5f0f3c1b5c774044e34c1e"),
        "large-v3-turbo" => ("ggml-large-v3-turbo.bin", "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69"),
        other => bail!("unknown model: '{}'", other),
    };
    Ok(ModelInfoOwned {
        filename: filename.to_string(),
        url: format!("{}/{}", base, filename),
        sha256: sha256.to_string(),
    })
}

/// Ensure the model file exists, downloading if needed.
pub fn ensure_model(model_name: &str, models_dir: &Path) -> Result<PathBuf> {
    let info = get_model_info(model_name)?;
    let model_path = models_dir.join(&info.filename);

    if model_path.exists() {
        tracing::info!("model already present: {}", model_path.display());
        return Ok(model_path);
    }

    std::fs::create_dir_all(models_dir)?;
    let part_path = models_dir.join(format!("{}.part", info.filename));

    tracing::info!("downloading model '{}' from {}", model_name, info.url);
    println!("Downloading model '{}'...", model_name);

    let response = reqwest::blocking::get(&info.url)
        .with_context(|| format!("failed to download model from {}", info.url))?;

    if !response.status().is_success() {
        bail!("download failed with status: {}", response.status());
    }

    let total_size = response.content_length().unwrap_or(0);
    let pb = ProgressBar::new(total_size);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
            .unwrap()
            .progress_chars("=>-"),
    );

    let mut file = std::fs::File::create(&part_path)
        .with_context(|| format!("failed to create {}", part_path.display()))?;

    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;
    let mut reader = response;

    let mut buf = vec![0u8; 8192];
    loop {
        let n = std::io::Read::read(&mut reader, &mut buf)?;
        if n == 0 { break; }
        file.write_all(&buf[..n])?;
        hasher.update(&buf[..n]);
        downloaded += n as u64;
        pb.set_position(downloaded);
    }

    pb.finish_with_message("download complete");
    file.flush()?;
    drop(file);

    // Verify checksum
    if !info.sha256.is_empty() {
        let hash = format!("{:x}", hasher.finalize());
        if hash != info.sha256 {
            std::fs::remove_file(&part_path).ok();
            bail!(
                "checksum mismatch for {}:\n  expected: {}\n  got:      {}",
                info.filename, info.sha256, hash
            );
        }
        tracing::info!("checksum verified for {}", info.filename);
    }

    std::fs::rename(&part_path, &model_path)?;
    println!("Model saved to {}", model_path.display());
    Ok(model_path)
}

/// Wrapper around WhisperContext + WhisperState for transcription.
pub struct Transcriber {
    ctx: WhisperContext,
    language: Option<String>,
}

impl Transcriber {
    /// Load a model from file.
    pub fn new(model_path: &Path, language: &str) -> Result<Self> {
        tracing::info!("loading whisper model from {}", model_path.display());
        let ctx = WhisperContext::new_with_params(
            model_path.to_str().context("invalid model path")?,
            WhisperContextParameters::default(),
        ).context("failed to load whisper model")?;

        let language = if language == "auto" {
            None
        } else {
            Some(language.to_string())
        };

        Ok(Self { ctx, language })
    }

    /// Transcribe audio samples (16kHz mono f32). Returns (text, detected_language).
    pub fn transcribe(&self, audio: &[f32]) -> Result<(String, Option<String>)> {
        let mut state = self.ctx.create_state()
            .context("failed to create whisper state")?;

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(self.language.as_deref());
        params.set_translate(false);
        params.set_single_segment(true);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_special(false);
        params.set_print_timestamps(false);

        state.full(params, audio)
            .context("whisper transcription failed")?;

        let num_segments = state.full_n_segments()
            .context("failed to get segment count")?;

        let mut text = String::new();
        for i in 0..num_segments {
            if let Ok(segment_text) = state.full_get_segment_text(i) {
                text.push_str(&segment_text);
            }
        }

        let detected_lang = state.full_lang_id()
            .ok()
            .and_then(|id| {
                whisper_rs::get_lang_str(id).ok().map(|s| s.to_string())
            });

        Ok((text.trim().to_string(), detected_lang))
    }
}
```

- [ ] **Step 2: Add module to main.rs**

Add `mod transcriber;` to `src/main.rs`.

- [ ] **Step 3: Verify compilation**

Run: `cargo check`
Expected: compiles (may take a while — whisper-rs compiles whisper.cpp)

- [ ] **Step 4: Commit**

```bash
git add src/transcriber.rs src/main.rs
git commit -m "feat: add transcriber module with model download and whisper-rs integration"
```

---

## Chunk 6: Clipboard Module

### Task 9: Clipboard Paste

**Files:**
- Create: `src/clipboard.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Write clipboard module**

Create `src/clipboard.rs`:

```rust
use anyhow::{Context, Result};
use arboard::Clipboard;
use rdev::{simulate, EventType, Key};
use std::thread;
use std::time::Duration;

/// Saved clipboard content for restoration.
enum SavedClipboard {
    Text(String),
    Image(arboard::ImageData<'static>),
    None,
}

/// Save the current clipboard content.
fn save_clipboard(clipboard: &mut Clipboard) -> SavedClipboard {
    if let Ok(text) = clipboard.get_text() {
        return SavedClipboard::Text(text);
    }
    if let Ok(image) = clipboard.get_image() {
        // Convert to owned so it has 'static lifetime
        let owned = arboard::ImageData {
            width: image.width,
            height: image.height,
            bytes: image.bytes.into_owned().into(),
        };
        return SavedClipboard::Image(owned);
    }
    tracing::debug!("clipboard content is not text or image, cannot save");
    SavedClipboard::None
}

/// Restore previously saved clipboard content.
fn restore_clipboard(clipboard: &mut Clipboard, saved: SavedClipboard) {
    match saved {
        SavedClipboard::Text(text) => {
            if let Err(e) = clipboard.set_text(&text) {
                tracing::debug!("failed to restore clipboard text: {}", e);
            }
        }
        SavedClipboard::Image(image) => {
            if let Err(e) = clipboard.set_image(image) {
                tracing::debug!("failed to restore clipboard image: {}", e);
            }
        }
        SavedClipboard::None => {}
    }
}

/// Simulate a key press + release with a small delay.
fn simulate_key(event: EventType) {
    if let Err(e) = simulate(&event) {
        tracing::error!("failed to simulate key event: {:?}", e);
    }
    thread::sleep(Duration::from_millis(20));
}

/// Paste text at cursor position via clipboard + Cmd+V.
pub fn paste_text(
    text: &str,
    restore_previous: bool,
    paste_delay_ms: u64,
    restore_delay_ms: u64,
) -> Result<()> {
    let mut clipboard = Clipboard::new().context("failed to open clipboard")?;

    // Save previous content if needed
    let saved = if restore_previous {
        save_clipboard(&mut clipboard)
    } else {
        SavedClipboard::None
    };

    // Set transcription in clipboard
    clipboard.set_text(text).context("failed to set clipboard text")?;

    // Wait for clipboard to propagate
    thread::sleep(Duration::from_millis(paste_delay_ms));

    // Simulate Cmd+V
    simulate_key(EventType::KeyPress(Key::MetaLeft));
    simulate_key(EventType::KeyPress(Key::KeyV));
    simulate_key(EventType::KeyRelease(Key::KeyV));
    simulate_key(EventType::KeyRelease(Key::MetaLeft));

    // Wait for paste to be processed
    thread::sleep(Duration::from_millis(restore_delay_ms));

    // Restore previous clipboard
    if restore_previous {
        restore_clipboard(&mut clipboard, saved);
    }

    Ok(())
}
```

- [ ] **Step 2: Add module to main.rs**

Add `mod clipboard;` to `src/main.rs`.

- [ ] **Step 3: Verify compilation**

Run: `cargo check`
Expected: compiles

- [ ] **Step 4: Commit**

```bash
git add src/clipboard.rs src/main.rs
git commit -m "feat: add clipboard module with paste simulation and content restore"
```

---

## Chunk 7: Main Orchestration + Integration

### Task 10: Wire Everything Together in main.rs

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Write the full main.rs orchestration**

Replace `src/main.rs`:

```rust
mod audio;
mod clipboard;
mod config;
mod history;
mod hotkey;
mod transcriber;

use anyhow::{Context, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

fn main() -> Result<()> {
    // Load config
    let config = config::Config::load().context("failed to load config")?;

    // Init logging
    // Note: tracing-appender does not support size-based rotation natively.
    // We use daily rotation as a practical alternative. The max_file_size_mb
    // config is reserved for future use with a custom rotation strategy.
    let log_path = config.log_path()?;
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file_appender = tracing_appender::rolling::daily(
        log_path.parent().unwrap(),
        "whisper-ptt.log",
    );
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    let filter = config.logging.level.parse::<tracing_subscriber::filter::LevelFilter>()
        .unwrap_or(tracing_subscriber::filter::LevelFilter::INFO);
    tracing_subscriber::fmt()
        .with_writer(non_blocking)
        .with_max_level(filter)
        .init();

    tracing::info!("whisper-ptt starting");

    // Ensure model is downloaded
    let models_dir = config.models_dir()?;
    let model_path = transcriber::ensure_model(&config.whisper.model, &models_dir)?;

    // Load whisper model
    println!("Loading whisper model (this may take a moment)...");
    let transcriber = transcriber::Transcriber::new(&model_path, &config.whisper.language)?;
    println!("Model loaded.");

    // Open history database
    let db_path = config.database_path()?;
    let history = history::History::open(&db_path)?;

    // Parse hotkey config
    let target_key = hotkey::parse_key(&config.hotkey.key)?;
    let mode = hotkey::HotkeyMode::from_str(&config.hotkey.mode)?;

    // Start hotkey listener
    let hotkey_rx = hotkey::start_listener(target_key, mode)?;

    // Shutdown flag
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })?;

    println!("whisper-ptt ready. Press '{}' to record. Ctrl+C to quit.", config.hotkey.key);
    tracing::info!("ready — listening for hotkey '{}'", config.hotkey.key);

    let mut was_muted = false;
    let mut active_capture: Option<audio::capture::AudioCapture> = None;
    let mut recording_start: Option<Instant> = None;

    while running.load(Ordering::SeqCst) {
        // Use recv_timeout to allow checking the shutdown flag
        let event = match hotkey_rx.recv_timeout(std::time::Duration::from_millis(200)) {
            Ok(event) => event,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        };

        match event {
            hotkey::HotkeyEvent::StartRecording => {
                // Ignore if already recording or transcribing
                if active_capture.is_some() {
                    continue;
                }

                tracing::info!("recording started");

                // Play start sound (blocking — must finish before mute)
                if let Err(e) = audio::feedback::play_start_sound_blocking() {
                    tracing::warn!("failed to play start sound: {}", e);
                }

                // Mute output if configured
                if config.audio.mute_output_during_recording {
                    match audio::mute::mute_output() {
                        Ok(prev) => was_muted = prev,
                        Err(e) => tracing::warn!("failed to mute output: {}", e),
                    }
                }

                // Start audio capture
                match audio::capture::AudioCapture::start(&config.audio.device) {
                    Ok(capture) => {
                        active_capture = Some(capture);
                        recording_start = Some(Instant::now());
                    }
                    Err(e) => {
                        tracing::error!("failed to start audio capture: {}", e);
                        // Unmute if we muted
                        if config.audio.mute_output_during_recording {
                            let _ = audio::mute::unmute_output(was_muted);
                        }
                    }
                }
            }

            hotkey::HotkeyEvent::StopRecording => {
                let capture = match active_capture.take() {
                    Some(c) => c,
                    None => continue, // Not recording
                };

                let duration_ms = recording_start.take()
                    .map(|s| s.elapsed().as_millis() as u64)
                    .unwrap_or(0);

                tracing::info!("recording stopped ({}ms)", duration_ms);

                // Stop capture and get audio
                let audio_data = match capture.stop() {
                    Ok(data) => data,
                    Err(e) => {
                        tracing::error!("failed to stop audio capture: {}", e);
                        if config.audio.mute_output_during_recording {
                            let _ = audio::mute::unmute_output(was_muted);
                        }
                        continue;
                    }
                };

                // Unmute output
                if config.audio.mute_output_during_recording {
                    if let Err(e) = audio::mute::unmute_output(was_muted) {
                        tracing::warn!("failed to unmute output: {}", e);
                    }
                }

                // Play stop sound (non-blocking)
                if let Err(e) = audio::feedback::play_stop_sound() {
                    tracing::warn!("failed to play stop sound: {}", e);
                }

                // Check minimum duration
                if duration_ms < config.whisper.min_duration_ms {
                    tracing::debug!("recording too short ({}ms < {}ms), discarding",
                        duration_ms, config.whisper.min_duration_ms);
                    continue;
                }

                if audio_data.is_empty() {
                    tracing::debug!("no audio data captured, skipping");
                    continue;
                }

                // Transcribe (blocking — PTT events buffer in channel during this)
                tracing::info!("transcribing {} samples...", audio_data.len());
                match transcriber.transcribe(&audio_data) {
                    Ok((text, lang)) => {
                        if text.is_empty() {
                            tracing::debug!("transcription returned empty text");
                            continue;
                        }

                        tracing::info!("transcribed: '{}' (lang: {:?})", text, lang);

                        // Paste
                        if let Err(e) = clipboard::paste_text(
                            &text,
                            config.clipboard.restore_previous,
                            config.clipboard.paste_delay_ms,
                            config.clipboard.restore_delay_ms,
                        ) {
                            tracing::error!("failed to paste text: {}", e);
                        }

                        // Save to history
                        if let Err(e) = history.insert(
                            &text,
                            lang.as_deref(),
                            &config.whisper.model,
                            duration_ms,
                        ) {
                            tracing::error!("failed to save to history: {}", e);
                        }
                    }
                    Err(e) => {
                        tracing::error!("transcription failed: {}", e);
                    }
                }

                // Drain any PTT events that queued during transcription
                // (spec: PTT events are ignored entirely during transcription)
                while hotkey_rx.try_recv().is_ok() {}
            }
        }
    }

    // Shutdown
    tracing::info!("shutting down");
    if config.audio.mute_output_during_recording {
        let _ = audio::mute::unmute_output(was_muted);
    }
    println!("whisper-ptt stopped.");

    Ok(())
}
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check`
Expected: compiles

- [ ] **Step 3: Commit**

```bash
git add src/main.rs
git commit -m "feat: wire all modules together in main event loop"
```

---

### Task 11: README

**Files:**
- Create: `README.md`

- [ ] **Step 1: Write README**

Create `README.md`:

```markdown
# whisper-ptt

Lightweight push-to-talk speech recognition for macOS. Uses OpenAI's Whisper model (via whisper.cpp) to transcribe your voice and paste the result at your cursor position.

## Features

- Push-to-talk with configurable hotkey (hold or toggle mode)
- Local transcription via whisper.cpp — no internet needed after model download
- Automatic language detection (or fixed language)
- Audio feedback sounds for recording start/stop
- Optional system output muting during recording
- SQLite history of all transcriptions
- Simple TOML configuration

## Requirements

- macOS (Accessibility + Microphone permissions)
- Rust toolchain (for building)

## Installation

```bash
git clone <repo-url>
cd whisper-ptt
cargo build --release
```

The binary will be at `target/release/whisper-ptt`.

## Usage

```bash
./target/release/whisper-ptt
```

On first run, the program:
1. Creates `~/.whisper-ptt/config.toml` with default settings
2. Downloads the configured Whisper model (~1.6 GB for large-v3-turbo)
3. Starts listening for the push-to-talk key

### macOS Permissions

Grant these in System Settings → Privacy & Security:
- **Accessibility**: required for global hotkey and paste simulation
- **Microphone**: required for audio capture

### fn Key Setup

If using the default `fn` key, go to System Settings → Keyboard and set "Press fn key to" → "Do Nothing". Otherwise the system may intercept it.

## Configuration

Edit `~/.whisper-ptt/config.toml`:

```toml
[hotkey]
key = "fn"          # fn, F18, RightAlt, LeftControl, etc.
mode = "hold"       # hold (walkie-talkie) or toggle

[whisper]
model = "large-v3-turbo"  # tiny, base, small, medium, large, large-v3-turbo
language = "auto"          # auto, fr, en, etc.
min_duration_ms = 500

[audio]
device = "default"
mute_output_during_recording = true

[clipboard]
restore_previous = true
paste_delay_ms = 100
restore_delay_ms = 200

[history]
database = "~/.whisper-ptt/history.db"

[logging]
level = "info"
max_file_size_mb = 10
```

## History

Query your transcription history:

```bash
sqlite3 ~/.whisper-ptt/history.db "SELECT created_at, text FROM transcriptions ORDER BY id DESC LIMIT 10;"
```

## Run at Login (launchd)

Create `~/Library/LaunchAgents/com.whisper-ptt.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.whisper-ptt</string>
    <key>ProgramArguments</key>
    <array>
        <string>/path/to/whisper-ptt</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <false/>
</dict>
</plist>
```

Then: `launchctl load ~/Library/LaunchAgents/com.whisper-ptt.plist`
```

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs: add README with usage, config, and launchd setup"
```

---

### Task 12: Build and Manual Test

- [ ] **Step 1: Build release binary**

Run: `cargo build --release`
Expected: compiles successfully (may take several minutes due to whisper.cpp)

- [ ] **Step 2: Run the program**

Run: `./target/release/whisper-ptt`
Expected: downloads model on first run, then prints "whisper-ptt ready"

- [ ] **Step 3: Test the push-to-talk flow**

1. Press and hold the configured key
2. Speak a short sentence
3. Release the key
4. Verify text appears at cursor
5. Check history: `sqlite3 ~/.whisper-ptt/history.db "SELECT * FROM transcriptions;"`

- [ ] **Step 4: Final commit with any adjustments**

```bash
git add -A
git commit -m "chore: build verification and any final adjustments"
```
