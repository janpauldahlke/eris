use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use schemars::schema::RootSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::executive::error::{FcpError, Result};
use crate::tools::context_view_hint::{API_TOOL_SNIPPET_CHARS, ToolContextViewHint};
use crate::tools::moltbook::client::{
    AuthMode, MoltbookClient, clean_path_segment, tool_result, validate_content_len,
};
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema)]
pub struct RegisterArgs {
    pub name: String,
    pub description: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct EmptyArgs {}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FeedSource {
    Personal,
    Posts,
    Submolt,
}

#[derive(Deserialize, JsonSchema)]
pub struct FeedArgs {
    #[serde(default)]
    pub source: Option<FeedSource>,
    #[serde(default)]
    pub sort: Option<String>,
    #[serde(default)]
    pub filter: Option<String>,
    #[serde(default)]
    pub submolt: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SearchResultType {
    #[default]
    All,
    Posts,
    Comments,
}

impl SearchResultType {
    fn as_api_str(self) -> &'static str {
        match self {
            SearchResultType::All => "all",
            SearchResultType::Posts => "posts",
            SearchResultType::Comments => "comments",
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct SearchArgs {
    pub q: String,
    /// API query parameter `type`: posts, comments, or all (default all).
    #[serde(default, rename = "type")]
    #[schemars(rename = "type")]
    pub result_type: Option<SearchResultType>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct CommentsArgs {
    pub post_id: String,
    #[serde(default)]
    pub sort: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct CommentArgs {
    pub post_id: String,
    pub content: String,
    #[serde(default)]
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VoteTarget {
    Post,
    Comment,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VoteDirection {
    Upvote,
    Downvote,
}

#[derive(Deserialize, JsonSchema)]
pub struct VoteArgs {
    pub target: VoteTarget,
    pub id: String,
    pub direction: VoteDirection,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PostKind {
    Text,
    Link,
    Image,
}

#[derive(Deserialize, JsonSchema)]
pub struct PostArgs {
    pub submolt_name: String,
    pub title: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub kind: Option<PostKind>,
}

#[derive(Deserialize, JsonSchema)]
pub struct VerifyArgs {
    pub verification_code: String,
    pub answer: String,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NotificationScope {
    Post,
    #[default]
    All,
}

#[derive(Deserialize, JsonSchema)]
pub struct NotificationsReadArgs {
    #[serde(default)]
    pub scope: NotificationScope,
    #[serde(default)]
    pub post_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DmAction {
    Check,
    ListRequests,
    ListConversations,
    ReadConversation,
    SendRequest,
    SendMessage,
    ApproveRequest,
    RejectRequest,
}

#[derive(Deserialize, JsonSchema)]
pub struct DmArgs {
    pub action: DmAction,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub to_owner: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub needs_human_input: Option<bool>,
    #[serde(default)]
    pub block: Option<bool>,
}

pub struct MoltbookRegisterTool {
    pub client: Arc<MoltbookClient>,
}

pub struct MoltbookStatusTool {
    pub client: Arc<MoltbookClient>,
}

pub struct MoltbookHomeTool {
    pub client: Arc<MoltbookClient>,
}

pub struct MoltbookFeedTool {
    pub client: Arc<MoltbookClient>,
}

pub struct MoltbookSearchTool {
    pub client: Arc<MoltbookClient>,
}

pub struct MoltbookCommentsTool {
    pub client: Arc<MoltbookClient>,
}

pub struct MoltbookCommentTool {
    pub client: Arc<MoltbookClient>,
}

pub struct MoltbookVoteTool {
    pub client: Arc<MoltbookClient>,
}

pub struct MoltbookPostTool {
    pub client: Arc<MoltbookClient>,
}

pub struct MoltbookVerifyTool {
    pub client: Arc<MoltbookClient>,
}

pub struct MoltbookNotificationsReadTool {
    pub client: Arc<MoltbookClient>,
}

pub struct MoltbookDmTool {
    pub client: Arc<MoltbookClient>,
}

/// Normalize numeric verification answers to two decimal places per Moltbook API convention.
fn normalize_moltbook_verify_answer(answer: &str) -> String {
    let t = answer.trim();
    if t.is_empty() {
        return String::new();
    }
    if let Ok(n) = t.parse::<f64>() {
        if !n.is_finite() {
            return t.to_string();
        }
        format!("{n:.2}")
    } else {
        t.to_string()
    }
}

macro_rules! parse_args {
    ($args:expr, $ty:ty) => {
        serde_json::from_value::<$ty>($args).map_err(FcpError::ParseFault)?
    };
}

#[async_trait]
impl Tool for MoltbookRegisterTool {
    fn name(&self) -> &'static str {
        "moltbook:register"
    }

    fn description(&self) -> &'static str {
        "Register a new Moltbook agent identity and return a claim URL plus API key. Only use when the human explicitly asks to register/create a Moltbook agent."
    }

    fn parameters_schema(&self) -> RootSchema {
        schemars::schema_for!(RegisterArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let parsed = parse_args!(args, RegisterArgs);
        let name = validate_content_len("name", &parsed.name, 2, 80)?;
        let description = validate_content_len("description", &parsed.description, 1, 500)?;
        let response = self
            .client
            .post(
                "/agents/register",
                Some(json!({ "name": name, "description": description })),
                AuthMode::None,
            )
            .await?;
        tool_result(
            self.name(),
            response,
            "Give the claim_url to the human and save the returned api_key outside the repo before making authenticated Moltbook calls.",
        )
    }
}

#[async_trait]
impl Tool for MoltbookStatusTool {
    fn name(&self) -> &'static str {
        "moltbook:status"
    }

    fn description(&self) -> &'static str {
        "Check Moltbook claim/account status and profile for the configured agent."
    }

    fn parameters_schema(&self) -> RootSchema {
        schemars::schema_for!(EmptyArgs)
    }

    fn context_view_hint(&self) -> ToolContextViewHint {
        ToolContextViewHint::Snippet {
            max_chars: API_TOOL_SNIPPET_CHARS,
        }
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let _parsed = parse_args!(args, EmptyArgs);
        let status = self
            .client
            .get("/agents/status", &[], AuthMode::Bearer)
            .await?;
        let me = self.client.get("/agents/me", &[], AuthMode::Bearer).await?;
        let combined = crate::tools::moltbook::client::MoltbookResponse {
            body: json!({ "status": status.body, "me": me.body }),
            rate_limit: me.rate_limit,
        };
        tool_result(
            self.name(),
            combined,
            "If status is pending_claim, ask the human to finish the claim flow.",
        )
    }
}

#[async_trait]
impl Tool for MoltbookHomeTool {
    fn name(&self) -> &'static str {
        "moltbook:home"
    }

    fn description(&self) -> &'static str {
        "Check Moltbook home dashboard. This is the first tool to use for an operator-requested Moltbook visit."
    }

    fn parameters_schema(&self) -> RootSchema {
        schemars::schema_for!(EmptyArgs)
    }

    fn context_view_hint(&self) -> ToolContextViewHint {
        ToolContextViewHint::Snippet {
            max_chars: API_TOOL_SNIPPET_CHARS * 2,
        }
    }

    fn allow_repeat_in_turn(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let _parsed = parse_args!(args, EmptyArgs);
        let response = self.client.get("/home", &[], AuthMode::Bearer).await?;
        tool_result(
            self.name(),
            response,
            "Summarize the dashboard. Do not post, approve DMs, or mark notifications read unless the user asked for that follow-up.",
        )
    }
}

#[async_trait]
impl Tool for MoltbookFeedTool {
    fn name(&self) -> &'static str {
        "moltbook:feed"
    }

    fn description(&self) -> &'static str {
        "Read Moltbook feeds or post listings after an active user request."
    }

    fn parameters_schema(&self) -> RootSchema {
        schemars::schema_for!(FeedArgs)
    }

    fn context_view_hint(&self) -> ToolContextViewHint {
        ToolContextViewHint::Snippet {
            max_chars: API_TOOL_SNIPPET_CHARS * 2,
        }
    }

    fn allow_repeat_in_turn(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let parsed = parse_args!(args, FeedArgs);
        let source = parsed.source.unwrap_or(FeedSource::Personal);
        let limit = clamp_limit(parsed.limit, 25, 50).to_string();
        let path = match source {
            FeedSource::Personal => "/feed".to_string(),
            FeedSource::Posts => "/posts".to_string(),
            FeedSource::Submolt => {
                let submolt = parsed.submolt.as_deref().ok_or_else(|| {
                    FcpError::SchemaViolation("submolt is required when source=submolt".into())
                })?;
                format!("/submolts/{}/feed", clean_path_segment("submolt", submolt)?)
            }
        };
        let query = [
            ("sort", parsed.sort),
            ("filter", parsed.filter),
            ("limit", Some(limit)),
            ("cursor", parsed.cursor),
        ];
        let response = self.client.get(&path, &query, AuthMode::Bearer).await?;
        tool_result(
            self.name(),
            response,
            "Upvote or comment only when the user explicitly wants engagement or the content genuinely warrants it.",
        )
    }
}

#[async_trait]
impl Tool for MoltbookSearchTool {
    fn name(&self) -> &'static str {
        "moltbook:search"
    }

    fn description(&self) -> &'static str {
        "Semantic search on Moltbook: find posts and comments by meaning (natural-language query), not only keywords."
    }

    fn parameters_schema(&self) -> RootSchema {
        schemars::schema_for!(SearchArgs)
    }

    fn context_view_hint(&self) -> ToolContextViewHint {
        ToolContextViewHint::Snippet {
            max_chars: API_TOOL_SNIPPET_CHARS * 2,
        }
    }

    fn allow_repeat_in_turn(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let parsed = parse_args!(args, SearchArgs);
        let q = validate_content_len("q", &parsed.q, 1, 500)?;
        let kind = parsed.result_type.unwrap_or_default();
        let limit = clamp_limit(parsed.limit, 20, 50).to_string();
        let query = [
            ("q", Some(q)),
            ("type", Some(kind.as_api_str().to_string())),
            ("limit", Some(limit)),
            ("cursor", parsed.cursor),
        ];
        let response = self.client.get("/search", &query, AuthMode::Bearer).await?;
        tool_result(
            self.name(),
            response,
            "Use `similarity` to judge relevance. For each hit you might engage with, open `moltbook:comments` on `post_id` before voting or commenting.",
        )
    }
}

#[async_trait]
impl Tool for MoltbookCommentsTool {
    fn name(&self) -> &'static str {
        "moltbook:comments"
    }

    fn description(&self) -> &'static str {
        "Read comments on a Moltbook post."
    }

    fn parameters_schema(&self) -> RootSchema {
        schemars::schema_for!(CommentsArgs)
    }

    fn context_view_hint(&self) -> ToolContextViewHint {
        ToolContextViewHint::Snippet {
            max_chars: API_TOOL_SNIPPET_CHARS * 2,
        }
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let parsed = parse_args!(args, CommentsArgs);
        let post_id = clean_path_segment("post_id", &parsed.post_id)?;
        let limit = clamp_limit(parsed.limit, 35, 100).to_string();
        let query = [
            ("sort", parsed.sort),
            ("limit", Some(limit)),
            ("cursor", parsed.cursor),
        ];
        let response = self
            .client
            .get(
                &format!("/posts/{post_id}/comments"),
                &query,
                AuthMode::Bearer,
            )
            .await?;
        tool_result(
            self.name(),
            response,
            "Reply only when the user asked or the conversation clearly needs a response.",
        )
    }
}

#[async_trait]
impl Tool for MoltbookCommentTool {
    fn name(&self) -> &'static str {
        "moltbook:comment"
    }

    fn description(&self) -> &'static str {
        "Create a thoughtful Moltbook comment or reply. May return a verification challenge."
    }

    fn parameters_schema(&self) -> RootSchema {
        schemars::schema_for!(CommentArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let parsed = parse_args!(args, CommentArgs);
        let post_id = clean_path_segment("post_id", &parsed.post_id)?;
        let content = validate_content_len("content", &parsed.content, 1, 40_000)?;
        let parent_id = parsed
            .parent_id
            .as_deref()
            .map(|id| clean_path_segment("parent_id", id))
            .transpose()?;
        let mut body = json!({ "content": content });
        if let Some(parent_id) = parent_id {
            body["parent_id"] = Value::String(parent_id);
        }
        let response = self
            .client
            .post(
                &format!("/posts/{post_id}/comments"),
                Some(body),
                AuthMode::Bearer,
            )
            .await?;
        tool_result(
            self.name(),
            response,
            "If verification_required is true, solve the challenge and call moltbook:verify before assuming the comment is visible.",
        )
    }
}

#[async_trait]
impl Tool for MoltbookVoteTool {
    fn name(&self) -> &'static str {
        "moltbook:vote"
    }

    fn description(&self) -> &'static str {
        "Upvote or downvote Moltbook content. Use only for content Eris genuinely evaluated."
    }

    fn parameters_schema(&self) -> RootSchema {
        schemars::schema_for!(VoteArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let parsed = parse_args!(args, VoteArgs);
        let id = clean_path_segment("id", &parsed.id)?;
        let path = match (parsed.target, parsed.direction) {
            (VoteTarget::Post, VoteDirection::Upvote) => format!("/posts/{id}/upvote"),
            (VoteTarget::Post, VoteDirection::Downvote) => format!("/posts/{id}/downvote"),
            (VoteTarget::Comment, VoteDirection::Upvote) => format!("/comments/{id}/upvote"),
            (VoteTarget::Comment, VoteDirection::Downvote) => {
                return Err(FcpError::SchemaViolation(
                    "Moltbook currently documents comment upvotes only".into(),
                ));
            }
        };
        let response = self.client.post(&path, None, AuthMode::Bearer).await?;
        tool_result(
            self.name(),
            response,
            "Summarize the vote. Follow suggestions only if the user asks.",
        )
    }
}

#[async_trait]
impl Tool for MoltbookPostTool {
    fn name(&self) -> &'static str {
        "moltbook:post"
    }

    fn description(&self) -> &'static str {
        "Create a Moltbook text or link post. Use only when the user explicitly asks to post or approves a drafted post."
    }

    fn parameters_schema(&self) -> RootSchema {
        schemars::schema_for!(PostArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let parsed = parse_args!(args, PostArgs);
        let submolt_name = validate_content_len("submolt_name", &parsed.submolt_name, 2, 30)?;
        let title = validate_content_len("title", &parsed.title, 1, 300)?;
        let mut body = json!({
            "submolt_name": submolt_name,
            "title": title,
        });
        if let Some(content) = parsed.content {
            body["content"] = Value::String(validate_content_len("content", &content, 0, 40_000)?);
        }
        if let Some(url) = parsed.url {
            let trimmed = validate_content_len("url", &url, 1, 2048)?;
            if !trimmed.starts_with("https://") && !trimmed.starts_with("http://") {
                return Err(FcpError::SchemaViolation(
                    "url must start with http:// or https://".into(),
                ));
            }
            body["url"] = Value::String(trimmed);
        }
        if let Some(kind) = parsed.kind {
            body["type"] = Value::String(
                match kind {
                    PostKind::Text => "text",
                    PostKind::Link => "link",
                    PostKind::Image => "image",
                }
                .into(),
            );
        }
        let response = self
            .client
            .post("/posts", Some(body), AuthMode::Bearer)
            .await?;
        tool_result(
            self.name(),
            response,
            "If verification_required is true, solve the challenge and call moltbook:verify before saying the post is published.",
        )
    }
}

#[async_trait]
impl Tool for MoltbookVerifyTool {
    fn name(&self) -> &'static str {
        "moltbook:verify"
    }

    fn description(&self) -> &'static str {
        "Submit an answer to a Moltbook AI verification challenge. The API expects a numeric string with exactly two decimal places (e.g. \"15.00\"). Parse challenge_text literally — extract the arithmetic the prompt asks for; do not substitute unrelated numbers from the prose. If the server returns \"already answered\" (HTTP 409), stop resubmitting that verification_code; create new content if you need a fresh challenge."
    }

    fn parameters_schema(&self) -> RootSchema {
        schemars::schema_for!(VerifyArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let parsed = parse_args!(args, VerifyArgs);
        let verification_code =
            validate_content_len("verification_code", &parsed.verification_code, 8, 256)?;
        let raw_answer = validate_content_len("answer", &parsed.answer, 1, 64)?;
        let answer = normalize_moltbook_verify_answer(&raw_answer);
        let answer = validate_content_len("answer", &answer, 1, 64)?;
        let response = self
            .client
            .post(
                "/verify",
                Some(json!({ "verification_code": verification_code, "answer": answer })),
                AuthMode::Bearer,
            )
            .await?;
        tool_result(
            self.name(),
            response,
            "Report whether the verification succeeded. Do not retry repeatedly after failures.",
        )
    }
}

#[async_trait]
impl Tool for MoltbookNotificationsReadTool {
    fn name(&self) -> &'static str {
        "moltbook:notifications_read"
    }

    fn description(&self) -> &'static str {
        "Mark Moltbook notifications read after they have been handled."
    }

    fn parameters_schema(&self) -> RootSchema {
        schemars::schema_for!(NotificationsReadArgs)
    }

    fn allow_repeat_in_turn(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let parsed = parse_args!(args, NotificationsReadArgs);
        let path = match parsed.scope {
            NotificationScope::All => "/notifications/read-all".to_string(),
            NotificationScope::Post => {
                let post_id = parsed.post_id.as_deref().ok_or_else(|| {
                    FcpError::SchemaViolation("post_id is required when scope=post".into())
                })?;
                format!(
                    "/notifications/read-by-post/{}",
                    clean_path_segment("post_id", post_id)?
                )
            }
        };
        let response = self.client.post(&path, None, AuthMode::Bearer).await?;
        tool_result(
            self.name(),
            response,
            "Confirm notifications were marked read.",
        )
    }
}

#[async_trait]
impl Tool for MoltbookDmTool {
    fn name(&self) -> &'static str {
        "moltbook:dm"
    }

    fn description(&self) -> &'static str {
        "Check or manage Moltbook direct messages. New requests and human-input flags require operator involvement."
    }

    fn parameters_schema(&self) -> RootSchema {
        schemars::schema_for!(DmArgs)
    }

    fn context_view_hint(&self) -> ToolContextViewHint {
        ToolContextViewHint::Snippet {
            max_chars: API_TOOL_SNIPPET_CHARS * 2,
        }
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let parsed = parse_args!(args, DmArgs);
        let response = match parsed.action {
            DmAction::Check => {
                self.client
                    .get("/agents/dm/check", &[], AuthMode::Bearer)
                    .await?
            }
            DmAction::ListRequests => {
                self.client
                    .get("/agents/dm/requests", &[], AuthMode::Bearer)
                    .await?
            }
            DmAction::ListConversations => {
                self.client
                    .get("/agents/dm/conversations", &[], AuthMode::Bearer)
                    .await?
            }
            DmAction::ReadConversation => {
                let id = required_segment(parsed.conversation_id.as_deref(), "conversation_id")?;
                self.client
                    .get(
                        &format!("/agents/dm/conversations/{id}"),
                        &[],
                        AuthMode::Bearer,
                    )
                    .await?
            }
            DmAction::SendRequest => {
                let message = required_message(parsed.message.as_deref())?;
                let mut body = json!({ "message": message });
                match (parsed.to, parsed.to_owner) {
                    (Some(to), None) => {
                        body["to"] = Value::String(validate_content_len("to", &to, 1, 80)?)
                    }
                    (None, Some(to_owner)) => {
                        body["to_owner"] =
                            Value::String(validate_content_len("to_owner", &to_owner, 1, 80)?);
                    }
                    _ => {
                        return Err(FcpError::SchemaViolation(
                            "provide exactly one of to or to_owner for send_request".into(),
                        ));
                    }
                }
                self.client
                    .post("/agents/dm/request", Some(body), AuthMode::Bearer)
                    .await?
            }
            DmAction::SendMessage => {
                let id = required_segment(parsed.conversation_id.as_deref(), "conversation_id")?;
                let message = required_message(parsed.message.as_deref())?;
                let mut body = json!({ "message": message });
                if let Some(needs_human_input) = parsed.needs_human_input {
                    body["needs_human_input"] = Value::Bool(needs_human_input);
                }
                self.client
                    .post(
                        &format!("/agents/dm/conversations/{id}/send"),
                        Some(body),
                        AuthMode::Bearer,
                    )
                    .await?
            }
            DmAction::ApproveRequest => {
                let id = required_segment(parsed.conversation_id.as_deref(), "conversation_id")?;
                self.client
                    .post(
                        &format!("/agents/dm/requests/{id}/approve"),
                        None,
                        AuthMode::Bearer,
                    )
                    .await?
            }
            DmAction::RejectRequest => {
                let id = required_segment(parsed.conversation_id.as_deref(), "conversation_id")?;
                let body = parsed.block.map(|block| json!({ "block": block }));
                self.client
                    .post(
                        &format!("/agents/dm/requests/{id}/reject"),
                        body,
                        AuthMode::Bearer,
                    )
                    .await?
            }
        };
        tool_result(
            self.name(),
            response,
            "Escalate new DM requests, sensitive topics, and needs_human_input messages to the human before acting.",
        )
    }
}

fn clamp_limit(raw: Option<u32>, default: u32, max: u32) -> u32 {
    raw.unwrap_or(default).clamp(1, max)
}

fn required_segment(raw: Option<&str>, label: &str) -> Result<String> {
    let value = raw.ok_or_else(|| FcpError::SchemaViolation(format!("{label} is required")))?;
    clean_path_segment(label, value)
}

fn required_message(raw: Option<&str>) -> Result<String> {
    let value = raw.ok_or_else(|| FcpError::SchemaViolation("message is required".into()))?;
    validate_content_len("message", value, 10, 1000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::Tool;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn normalize_moltbook_verify_answer_formats_two_decimals() {
        assert_eq!(normalize_moltbook_verify_answer("40"), "40.00");
        assert_eq!(normalize_moltbook_verify_answer(" 40.5 "), "40.50");
        assert_eq!(normalize_moltbook_verify_answer("15.00"), "15.00");
    }

    #[test]
    fn normalize_moltbook_verify_answer_preserves_non_numeric() {
        assert_eq!(normalize_moltbook_verify_answer("  maybe  "), "maybe");
    }

    #[tokio::test]
    async fn home_sends_auth_and_returns_rate_limit() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/home"))
            .and(header("authorization", "Bearer test-key"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("X-RateLimit-Remaining", "59")
                    .set_body_json(json!({"your_account":{"name":"Eris"}})),
            )
            .mount(&server)
            .await;

        let client = Arc::new(
            MoltbookClient::for_test(format!("{}/api/v1", server.uri()), Some("test-key".into()))
                .expect("client"),
        );
        let tool = MoltbookHomeTool { client };
        let out = tool.execute(json!({})).await.expect("home");
        assert!(out.contains("Eris"));
        assert!(out.contains("59"));
        assert!(!out.contains("test-key"));
    }

    #[tokio::test]
    async fn post_surfaces_verification_challenge() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/posts"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": true,
                "verification_required": true,
                "post": {
                    "id": "post-1",
                    "verification": {
                        "verification_code": "moltbook_verify_abc",
                        "challenge_text": "A lobster swims twenty meters and slows by five",
                        "instructions": "Respond with only the number"
                    }
                }
            })))
            .mount(&server)
            .await;

        let client = Arc::new(
            MoltbookClient::for_test(format!("{}/api/v1", server.uri()), Some("test-key".into()))
                .expect("client"),
        );
        let tool = MoltbookPostTool { client };
        let out = tool
            .execute(json!({
                "submolt_name": "general",
                "title": "Hello",
                "content": "Testing"
            }))
            .await
            .expect("post");
        assert!(out.contains("verification_required"));
        assert!(out.contains("moltbook:verify"));
    }

    #[tokio::test]
    async fn notifications_read_defaults_scope_to_all() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/notifications/read-all"))
            .and(header("authorization", "Bearer test-key"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"success": true, "marked_read": 4})),
            )
            .mount(&server)
            .await;

        let client = Arc::new(
            MoltbookClient::for_test(format!("{}/api/v1", server.uri()), Some("test-key".into()))
                .expect("client"),
        );
        let tool = MoltbookNotificationsReadTool { client };
        let out = tool
            .execute(json!({}))
            .await
            .expect("notifications_read with empty args");
        assert!(out.contains("marked_read"));
    }

    #[tokio::test]
    async fn search_sends_query_and_returns_hits() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/search"))
            .and(header("authorization", "Bearer test-key"))
            .and(query_param("q", "agents and memory"))
            .and(query_param("type", "all"))
            .and(query_param("limit", "20"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": true,
                "query": "agents and memory",
                "results": [{"id": "p1", "type": "post", "post_id": "p1", "similarity": 0.88}],
                "count": 1,
                "has_more": false
            })))
            .mount(&server)
            .await;

        let client = Arc::new(
            MoltbookClient::for_test(format!("{}/api/v1", server.uri()), Some("test-key".into()))
                .expect("client"),
        );
        let tool = MoltbookSearchTool { client };
        let out = tool
            .execute(json!({"q": "agents and memory"}))
            .await
            .expect("search");
        assert!(out.contains("p1"));
        assert!(out.contains("0.88"));
        assert!(out.contains("moltbook:comments"));
    }

    #[tokio::test]
    async fn search_clamps_limit_to_fifty() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/search"))
            .and(header("authorization", "Bearer test-key"))
            .and(query_param("limit", "50"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"success": true, "results": []})),
            )
            .mount(&server)
            .await;

        let client = Arc::new(
            MoltbookClient::for_test(format!("{}/api/v1", server.uri()), Some("test-key".into()))
                .expect("client"),
        );
        let tool = MoltbookSearchTool { client };
        tool.execute(json!({"q": "topic", "limit": 999}))
            .await
            .expect("search clamp");
    }

    #[tokio::test]
    async fn search_rejects_empty_query() {
        let client = Arc::new(
            MoltbookClient::for_test("http://127.0.0.1:9/api/v1".into(), Some("test-key".into()))
                .expect("client"),
        );
        let tool = MoltbookSearchTool { client };
        let err = tool
            .execute(json!({"q": "   "}))
            .await
            .expect_err("empty q");
        assert!(matches!(err, FcpError::SchemaViolation(_)));
    }

    #[tokio::test]
    async fn search_rejects_query_over_500_chars() {
        let client = Arc::new(
            MoltbookClient::for_test("http://127.0.0.1:9/api/v1".into(), Some("test-key".into()))
                .expect("client"),
        );
        let tool = MoltbookSearchTool { client };
        let q = "a".repeat(501);
        let err = tool
            .execute(json!({"q": q}))
            .await
            .expect_err("oversized q");
        assert!(matches!(err, FcpError::SchemaViolation(_)));
    }

    #[tokio::test]
    async fn dm_send_request_requires_one_recipient() {
        let client = Arc::new(
            MoltbookClient::for_test("http://127.0.0.1:9/api/v1".into(), Some("test-key".into()))
                .expect("client"),
        );
        let tool = MoltbookDmTool { client };
        let err = tool
            .execute(json!({
                "action": "send_request",
                "to": "A",
                "to_owner": "@b",
                "message": "hello there, please connect"
            }))
            .await
            .expect_err("schema violation");
        assert!(matches!(err, FcpError::SchemaViolation(_)));
    }
}
