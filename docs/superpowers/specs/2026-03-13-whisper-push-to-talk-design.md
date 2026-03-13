# Whisper Push-to-Talk — Design Specification

## Overview

A lightweight, background macOS desktop application for automatic speech recognition (ASR) using OpenAI's Whisper model. The program listens for a configurable push-to-talk hotkey, captures audio from the microphone, transcribes it locally via whisper.cpp, and pastes the result at the cursor position. All transcriptions are stored in a local SQLite database for history.

**Primary goals**: simplicity, low latency, single binary, no UI.

## Architecture

Single Rust binary with 7 internal modules:

```
whisper-ptt
├── config        # Config loading/validation (TOML)
├── hotkey        # Global keyboard listener (rdev)
├── audio         # Mic capture (cpal) + feedback sounds + output muting (CoreAudio)
├── transcriber   # Whisper model loading + transcription (whisper-rs)
├── clipboard     # Copy to clipboard (arboard) + simulate Cmd+V (rdev)
├── history       # SQLite storage (rusqlite)
└── main          # Orchestration: event loop, lifecycle
```

## Configuration

File: `~/.whisper-ptt/config.toml` (created with defaults on first run).

```toml
[hotkey]
key = "fn"          # Key name: "fn", "F18", "RightAlt", "LeftControl", etc.
mode = "hold"       # "hold" (walkie-talkie) or "toggle" (press start, press stop)

[whisper]
model = "large-v3-turbo"  # tiny, base, small, medium, large, large-v3-turbo
language = "auto"          # "auto" for detection, or "fr", "en", etc.

[audio]
device = "default"                  # "default" or device name
mute_output_during_recording = true # Mute system output during capture

[clipboard]
restore_previous = true  # Restore previous clipboard content after pasting

[history]
database = "~/.whisper-ptt/history.db"
```

## Data Flow

```
1. Startup
   ├── Load config from ~/.whisper-ptt/config.toml
   ├── Init logging (tracing → ~/.whisper-ptt/whisper-ptt.log)
   ├── Check/download Whisper model to ~/.whisper-ptt/models/
   ├── Load model into memory (WhisperContext)
   ├── Open SQLite database
   ├── Start hotkey listener thread
   └── Log "Ready"

2. Recording cycle
   ├── PTT key pressed (or first press in toggle mode)
   │   ├── Play start sound
   │   ├── Mute system output (if enabled, via CoreAudio)
   │   └── Start audio capture (cpal, PCM 16kHz mono)
   │
   ├── PTT key released (or second press in toggle mode)
   │   ├── Stop audio capture → Vec<f32>
   │   ├── Unmute system output (restore previous volume)
   │   └── Play stop sound
   │
   ├── Transcription
   │   ├── whisper_ctx.full(audio_buffer, params)
   │   └── → String
   │
   ├── Paste
   │   ├── Save current clipboard content (if restore_previous)
   │   ├── Copy transcription to clipboard (arboard)
   │   ├── Wait ~50ms
   │   ├── Simulate Cmd+V (rdev)
   │   ├── Wait ~50ms
   │   └── Restore previous clipboard (if restore_previous)
   │
   └── History
       └── INSERT INTO transcriptions (text, language, model, duration_ms, created_at)

3. Shutdown (SIGINT/SIGTERM)
   ├── Unmute output if muted
   └── Exit
```

## Module Details

### config

- Loads and validates `~/.whisper-ptt/config.toml` via `serde` + `toml`
- If file doesn't exist, creates it with default values
- Validates key name against known `rdev::Key` variants
- Validates model name against supported list
- Resolves `~` paths to absolute paths via `dirs` crate

### hotkey

- Dedicated thread running `rdev::listen` for global keyboard events
- Communicates with main thread via `std::sync::mpsc::channel<HotkeyEvent>`
- Two modes:
  - **Hold**: `KeyPress` → `StartRecording`, `KeyRelease` → `StopRecording`
  - **Toggle**: first `KeyPress` → `StartRecording`, second `KeyPress` → `StopRecording` (ignores `KeyRelease`)
- Anti-repeat filtering: tracks key state to ignore OS key-repeat events in hold mode
- Maps config key names to `rdev::Key` enum variants

```rust
enum HotkeyEvent {
    StartRecording,
    StopRecording,
}
```

### audio

**Capture:**
- Opens input stream on configured device (or default) via `cpal`
- Captures PCM audio, resamples to 16kHz mono `Vec<f32>` (Whisper's expected format)
- Runs in dedicated thread, sends completed buffer via channel

**Feedback sounds:**
- Two `.wav` files embedded in binary via `include_bytes!`: `start.wav` and `stop.wav`
- Played via `rodio` on default output device, non-blocking

**Output muting (when `mute_output_during_recording = true`):**
- Uses `coreaudio-sys` to read/write `kAudioDevicePropertyMute` on default output device
- Sequence: play start sound → mute → capture → stop capture → unmute → play stop sound
- On shutdown: always unmute if currently muted (safety net)

### transcriber

- Loads model file into `whisper_rs::WhisperContext` at startup (stays resident in memory)
- Downloads model from HuggingFace whisper.cpp releases if not present in `~/.whisper-ptt/models/`
  - Download with progress bar via `reqwest` (blocking) + `indicatif`
- Supported models: `tiny`, `base`, `small`, `medium`, `large`, `large-v3-turbo`
- Transcription params:
  - `language`: configured value or `None` for auto-detection
  - `translate`: `false`
  - `single_segment`: `true` (short PTT utterances)
  - `print_progress`: `false`
- Synchronous call to `whisper_ctx.full()`
- Returns `String` (transcribed text)
- While transcription is in progress, PTT events are ignored (no queuing)

### clipboard

- Uses `arboard::Clipboard` for clipboard access
- Sequence:
  1. `clipboard.get_text()` — save previous content (if `restore_previous`)
  2. `clipboard.set_text(transcription)`
  3. Wait ~50ms
  4. Simulate `Cmd+V` via `rdev::simulate` (`MetaLeft` + `KeyV`)
  5. Wait ~50ms
  6. `clipboard.set_text(previous)` — restore (if `restore_previous`)
- Handles case where previous clipboard content is not text (image, etc.) gracefully

### history

SQLite database at configured path (default `~/.whisper-ptt/history.db`).

```sql
CREATE TABLE IF NOT EXISTS transcriptions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    text TEXT NOT NULL,
    language TEXT,
    model TEXT NOT NULL,
    duration_ms INTEGER,
    created_at TEXT NOT NULL  -- ISO 8601
);
```

- Database and table created on first run
- One INSERT per successful transcription
- No automatic purge — file stays small even with thousands of entries
- User can query with any SQLite tool (`sqlite3`, DB Browser, etc.)

### main

- Orchestrates all modules
- Event loop: blocks on `hotkey_receiver.recv()`, dispatches to audio/transcriber/clipboard/history
- Intercepts `SIGINT`/`SIGTERM` via `ctrlc` crate for clean shutdown
- No daemonization — user runs in terminal or configures `launchd`
- Example `launchd` plist provided in documentation

## Dependencies

| Crate | Purpose |
|---|---|
| `whisper-rs` (git: codeberg.org/tazz4843/whisper-rs) | Whisper.cpp Rust bindings |
| `cpal` | Cross-platform audio capture (CoreAudio on macOS) |
| `rodio` | Audio playback for feedback sounds |
| `rdev` | Global keyboard listener + key simulation |
| `arboard` | Cross-platform clipboard access |
| `coreaudio-sys` | macOS output muting via CoreAudio API |
| `rusqlite` (feature: `bundled`) | SQLite with embedded engine |
| `serde` + `toml` | Config deserialization |
| `reqwest` (feature: `blocking`) | HTTP model download |
| `indicatif` | Terminal progress bar for downloads |
| `dirs` | User directory resolution (~) |
| `tracing` + `tracing-subscriber` | Logging |
| `ctrlc` | Signal handling for clean shutdown |

## macOS Permissions

The application requires two macOS permissions (System Settings → Privacy & Security):
- **Accessibility**: for global keyboard listening and Cmd+V simulation
- **Microphone**: for audio capture

These must be granted manually by the user. The app logs a clear error if permissions are missing.

## File Layout on Disk

```
~/.whisper-ptt/
├── config.toml              # User configuration
├── models/
│   └── ggml-large-v3-turbo.bin  # Downloaded Whisper model(s)
├── history.db               # SQLite transcription history
└── whisper-ptt.log          # Log file
```

## Non-Goals (out of scope)

- GUI / tray icon / menu bar app
- Real-time streaming transcription
- Translation mode
- Multiple simultaneous recordings
- Automatic purge or rotation of history
- Built-in history viewer
- Daemonization (user manages via launchd or terminal)
