use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, GenericImageView, ImageFormat};

use crate::config::VisionConfig;
use crate::executive::error::{FcpError, Result};

/// Output of [`normalize_upload`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedImage {
    pub width: u32,
    pub height: u32,
    pub bytes: Vec<u8>,
}

/// Decode, fit within `target_max_px`, and re-encode as JPEG (runs on a blocking thread).
pub async fn normalize_upload(raw: &[u8], config: &VisionConfig) -> Result<NormalizedImage> {
    let config = config.clone();
    let raw = raw.to_vec();
    tokio::task::spawn_blocking(move || normalize_upload_blocking(&raw, &config))
        .await
        .map_err(|e| FcpError::ToolFault {
            tool_name: "vision:upload".into(),
            reason: format!("normalize task join failed: {e}"),
        })?
}

fn normalize_upload_blocking(raw: &[u8], config: &VisionConfig) -> Result<NormalizedImage> {
    let format = image::guess_format(raw).map_err(|e| FcpError::ToolFault {
        tool_name: "vision:upload".into(),
        reason: format!("unrecognized image format: {e}"),
    })?;
    if !format_allowed(format, config) {
        return Err(FcpError::ToolFault {
            tool_name: "vision:upload".into(),
            reason: format!("image format {:?} not allowed", format),
        });
    }

    let img = image::load_from_memory(raw).map_err(|e| FcpError::ToolFault {
        tool_name: "vision:upload".into(),
        reason: format!("decode failed: {e}"),
    })?;

    let (w, h) = img.dimensions();
    let max_px = config.target_max_px.max(1);
    let resized: DynamicImage = if w <= max_px && h <= max_px {
        img
    } else {
        img.thumbnail(max_px, max_px)
    };

    let (buf, out_w, out_h) = encode_within_budget(&resized, config)?;

    Ok(NormalizedImage {
        width: out_w,
        height: out_h,
        bytes: buf,
    })
}

fn encode_jpeg(img: &DynamicImage, quality: u8) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut enc = JpegEncoder::new_with_quality(&mut buf, quality.clamp(1, 100));
    enc.encode_image(img).map_err(|e| FcpError::ToolFault {
        tool_name: "vision:upload".into(),
        reason: format!("jpeg encode failed: {e}"),
    })?;
    Ok(buf)
}

/// Re-encode as JPEG, stepping quality down and (if needed) shrinking further for camera-sized sources.
fn encode_within_budget(img: &DynamicImage, config: &VisionConfig) -> Result<(Vec<u8>, u32, u32)> {
    let max_bytes = config.max_output_bytes.max(1);
    let base_quality = config.jpeg_quality.clamp(1, 100);
    let max_px = config.target_max_px.max(1);

    let mut working = img.clone();
    let mut edge = max_px;

    loop {
        let mut quality = base_quality;
        loop {
            let buf = encode_jpeg(&working, quality)?;
            if (buf.len() as u64) <= max_bytes {
                let (w, h) = working.dimensions();
                return Ok((buf, w, h));
            }
            if quality <= 45 {
                break;
            }
            quality = quality.saturating_sub(15);
        }

        if edge <= 448 {
            return Err(FcpError::ToolFault {
                tool_name: "vision:upload".into(),
                reason: format!(
                    "normalized image still exceeds max_output_bytes {max_bytes} after resize/compress"
                ),
            });
        }
        edge = (edge * 3) / 4;
        working = working.thumbnail(edge, edge);
    }
}

fn format_allowed(format: ImageFormat, config: &VisionConfig) -> bool {
    let ext = match format {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpeg",
        ImageFormat::Gif => "gif",
        ImageFormat::WebP => "webp",
        _ => return false,
    };
    config.allowed_extensions.iter().any(|e| {
        e.eq_ignore_ascii_case(ext) || (ext == "jpeg" && e.eq_ignore_ascii_case("jpg"))
    })
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use image::{ImageBuffer, Rgb};

    fn test_config() -> VisionConfig {
        VisionConfig {
            max_output_bytes: 5 * 1024 * 1024,
            ..VisionConfig::default()
        }
    }

    #[test]
    fn normalize_png_to_jpeg() {
        // Keep fixtures small: parallel `cargo test` runs many image tests at once.
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_fn(640, 480, |x, y| {
            Rgb([(x % 256) as u8, (y % 256) as u8, 128])
        });
        let mut raw = Vec::new();
        img.write_to(&mut Cursor::new(&mut raw), ImageFormat::Png)
            .expect("png encode");

        let out = normalize_upload_blocking(&raw, &test_config()).expect("normalize");
        assert!(out.width <= 896);
        assert!(out.height <= 896);
        assert!(out.bytes.starts_with(&[0xFF, 0xD8]));
    }

    #[test]
    fn rejects_oversized_output() {
        let tiny = VisionConfig {
            max_output_bytes: 64,
            ..VisionConfig::default()
        };
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> =
            ImageBuffer::from_fn(640, 640, |_, _| Rgb([200, 100, 50]));
        let mut raw = Vec::new();
        img.write_to(&mut Cursor::new(&mut raw), ImageFormat::Png)
            .expect("png");
        assert!(normalize_upload_blocking(&raw, &tiny).is_err());
    }

    #[test]
    fn large_png_compresses_within_budget() {
        let cfg = VisionConfig {
            max_output_bytes: 400 * 1024,
            ..VisionConfig::default()
        };
        // 1280×960 is enough to exercise resize + JPEG budget without OOM when tests run in parallel.
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> =
            ImageBuffer::from_fn(1280, 960, |x, y| Rgb([(x % 256) as u8, (y % 256) as u8, 128]));
        let mut raw = Vec::new();
        img.write_to(&mut Cursor::new(&mut raw), ImageFormat::Png)
            .expect("png");
        let out = normalize_upload_blocking(&raw, &cfg).expect("normalize");
        assert!(out.bytes.len() as u64 <= cfg.max_output_bytes);
        assert!(out.width <= 896);
        assert!(out.height <= 896);
    }
}
