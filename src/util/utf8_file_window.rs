//! Read a bounded byte range from a file and decode a lossless UTF-8 substring (trim broken
//! leading/trailing multibyte sequences at window edges).

use std::path::Path;

use tokio::fs::{metadata, File};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

/// Returns `(relative_byte_start_in_buf, valid_utf8_str)` for `bytes`.
fn lossless_utf8_block(bytes: &[u8]) -> (usize, &str) {
    let mut rel_start = 0usize;
    while rel_start < bytes.len().min(4) {
        let tail = &bytes[rel_start..];
        let mut end = tail.len();
        while end > 0 {
            if let Ok(s) = std::str::from_utf8(&tail[..end]) {
                return (rel_start, s);
            }
            end -= 1;
        }
        rel_start += 1;
    }
    (0, "")
}

/// Read up to `max_read_bytes` raw bytes from `path` starting at `byte_offset` (clamped),
/// then decode the longest valid UTF-8 prefix/suffix within that slice.
///
/// Returns `(text, lens_start_byte_in_file, lens_raw_read_end_byte_in_file, file_total_bytes)`.
/// `lens_raw_read_end_byte_in_file` is where the next non-overlapping window should begin.
pub async fn read_utf8_file_window(
    path: &Path,
    byte_offset: usize,
    max_read_bytes: usize,
) -> std::io::Result<(String, usize, usize, usize)> {
    let meta = metadata(path).await?;
    let total = meta.len() as usize;
    let start = byte_offset.min(total);
    let avail = total.saturating_sub(start);
    if avail == 0 {
        return Ok((String::new(), start, start, total));
    }
    let want = max_read_bytes.saturating_add(8).min(avail);
    let mut f = File::open(path).await?;
    f.seek(std::io::SeekFrom::Start(start as u64)).await?;
    let mut buf = vec![0u8; want];
    let n = f.read(&mut buf).await?;
    buf.truncate(n);
    let (rel, s) = lossless_utf8_block(&buf);
    let aligned_start = start.saturating_add(rel);
    let raw_end = start.saturating_add(n);
    Ok((s.to_string(), aligned_start, raw_end, total))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use tokio::fs;

    #[tokio::test]
    async fn window_skips_split_utf8_at_start() {
        let dir = tempdir().expect("tempdir");
        let p = dir.path().join("x.txt");
        // Euro is 3 bytes (€); offset 1 lands inside that character.
        fs::write(&p, "€hello").await.expect("write");
        let total = fs::metadata(&p).await.expect("m").len() as usize;
        let (s, a, e, t) = read_utf8_file_window(&p, 1, 100).await.expect("read");
        assert_eq!(s, "hello");
        assert_eq!(t, total);
        assert!(a >= 1);
        assert!(e <= t);
    }
}
