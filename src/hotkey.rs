use anyhow::{Result, bail};
use rdev::{Event, EventType, Key};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

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
        // F13–F20 are not in rdev's Key enum; use Unknown with macOS CGKeyCodes
        "f13" => Ok(Key::Unknown(105)),
        "f14" => Ok(Key::Unknown(107)),
        "f15" => Ok(Key::Unknown(113)),
        "f16" => Ok(Key::Unknown(106)),
        "f17" => Ok(Key::Unknown(64)),
        "f18" => Ok(Key::Unknown(79)),
        "f19" => Ok(Key::Unknown(80)),
        "f20" => Ok(Key::Unknown(90)),
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
///
/// For the fn/Globe key, uses IOHIDManager (works under `launchd`).
/// For all other keys, uses CGEventTap via `rdev::listen`.
pub fn start_listener(target_key: Key, mode: HotkeyMode) -> Result<mpsc::Receiver<HotkeyEvent>> {
    // The fn/Globe key cannot be captured by CGEventTap under launchd.
    // Delegate to the IOHIDManager-based listener.
    if target_key == Key::Function {
        tracing::info!("using IOHIDManager for fn/Globe key capture");
        return crate::hid_listener::start_fn_listener(mode);
    }

    let (tx, rx) = mpsc::channel();
    let (startup_tx, startup_rx) = mpsc::sync_channel::<Result<(), String>>(1);

    thread::spawn(move || {
        let mut state = HotkeyState::new(target_key, mode);
        let started = Arc::new(AtomicBool::new(false));
        let started_clone = started.clone();
        let startup_tx_clone = startup_tx.clone();

        let callback = move |event: Event| {
            // Signal successful start on first event received
            if !started_clone.swap(true, Ordering::SeqCst) {
                let _ = startup_tx_clone.send(Ok(()));
            }
            if let Some(hotkey_event) = state.process(&event.event_type) {
                let _ = tx.send(hotkey_event);
            }
        };

        if let Err(e) = rdev::listen(callback) {
            // rdev::listen returned immediately → CGEventTap failed
            let _ = startup_tx.send(Err(format!("{:?}", e)));
            tracing::error!("hotkey listener failed: {:?}", e);
        }
    });

    // Wait briefly — if rdev::listen fails (e.g. no Accessibility permission),
    // it fails fast. If it blocks, the listener is running successfully.
    match startup_rx.recv_timeout(Duration::from_secs(3)) {
        Ok(Err(e)) => bail!(
            "hotkey listener failed to start (Accessibility permission missing?): {}",
            e
        ),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            // No error and no events yet — listener is running fine
            Ok(rx)
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            bail!("hotkey listener thread terminated unexpectedly")
        }
        Ok(Ok(())) => Ok(rx),
    }
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
        assert_eq!(parse_key("F18").unwrap(), Key::Unknown(79));
        assert_eq!(parse_key("f18").unwrap(), Key::Unknown(79));
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
        let mut state = HotkeyState::new(Key::Unknown(79), HotkeyMode::Toggle);

        // First press → Start
        let event = state.process(&EventType::KeyPress(Key::Unknown(79)));
        assert_eq!(event, Some(HotkeyEvent::StartRecording));

        // Release → ignored
        let event = state.process(&EventType::KeyRelease(Key::Unknown(79)));
        assert_eq!(event, None);

        // Second press → Stop
        let event = state.process(&EventType::KeyPress(Key::Unknown(79)));
        assert_eq!(event, Some(HotkeyEvent::StopRecording));

        // Release → ignored
        let event = state.process(&EventType::KeyRelease(Key::Unknown(79)));
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
