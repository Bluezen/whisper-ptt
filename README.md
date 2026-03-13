# whisper-ptt

Lightweight push-to-talk speech recognition for macOS. Uses OpenAI's Whisper model (via whisper.cpp) to transcribe your voice and paste the result at your cursor position.

## Features

- Push-to-talk with configurable hotkey (hold or toggle mode)
- Local transcription via whisper.cpp — no internet needed after model download
- Automatic language detection (or fixed language)
- Audio feedback sounds for recording start/stop
- Optional system output muting during recording
- macOS notifications for transcription progress and result
- SQLite history of all transcriptions
- Simple TOML configuration

## Requirements

- macOS (Accessibility + Microphone permissions)
- Rust toolchain (for building)

## Installation

```bash
git clone https://github.com/Bluezen/whisper-ptt.git
cd whisper-ptt
cargo build --release
./scripts/bundle.sh
```

This produces `target/WhisperPTT.app` — a ready-to-use macOS Application Bundle.

## Usage

### First launch (permissions)

```bash
open target/WhisperPTT.app
```

macOS will request the following permissions:
- **Microphone** — audio capture for transcription
- **Accessibility** — Cmd+V simulation to paste text
- **Input Monitoring** — global listening for the push-to-talk key

Grant all three, then the program runs in the background (no Dock icon).

### Direct launch (development)

```bash
./target/release/whisper-ptt
```

> **Note**: when launching directly from a terminal, permissions are associated with the terminal app, not the binary itself.

### First startup

On first launch, the program:
1. Creates `~/.whisper-ptt/config.toml` with the default configuration
2. Downloads the configured Whisper model (~1.6 GB for large-v3-turbo)
3. Starts listening for the push-to-talk key

### fn Key Setup

If you use the default `fn` key, go to System Settings → Keyboard and set "Press fn key to" → "Do Nothing". Otherwise the system will intercept it.

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
restore_previous = false
paste_delay_ms = 100
restore_delay_ms = 200

[history]
database = "~/.whisper-ptt/history.db"

[notifications]
enabled = true          # macOS notifications (transcription progress + result)

[logging]
level = "info"
max_file_size_mb = 10
```

### Clipboard tip

By default, `restore_previous` is set to `false`, which means the transcribed text stays in your clipboard after pasting. This lets you paste it again with Cmd+V as many times as you want — useful if you need the text in multiple places. Set it to `true` if you prefer the clipboard to be restored to its previous content after pasting.

## History

Query your transcription history:

```bash
sqlite3 ~/.whisper-ptt/history.db "SELECT created_at, text FROM transcriptions ORDER BY id DESC LIMIT 10;"
```

## Run at Login (launchd)

### Prerequisites

1. **Build and create the bundle**: `cargo build --release && ./scripts/bundle.sh`
2. **Launch once**: `open target/WhisperPTT.app`
3. **Grant all permissions** (Microphone, Accessibility, Input Monitoring)

> Permissions are associated with the `.app` and persist across reboots. Under launchd, the program uses IOHIDManager to capture the fn key — no terminal needed.

### Launch Agent Configuration

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
        <string>/path/to/WhisperPTT.app/Contents/MacOS/whisper-ptt</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/tmp/whisper-ptt.stdout.log</string>
    <key>StandardErrorPath</key>
    <string>/tmp/whisper-ptt.stderr.log</string>
</dict>
</plist>
```

Replace `/path/to/WhisperPTT.app` with the absolute path to the bundle.

```bash
# Load the Launch Agent
launchctl load ~/Library/LaunchAgents/com.whisper-ptt.plist

# Reload after modifying the plist
launchctl unload ~/Library/LaunchAgents/com.whisper-ptt.plist
launchctl load ~/Library/LaunchAgents/com.whisper-ptt.plist
```

### Troubleshooting

```bash
# Check that the process is running
launchctl list | grep whisper-ptt

# View startup logs
cat /tmp/whisper-ptt.stderr.log

# View application log
ls ~/.whisper-ptt/whisper-ptt.log.*
cat ~/.whisper-ptt/whisper-ptt.log.$(date +%Y-%m-%d)
```
