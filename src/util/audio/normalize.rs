use std::path::Path;
use std::process::{Command, Stdio};

use tempfile::NamedTempFile;

use crate::config::AudioConfig;
use crate::executive::error::{FcpError, Result};

/// Output of [`normalize_upload`].
#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedAudio {
    pub duration_secs: f32,
    pub bytes: Vec<u8>,
}

/// Decode/resample to target rate mono WAV (runs on a blocking thread).
pub async fn normalize_upload(raw: &[u8], config: &AudioConfig) -> Result<NormalizedAudio> {
    let config = config.clone();
    let raw = raw.to_vec();
    tokio::task::spawn_blocking(move || normalize_upload_blocking(&raw, &config))
        .await
        .map_err(|e| FcpError::ToolFault {
            tool_name: "audio:upload".into(),
            reason: format!("normalize task join failed: {e}"),
        })?
}

fn normalize_upload_blocking(raw: &[u8], config: &AudioConfig) -> Result<NormalizedAudio> {
    if raw.len() as u64 > config.max_upload_bytes {
        return Err(FcpError::ToolFault {
            tool_name: "audio:upload".into(),
            reason: format!(
                "upload {} bytes exceeds max_upload_bytes {}",
                raw.len(),
                config.max_upload_bytes
            ),
        });
    }

    if let Some(fast) = try_pass_through_wav(raw, config)? {
        return Ok(fast);
    }

    let suffix = sniff_suffix(raw);
    if !extension_allowed(&suffix, config) {
        return Err(FcpError::ToolFault {
            tool_name: "audio:upload".into(),
            reason: format!("audio format `{suffix}` not allowed"),
        });
    }

    normalize_with_ffmpeg(raw, &suffix, config)
}

fn extension_allowed(ext: &str, config: &AudioConfig) -> bool {
    let e = ext.trim_start_matches('.').to_ascii_lowercase();
    config.allowed_extensions.iter().any(|a| a.to_ascii_lowercase() == e)
}

fn sniff_suffix(raw: &[u8]) -> String {
    if raw.len() >= 12 && &raw[0..4] == b"RIFF" && &raw[8..12] == b"WAVE" {
        return "wav".into();
    }
    if raw.len() >= 3 && (&raw[0..3] == b"ID3" || (raw[0] == 0xFF && (raw[1] & 0xE0) == 0xE0)) {
        return "mp3".into();
    }
    if raw.len() >= 4 && &raw[0..4] == b"OggS" {
        return "ogg".into();
    }
    if raw.len() >= 4 && &raw[0..4] == b"fLaC" {
        return "flac".into();
    }
    if raw.len() >= 4 && &raw[0..4] == b"RIFF" {
        return "webm".into();
    }
    if raw.len() >= 8 && &raw[4..8] == b"ftyp" {
        return "m4a".into();
    }
    "bin".into()
}

fn try_pass_through_wav(raw: &[u8], config: &AudioConfig) -> Result<Option<NormalizedAudio>> {
    let Some((sample_rate, channels, data)) = parse_pcm_wav(raw)? else {
        return Ok(None);
    };
    if sample_rate != config.target_sample_rate
        || channels != config.target_channels
        || data.is_empty()
    {
        return Ok(None);
    }
    let bytes_per_sample = 2u32;
    let frame_bytes = (channels as u32) * bytes_per_sample;
    if frame_bytes == 0 {
        return Ok(None);
    }
    let frames = data.len() as u32 / frame_bytes;
    let duration_secs = frames as f32 / sample_rate as f32;
    if duration_secs > config.max_duration_secs as f32 {
        return Err(FcpError::ToolFault {
            tool_name: "audio:upload".into(),
            reason: format!(
                "audio duration {duration_secs:.1}s exceeds max_duration_secs {}",
                config.max_duration_secs
            ),
        });
    }
    Ok(Some(NormalizedAudio {
        duration_secs,
        bytes: raw.to_vec(),
    }))
}

fn parse_pcm_wav(raw: &[u8]) -> Result<Option<(u32, u16, Vec<u8>)>> {
    if raw.len() < 44 || &raw[0..4] != b"RIFF" || &raw[8..12] != b"WAVE" {
        return Ok(None);
    }
    let mut pos = 12usize;
    let mut sample_rate = 0u32;
    let mut channels = 0u16;
    let mut bits = 0u16;
    let mut data = Vec::new();
    while pos + 8 <= raw.len() {
        let chunk_id = &raw[pos..pos + 4];
        let chunk_size = u32::from_le_bytes(raw[pos + 4..pos + 8].try_into().map_err(|_| {
            FcpError::ToolFault {
                tool_name: "audio:upload".into(),
                reason: "invalid WAV chunk header".into(),
            }
        })?) as usize;
        pos += 8;
        if pos + chunk_size > raw.len() {
            break;
        }
        if chunk_id == b"fmt " && chunk_size >= 16 {
            channels = u16::from_le_bytes(raw[pos..pos + 2].try_into().map_err(|_| {
                FcpError::ToolFault {
                    tool_name: "audio:upload".into(),
                    reason: "invalid WAV fmt chunk".into(),
                }
            })?);
            sample_rate = u32::from_le_bytes(raw[pos + 4..pos + 8].try_into().map_err(|_| {
                FcpError::ToolFault {
                    tool_name: "audio:upload".into(),
                    reason: "invalid WAV sample rate".into(),
                }
            })?);
            bits = u16::from_le_bytes(raw[pos + 14..pos + 16].try_into().map_err(|_| {
                FcpError::ToolFault {
                    tool_name: "audio:upload".into(),
                    reason: "invalid WAV bits per sample".into(),
                }
            })?);
        } else if chunk_id == b"data" {
            data = raw[pos..pos + chunk_size].to_vec();
        }
        pos += chunk_size + (chunk_size % 2);
    }
    if sample_rate == 0 || channels == 0 || bits != 16 || data.is_empty() {
        return Ok(None);
    }
    Ok(Some((sample_rate, channels, data)))
}

fn normalize_with_ffmpeg(raw: &[u8], suffix: &str, config: &AudioConfig) -> Result<NormalizedAudio> {
    let ffmpeg = which_ffmpeg()?;
    let input = NamedTempFile::new().map_err(|e| FcpError::ToolFault {
        tool_name: "audio:upload".into(),
        reason: format!("temp input file: {e}"),
    })?;
    let input_path = input.path().with_extension(suffix);
    std::fs::write(&input_path, raw).map_err(|e| FcpError::ToolFault {
        tool_name: "audio:upload".into(),
        reason: format!("write temp audio input: {e}"),
    })?;

    let output = NamedTempFile::new().map_err(|e| FcpError::ToolFault {
        tool_name: "audio:upload".into(),
        reason: format!("temp output file: {e}"),
    })?;
    let output_path = output.path().with_extension("wav");

    let max_d = config.max_duration_secs.to_string();
    let ar = config.target_sample_rate.to_string();
    let ac = config.target_channels.to_string();
    let status = Command::new(&ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-i",
        ])
        .arg(&input_path)
        .args([
            "-ar",
            &ar,
            "-ac",
            &ac,
            "-t",
            &max_d,
            "-f",
            "wav",
        ])
        .arg(&output_path)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .map_err(|e| FcpError::ToolFault {
            tool_name: "audio:upload".into(),
            reason: format!("ffmpeg spawn failed: {e}"),
        })?;

    if !status.success() {
        return Err(FcpError::ToolFault {
            tool_name: "audio:upload".into(),
            reason: "ffmpeg failed to normalize audio (check format and duration)".into(),
        });
    }

    let out_bytes = std::fs::read(&output_path).map_err(|e| FcpError::ToolFault {
        tool_name: "audio:upload".into(),
        reason: format!("read ffmpeg output: {e}"),
    })?;
    if out_bytes.len() as u64 > config.max_upload_bytes {
        return Err(FcpError::ToolFault {
            tool_name: "audio:upload".into(),
            reason: format!(
                "normalized audio {} bytes exceeds max_upload_bytes {}",
                out_bytes.len(),
                config.max_upload_bytes
            ),
        });
    }

    let duration_secs = wav_duration_secs(&out_bytes).unwrap_or(0.0);
    let _ = input.close();
    let _ = output.close();

    Ok(NormalizedAudio {
        duration_secs,
        bytes: out_bytes,
    })
}

fn wav_duration_secs(raw: &[u8]) -> Option<f32> {
    let (rate, channels, data) = parse_pcm_wav(raw).ok()??;
    let frame_bytes = (channels as u32) * 2;
    if frame_bytes == 0 {
        return None;
    }
    let frames = data.len() as u32 / frame_bytes;
    Some(frames as f32 / rate as f32)
}

fn which_ffmpeg() -> Result<std::path::PathBuf> {
    if let Ok(p) = std::env::var("FFMPEG") {
        let path = Path::new(&p);
        if path.is_file() {
            return Ok(path.to_path_buf());
        }
    }
    let path = Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()
        .filter(|s| s.success())
        .map(|_| std::path::PathBuf::from("ffmpeg"));
    path.ok_or_else(|| FcpError::ToolFault {
        tool_name: "audio:upload".into(),
        reason: "ffmpeg not found on PATH — install ffmpeg for voice uploads (mp3/webm/m4a)".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_wav(sample_rate: u32, channels: u16, seconds: f32) -> Vec<u8> {
        let frames = (sample_rate as f32 * seconds) as u32;
        let sample_count = frames * channels as u32;
        let data_size = sample_count * 2;
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_size).to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes()); // PCM
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&sample_rate.to_le_bytes());
        let byte_rate = sample_rate * channels as u32 * 2;
        out.extend_from_slice(&byte_rate.to_le_bytes());
        out.extend_from_slice(&(channels * 2).to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_size.to_le_bytes());
        out.extend_from_slice(&vec![0u8; data_size as usize]);
        out
    }

    #[test]
    fn pass_through_16k_mono_wav() {
        let config = AudioConfig::default();
        let wav = make_wav(16000, 1, 1.0);
        let out = normalize_upload_blocking(&wav, &config).expect("normalize");
        assert_eq!(out.bytes, wav);
        assert!((out.duration_secs - 1.0).abs() < 0.05);
    }

    #[test]
    fn rejects_oversized_upload() {
        let mut config = AudioConfig::default();
        config.max_upload_bytes = 10;
        let wav = make_wav(16000, 1, 1.0);
        let err = normalize_upload_blocking(&wav, &config).unwrap_err();
        assert!(err.to_string().contains("max_upload_bytes"));
    }
}
