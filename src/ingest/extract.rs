//! Text extraction from uploaded document bytes (PDF, Markdown, plain text).
//!
//! PDF extraction uses `pdftotext` (poppler-utils) via subprocess rather than the
//! `pdf_extract` Rust crate. Reasons:
//! - Memory isolation: the subprocess can't OOM the parent process
//! - Killable on timeout: unlike `spawn_blocking`, a subprocess can be SIGKILL'd
//! - Reliability: poppler handles all PDF variants (pdf_extract was causing system freezes)
//! - Already a system dependency on most Linux installs

use std::time::Duration;

use tokio::process::Command;

use crate::executive::error::{FcpError, Result};

/// Timeout for PDF text extraction subprocess.
const PDF_EXTRACT_TIMEOUT: Duration = Duration::from_secs(30);

/// Extract UTF-8 text from raw file bytes by extension (no leading dot).
///
/// For PDFs, requires `pdftotext` (from poppler-utils) on the system PATH.
/// The raw bytes are written to a temporary file, extracted, then cleaned up.
pub async fn extract_text(raw_bytes: &[u8], extension: &str) -> Result<String> {
    let ext = extension.trim().to_lowercase();
    match ext.as_str() {
        "pdf" => extract_pdf(raw_bytes).await,
        "md" | "markdown" | "txt" => Ok(String::from_utf8_lossy(raw_bytes).into_owned()),
        _ => Err(FcpError::ToolFault {
            tool_name: "doc:ingest".into(),
            reason: format!(
                "unsupported document extension '{ext}'; supported: pdf, md, markdown, txt"
            ),
        }),
    }
}

/// Extract text from a PDF file path directly (avoids writing a temp file when
/// the file already exists on disk).
pub async fn extract_text_from_path(path: &std::path::Path) -> Result<String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_lowercase();
    match ext.as_str() {
        "pdf" => extract_pdf_from_path(path).await,
        "md" | "markdown" | "txt" => {
            let bytes = tokio::fs::read(path).await.map_err(FcpError::Io)?;
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        }
        _ => Err(FcpError::ToolFault {
            tool_name: "doc:ingest".into(),
            reason: format!(
                "unsupported document extension '{ext}'; supported: pdf, md, markdown, txt"
            ),
        }),
    }
}

async fn extract_pdf(raw_bytes: &[u8]) -> Result<String> {
    let byte_len = raw_bytes.len();

    // Write to a temp file so pdftotext can read it.
    let tmp = tempfile::NamedTempFile::new().map_err(FcpError::Io)?;
    tokio::fs::write(tmp.path(), raw_bytes)
        .await
        .map_err(FcpError::Io)?;

    tracing::info!(
        event = "fcp.document_ingest.pdf_extract_start",
        bytes = byte_len,
        tmp_path = %tmp.path().display(),
        "Extracting PDF via pdftotext subprocess"
    );

    extract_pdf_from_path(tmp.path()).await
}

async fn extract_pdf_from_path(path: &std::path::Path) -> Result<String> {
    let byte_len = path.metadata().map(|m| m.len()).unwrap_or(0);

    let result = tokio::time::timeout(PDF_EXTRACT_TIMEOUT, async {
        Command::new("pdftotext")
            .arg(path.as_os_str())
            .arg("-") // output to stdout
            .output()
            .await
    })
    .await;

    match result {
        Ok(Ok(output)) => {
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(FcpError::ToolFault {
                    tool_name: "doc:ingest".into(),
                    reason: format!(
                        "pdftotext failed (exit {}, {byte_len} bytes): {}",
                        output.status.code().unwrap_or(-1),
                        stderr.trim()
                    ),
                });
            }
            let text = String::from_utf8_lossy(&output.stdout).into_owned();
            tracing::info!(
                event = "fcp.document_ingest.pdf_extract_done",
                bytes = byte_len,
                text_chars = text.len(),
                "PDF text extraction complete (pdftotext)"
            );
            Ok(text)
        }
        Ok(Err(e)) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                Err(FcpError::ToolFault {
                    tool_name: "doc:ingest".into(),
                    reason: "pdftotext not found — install poppler-utils (apt install poppler-utils)".into(),
                })
            } else {
                Err(FcpError::ToolFault {
                    tool_name: "doc:ingest".into(),
                    reason: format!("pdftotext spawn failed: {e}"),
                })
            }
        }
        Err(_elapsed) => Err(FcpError::ToolFault {
            tool_name: "doc:ingest".into(),
            reason: format!(
                "pdftotext timed out after {}s ({byte_len} bytes)",
                PDF_EXTRACT_TIMEOUT.as_secs()
            ),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn markdown_roundtrip() {
        let raw = b"# Title\n\nHello world.";
        let text = extract_text(raw, "md").await.expect("md");
        assert!(text.contains("Hello world"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn unsupported_extension_errors() {
        let err = extract_text(b"data", "docx").await.expect_err("docx");
        assert!(matches!(err, FcpError::ToolFault { .. }));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pdf_extraction_requires_pdftotext() {
        let fake_pdf = b"%PDF-1.0\n%%EOF\n";
        let result = extract_text(fake_pdf, "pdf").await;
        match result {
            Ok(text) => assert!(text.trim().is_empty() || text.len() < 100),
            Err(e) => assert!(matches!(e, FcpError::ToolFault { .. })),
        }
    }
}
