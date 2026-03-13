use anyhow::Result;

#[cfg(target_os = "macos")]
mod macos {
    use anyhow::Result;
    use coreaudio_sys::*;
    use std::mem;

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
