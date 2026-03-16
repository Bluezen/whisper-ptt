use anyhow::{Context, Result};
use arboard::Clipboard;
use std::thread;
use std::time::Duration;

/// CoreGraphics FFI for reliable Cmd+V simulation with modifier flags.
#[cfg(target_os = "macos")]
mod cg_ffi {
    use std::ffi::c_void;

    pub type CGEventRef = *mut c_void;
    pub type CGEventSourceRef = *mut c_void;
    pub type CGKeyCode = u16;
    pub type CGEventFlags = u64;
    pub type CGEventTapLocation = u32;
    pub type CGEventSourceStateID = i32;

    pub const KCG_HID_EVENT_TAP: CGEventTapLocation = 0;
    pub const KCG_EVENT_SOURCE_STATE_HID_SYSTEM: CGEventSourceStateID = 1;
    pub const KCG_EVENT_FLAG_MASK_COMMAND: CGEventFlags = 0x00100000;

    /// macOS virtual keycode for 'V'
    pub const KVK_ANSI_V: CGKeyCode = 9;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        pub fn CGEventSourceCreate(stateID: CGEventSourceStateID) -> CGEventSourceRef;
        pub fn CGEventCreateKeyboardEvent(
            source: CGEventSourceRef,
            virtualKey: CGKeyCode,
            keyDown: bool,
        ) -> CGEventRef;
        pub fn CGEventSetFlags(event: CGEventRef, flags: CGEventFlags);
        pub fn CGEventPost(tap: CGEventTapLocation, event: CGEventRef);
        pub fn CFRelease(cf: *mut c_void);
    }
}

/// Simulate Cmd+V paste using CGEvent with explicit Command modifier flag.
/// Each key event carries the modifier directly, avoiding the race condition
/// where separate modifier and key events can arrive out of order.
#[cfg(target_os = "macos")]
fn simulate_paste() -> Result<()> {
    unsafe {
        let source = cg_ffi::CGEventSourceCreate(cg_ffi::KCG_EVENT_SOURCE_STATE_HID_SYSTEM);
        if source.is_null() {
            anyhow::bail!("CGEventSourceCreate returned null");
        }

        // Key-down V with Command flag
        let v_down = cg_ffi::CGEventCreateKeyboardEvent(source, cg_ffi::KVK_ANSI_V, true);
        if v_down.is_null() {
            cg_ffi::CFRelease(source);
            anyhow::bail!("CGEventCreateKeyboardEvent (down) returned null");
        }
        cg_ffi::CGEventSetFlags(v_down, cg_ffi::KCG_EVENT_FLAG_MASK_COMMAND);
        cg_ffi::CGEventPost(cg_ffi::KCG_HID_EVENT_TAP, v_down);
        cg_ffi::CFRelease(v_down);

        thread::sleep(Duration::from_millis(20));

        // Key-up V with Command flag
        let v_up = cg_ffi::CGEventCreateKeyboardEvent(source, cg_ffi::KVK_ANSI_V, false);
        if v_up.is_null() {
            cg_ffi::CFRelease(source);
            anyhow::bail!("CGEventCreateKeyboardEvent (up) returned null");
        }
        cg_ffi::CGEventSetFlags(v_up, cg_ffi::KCG_EVENT_FLAG_MASK_COMMAND);
        cg_ffi::CGEventPost(cg_ffi::KCG_HID_EVENT_TAP, v_up);
        cg_ffi::CFRelease(v_up);

        cg_ffi::CFRelease(source);
    }
    Ok(())
}

/// Fallback paste simulation using rdev for non-macOS platforms.
#[cfg(not(target_os = "macos"))]
fn simulate_paste() -> Result<()> {
    use rdev::{simulate, EventType, Key};

    fn simulate_key(event: EventType) {
        if let Err(e) = simulate(&event) {
            tracing::error!("failed to simulate key event: {:?}", e);
        }
        thread::sleep(Duration::from_millis(20));
    }

    simulate_key(EventType::KeyPress(Key::MetaLeft));
    simulate_key(EventType::KeyPress(Key::KeyV));
    simulate_key(EventType::KeyRelease(Key::KeyV));
    simulate_key(EventType::KeyRelease(Key::MetaLeft));
    Ok(())
}

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
    simulate_paste()?;

    // Wait for paste to be processed
    thread::sleep(Duration::from_millis(restore_delay_ms));

    // Restore previous clipboard
    if restore_previous {
        restore_clipboard(&mut clipboard, saved);
    }

    Ok(())
}
