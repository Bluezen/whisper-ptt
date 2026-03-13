use anyhow::{bail, Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

struct ModelInfoOwned {
    filename: String,
    url: String,
    sha256: String,
}

fn get_model_info(name: &str) -> Result<ModelInfoOwned> {
    let base = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";
    let (filename, sha256) = match name {
        "tiny" => (
            "ggml-tiny.bin",
            "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
        ),
        "base" => (
            "ggml-base.bin",
            "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b7f0291dbddd5c0b24",
        ),
        "small" => (
            "ggml-small.bin",
            "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1571c4b527",
        ),
        "medium" => (
            "ggml-medium.bin",
            "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208",
        ),
        "large" => (
            "ggml-large-v3.bin",
            "ad82bf6a9043ceed055076d0fd39f5f186ff25b81e5f0f3c1b5c774044e34c1e",
        ),
        "large-v3-turbo" => (
            "ggml-large-v3-turbo.bin",
            "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
        ),
        other => bail!("unknown model: '{}'", other),
    };
    Ok(ModelInfoOwned {
        filename: filename.to_string(),
        url: format!("{}/{}", base, filename),
        sha256: sha256.to_string(),
    })
}

/// Ensure the model file exists, downloading if needed.
pub fn ensure_model(model_name: &str, models_dir: &Path) -> Result<PathBuf> {
    let info = get_model_info(model_name)?;
    let model_path = models_dir.join(&info.filename);

    if model_path.exists() {
        tracing::info!("model already present: {}", model_path.display());
        return Ok(model_path);
    }

    std::fs::create_dir_all(models_dir)?;
    let part_path = models_dir.join(format!("{}.part", info.filename));

    tracing::info!("downloading model '{}' from {}", model_name, info.url);
    println!("Downloading model '{}'...", model_name);

    let response = reqwest::blocking::get(&info.url)
        .with_context(|| format!("failed to download model from {}", info.url))?;

    if !response.status().is_success() {
        bail!("download failed with status: {}", response.status());
    }

    let total_size = response.content_length().unwrap_or(0);
    let pb = ProgressBar::new(total_size);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
            .unwrap()
            .progress_chars("=>-"),
    );

    let mut file = std::fs::File::create(&part_path)
        .with_context(|| format!("failed to create {}", part_path.display()))?;

    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;
    let mut reader = response;

    let mut buf = vec![0u8; 8192];
    let download_result: Result<()> = (|| {
        loop {
            let n = std::io::Read::read(&mut reader, &mut buf)?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])?;
            hasher.update(&buf[..n]);
            downloaded += n as u64;
            pb.set_position(downloaded);
        }
        Ok(())
    })();

    if let Err(e) = download_result {
        pb.abandon_with_message("download failed");
        drop(file);
        std::fs::remove_file(&part_path).ok();
        return Err(e);
    }

    pb.finish_with_message("download complete");
    file.flush()?;
    drop(file);

    // Verify checksum
    if !info.sha256.is_empty() {
        let hash = format!("{:x}", hasher.finalize());
        if hash != info.sha256 {
            std::fs::remove_file(&part_path).ok();
            bail!(
                "checksum mismatch for {}:\n  expected: {}\n  got:      {}",
                info.filename,
                info.sha256,
                hash
            );
        }
        tracing::info!("checksum verified for {}", info.filename);
    }

    std::fs::rename(&part_path, &model_path)?;
    println!("Model saved to {}", model_path.display());
    Ok(model_path)
}

/// Wrapper around WhisperContext for transcription.
pub struct Transcriber {
    ctx: WhisperContext,
    language: Option<String>,
}

impl Transcriber {
    /// Load a model from file.
    pub fn new(model_path: &Path, language: &str) -> Result<Self> {
        tracing::info!("loading whisper model from {}", model_path.display());
        let ctx = WhisperContext::new_with_params(model_path, WhisperContextParameters::default())
            .context("failed to load whisper model")?;

        let language = if language == "auto" {
            None
        } else {
            Some(language.to_string())
        };

        Ok(Self { ctx, language })
    }

    /// Transcribe audio samples (16kHz mono f32). Returns (text, detected_language).
    pub fn transcribe(&self, audio: &[f32]) -> Result<(String, Option<String>)> {
        let mut state = self
            .ctx
            .create_state()
            .context("failed to create whisper state")?;

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(self.language.as_deref());
        params.set_translate(false);
        params.set_single_segment(true);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_special(false);
        params.set_print_timestamps(false);

        state
            .full(params, audio)
            .context("whisper transcription failed")?;

        let num_segments = state.full_n_segments();

        let mut text = String::new();
        for i in 0..num_segments {
            if let Some(segment) = state.get_segment(i) {
                match segment.to_str() {
                    Ok(segment_text) => text.push_str(segment_text),
                    Err(e) => tracing::warn!("segment {} contains invalid UTF-8: {}", i, e),
                }
            }
        }

        let lang_id = state.full_lang_id_from_state();
        let detected_lang = whisper_rs::get_lang_str(lang_id).map(|s| s.to_string());

        Ok((text.trim().to_string(), detected_lang))
    }
}
