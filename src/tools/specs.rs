//! Embedded TOML descriptors for **JIT tool routing**: `when_to_use`, examples, hints—not runtime output schemas.
//!
//! **User-facing wording** after tools (`message_to_user` in Idle) is enforced by orchestrator injects:
//! success path [`crate::orchestrator::llm_support::post_tool_guidance::POST_TOOL_USER_REPLY_GUIDANCE`], tool-failure recover
//! [`crate::orchestrator::llm_support::post_tool_guidance::POST_TOOL_FAILURE_TRUST_GUIDANCE`], so we do not repeat that in every descriptor block.

pub const DESCRIPTOR_TOMLS: &[&str] = &[
    r#"descriptor_version = 1
tool_name = "agenda:complete"
short_description = "Mark a queued agenda task as completed."
when_to_use = "Use when the user clearly finished the task (especially after an agenda-linked alarm): call with task_id from agenda:list or AGENDA_CONFIRM line. Prefer explicit user wording (\"done\", \"finished\") before closing."
when_not_to_use = "Do not use to create tasks. Do not infer completion from vague one-line replies; ask or use agenda:remind_at if they need another reminder."
routing_hints = ["task done", "complete task", "mark done", "done with reminder", "alarm task finished", "finished the goldfish check"]

[[examples_good]]
name = "complete_task"
args = { task_id = "a03e", result_summary = "Completed successfully" }
rationale = "Closes an existing task by task_id."

[[examples_bad]]
name = "missing_id"
args = { result_summary = "done" }
rationale = "task_id is required."
"#,
    r#"descriptor_version = 1
tool_name = "agenda:list"
short_description = "List pending agenda tasks."
when_to_use = "Use to inspect queued work before adding/completing tasks."
when_not_to_use = "Do not use for task creation or completion."
routing_hints = ["show tasks", "list agenda", "pending tasks"]

[[examples_good]]
name = "list_tasks"
args = {}
rationale = "Lists queued tasks."

[[examples_bad]]
name = "irrelevant_arg"
args = { status = "pending" }
rationale = "Tool does not require args."
"#,
    r#"descriptor_version = 1
tool_name = "agenda:push"
short_description = "Queue a background agenda task."
when_to_use = "Use to create a new background task for later completion."
when_not_to_use = "Do not use to read tasks, mark completion, remove, cancel, or schedule reminders; use agenda:list, agenda:complete, agenda:remove, or agenda:remind_at."
routing_hints = [
    "add task",
    "todo",
    "queue task",
    "queue work",
    "background task",
    "add to my list",
    "new errand",
    "track this for later",
]

[[examples_good]]
name = "push_task"
args = { description = "Summarize today's logs" }
rationale = "Creates a pending task."

[[examples_bad]]
name = "missing_description"
args = {}
rationale = "description is required."
"#,
    r#"descriptor_version = 1
tool_name = "agenda:remove"
short_description = "Remove a pending agenda task without completion logging."
when_to_use = "Use to cancel a queued task: pass task_id from agenda:list, or description_match with a substring of the real task text (must match exactly one task)."
when_not_to_use = "Do not use agenda:push to cancel; do not use for finished-task logging (use agenda:complete)."
routing_hints = ["remove task", "cancel agenda", "delete from list", "drop task", "never mind"]

[[examples_good]]
name = "remove_by_id"
args = { task_id = "a03e" }
rationale = "Exact id from agenda:list."

[[examples_good]]
name = "remove_by_substring"
args = { description_match = "goldfish" }
rationale = "Substring of the stored description; must be unique among pending tasks."

[[examples_bad]]
name = "meta_instruction_as_match"
args = { description_match = "Remove fish task from agenda" }
rationale = "Pass a substring of the task description, not meta-instructions."

[[examples_bad]]
name = "both_selectors"
args = { task_id = "a03e", description_match = "foo" }
rationale = "Provide only one of task_id or description_match."
"#,
    r#"descriptor_version = 1
tool_name = "agenda:remind_at"
short_description = "Create or update an agenda row and link it to a fire time in .fcp/tools/alarms.json (task + alarm). Default for user reminders, including wall time (e.g. remind me at 3pm to call X)."
when_to_use = "Use when the user ties the reminder to a todo or new description: task_id or new description, plus minutes or hour:minute. After AGENDA_CONFIRM, snooze with same task_id. This is the only tool that writes both agenda and linked alarm. Prefer this over clock:alarm for phrasing like remind me at, remind me in, remind me about, or anything that is a task/errand to track."
when_not_to_use = "Do not use for a generic relative timer with no task meaning (use clock:timer). Do not use for a wake-only or alarm-clock-only ping with no todo (use clock:alarm). Do not use for listing or completing tasks alone; use agenda:list or agenda:complete."
routing_hints = [
    "remind me at",
    "remind me in",
    "remind me about",
    "remind me tomorrow",
    "remember to",
    "do not forget",
    "nudge me at",
    "ping me at",
    "todo reminder",
    "snooze this task",
    "alarm for my task",
    "in 10 minutes for this",
    "in two minutes",
    "in 2 minutes",
    "at 3pm for this",
    "schedule this reminder",
    "on my agenda",
    "on my todo list",
    "task_id reminder",
    "agenda item",
    "multi-step task",
    "several steps later",
    "then send email",
    "after that do",
    "remind yourself to",
    "delayed checklist",
]

[[examples_good]]
name = "relative_minutes"
args = { task_id = "a03e", minutes = 30 }
rationale = "Existing task, relative reminder."

[[examples_good]]
name = "snooze_after_alarm"
args = { task_id = "a03e", minutes = 10 }
rationale = "Same task_id as AGENDA_CONFIRM; replaces prior linked alarm."

[[examples_good]]
name = "wall_clock_new_task"
args = { description = "Call dentist", hour = 14, minute = 30 }
rationale = "New agenda row plus wall-clock alarm."

[[examples_bad]]
name = "both_schedules"
args = { task_id = "a03e", minutes = 10, hour = 9, minute = 0 }
rationale = "Provide either minutes or hour+minute, not both."
"#,
    r#"descriptor_version = 1
tool_name = "memory:commit"
short_description = "Commit one staged memory to vault and semantic index."
when_to_use = "Use when the user asked to save permanently, keep in the vault, or finalize staged content to disk; or in a later turn after staging when they want it persisted."
when_not_to_use = "Do not use for bulk-only workflows; use memory:commit_all. Do not chain immediately after memory:stage in the same multi-step turn if the user only said remember or note without asking to save to the vault or disk."
routing_hints = ["commit staged memory", "persist one memory", "save staged item", "save to vault", "keep forever"]

[[examples_good]]
name = "commit_by_id"
args = { staged_id = "staged-uuid" }
rationale = "Preferred commit path."

[[examples_bad]]
name = "missing_selector"
args = {}
rationale = "Either staged_id or title is required."
"#,
    r#"descriptor_version = 1
tool_name = "memory:commit_all"
short_description = "Commit all currently staged memories with best effort."
when_to_use = "Use to persist all staged entries in one operation."
when_not_to_use = "Do not use when only one staged item should be committed."
routing_hints = ["commit all memories", "flush staged memory", "bulk commit staged"]

[[examples_good]]
name = "commit_all"
args = {}
rationale = "Commits all staged entries."

[[examples_bad]]
name = "unexpected_arg"
args = { force = true }
rationale = "Tool does not require args."
"#,
    r#"descriptor_version = 1
tool_name = "memory:query"
short_description = "Search long-term semantic memory for facts, user identity, preferences, and past context."
when_to_use = "Use for fuzzy recall, who am I / what is my name, user preferences, and anything stored in indexed vault memory. Prefer query alone; use filter_tag only when you know an exact frontmatter tag. Optional: top_k (1..25), max_total_chars, min_score (0..1), vault_path_prefix (e.g. 30_Synthesis/) to narrow by vault path. In multi-step flows (reminder then email, contact then send), query memory for stored addresses or contact facts before guessing from inbox metadata."
when_not_to_use = "Do not use for exact file reads by path; use vault:read."
routing_hints = [
    "search memory",
    "do you remember",
    "what is my name",
    "who am I",
    "user preferences",
    "my identity",
    "recall context",
    "semantic query",
    "look up contact",
    "stored email address",
    "email address in memory",
    "contact info we saved",
    "before sending email",
    "before I email",
    "before I mail",
    "find their email",
    "multi-step reminder",
    "after the alarm",
    "when the timer fires",
    "synthesis folder",
    "30_Synthesis",
    "what did I save about",
    "facts about the user",
    "long-term recall",
    "vector memory",
]

[[examples_good]]
name = "query_broad"
args = { query = "coffee preference" }
rationale = "Default: semantic search without filter_tag first."

[[examples_good]]
name = "query_with_known_tag"
args = { query = "notes about me", filter_tag = "user" }
rationale = "Optional narrowing when the tag is known from vault metadata."

[[examples_good]]
name = "query_with_path_prefix"
args = { query = "Pauline", vault_path_prefix = "30_Synthesis/", top_k = 8 }
rationale = "Narrow to Synthesis folder when looking for specific concepts."

[[examples_bad]]
name = "empty_query"
args = { query = "" }
rationale = "query cannot be empty."
"#,
    r#"descriptor_version = 1
tool_name = "memory:stage"
short_description = "Stage memory with title/content/tags into ephemeral cache (TTL); does not write vault files."
when_to_use = "Use to capture facts the user wants held in staging; returns staged_id for a later commit when they ask to save to the vault."
when_not_to_use = "Do not use when the user only wants to search existing memory (memory:query) or read a path (vault:read)."
routing_hints = ["remember this", "stage memory", "temporary memory", "hold in staging", "not yet saved to vault"]

[[examples_good]]
name = "stage_fact"
args = { title = "hagbard_profile", content = "User prefers concise responses.", tags = ["user", "preference"] }
rationale = "Valid staged memory payload."

[[examples_bad]]
name = "missing_tags"
args = { title = "x", content = "y" }
rationale = "tags is required and must be non-empty."
"#,
    r#"descriptor_version = 1
tool_name = "memory:staged_list"
short_description = "List currently staged ephemeral memories and metadata."
when_to_use = "Use before memory commits to inspect available staged ids."
when_not_to_use = "Do not use for semantic retrieval from vector memory."
routing_hints = ["show staged memory", "list staged ids", "what is staged"]

[[examples_good]]
name = "list_staged"
args = { include_content_preview = false }
rationale = "Shows staged entries."

[[examples_bad]]
name = "wrong_key"
args = { preview = true }
rationale = "Expected include_content_preview."
"#,
    r#"descriptor_version = 1
tool_name = "system:health"
short_description = "Structured JSON: FCP Ollama host and chat/embed models, CPU and RAM, plus ollama ps; follow report_hint when summarizing."
when_to_use = "Use when the user asks for runtime health or diagnostics. Always summarize Ollama (URL + models), CPU usage, and RAM from the tool JSON."
when_not_to_use = "Do not use for vault or memory operations."
routing_hints = ["health check", "system status", "cpu usage", "memory usage", "ollama status", "diagnostics"]

[[examples_good]]
name = "health"
args = {}
rationale = "No args required."

[[examples_bad]]
name = "unexpected_args"
args = { verbose = true }
rationale = "Tool does not require args."
"#,
    r#"descriptor_version = 1
tool_name = "vault:list"
short_description = "List file entries in a vault subdirectory."
when_to_use = "Use when you need filenames in a folder before selecting a file."
when_not_to_use = "Do not use for recursive semantic search or file reading."
routing_hints = ["list files", "show directory", "browse folder", "what files exist"]

[[examples_good]]
name = "list_episodic"
args = { directory = "20_Discourse" }
rationale = "Lists files in a concrete folder."

[[examples_bad]]
name = "wrong_key"
args = { path = "20_Discourse" }
rationale = "directory is required."
"#,
    r#"descriptor_version = 1
tool_name = "vault:read"
short_description = "Read a vault file by relative_path."
when_to_use = "Use when you need exact file contents from the workspace vault."
when_not_to_use = "Do not use for listing directories or writing files."
routing_hints = ["read file", "open note", "show file", "inspect markdown"]

[[examples_good]]
name = "read_project_note"
args = { relative_path = "20_Discourse/today.md" }
rationale = "Reads a concrete file by relative path."

[[examples_bad]]
name = "wrong_field_name"
args = { path = "20_Discourse/today.md" }
rationale = "Invalid key; must use relative_path."
"#,
    r#"descriptor_version = 1
tool_name = "vault:write"
short_description = "Write content to a vault file using overwrite or append mode."
when_to_use = "Use when you need to create or update a vault file on disk."
when_not_to_use = "Do not use for reading, listing, or writing immutable 00_Invariants paths."
routing_hints = ["save note", "write file", "append note", "create markdown"]

[[examples_good]]
name = "write_overwrite"
args = { relative_path = "20_Discourse/new_note.md", content = "Hello", mode = "overwrite" }
rationale = "Valid write request with required fields."

[[examples_bad]]
name = "missing_mode"
args = { relative_path = "20_Discourse/new_note.md", content = "Hello" }
rationale = "mode is required."
"#,
    r#"descriptor_version = 1
tool_name = "ephemeral:buffer_query"
short_description = "Search inside a staged chunked buffer by buffer_id (lexical or semantic over chunks)."
when_to_use = "Use after vault:read or web:fetch returned a buffer receipt when you need keyword or semantic hits instead of paging the whole blob."
when_not_to_use = "Do not use without a valid buffer_id from a recent staging receipt; do not use for vault paths directly."
routing_hints = ["search staged buffer", "find in large file buffer", "query buffered chunks", "keyword in fetched page", "HITL section in book buffer", "snippet search buffer_id"]

[[examples_good]]
name = "query_buffer"
args = { buffer_id = "buf_1", query = "Human-in-the-Loop", top_k = 3 }
rationale = "Queries staged chunks by buffer_id."

[[examples_bad]]
name = "missing_buffer_id"
args = { query = "updates" }
rationale = "buffer_id is required."
"#,
    r#"descriptor_version = 1
tool_name = "web:fetch"
short_description = "Fetch and sanitize a webpage into an artifact receipt."
when_to_use = "Use for URL retrieval before targeted querying of fetched content."
when_not_to_use = "Do not use for local file reads or direct semantic vault search."
routing_hints = ["open website", "read web page", "fetch url", "news from"]

[[examples_good]]
name = "fetch_url"
args = { url = "https://example.com" }
rationale = "Valid fully-qualified URL."

[[examples_bad]]
name = "bad_url"
args = { url = "example.com" }
rationale = "URL must start with http:// or https://."
"#,
    r#"descriptor_version = 1
tool_name = "clock:now"
short_description = "Return current local time as HH:MM : DD/MM/YY plus timezone/offset."
when_to_use = "Use to ground scheduling in the real-world clock before setting timers or alarms; when answering the user, prefer that time format."
when_not_to_use = "Do not use to schedule alarms or timers; use clock:timer, clock:alarm, or agenda:remind_at for agenda tasks."
routing_hints = ["what time is it", "current time", "timezone", "now", "date and time"]

[[examples_good]]
name = "now"
args = {}
rationale = "No arguments."

[[examples_bad]]
name = "unexpected"
args = { tz = "UTC" }
rationale = "Tool takes no parameters."
"#,
    r#"descriptor_version = 1
tool_name = "clock:timer"
short_description = "Schedule a relative timer (in N minutes) with a free-text label — not tied to the agenda file."
when_to_use = "Use for generic one-off pings: stretch, drink water, eye break — label only, no agenda row in .fcp/tools/agenda.json."
when_not_to_use = "Do not use for wall-clock at 7am (use clock:alarm). Do not use to attach a reminder to a queued agenda task or to create an agenda-linked alarm (use agenda:remind_at with task_id or description + schedule)."
routing_hints = ["in 30 minutes generic timer", "countdown", "timer with label only", "half an hour ping", "not my agenda list"]

[[examples_good]]
name = "stretch"
args = { minutes = 30, label = "stretch" }
rationale = "Relative delay with label."

[[examples_bad]]
name = "zero_minutes"
args = { minutes = 0, label = "x" }
rationale = "minutes must be positive."
"#,
    r#"descriptor_version = 1
tool_name = "clock:alarm"
short_description = "Wall-clock alarm at hour:minute local (24h) with a label only — no agenda row. Narrow use: wake-style or alarm-only pings, not tracked todos."
when_to_use = "Use only when the user wants a fixed local time alarm without a todo to track or complete: wake up, alarm clock, short bell, no list item. Not for remind me to do X."
when_not_to_use = "Do not use for remind me to errands, tasks, or anything that belongs on the agenda — use agenda:remind_at. Do not use for in N minutes (use clock:timer). Do not use for a specific agenda task_id (use agenda:remind_at)."
routing_hints = ["wake me up", "wake alarm", "alarm clock only", "no task just alarm", "not on my todo list", "just wake me", "standalone alarm no agenda", "no errand to track", "bell only"]

[[examples_good]]
name = "morning"
args = { hour = 7, minute = 0, label = "wake up" }
rationale = "Wall time with label."

[[examples_bad]]
name = "bad_hour"
args = { hour = 25, minute = 0, label = "x" }
rationale = "hour must be 0-23."
"#,
    r#"descriptor_version = 1
tool_name = "weather:current"
short_description = "Current weather (instant variables) for a city via Open-Meteo geocoding + forecast."
when_to_use = "Use when the user wants present conditions at a named place: temperature, and when returned by the API also precipitation/rain and cloud or sun-related fields. Use city name; add country_code if the name is ambiguous (e.g. Springfield)."
when_not_to_use = "Do not use for multi-day hourly series; use weather:forecast. Do not use for arbitrary URLs."
routing_hints = ["weather now", "temperature outside", "is it raining", "rainfall", "cloudy or sunny", "current conditions", "humidity today"]

[[examples_good]]
name = "city"
args = { city = "Hamburg" }
rationale = "Resolve a major city without country filter."

[[examples_good]]
name = "city_and_country"
args = { city = "Springfield", country_code = "US" }
rationale = "Disambiguate with ISO country code."

[[examples_bad]]
name = "empty_city"
args = { city = "" }
rationale = "city must be non-empty."
"#,
    r#"descriptor_version = 1
tool_name = "weather:forecast"
short_description = "Hourly weather forecast for a city (several days) via Open-Meteo: temperature plus precipitation and cloud cover when available."
when_to_use = "Use when the user wants upcoming hours/days: temperature trends, and when the tool returns them also rain/precipitation and cloud or sun-related patterns, not only instant conditions."
when_not_to_use = "Do not use for only current conditions; use weather:current. Do not use for arbitrary URLs."
routing_hints = ["weather forecast", "hourly temperature", "next days weather", "will it rain tomorrow", "rainfall outlook", "sunny or cloudy week"]

[[examples_good]]
name = "forecast_city"
args = { city = "Berlin" }
rationale = "Hourly series for a named city."

[[examples_bad]]
name = "empty_city"
args = { city = "" }
rationale = "city must be non-empty."
"#,
    r#"descriptor_version = 1
tool_name = "wiki:summary"
short_description = "English Wikipedia lead summary by article title (REST page/summary)."
when_to_use = "Use for encyclopedia-style facts: what/who is X, short overview from English Wikipedia. User names a topic, not a URL."
when_not_to_use = "Do not use for pasted URLs or non-Wikipedia sites (use web:fetch). Do not use to search the user vault or long-term memory (use vault:read, vault:list, memory:query). Do not duplicate weather tools for place conditions."
routing_hints = ["wikipedia", "encyclopedia", "what is", "who was", "summary of topic", "general knowledge", "define"]

[[examples_good]]
name = "planet"
args = { title = "Earth" }
rationale = "Article title resolves to a standard page."

[[examples_good]]
name = "disambiguation_style"
args = { title = "Rust (programming language)" }
rationale = "Parenthetical title is valid for Wikipedia."

[[examples_bad]]
name = "empty_title"
args = { title = "" }
rationale = "title must be non-empty."

[[examples_bad]]
name = "url_instead"
args = { title = "https://en.wikipedia.org/wiki/Foo" }
rationale = "User pasted a URL; use web:fetch instead."
"#,
    r#"descriptor_version = 1
tool_name = "mail:check"
short_description = "List recent or filtered Gmail messages with subject, from, date, and preview per row."
when_to_use = "Use when the user wants to see recent or filtered messages. Supports Gmail search query syntax (e.g. is:unread, from:boss@co.com). Each row includes id, thread, subject, from, date, and a short preview; use mail:read for full body."
when_not_to_use = "Do not use to read full message body (use mail:read) or send mail (use mail:write)."
routing_hints = [
    "check email",
    "new mail",
    "inbox",
    "unread messages",
    "check gmail",
    "any new emails",
    "email summary",
    "subject line",
    "who emailed me",
    "gmail search",
    "list threads",
    "recent gmail",
    "from: filter",
    "search inbox",
    "mail listing",
]

[[examples_good]]
name = "check_unread"
args = { query = "is:unread", max_results = 5 }
rationale = "Filters for unread messages."

[[examples_good]]
name = "check_recent"
args = {}
rationale = "Lists recent messages with defaults."

[[examples_bad]]
name = "read_full_message"
args = { query = "subject:report" }
rationale = "Listing shows previews only; use mail:read for full content."
"#,
    r#"descriptor_version = 1
tool_name = "mail:read"
short_description = "Read full content of a Gmail message by ID."
when_to_use = "Use to read the full content of a specific message by ID (from mail:check results). Returns parsed headers and body text."
when_not_to_use = "Do not use without a message_id from mail:check. Do not use to list messages or send mail."
routing_hints = [
    "read email",
    "open message",
    "show email",
    "email details",
    "message content",
    "full email",
    "full gmail body",
    "message_id",
    "open thread body",
    "read that message",
]

[[examples_good]]
name = "read_by_id"
args = { message_id = "18f1a2b3c4d5e6f7" }
rationale = "Reads a specific message by ID from mail:check."

[[examples_bad]]
name = "missing_id"
args = {}
rationale = "message_id is required."
"#,
    r#"descriptor_version = 1
tool_name = "mail:write"
short_description = "Send an email via Gmail."
when_to_use = "Use to compose and send an email. Requires to, subject, and body. Optionally cc and bcc."
when_not_to_use = "Do not use to read or check mail. Do not use without explicit user intent to send."
routing_hints = [
    "send email",
    "compose mail",
    "write email",
    "reply",
    "email to",
    "send a message",
    "dispatch mail",
    "send them the greeting",
    "mail write",
]

[[examples_good]]
name = "send_basic"
args = { to = "colleague@example.com", subject = "Meeting notes", body = "Here are the notes from today." }
rationale = "Valid send with required fields."

[[examples_good]]
name = "send_with_cc"
args = { to = "main@example.com", subject = "Update", body = "Status update.", cc = "team@example.com" }
rationale = "CC is optional but valid."

[[examples_bad]]
name = "empty_to"
args = { to = "", subject = "Hi", body = "Hello" }
rationale = "to must be a valid email address."
"#,
    r#"descriptor_version = 1
tool_name = "mail:digest"
short_description = "List many Gmail messages (metadata + snippet) in one block — default is mail from today."
when_to_use = "Use when the user wants a summary or digest of recent mail across several messages. Default query is mail from today (local date); override with Gmail search syntax. Returns snippets only; use mail:read for full body."
when_not_to_use = "Do not use for a single message (use mail:read) or a quick inbox peek (use mail:check). Do not use to send or move mail."
routing_hints = ["summarize email", "today's mail", "digest", "recap inbox", "what email did I get", "overview of messages", "recent gmail batch"]

[[examples_good]]
name = "digest_today_default"
args = {}
rationale = "Uses default after: today for a batch digest."

[[examples_good]]
name = "digest_unread_week"
args = { query = "is:unread newer_than:7d", max_messages = 30 }
rationale = "Custom query for a weekly unread digest."

[[examples_bad]]
name = "single_message"
args = { query = "rfc822msgid:foo" }
rationale = "One specific message; use mail:read for full content."
"#,
    r#"descriptor_version = 1
tool_name = "mail:delete"
short_description = "Trash or permanently delete a Gmail message by id."
when_to_use = "Use when the user clearly wants to delete or discard a specific message they already identified (message_id from mail:check or mail:digest). Default is Trash (recoverable)."
when_not_to_use = "Do not use without a message_id. Do not use permanent=true unless the user explicitly asks for permanent deletion."
routing_hints = ["delete email", "trash this message", "remove mail", "discard email", "get rid of message"]

[[examples_good]]
name = "trash_by_id"
args = { message_id = "18f1a2b3c4d5e6f7" }
rationale = "Moves message to Trash."

[[examples_bad]]
name = "missing_id"
args = {}
rationale = "message_id is required."
"#,
    r#"descriptor_version = 1
tool_name = "mail:move"
short_description = "Move a message to a label (folder) or Spam; creates user labels if missing."
when_to_use = "Use when the user wants to file, label, or move a specific message (message_id from mail:check). Target \"spam\" moves to Spam. Other names add a user label and create it if needed."
when_not_to_use = "Do not use without message_id. Do not use to read mail. If the user only wants to delete, use mail:delete."
routing_hints = ["move to folder", "label this email", "file under", "move to spam", "put mail in ebay", "organize email into label"]

[[examples_good]]
name = "move_spam"
args = { message_id = "abc123", target = "spam" }
rationale = "Uses system Spam label."

[[examples_good]]
name = "move_new_label"
args = { message_id = "abc123", target = "ebay" }
rationale = "Creates label ebay if missing and moves the message."

[[examples_bad]]
name = "empty_target"
args = { message_id = "x", target = "" }
rationale = "target must name a folder or spam."
"#,
    r#"descriptor_version = 1
tool_name = "ephemeral:buffer_page"
short_description = "Fetch the next window of chunks from a staged large buffer by buffer_id."
when_to_use = "Use after vault:read returned a buffer receipt for an oversized file, or after web:fetch, to read sequential chunk pages without loading the whole blob into context."
when_not_to_use = "Do not use without a valid buffer_id from a recent vault:read or web:fetch receipt."
routing_hints = ["next page of large file", "read buffer chunks", "paginate staged vault file", "buffer_id page", "continue reading the file", "next section of the note", "read more of the document", "rest of the file", "sequential chunks", "keep going through the note", "following pages of the artifact"]

[[examples_good]]
name = "page_zero"
args = { buffer_id = "buf_1", page = 0, page_size = 2 }
rationale = "Reads first two chunks."

[[examples_bad]]
name = "missing_id"
args = { page = 0 }
rationale = "buffer_id is required."
"#,
];
