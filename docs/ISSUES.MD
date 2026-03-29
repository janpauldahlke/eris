4. The Global Kill Switch & Async Refactor (src/main.rs & src/executive/router.rs)

We will transform the sync entrypoint into an async actor model entrypoint with graceful shutdown.

Rust Signatures:

// src/main.rs #[tokio::main]
async fn main() -> ExitCode {
// ...
}

// src/executive/router.rs
pub async fn execute_command(
cmd: Commands,
cancel_token: tokio_util::sync::CancellationToken
) -> Result<()> {
// ...
}

Failing Unit Tests/Integration Checks:

We will set up a test in router.rs simulating a Ctrl+C event that triggers token.cancel(), ensuring the executor yields immediately without panicking.

Process

Once approved, I will implement Step 1 and the failing tests for Step 2. I will then halt and wait for you to run cargo test and provide the compiler output, strictly following the Anti-One-Shot and Halt-and-Catch-Fire directives.
