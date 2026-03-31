use tokio::net::TcpStream;
use tokio::time::{timeout, Duration};
use crate::executive::error::{FcpError, Result};

pub async fn run_preflight_checks() -> Result<()> {
    // Check 1: Ollama Ping
    let client = reqwest::Client::new();
    match timeout(Duration::from_secs(2), client.get("http://localhost:11434/api/tags").send()).await {
        Ok(Ok(res)) if res.status().is_success() => (),
        _ => return Err(FcpError::NetworkFault("FATAL: Ollama daemon not responding. Ensure Ollama is running.".into())),
    }

    // Check 2: Qdrant Ping
    match timeout(Duration::from_secs(2), TcpStream::connect("127.0.0.1:6334")).await {
        Ok(Ok(_)) => (),
        _ => return Err(FcpError::NetworkFault("FATAL: Qdrant sidecar not detected. Run your vector db.".into())),
    }

    Ok(())
}
