mod audio;
mod clipboard;
mod config;
mod hid_listener;
mod history;
mod hotkey;
mod transcriber;

use anyhow::{Context, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// Send a macOS notification via osascript (non-blocking).
#[cfg(target_os = "macos")]
fn notify(title: &str, message: &str) {
    let safe = message.replace('\\', "\\\\").replace('"', "\\\"");
    let _ = std::process::Command::new("osascript")
        .arg("-e")
        .arg(format!(
            r#"display notification "{safe}" with title "{title}""#
        ))
        .spawn();
}

#[cfg(not(target_os = "macos"))]
fn notify(_title: &str, _message: &str) {}

fn main() -> Result<()> {
    eprintln!("whisper-ptt pid={} starting", std::process::id());

    // Load config
    let config = config::Config::load().context("failed to load config")?;
    eprintln!(
        "config loaded from {}",
        config::config_path().unwrap_or_default().display()
    );

    // Init logging
    let log_path = config.log_path()?;
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file_appender = tracing_appender::rolling::daily(
        log_path.parent().unwrap(),
        "whisper-ptt.log",
    );
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    let filter = config
        .logging
        .level
        .parse::<tracing_subscriber::filter::LevelFilter>()
        .unwrap_or(tracing_subscriber::filter::LevelFilter::INFO);
    tracing_subscriber::fmt()
        .with_writer(non_blocking)
        .with_max_level(filter)
        .init();

    tracing::info!("whisper-ptt starting (pid={})", std::process::id());

    // Ensure model is downloaded
    let models_dir = config.models_dir()?;
    let model_path = transcriber::ensure_model(&config.whisper.model, &models_dir)?;

    // Load whisper model
    eprintln!("loading whisper model ({})...", config.whisper.model);
    let transcriber = transcriber::Transcriber::new(&model_path, &config.whisper.language)?;
    eprintln!("model loaded");

    // Open history database
    let db_path = config.database_path()?;
    let history = history::History::open(&db_path)?;

    // Parse hotkey config
    let target_key = hotkey::parse_key(&config.hotkey.key)?;
    let mode = hotkey::HotkeyMode::from_str(&config.hotkey.mode)?;

    // Start hotkey listener
    let hotkey_rx = hotkey::start_listener(target_key, mode)
        .context("failed to start hotkey listener — check Accessibility permission")?;
    eprintln!(
        "hotkey listener started for key '{}' (mode: {})",
        config.hotkey.key, config.hotkey.mode
    );

    // Shutdown flag
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })?;

    eprintln!(
        "whisper-ptt ready — press '{}' to record",
        config.hotkey.key
    );
    println!(
        "whisper-ptt ready. Press '{}' to record. Ctrl+C to quit.",
        config.hotkey.key
    );
    tracing::info!("ready -- listening for hotkey '{}'", config.hotkey.key);

    let mut was_muted = false;
    let mut active_capture: Option<audio::capture::AudioCapture> = None;
    let mut recording_start: Option<Instant> = None;

    while running.load(Ordering::SeqCst) {
        let event = match hotkey_rx.recv_timeout(std::time::Duration::from_millis(200)) {
            Ok(event) => event,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        };

        match event {
            hotkey::HotkeyEvent::StartRecording => {
                if active_capture.is_some() {
                    continue;
                }

                tracing::info!("recording started");

                // Play start sound (blocking -- must finish before mute)
                if let Err(e) = audio::feedback::play_start_sound_blocking() {
                    tracing::warn!("failed to play start sound: {}", e);
                }

                // Mute output if configured
                if config.audio.mute_output_during_recording {
                    match audio::mute::mute_output() {
                        Ok(prev) => was_muted = prev,
                        Err(e) => tracing::warn!("failed to mute output: {}", e),
                    }
                }

                // Start audio capture
                match audio::capture::AudioCapture::start(&config.audio.device) {
                    Ok(capture) => {
                        active_capture = Some(capture);
                        recording_start = Some(Instant::now());
                    }
                    Err(e) => {
                        tracing::error!("failed to start audio capture: {}", e);
                        if config.audio.mute_output_during_recording {
                            let _ = audio::mute::unmute_output(was_muted);
                        }
                    }
                }
            }

            hotkey::HotkeyEvent::StopRecording => {
                let capture = match active_capture.take() {
                    Some(c) => c,
                    None => continue,
                };

                let duration_ms = recording_start
                    .take()
                    .map(|s| s.elapsed().as_millis() as u64)
                    .unwrap_or(0);

                tracing::info!("recording stopped ({}ms)", duration_ms);

                // Stop capture and get audio
                let audio_data = match capture.stop() {
                    Ok(data) => data,
                    Err(e) => {
                        tracing::error!("failed to stop audio capture: {}", e);
                        if config.audio.mute_output_during_recording {
                            let _ = audio::mute::unmute_output(was_muted);
                        }
                        continue;
                    }
                };

                // Unmute output
                if config.audio.mute_output_during_recording {
                    if let Err(e) = audio::mute::unmute_output(was_muted) {
                        tracing::warn!("failed to unmute output: {}", e);
                    }
                }

                // Play stop sound (non-blocking)
                if let Err(e) = audio::feedback::play_stop_sound() {
                    tracing::warn!("failed to play stop sound: {}", e);
                }

                // Check minimum duration
                if duration_ms < config.whisper.min_duration_ms {
                    tracing::debug!(
                        "recording too short ({}ms < {}ms), discarding",
                        duration_ms,
                        config.whisper.min_duration_ms
                    );
                    continue;
                }

                if audio_data.is_empty() {
                    tracing::debug!("no audio data captured, skipping");
                    continue;
                }

                // Transcribe
                tracing::info!("transcribing {} samples...", audio_data.len());
                if config.notifications.enabled {
                    notify("whisper-ptt", "Transcribing…");
                }
                match transcriber.transcribe(&audio_data) {
                    Ok((text, lang)) => {
                        if text.is_empty() {
                            tracing::debug!("transcription returned empty text");
                            continue;
                        }

                        tracing::info!("transcribed: '{}' (lang: {:?})", text, lang);
                        if config.notifications.enabled {
                            let preview = if text.len() > 100 {
                                format!("{}…", &text[..100])
                            } else {
                                text.clone()
                            };
                            notify("whisper-ptt", &preview);
                        }

                        // Paste
                        if let Err(e) = clipboard::paste_text(
                            &text,
                            config.clipboard.restore_previous,
                            config.clipboard.paste_delay_ms,
                            config.clipboard.restore_delay_ms,
                        ) {
                            tracing::error!("failed to paste text: {}", e);
                        }

                        // Save to history
                        if let Err(e) = history.insert(
                            &text,
                            lang.as_deref(),
                            &config.whisper.model,
                            duration_ms,
                        ) {
                            tracing::error!("failed to save to history: {}", e);
                        }
                    }
                    Err(e) => {
                        tracing::error!("transcription failed: {}", e);
                    }
                }

                // Drain any PTT events that queued during transcription
                while hotkey_rx.try_recv().is_ok() {}
            }
        }
    }

    // Shutdown
    tracing::info!("shutting down");
    eprintln!("whisper-ptt shutting down");
    if config.audio.mute_output_during_recording {
        let _ = audio::mute::unmute_output(was_muted);
    }
    println!("whisper-ptt stopped.");

    Ok(())
}
