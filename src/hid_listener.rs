//! IOHIDManager-based fn/Globe key listener for macOS.
//!
//! CGEventTap (used by `rdev`) cannot reliably capture the fn/Globe key when
//! the process is launched by `launchd`. IOHIDManager operates at the HID
//! driver level — below the system's fn-key interception — and works in any
//! execution context.

use crate::hotkey::{HotkeyEvent, HotkeyMode};
use anyhow::{bail, Result};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

// ── Apple HID constants ─────────────────────────────────────────────────────

/// Apple Vendor Top Case usage page (contains the fn/Globe key).
const KHID_PAGE_APPLE_VENDOR_TOP_CASE: u32 = 0x00FF;

/// fn/Globe key usage within the Apple Vendor Top Case page.
const KHID_USAGE_AV_TOP_CASE_KEYBOARD_FN: u32 = 0x0003;

// ── IOKit HID FFI ───────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
mod ffi {
    use std::ffi::c_void;

    pub type IOHIDManagerRef = *mut c_void;
    pub type IOHIDValueRef = *const c_void;
    pub type IOHIDElementRef = *const c_void;
    pub type IOReturn = i32;
    pub type IOOptionBits = u32;

    pub const KIOHID_OPTIONS_TYPE_NONE: IOOptionBits = 0;

    pub type IOHIDValueCallback = extern "C" fn(
        context: *mut c_void,
        result: IOReturn,
        sender: *mut c_void,
        value: IOHIDValueRef,
    );

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        pub fn IOHIDManagerCreate(
            allocator: core_foundation_sys::base::CFAllocatorRef,
            options: IOOptionBits,
        ) -> IOHIDManagerRef;

        pub fn IOHIDManagerSetDeviceMatching(
            manager: IOHIDManagerRef,
            matching: core_foundation_sys::dictionary::CFDictionaryRef,
        );

        pub fn IOHIDManagerRegisterInputValueCallback(
            manager: IOHIDManagerRef,
            callback: IOHIDValueCallback,
            context: *mut c_void,
        );

        pub fn IOHIDManagerScheduleWithRunLoop(
            manager: IOHIDManagerRef,
            run_loop: core_foundation_sys::runloop::CFRunLoopRef,
            run_loop_mode: core_foundation_sys::string::CFStringRef,
        );

        pub fn IOHIDManagerOpen(
            manager: IOHIDManagerRef,
            options: IOOptionBits,
        ) -> IOReturn;

        pub fn IOHIDValueGetElement(value: IOHIDValueRef) -> IOHIDElementRef;
        pub fn IOHIDElementGetUsagePage(element: IOHIDElementRef) -> u32;
        pub fn IOHIDElementGetUsage(element: IOHIDElementRef) -> u32;
        pub fn IOHIDValueGetIntegerValue(value: IOHIDValueRef) -> isize;
    }
}

// ── Callback context ────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
struct CallbackContext {
    tx: mpsc::Sender<HotkeyEvent>,
    mode: HotkeyMode,
    is_recording: bool,
}

#[cfg(target_os = "macos")]
extern "C" fn hid_value_callback(
    context: *mut std::ffi::c_void,
    _result: ffi::IOReturn,
    _sender: *mut std::ffi::c_void,
    value: ffi::IOHIDValueRef,
) {
    unsafe {
        let element = ffi::IOHIDValueGetElement(value);
        let page = ffi::IOHIDElementGetUsagePage(element);
        let usage = ffi::IOHIDElementGetUsage(element);

        if page != KHID_PAGE_APPLE_VENDOR_TOP_CASE
            || usage != KHID_USAGE_AV_TOP_CASE_KEYBOARD_FN
        {
            return;
        }

        let pressed = ffi::IOHIDValueGetIntegerValue(value) != 0;
        let ctx = &mut *(context as *mut CallbackContext);

        let event = match ctx.mode {
            HotkeyMode::Hold => {
                if pressed {
                    Some(HotkeyEvent::StartRecording)
                } else {
                    Some(HotkeyEvent::StopRecording)
                }
            }
            HotkeyMode::Toggle => {
                if pressed {
                    if ctx.is_recording {
                        ctx.is_recording = false;
                        Some(HotkeyEvent::StopRecording)
                    } else {
                        ctx.is_recording = true;
                        Some(HotkeyEvent::StartRecording)
                    }
                } else {
                    None
                }
            }
        };

        if let Some(ev) = event {
            let _ = ctx.tx.send(ev);
        }
    }
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Start a low-level HID listener for the fn/Globe key.
///
/// Unlike `rdev::listen` (CGEventTap), this works when the process is
/// launched by `launchd` because IOHIDManager captures events before the
/// system's fn-key interception layer.
///
/// Requires **Input Monitoring** permission (System Settings → Privacy &
/// Security → Input Monitoring).
#[cfg(target_os = "macos")]
pub fn start_fn_listener(mode: HotkeyMode) -> Result<mpsc::Receiver<HotkeyEvent>> {
    let (tx, rx) = mpsc::channel();
    let (startup_tx, startup_rx) = mpsc::sync_channel::<Result<(), String>>(1);

    thread::spawn(move || {
        unsafe {
            // 1. Create HID manager
            let manager = ffi::IOHIDManagerCreate(
                core_foundation_sys::base::kCFAllocatorDefault,
                ffi::KIOHID_OPTIONS_TYPE_NONE,
            );
            if manager.is_null() {
                let _ = startup_tx.send(Err("IOHIDManagerCreate returned null".into()));
                return;
            }

            // 2. Match ALL HID devices (NULL = no filter).
            //    We filter for the fn key in the callback.
            ffi::IOHIDManagerSetDeviceMatching(manager, std::ptr::null());

            // 3. Allocate callback context (leaked into a raw ptr; lives as
            //    long as the run-loop thread).
            let ctx = Box::into_raw(Box::new(CallbackContext {
                tx,
                mode,
                is_recording: false,
            }));

            ffi::IOHIDManagerRegisterInputValueCallback(
                manager,
                hid_value_callback,
                ctx as *mut std::ffi::c_void,
            );

            // 4. Schedule on the current thread's run loop
            ffi::IOHIDManagerScheduleWithRunLoop(
                manager,
                core_foundation_sys::runloop::CFRunLoopGetCurrent(),
                core_foundation_sys::runloop::kCFRunLoopDefaultMode,
            );

            // 5. Open — may fail if Input Monitoring permission is missing
            let ret = ffi::IOHIDManagerOpen(manager, ffi::KIOHID_OPTIONS_TYPE_NONE);
            if ret != 0 {
                let _ = startup_tx.send(Err(format!(
                    "IOHIDManagerOpen failed (code {}). Check Input Monitoring permission.",
                    ret
                )));
                drop(Box::from_raw(ctx));
                return;
            }

            // Signal success
            let _ = startup_tx.send(Ok(()));

            tracing::info!("fn key HID listener running (IOHIDManager)");

            // 6. Block on the run loop — processes HID callbacks
            core_foundation_sys::runloop::CFRunLoopRun();

            // Unreachable in practice
            drop(Box::from_raw(ctx));
        }
    });

    // Wait for startup confirmation
    match startup_rx.recv_timeout(Duration::from_secs(3)) {
        Ok(Ok(())) => {
            tracing::info!("fn key HID listener started successfully");
            Ok(rx)
        }
        Ok(Err(e)) => bail!("fn key HID listener failed: {}", e),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            // No error reported — assume success (CFRunLoop is blocking)
            Ok(rx)
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            bail!("fn key HID listener thread terminated unexpectedly")
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn start_fn_listener(_mode: HotkeyMode) -> Result<mpsc::Receiver<HotkeyEvent>> {
    bail!("fn/Globe key HID listener is only supported on macOS")
}
