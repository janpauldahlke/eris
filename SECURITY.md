# Security policy

## Reporting a vulnerability

Please report vulnerabilities privately via GitHub's [private vulnerability reporting](https://github.com/janpauldahlke/eris/security/advisories/new) or by email to **janpauldahlke@gmail.com**. Do not open public issues for exploitable problems. You can expect an initial response within a week.

## Privacy posture (what Eris does and does not send)

Eris is local-first by architecture:

- **No telemetry leaves the machine.** `.fcp/telemetry/` contains local log files only. There is no analytics, crash reporting, or phone-home of any kind.
- **Your vault and memories stay local.** The Markdown vault, the ephemeral memory snapshots, and the Qdrant semantic index all live on your disk.
- **Network egress is opt-in and enumerable.** The complete list of outbound surfaces, each disabled unless you configure it: user-initiated web fetch/search (allowlist- and consent-gated), Open-Meteo weather, Wikipedia summaries, Google Workspace mail/calendar (your OAuth credentials), Discord sidecar (your bot token), and the Moltbook client (your API key). LLM inference and embeddings go to `localhost` (llama-server / Ollama / Qdrant).

## Scope notes

- The tool **gatekeeper** is a protocol/safety boundary for the LLM, not a security sandbox against a malicious local user.
- Web fetching executes against remote content; the allowlist, consent gate, and session budget in `src/tools/web/` are the controls. Reports about bypasses of those controls are in scope and welcome.
