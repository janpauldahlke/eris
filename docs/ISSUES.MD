# Step 3: The Snapshot Daemon

We must tie the snapshot mechanism to a background `tokio` task and the `CancellationToken` from S00.1.

**1. The Daemon (`src/memory/ephemeral.rs`)**

```rust
use tokio_util::sync::CancellationToken;
use std::sync::Arc;

/// Spawns a background task that sleeps for `interval_secs`.
/// It wakes up to snapshot the memory, OR snapshots immediately and exits if `cancel_token` is triggered.
pub fn spawn_snapshot_daemon(
    memory: Arc<EphemeralMemory>,
    vault_root: std::path::PathBuf,
    interval_secs: u64,
    cancel_token: CancellationToken
) { todo!() }

2. Failing Integration Test

    test_daemon_snapshots_on_cancellation: Create an EphemeralMemory, insert a key, spawn the daemon with a massive interval (e.g., 9999s). Immediately trigger the cancel_token. Wait a few milliseconds, then assert the .bin file was successfully written to the temp directory.

3. HARD STOP. Implement the daemon using tokio::select! over the interval interval and the cancel token. Make the test pass.
```
