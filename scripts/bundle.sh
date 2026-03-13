#!/usr/bin/env bash
# ──────────────────────────────────────────────────────────────────────
# bundle.sh — Package whisper-ptt into a macOS .app bundle
#
# Usage:
#   ./scripts/bundle.sh            # uses target/release/whisper-ptt
#   ./scripts/bundle.sh debug      # uses target/debug/whisper-ptt
#
# Output:
#   target/WhisperPTT.app/
#     Contents/
#       Info.plist
#       MacOS/
#         whisper-ptt
# ──────────────────────────────────────────────────────────────────────
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

PROFILE="${1:-release}"
BINARY="$PROJECT_DIR/target/$PROFILE/whisper-ptt"

if [[ ! -f "$BINARY" ]]; then
    echo "error: binary not found at $BINARY"
    echo "       run 'cargo build --release' first"
    exit 1
fi

APP_DIR="$PROJECT_DIR/target/WhisperPTT.app"
CONTENTS="$APP_DIR/Contents"
MACOS="$CONTENTS/MacOS"

# Clean previous bundle
rm -rf "$APP_DIR"

# Create structure
mkdir -p "$MACOS"

# Copy Info.plist
cp "$PROJECT_DIR/bundle/Info.plist" "$CONTENTS/Info.plist"

# Copy binary
cp "$BINARY" "$MACOS/whisper-ptt"

# Ad-hoc code sign (required for TCC to track the .app properly)
codesign --force --sign - "$APP_DIR"

echo "✓ WhisperPTT.app created at $APP_DIR"
echo ""
echo "First-time setup:"
echo "  open $APP_DIR"
echo ""
echo "This will trigger macOS permission prompts for:"
echo "  • Microphone"
echo "  • Accessibility"
echo "  • Input Monitoring"
echo ""
echo "For launchd, use this in your plist:"
echo "  <key>ProgramArguments</key>"
echo "  <array>"
echo "    <string>$MACOS/whisper-ptt</string>"
echo "  </array>"
