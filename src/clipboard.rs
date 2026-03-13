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
    clipboard
        .set_text(text)
        .context("failed to set clipboard text")?;

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
