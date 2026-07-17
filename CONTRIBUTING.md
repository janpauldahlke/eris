# Contributing to Eris

Thanks for considering a contribution. Eris is a solo-maintained project; this file sets expectations for both sides.

## Licensing of contributions (inbound = outbound)

By contributing, you agree that your contributions are licensed under the [Apache License 2.0](LICENSE), the same license as the project. There is no CLA. Instead, certify the [Developer Certificate of Origin](https://developercertificate.org/) by signing off your commits:

```bash
git commit -s
```

This adds a `Signed-off-by: Your Name <you@example.com>` line asserting you have the right to submit the work.

## The non-negotiable engineering rules

These are enforced in review and partly by CI (`#![deny(clippy::unwrap_used)]`, `#![forbid(unsafe_code)]`):

1. **Zero panics.** `unwrap()`/`expect()` only inside `#[test]`. Production code uses `?` and the `FcpError` taxonomy.
2. **No `unsafe`.** Anywhere. The binary stays 100% memory-safe.
3. **Don't block the tokio runtime.** CPU-heavy work goes through `tokio::task::spawn_blocking`.
4. **Actor model, no shared mutable state.** No `Arc<Mutex<T>>` across threads; communicate via `tokio::sync::mpsc`/`watch`.
5. **Tests that touch the filesystem use `tempfile`.** No orphaned files, no `./tmp`.
6. **No `println!` in logic.** It corrupts the ratatui buffer; use `tracing` (`debug!`/`info!`/`warn!`/`error!`).

## Practical notes

- **Build/test:** `cargo build`, `cargo test --bin eris`. CI runs clippy plus the test suite in module-filtered shards (see `.github/workflows/ci.yml`).
- **Adding a tool:** read [docs/HOW_TO/ADDING_A_TOOL.md](docs/HOW_TO/ADDING_A_TOOL.md) first — a tool touches 5+ places (impl, registration, gatekeeper allowlists, descriptor TOML in `specs.rs`, inventory test).
- **Architecture:** [docs/updated_architecture/](docs/updated_architecture/README.md) is code-aligned and honest about the debt. `orchestrator/core/` is dense; small focused PRs there, please.
- **Before large PRs:** open an issue first. Eris is deliberately scoped (see "Core vs. extras" in the README); features outside that scope may be declined regardless of quality.

## Maintenance rhythm

Issues and PRs are triaged on a best-effort schedule, typically twice a week. This is a nights-and-weekends project — a slow response is not a rejection.
