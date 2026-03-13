use anyhow::{bail, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, StreamConfig};
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
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
        let config = device
            .default_input_config()
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
                    if let Ok(mut buf) = buffer_clone.lock() {
                        buf.extend_from_slice(data);
                    }
                },
                |err| tracing::error!("audio capture error: {}", err),
                None,
            )?,
            SampleFormat::I16 => {
                let buffer_clone = Arc::clone(&buffer);
                device.build_input_stream(
                    &stream_config,
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        if let Ok(mut buf) = buffer_clone.lock() {
                            buf.extend(data.iter().map(|&s| s as f32 / 32768.0));
                        }
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
                        if let Ok(mut buf) = buffer_clone_u16.lock() {
                            buf.extend(data.iter().map(|&s| (s as f32 / 65536.0) * 2.0 - 1.0));
                        }
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

        // Process in full chunks
        let mut output = Vec::new();
        let full_chunks = mono.len() / chunk_size;
        for i in 0..full_chunks {
            let start = i * chunk_size;
            let chunk = &mono[start..start + chunk_size];
            let resampled = resampler.process(&[chunk], None)?;
            if let Some(channel) = resampled.into_iter().next() {
                output.extend(channel);
            }
        }

        // Process the remaining partial chunk (if any) and flush resampler
        let remainder = &mono[full_chunks * chunk_size..];
        if !remainder.is_empty() {
            let resampled = resampler.process_partial(Some(&[remainder]), None)?;
            if let Some(channel) = resampled.into_iter().next() {
                output.extend(channel);
            }
        } else {
            // Flush any remaining samples in the resampler's internal buffer
            let resampled = resampler.process_partial(None::<&[&[f32]]>, None)?;
            if let Some(channel) = resampled.into_iter().next() {
                output.extend(channel);
            }
        }

        Ok(output)
    }
}
