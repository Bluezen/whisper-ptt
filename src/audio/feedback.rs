use anyhow::Result;
use rodio::{Decoder, OutputStream, Sink};
use std::io::Cursor;

const START_WAV: &[u8] = include_bytes!("../../assets/start.wav");
const STOP_WAV: &[u8] = include_bytes!("../../assets/stop.wav");

/// Play the start recording sound. Blocks until playback is complete.
pub fn play_start_sound_blocking() -> Result<()> {
    play_wav_bytes_blocking(START_WAV)
}

/// Play the stop recording sound. Non-blocking.
pub fn play_stop_sound() -> Result<()> {
    play_wav_bytes_nonblocking(STOP_WAV)
}

/// Play WAV bytes (owned copy so Decoder gets 'static-compatible data).
fn play_wav_bytes_blocking(wav_data: &[u8]) -> Result<()> {
    let owned: Vec<u8> = wav_data.to_vec();
    let (_stream, stream_handle) = OutputStream::try_default()?;
    let cursor = Cursor::new(owned);
    let source = Decoder::new(cursor)?;
    let sink = Sink::try_new(&stream_handle)?;
    sink.append(source);
    sink.sleep_until_end();
    Ok(())
}

fn play_wav_bytes_nonblocking(wav_data: &'static [u8]) -> Result<()> {
    std::thread::spawn(move || {
        if let Err(e) = play_wav_bytes_blocking(wav_data) {
            tracing::warn!("failed to play sound: {}", e);
        }
    });
    Ok(())
}
