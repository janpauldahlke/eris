pub const DESCRIPTOR_TOMLS: &[&str] = &[
    r#"descriptor_version = 1
tool_name = "agenda:complete"
short_description = "Mark a queued agenda task as completed."
when_to_use = "Use after finishing a background task to close it."
when_not_to_use = "Do not use to create tasks."
routing_hints = ["task done", "complete task", "mark done"]

[[examples_good]]
name = "complete_task"
args = { id = "task-uuid", result = "Completed successfully" }
rationale = "Closes an existing task by id."

[[examples_bad]]
name = "missing_id"
args = { result = "done" }
rationale = "id is required."
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
when_not_to_use = "Do not use to read tasks or mark completion."
routing_hints = ["add task", "remind me", "todo", "queue task"]

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
tool_name = "memory:commit"
short_description = "Commit one staged memory to vault and semantic index."
when_to_use = "Use to persist a specific staged memory by staged_id."
when_not_to_use = "Do not use for bulk commits."
routing_hints = ["commit staged memory", "persist one memory", "save staged item"]

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
when_to_use = "Use for fuzzy recall, who am I / what is my name, user preferences, and anything stored in indexed vault memory. Prefer query alone; use filter_tag only when you know an exact frontmatter tag."
when_not_to_use = "Do not use for exact file reads by path; use vault:read."
routing_hints = ["search memory", "do you remember", "what is my name", "who am I", "user preferences", "my identity", "recall context", "semantic query"]

[[examples_good]]
name = "query_broad"
args = { query = "coffee preference" }
rationale = "Default: semantic search without filter_tag first."

[[examples_good]]
name = "query_with_known_tag"
args = { query = "notes about me", filter_tag = "user" }
rationale = "Optional narrowing when the tag is known from vault metadata."

[[examples_bad]]
name = "empty_query"
args = { query = "" }
rationale = "query cannot be empty."
"#,
    r#"descriptor_version = 1
tool_name = "memory:stage"
short_description = "Stage memory with title/content/tags into ephemeral cache."
when_to_use = "Use to capture facts before explicit commit."
when_not_to_use = "Do not use when immediate persistence is required."
routing_hints = ["remember this", "stage memory", "temporary memory"]

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
short_description = "Return system CPU/RAM/disk and ollama status diagnostics."
when_to_use = "Use when user asks for runtime health or diagnostics."
when_not_to_use = "Do not use for vault or memory operations."
routing_hints = ["health check", "system status", "cpu usage", "diagnostics"]

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
args = { directory = "10_Episodic" }
rationale = "Lists files in a concrete folder."

[[examples_bad]]
name = "wrong_key"
args = { path = "10_Episodic" }
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
args = { relative_path = "10_Episodic/today.md" }
rationale = "Reads a concrete file by relative path."

[[examples_bad]]
name = "wrong_field_name"
args = { path = "10_Episodic/today.md" }
rationale = "Invalid key; must use relative_path."
"#,
    r#"descriptor_version = 1
tool_name = "vault:write"
short_description = "Write content to a vault file using overwrite or append mode."
when_to_use = "Use when you need to create or update a vault file on disk."
when_not_to_use = "Do not use for reading, listing, or writing immutable 00_Core paths."
routing_hints = ["save note", "write file", "append note", "create markdown"]

[[examples_good]]
name = "write_overwrite"
args = { relative_path = "10_Episodic/new_note.md", content = "Hello", mode = "overwrite" }
rationale = "Valid write request with required fields."

[[examples_bad]]
name = "missing_mode"
args = { relative_path = "10_Episodic/new_note.md", content = "Hello" }
rationale = "mode is required."
"#,
    r#"descriptor_version = 1
tool_name = "web:artifact_query"
short_description = "Query a previously fetched web artifact by artifact_id."
when_to_use = "Use after web:fetch to extract targeted snippets."
when_not_to_use = "Do not use without a valid artifact_id from web:fetch."
routing_hints = ["search fetched page", "query artifact", "find in web artifact"]

[[examples_good]]
name = "query_artifact"
args = { artifact_id = "artifact-uuid", query = "latest updates", top_k = 3 }
rationale = "Queries cached web chunks."

[[examples_bad]]
name = "missing_artifact_id"
args = { query = "updates" }
rationale = "artifact_id is required."
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
when_not_to_use = "Do not use to schedule; use clock:timer or clock:alarm."
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
short_description = "Schedule a relative timer (in N minutes) with a label."
when_to_use = "Use for remind me in X minutes, stretch timer, drink water soon."
when_not_to_use = "Do not use for wall-clock at 7am; use clock:alarm."
routing_hints = ["in 30 minutes", "timer", "remind me in", "half an hour", "countdown"]

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
short_description = "Schedule a wall-clock alarm at hour:minute local (24h)."
when_to_use = "Use for wake me at 7:00, alarm at 14:30, at eight am tomorrow logic."
when_not_to_use = "Do not use for in N minutes; use clock:timer."
routing_hints = ["at 7am", "wake me", "alarm at", "remind me at", "o'clock", "tomorrow morning"]

[[examples_good]]
name = "morning"
args = { hour = 7, minute = 0, label = "wake up" }
rationale = "Wall time with label."

[[examples_bad]]
name = "bad_hour"
args = { hour = 25, minute = 0, label = "x" }
rationale = "hour must be 0-23."
"#,
];
