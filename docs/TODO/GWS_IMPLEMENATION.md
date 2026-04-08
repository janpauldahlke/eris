# Gmail Tool Integration Plan

## Phase 1: Build-Time Codegen with `gws-builder`

### 1a. Add dependencies

In [Cargo.toml](Cargo.toml):

- **build-dependency**: `gws-builder = "0.1"`
- **runtime dependencies** (required by generated code): `base64 = "0.22"`
  - `serde` and `serde_json` already present

### 1b. Create `build.rs`

New file at repo root. Conditional generation logic:

- If env `GWS_GEN=1` is set OR `src/generated/gws_types/` does not exist or is empty (no `.rs` files) -> run `gws_builder::generate`
- Otherwise -> skip (prints cargo warning that generation was skipped)

Gmail v1 whitelist -- only the methods we need for our three tools:

```rust
ServiceSpec::whitelist("gmail", "v1", vec![
    "users.messages.list".into(),    // mail:check
    "users.messages.get".into(),     // mail:read
    "users.messages.send".into(),    // mail:write
])
```

Output: `src/generated/gws_types/` with `mod.rs`, `gmail.rs`, `serde_helpers.rs`.

### 1c. Wire generated module into the crate

- Create `src/generated/mod.rs` (just `pub mod gws_types;`)
- Add `mod generated;` to `src/main.rs`
- Commit the generated files so builds don't require network access

---

## Phase 2: Google Service Account Auth Layer

### 2a. Config additions

In [src/config.rs](src/config.rs), add an optional `GoogleConfig` struct:

```rust
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct GoogleConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Path to the service account JSON key file
    pub service_account_key: Option<PathBuf>,
    /// Gmail address to impersonate via domain-wide delegation
    pub impersonate_user: Option<String>,
}
```

Add `google: GoogleConfig` field to `AppConfig` (with `#[serde(default)]`).

Config in `.fcp/config.toml`:

```toml
[google]
enabled = true
service_account_key = "/path/to/eris-sa.json"
impersonate_user = "eris@yourdomain.com"
```

### 2b. Create `src/util/google_auth.rs`

A new module that handles the service account JWT -> access token flow:

- Read the service account JSON key file (parse `client_email`, `private_key`, `token_uri`)
- Build a JWT assertion (header + claims with scopes + target user)
- Sign with RS256 using the private key
- POST to Google's token endpoint (`https://oauth2.googleapis.com/token`)
- Cache the access token in-memory with expiry tracking
- Auto-refresh before expiry (tokens last ~3600s; refresh at ~3500s)
- New dependency: `jsonwebtoken = "9"` (for RS256 JWT signing)

Key type: `GoogleAuth` with `async fn access_token(&self) -> Result<String>` that returns a cached or freshly-obtained token.

### 2c. Create `src/util/gmail_client.rs`

Wraps `reqwest::Client` + `GoogleAuth`:

- `GmailClient::new(config: GoogleConfig) -> Result<Option<Self>>` -- returns `None` when config is disabled/incomplete (graceful degradation)
- `async fn list_messages(&self, query: Option<&str>, max_results: u32) -> Result<String>` -- GET `/gmail/v1/users/me/messages`
- `async fn get_message(&self, message_id: &str) -> Result<String>` -- GET `/gmail/v1/users/me/messages/{id}?format=full`
- `async fn send_message(&self, to: &str, subject: &str, body: &str) -> Result<String>` -- POST `/gmail/v1/users/me/messages/send` with RFC 2822 base64url-encoded message
- All methods inject `Authorization: Bearer {token}` via `GoogleAuth`
- Uses generated types from `gws_types::gmail` to deserialize responses

Update [src/util/mod.rs](src/util/mod.rs) to export the new modules.

---

## Phase 3: Mail Tool Implementations

Create `src/tools/mail/` following the existing tool pattern (see [src/tools/weather/current.rs](src/tools/weather/current.rs) as reference).

### 3a. `src/tools/mail/mod.rs`

Module index, re-exports:

```rust
mod check;
mod read;
mod write;
pub use check::MailCheckTool;
pub use read::MailReadTool;
pub use write::MailWriteTool;
```

Add `pub mod mail;` to [src/tools/mod.rs](src/tools/mod.rs).

### 3b. `mail:check` (`src/tools/mail/check.rs`)

- Struct: `MailCheckTool { client: Arc<GmailClient> }`
- Args: `{ query: Option<String>, max_results: Option<u32> }` -- `query` is Gmail search syntax (e.g. `"is:unread"`, `"from:boss@co.com"`)
- Calls `client.list_messages(query, max_results.unwrap_or(10))`
- Returns formatted summary: count + snippet of each message (sender, subject, date)
- `ToolContextViewHint::Snippet` to keep LLM context lean

### 3c. `mail:read` (`src/tools/mail/read.rs`)

- Struct: `MailReadTool { client: Arc<GmailClient> }`
- Args: `{ message_id: String }`
- Calls `client.get_message(&message_id)`
- Parses headers (From, To, Subject, Date) + body text from the `Message` payload
- Returns formatted message content, truncated to a sane limit (`vault_read_ratio`-style)
- `ToolContextViewHint::Snippet`

### 3d. `mail:write` (`src/tools/mail/write.rs`)

- Struct: `MailWriteTool { client: Arc<GmailClient> }`
- Args: `{ to: String, subject: String, body: String, cc: Option<String>, bcc: Option<String> }`
- Validates email addresses (basic format check)
- Calls `client.send_message(to, subject, body)`
- Returns confirmation with message ID
- This tool should probably be blocked in `Reflect` state (write-like)

---

## Phase 4: Gatekeeper, Descriptors, Recovery

### 4a. Register tools in [src/executive/router.rs](src/executive/router.rs)

After building `GmailClient` (conditional on `config.google.enabled`):

```rust
if let Some(gmail) = gmail_client {
    let gmail = Arc::new(gmail);
    gatekeeper.register(Arc::new(MailCheckTool { client: gmail.clone() }));
    gatekeeper.register(Arc::new(MailReadTool { client: gmail.clone() }));
    gatekeeper.register(Arc::new(MailWriteTool { client: gmail }));
}
```

Similar pattern to how `memory:commit`/`memory:query` are conditionally registered when semantic brain is available.

### 4b. Update gatekeeper state matrix in [src/tools/gatekeeper.rs](src/tools/gatekeeper.rs)

In `state_allows_tool`:

- `AgentState::Chat` -- allow `mail:check`, `mail:read`, `mail:write`
- `AgentState::Reflect` -- allow `mail:check`, `mail:read` (not `mail:write`)
- `AgentState::Idle` -- allow `mail:check`, `mail:read`, `mail:write`
- `AgentState::Recover` -- already allows all (`true`)

Update `test_policy_covers_all_current_tools` to include `mail:check`, `mail:read`, `mail:write`.

### 4c. Add TOML descriptors in [src/tools/specs.rs](src/tools/specs.rs)

Three new `DESCRIPTOR_TOMLS` entries:

- **`mail:check`** -- `routing_hints`: "check email", "new mail", "inbox", "unread messages", "check gmail"
  - `when_to_use`: "Use when the user wants to see recent or filtered messages. Supports Gmail search query syntax."
  - `when_not_to_use`: "Do not use to read full message content (use mail:read) or send mail (use mail:write)."

- **`mail:read`** -- `routing_hints`: "read email", "open message", "show email", "email details", "message content"
  - `when_to_use`: "Use to read the full content of a specific message by ID (from mail:check results)."
  - `when_not_to_use`: "Do not use without a message_id from mail:check."

- **`mail:write`** -- `routing_hints`: "send email", "compose mail", "write email", "reply", "email to"
  - `when_to_use`: "Use to compose and send an email. Requires to, subject, and body."
  - `when_not_to_use`: "Do not use to read or check mail."

### 4d. Agent recovery prompts

In [src/orchestrator/llm_support/post_tool_guidance.rs](../../src/orchestrator/llm_support/post_tool_guidance.rs) or a new `src/tools/mail/prompts.rs`:

- **Auth failure recovery**: When GoogleAuth returns 401/403, the tool returns a clear error message guiding the agent to inform the user that Gmail credentials are not configured or the service account lacks delegation.
- **Rate limit recovery**: Gmail API has rate limits. If 429 is returned, the tool should advise the agent to wait and retry, or inform the user.
- **Empty inbox hint**: When `mail:check` returns zero results, the tool result should phrase it clearly so the LLM doesn't hallucinate messages.

### 4e. Tool router enrichment

In [src/orchestrator/tool_router.rs](src/orchestrator/tool_router.rs), add fallback hints in `enrich_for_routing` if needed (the TOML `routing_hints` already feed the embedding-based router, but explicit fallbacks help).

---

## Phase 5: Testing

### 5a. Unit tests (per tool)

- Schema validation tests (valid/invalid args)
- Happy-path tests using `wiremock` to mock Gmail API responses
- Error handling tests (401, 404, 429, network failure)
- All following `tempfile` rule for any fs writes

### 5b. GoogleAuth tests

- JWT generation with a test key
- Token caching + refresh logic
- Error paths (missing key file, invalid JSON)

### 5c. Integration test structure

- Mock `GmailClient` for gatekeeper/orchestrator-level tests
- Verify state matrix enforcement (mail:write blocked in Reflect)
- Descriptor coverage assertion (`assert_covers_registered_tools` already enforces this at boot)

---

## Dependency Summary

New in `Cargo.toml`:

- `[build-dependencies]`: `gws-builder = "0.1"`
- `[dependencies]`: `base64 = "0.22"`, `jsonwebtoken = "9"` (for RS256 JWT)

---

## File Change Map

| File                                        | Change                                |
| ------------------------------------------- | ------------------------------------- |
| `Cargo.toml`                                | Add build-dep + runtime deps          |
| `build.rs` (new)                            | Conditional gws-builder codegen       |
| `src/generated/mod.rs` (new)                | Module index for generated code       |
| `src/generated/gws_types/` (new, generated) | Gmail types + ActionDescriptors       |
| `src/main.rs`                               | Add `mod generated;`                  |
| `src/config.rs`                             | Add `GoogleConfig` struct + field     |
| `src/util/mod.rs`                           | Add google_auth, gmail_client exports |
| `src/util/google_auth.rs` (new)             | Service account JWT -> token          |
| `src/util/gmail_client.rs` (new)            | Gmail API wrapper                     |
| `src/tools/mod.rs`                          | Add `pub mod mail;`                   |
| `src/tools/mail/mod.rs` (new)               | Module index                          |
| `src/tools/mail/check.rs` (new)             | `mail:check` tool                     |
| `src/tools/mail/read.rs` (new)              | `mail:read` tool                      |
| `src/tools/mail/write.rs` (new)             | `mail:write` tool                     |
| `src/tools/specs.rs`                        | 3 new TOML descriptor blocks          |
| `src/tools/gatekeeper.rs`                   | State matrix + test update            |
| `src/executive/router.rs`                   | Register mail tools (conditional)     |

---

## Implementation Order (Anti-One-Shot)

Per `.cursorrules`, each step is one function or one test at a time, with `cargo test` between steps:

1. `Cargo.toml` dependency additions
2. `build.rs` (conditional codegen)
3. Run `GWS_GEN=1 cargo build` to generate types, commit output
4. `src/generated/mod.rs` + wire into `src/main.rs`
5. `GoogleConfig` in `config.rs` + test
6. `GoogleAuth` in `util/google_auth.rs` + test
7. `GmailClient` in `util/gmail_client.rs` + test
8. `MailCheckTool` + test
9. `MailReadTool` + test
10. `MailWriteTool` + test
11. Descriptors in `specs.rs`
12. Gatekeeper state matrix + test
13. Registration in `router.rs`
14. End-to-end descriptor coverage verification (`cargo test`)

---

name: Gmail Tool Integration
overview: Integrate `gws-builder` into Eris to generate Gmail v1 types at build time, then implement three mail tools (`mail:check`, `mail:read`, `mail:write`) backed by a service-account OAuth2 client, and wire them into the full tool lifecycle (gatekeeper, descriptors, recovery, routing).
todos:

- id: deps
  content: Add gws-builder (build-dep), base64, jsonwebtoken to Cargo.toml
  status: pending
- id: build-rs
  content: Create build.rs with conditional Gmail v1 codegen (GWS_GEN env or missing folder)
  status: pending
- id: run-codegen
  content: Run GWS_GEN=1 cargo build, verify generated types, commit src/generated/gws_types/
  status: pending
- id: wire-module
  content: Create src/generated/mod.rs + add mod generated to main.rs
  status: pending
- id: google-config
  content: Add GoogleConfig struct + field to AppConfig in config.rs
  status: pending
- id: google-auth
  content: Implement GoogleAuth (service account JWT -> access token) in src/util/google_auth.rs
  status: pending
- id: gmail-client
  content: Implement GmailClient (list/get/send + Bearer auth) in src/util/gmail_client.rs
  status: pending
- id: mail-check
  content: Implement mail:check tool in src/tools/mail/check.rs
  status: pending
- id: mail-read
  content: Implement mail:read tool in src/tools/mail/read.rs
  status: pending
- id: mail-write
  content: Implement mail:write tool in src/tools/mail/write.rs
  status: pending
- id: descriptors
  content: Add 3 TOML descriptor blocks for mail tools in specs.rs
  status: pending
- id: gatekeeper
  content: Update state_allows_tool matrix + test_policy_covers_all_current_tools
  status: pending
- id: register
  content: Conditionally register mail tools in router.rs (like semantic brain pattern)
  status: pending
  isProject: false

---
