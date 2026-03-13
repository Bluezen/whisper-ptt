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
./scripts/bundle.sh
```

Ceci produit `target/WhisperPTT.app` — un Application Bundle macOS prêt à l'emploi.

## Usage

### Premier lancement (permissions)

```bash
open target/WhisperPTT.app
```

macOS demandera les permissions suivantes :
- **Microphone** — capture audio pour la transcription
- **Accessibilité** — simulation Cmd+V pour coller le texte
- **Surveillance de l'entrée** — écoute globale de la touche push-to-talk

Accordez les trois, puis le programme démarre en arrière-plan (pas d'icône dans le Dock).

### Lancement direct (développement)

```bash
./target/release/whisper-ptt
```

> **Note** : en lancement direct depuis un terminal, les permissions sont associées au terminal, pas au binaire.

### Premier démarrage

Au premier lancement, le programme :
1. Crée `~/.whisper-ptt/config.toml` avec la configuration par défaut
2. Télécharge le modèle Whisper configuré (~1.6 Go pour large-v3-turbo)
3. Commence à écouter la touche push-to-talk

### fn Key Setup

Si vous utilisez la touche `fn` par défaut, allez dans Réglages Système → Clavier et réglez "Appuyer sur la touche fn pour" → "Ne rien faire". Sinon le système l'interceptera.

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

### Prérequis

1. **Compilez et créez le bundle** : `cargo build --release && ./scripts/bundle.sh`
2. **Lancez une première fois** : `open target/WhisperPTT.app`
3. **Accordez toutes les permissions** (Microphone, Accessibilité, Surveillance de l'entrée)

> Les permissions sont associées au `.app` et persistent entre les redémarrages. Sous launchd, le programme utilise IOHIDManager pour capter la touche fn — pas besoin de terminal.

### Configuration du Launch Agent

Créez `~/Library/LaunchAgents/com.whisper-ptt.plist` :

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

Remplacez `/path/to/WhisperPTT.app` par le chemin absolu du bundle.

```bash
# Charger le Launch Agent
launchctl load ~/Library/LaunchAgents/com.whisper-ptt.plist

# Recharger après modification du plist
launchctl unload ~/Library/LaunchAgents/com.whisper-ptt.plist
launchctl load ~/Library/LaunchAgents/com.whisper-ptt.plist
```

### Diagnostic

```bash
# Vérifier que le processus tourne
launchctl list | grep whisper-ptt

# Consulter les logs de démarrage
cat /tmp/whisper-ptt.stderr.log

# Consulter le log applicatif
ls ~/.whisper-ptt/whisper-ptt.log.*
cat ~/.whisper-ptt/whisper-ptt.log.$(date +%Y-%m-%d)
```
