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
tool_name = "agenda:remind_self"
short_description = "Create or update a self-driven agenda row and link it to a fire time in .fcp/tools/alarms.json; on fire, the agent gets a structured SELF_REMINDER payload (plan + checklist) and executes autonomously."
when_to_use = "Use for multi-step background loops where the agent should resume work on its own: provide task_id or new description, required plan text, optional checklist, and minutes or hour:minute. Best for recurring browse/research/checkpoint cycles where Done/Snooze prompting to the user is not desired."
when_not_to_use = "Do not use for normal user reminders that should ask Done/Snooze (use agenda:remind_at). Do not use for generic label-only countdowns (use clock:timer) or wake-only wall alarms (use clock:alarm)."
routing_hints = [
    "plan for myself",
    "remind myself with steps",
    "set up a self loop",
    "agent self reminder",
    "wake me with checklist",
    "continue this later automatically",
    "resume this workflow in 10 minutes",
    "self-driven reminder",
    "loop this task autonomously",
    "come back to this with checklist",
]

[[examples_good]]
name = "new_self_loop_minutes"
args = { description = "Moltbook cycle until 18:30", plan = "Open home, scan 3 threads, welcome newcomers, then summarize.", checklist = ["clock:now", "moltbook:home", "moltbook:search", "moltbook:comment"], minutes = 5 }
rationale = "Creates a self-driven agenda row and alarm with structured plan."

[[examples_good]]
name = "extend_existing_self_loop"
args = { task_id = "a03e", plan = "Resume research: read latest notes then stage summary.", checklist = ["vault:read", "memory:stage"], hour = 17, minute = 45 }
rationale = "Reuses existing row by task_id and reschedules with updated plan/checklist."

[[examples_bad]]
name = "missing_plan"
args = { description = "continue", minutes = 10 }
rationale = "plan is required."

[[examples_bad]]
name = "both_schedules"
args = { description = "continue", plan = "x", minutes = 10, hour = 9, minute = 0 }
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
short_description = "Search long-term memory (semantic similarity or recency / where-you-left-off)."
when_to_use = "Use for fuzzy recall (default semantic), who am I / preferences, and indexed vault memory. For resuming work after a break, use memory_sort = \"recency\" (latest by vault file mtime / commit time), not for fuzzy conceptual search. Prefer query alone for semantic; use filter_tag only when you know an exact frontmatter tag. Optional: top_k (1..25), max_total_chars, min_score (0..1, semantic only), vault_path_prefix (e.g. 30_Synthesis/). Multi-step flows: query memory for stored contacts before guessing from inbox metadata."
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
    "where we left off",
    "latest notes",
    "most recent saved",
]

[[examples_good]]
name = "query_broad"
args = { query = "coffee preference" }
rationale = "Default: semantic search without filter_tag first."

[[examples_good]]
name = "query_recency_resume"
args = { query = "resume", memory_sort = "recency", top_k = 5 }
rationale = "Latest touched vault / committed memories; not embedding similarity."

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
short_description = "Structured JSON: FCP Ollama host and chat/embed models, CPU and RAM, ollama ps, optional gpu.nvidia_smi; follow report_hint when summarizing."
when_to_use = "Use when the user asks for runtime health or diagnostics. Always summarize Ollama (URL + models), CPU usage, and RAM from the tool JSON; when gpu.nvidia_smi.available is true, include NVIDIA GPU utilization and memory from gpus."
when_not_to_use = "Do not use for vault or memory operations."
routing_hints = ["health check", "system status", "cpu usage", "memory usage", "gpu usage", "nvidia", "ollama status", "diagnostics"]

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
tool_name = "web:find"
short_description = "Lexical search within a vault-cached web page (by artifact_id from web:fetch)."
when_to_use = "Use after web:fetch on the same host before another fetch; searches mission page chunks on disk."
when_not_to_use = "Do not use without artifact_id from web:fetch (UUID from receipt, not browser39 session paths). Not for vault notes (use vault:search)."
suggested_skills = ["web-fetch-workflow"]
routing_hints = ["search fetched page", "query artifact", "find in web artifact", "web find"]

[[examples_good]]
name = "query_artifact"
args = { artifact_id = "artifact-uuid", query = "latest updates", top_k = 3 }
rationale = "Returns snippets from vault chunks."

[[examples_bad]]
name = "missing_artifact_id"
args = { query = "updates" }
rationale = "artifact_id is required."
"#,
    r#"descriptor_version = 1
tool_name = "web:fetch"
short_description = "Fetch one URL into vault web mission cache (browser39); anti-crawl budgets apply."
when_to_use = "Fetch one URL into a web mission (browser39). Receipt JSON is not the full page — use web:find on artifact_id to read stored chunks. Then web:find before re-fetching same host unless same mission_id. Omit fetch_budget unless user caps pages."
when_not_to_use = "Do not use for headlines digest (news:today). URLs must match .fcp/web_allowlist.toml when allowlist_enabled. Page bodies live under 20_Discourse/web/missions/ — use web:find with artifact_id, not vault:read on mission paths."
suggested_skills = ["web-fetch-workflow"]
routing_hints = ["open website", "read web page", "fetch url", "look up this url", "browse a link"]

[[examples_good]]
name = "fetch_url"
args = { url = "https://www.taz.de/", mission_note = "pricing section" }
rationale = "Valid URL; omit fetch_budget so default mission budget applies."

[[examples_bad]]
name = "bad_url"
args = { url = "example.com", mission_note = "x" }
rationale = "URL must start with http:// or https://."
"#,
    r#"descriptor_version = 1
tool_name = "web:search"
short_description = "Search the web via browser39 [search].engine (default DuckDuckGo HTML); caches the results page like web:fetch."
when_to_use = "Use when the user asks to search the web in natural language (no URL). Query must be plain text. Search provider URL must be on web_allowlist (e.g. html.duckduckgo.com)."
when_not_to_use = "Do not use when the user gave a full URL (use web:fetch). After search, use web:find on artifact_id — do not web:fetch the SERP URL again. Not for BBC headline digest (news:today). Disabled when [web].search_enabled is false."
suggested_skills = ["web-fetch-workflow"]
routing_hints = ["search the web", "google", "look up online", "find on the internet", "duckduckgo"]

[[examples_good]]
name = "bundesliga_search"
args = { query = "bundesliga letzter spieltag" }
rationale = "Plain-language query; engine URL built from config."

[[examples_bad]]
name = "url_not_query"
args = { query = "https://www.bbc.com/news" }
rationale = "Use web:fetch for URLs, not web:search."
"#,
    r#"descriptor_version = 1
tool_name = "news:today"
short_description = "Fetch a news homepage and return ranked headline links; optionally deep-fetch a few top articles in one call (reuses the web:fetch pipeline internally)."
when_to_use = "Use for today's headlines, top stories, or a news digest from a homepage. Pass homepage_url for any allowlisted outlet (e.g. https://www.bbc.com/, https://taz.de/). Omit homepage_url to use news_today_default_homepage, or pass category (world, uk, politics, …) to build a section URL from news_today_site_base. Prefer over repeating identical web:fetch in the same turn. Set deep_fetch_top_n (1–3) for article bodies."
when_not_to_use = "Do not use for a one-off article URL (use web:fetch) or plain-language web search (use web:search). Requires matching .fcp/web_allowlist.toml patterns. Not registered when news_today_enabled is false."
suggested_skills = ["web-fetch-workflow"]
routing_hints = ["todays news", "headlines", "top stories", "morning news", "news digest", "breaking news", "what is happening", "front page", "politics headlines", "science news", "business news", "economics news", "world news", "uk news"]

[[examples_good]]
name = "default_bbc_news_listing"
args = {}
rationale = "Uses config default homepage (BBC home https://www.bbc.com/ unless overridden via news_today_default_homepage)."

[[examples_good]]
name = "bbc_politics_section"
args = { category = "politics" }
rationale = "Fetches the BBC politics section listing instead of the generic news hub."

[[examples_good]]
name = "bbc_science_section"
args = { category = "science", max_headlines = 8 }
rationale = "Science, climate, and environment listing with a headline cap."

[[examples_good]]
name = "bbc_homepage"
args = { homepage_url = "https://www.bbc.com/news", max_headlines = 10 }
rationale = "Explicit homepage and headline cap."

[[examples_good]]
name = "with_deep_fetch"
args = { homepage_url = "https://www.bbc.com/news", deep_fetch_top_n = 2 }
rationale = "Fetches two top-ranked article URLs after the homepage."

[[examples_bad]]
name = "bad_homepage_scheme"
args = { homepage_url = "ftp://example.com/news" }
rationale = "homepage_url must start with http:// or https://."
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
short_description = "Current weather for a city via Open-Meteo; returns a pre-computed report string."
when_to_use = "Use when the user wants present conditions at a named place. The tool returns a pre-formatted markdown `report` — do not invent numbers. Use city name; add country_code if ambiguous (e.g. Springfield)."
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
short_description = "Multi-day weather forecast for a city via Open-Meteo; returns a pre-computed report (next 24h + daily outlook)."
when_to_use = "Use when the user wants upcoming hours or days. The tool returns a pre-formatted markdown `report` — do not invent numbers or reinterpret raw data."
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
tool_name = "db:find_connections"
short_description = "Up to three train/transit connections in Germany between two named places, with delays and platforms when available."
when_to_use = "Use when the user asks for train or transit connections, departure times, ICE/IC/RE options, next trains between two cities or stations, or arrival by a given time. Requires explicit timezone on `when`. When this tool is offered, the system prompt includes `[SESSION_REFERENCE_TIME]` with the session wall clock—use its calendar year for bare dates (no year given). Prefer clearer station names (e.g. Hamburg Hbf) if a city is ambiguous. On transient upstream faults, preserve original from/to/when intent and retry with bounded attempts."
when_not_to_use = "Do not use for arbitrary URLs or web pages (use web:fetch). Do not use for current weather (use weather:current). Do not invent station ids; pass human-readable from/to strings only."
suggested_skills = ["db-connections-recovery"]
routing_hints = [
    "train from",
    "train to",
    "Zugverbindung",
    "ICE",
    "IC ",
    "RE ",
    "RB ",
    "nächste Verbindung",
    "connection to",
    "departure Hamburg",
    "Berlin nach",
    "abfahrt",
    "ankunft",
    "Gleis",
    "Verspätung",
    "Fahrplan",
    "Deutsche Bahn",
    "DB Navigator",
    "how do I get from",
    "transit between",
]

[[examples_good]]
name = "major_route"
args = { from = "Hamburg Hbf", to = "Berlin Hbf", when = "2026-04-15T08:00:00+02:00", time_constraint = "departure" }
rationale = "RFC3339 `when` with offset; departure search."

[[examples_good]]
name = "arrival_constraint"
args = { from = "München Hbf", to = "Frankfurt(Main)Hbf", when = "2026-04-16T18:30:00+02:00", time_constraint = "arrival" }
rationale = "Latest arrival interpretation when user says arrive by."

[[examples_bad]]
name = "no_timezone"
args = { from = "Hamburg", to = "Berlin", when = "2026-04-15T08:00:00" }
rationale = "`when` must include offset (+02:00 or Z)."

[[examples_bad]]
name = "empty_from"
args = { from = "", to = "Berlin", when = "2026-04-15T08:00:00+02:00" }
rationale = "`from` must be non-empty."
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
when_to_use = "Use to compose and send an email. Requires to, subject, and body. Optionally cc and bcc. Before sending, verify the recipient from vault or committed memory (vault:search/vault:read or memory:query) rather than guessing addresses."
when_not_to_use = "Do not use to read or check mail. Do not use without explicit user intent to send. Never fabricate recipient emails (for example user@example.com placeholders)."
suggested_skills = ["mail-recipient-verify"]
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
tool_name = "calendar:list"
short_description = "List Google Calendar events in a time window (default: local today)."
when_to_use = "Use when the user asks what is on their Google Calendar, meetings today or this week, or free/busy at a glance. Returns one line per event with id for calendar:get / calendar:update / calendar:delete. When calendar tools are offered, the system prompt includes [SESSION_REFERENCE_TIME]—use its default year for RFC3339 time_min/time_max if the user gives a date without a year."
when_not_to_use = "Do not use for local vault-only todos (use agenda:list). Do not use without Google Workspace google.enabled."
routing_hints = ["google calendar", "meetings today", "schedule this week", "appointments", "what is on my calendar", "list events", "am I free tomorrow"]

[[examples_good]]
name = "today_default"
args = {}
rationale = "Lists primary calendar for local today."

[[examples_good]]
name = "custom_range"
args = { time_min = "2026-04-15T00:00:00+02:00", time_max = "2026-04-16T00:00:00+02:00", max_results = 50 }
rationale = "Explicit RFC3339 window."

[[examples_bad]]
name = "max_only"
args = { time_max = "2026-04-20T00:00:00Z" }
rationale = "time_max alone is invalid; omit both for today or supply time_min."
"#,
    r#"descriptor_version = 1
tool_name = "calendar:get"
short_description = "Fetch one Google Calendar event by id (full JSON + summary line)."
when_to_use = "Use after calendar:list when the user wants details, attendees, Meet link, or description for a specific event_id."
when_not_to_use = "Do not use to list many events (use calendar:list). Do not invent event ids."
routing_hints = ["event details", "open this meeting", "calendar event by id", "read google calendar event"]

[[examples_good]]
name = "primary_event"
args = { event_id = "abc123example" }
rationale = "Reads from primary calendar."

[[examples_bad]]
name = "missing_id"
args = {}
rationale = "event_id is required."
"#,
    r#"descriptor_version = 1
tool_name = "calendar:create"
short_description = "Create a Google Calendar event (title + RFC3339 start/end)."
when_to_use = "Use when the user wants a new meeting or block on Google Calendar with explicit start and end times (RFC3339 with offset). Use [SESSION_REFERENCE_TIME] in the system prompt for the calendar year when the user omits it."
when_not_to_use = "Do not use for local-only reminders without a calendar (use agenda:remind_at or clock:timer). Do not omit end time."
routing_hints = ["schedule a meeting", "add to google calendar", "block my calendar", "create calendar appointment"]

[[examples_good]]
name = "one_hour"
args = { summary = "Team sync", start_datetime = "2026-04-16T15:00:00+02:00", end_datetime = "2026-04-16T16:00:00+02:00", time_zone = "Europe/Berlin" }
rationale = "Creates a timed event on primary calendar."

[[examples_bad]]
name = "empty_title"
args = { summary = "", start_datetime = "2026-04-16T10:00:00Z", end_datetime = "2026-04-16T11:00:00Z" }
rationale = "summary must be non-empty."
"#,
    r#"descriptor_version = 1
tool_name = "calendar:update"
short_description = "Patch fields on an existing Google Calendar event."
when_to_use = "Use when the user wants to rename, reschedule, or edit description/location of an event they identified (event_id from calendar:list). For new start/end datetimes, use RFC3339 with offset; [SESSION_REFERENCE_TIME] supplies the year if omitted."
when_not_to_use = "Do not use without event_id. When changing times, both start_datetime and end_datetime are required together."
routing_hints = ["reschedule meeting", "change calendar event", "move appointment", "rename meeting", "update google calendar"]

[[examples_good]]
name = "rename_only"
args = { event_id = "evt1", summary = "Renamed standup" }
rationale = "Patch summary only."

[[examples_bad]]
name = "start_only"
args = { event_id = "evt1", start_datetime = "2026-04-16T12:00:00Z" }
rationale = "Changing time requires both start and end."
"#,
    r#"descriptor_version = 1
tool_name = "calendar:delete"
short_description = "Delete a Google Calendar event by id."
when_to_use = "Use when the user clearly wants to remove or cancel a specific calendar event (event_id from calendar:list)."
when_not_to_use = "Do not use for email or vault files. Do not use without event_id."
routing_hints = ["cancel meeting", "delete calendar event", "remove from google calendar", "clear that appointment"]

[[examples_good]]
name = "delete_known"
args = { event_id = "evt_deadbeef" }
rationale = "Deletes from primary calendar."

[[examples_bad]]
name = "missing_id"
args = {}
rationale = "event_id is required."
"#,
    r#"descriptor_version = 1
tool_name = "vault:search"
short_description = "Lexically scan vault file contents recursively and return top files with hit excerpts."
when_to_use = "Use when the user wants to locate where something was discussed inside the vault by keywords or phrases (e.g. 'where did we talk about the database migration?'). Returns top-N matching files with line snippets so you can summarize."
when_not_to_use = "Do not use for exact file reads by path (vault:read), folder listings (vault:list), or fuzzy/conceptual recall via embeddings (memory:query). Do not use to write or modify files."
routing_hints = [
    "search the vault",
    "find in my notes",
    "where did we discuss",
    "which file mentions",
    "look up keyword in vault",
    "scan vault contents",
    "grep my notes",
    "find that paragraph about",
    "search markdown",
    "locate references to",
]

[[examples_good]]
name = "search_keywords"
args = { query = "database migration" }
rationale = "Multi-word AND scan across the vault."

[[examples_good]]
name = "scoped_folder"
args = { query = "topology", directory = "10_Topology" }
rationale = "Narrows scan to one subtree."

[[examples_bad]]
name = "empty_query"
args = { query = "" }
rationale = "query cannot be empty."
"#,
    r#"descriptor_version = 1
tool_name = "vault:taglist"
short_description = "Synthesis-only frontmatter tag map: returns tag→count (and optional paths) for notes under 30_Synthesis/. Lets you orient before guessing keywords for vault:search."
when_to_use = "Use to discover what topics/tags exist in the synthesis vault and where the gravity of recent discourse sits, without having to invent a search keyword. Pair top_k or prefix to scan the taxonomy; pass tag to drill into the file paths under one tag and follow up with vault:read."
when_not_to_use = "Do not use for full-text matching across notes (use vault:search). Do not use for non-synthesis folders (00_Invariants, 10_Topology, 20_Discourse, 99_USER_UPLOADED) — they are intentionally skipped because they do not yet use consistent frontmatter. Do not use to read or write notes."
routing_hints = [
    "map of tags",
    "which tags exist",
    "list vault tags",
    "tag taxonomy",
    "tag frequencies",
    "where is the gravity",
    "notes about tag",
    "synthesis tag map",
    "browse vault topics",
    "what have we been discussing",
]

[[examples_good]]
name = "browse_top_tags"
args = { top_k = 25 }
rationale = "Shows the densest tags first; default compact form."

[[examples_good]]
name = "drill_into_tag"
args = { tag = "sandbox" }
rationale = "Returns synthesis paths whose frontmatter contains this tag; case-insensitive."

[[examples_good]]
name = "prefix_filter_with_paths"
args = { prefix = "agent", include_paths = true }
rationale = "Find tags starting with 'agent' and the notes under each."

[[examples_bad]]
name = "use_for_full_text"
args = { tag = "I need files mentioning database migration" }
rationale = "Tag must be a frontmatter tag, not a phrase — use vault:search for full text."
"#,
    r#"descriptor_version = 1
tool_name = "skills:list"
short_description = "List available skill metadata from 10_Topology/skills (id, title, priority, triggers)."
when_to_use = "Use when the user asks what skills exist, wants a skills index, or wants to inspect available skill IDs before reading one."
when_not_to_use = "Do not use to load full skill procedures; use skills:read for one specific skill id."
routing_hints = ["list skills", "what skills are available", "show skills", "skill index", "available skills"]

[[examples_good]]
name = "list_skills"
args = {}
rationale = "Returns structured metadata for each vault skill."

[[examples_bad]]
name = "read_instead_of_list"
args = { id = "mail-recipient-verify" }
rationale = "skills:list takes no id; use skills:read."
"#,
    r#"descriptor_version = 1
tool_name = "skills:read"
short_description = "Read one skill by id from 10_Topology/skills and return structured fields including body."
when_to_use = "Use when the user asks to inspect a specific skill's procedure/details by id."
when_not_to_use = "Do not use without a concrete id. Do not use to discover all skills; use skills:list first."
routing_hints = ["read skill", "show skill", "open skill", "inspect skill", "skill details", "skill by id"]

[[examples_good]]
name = "read_skill"
args = { id = "mail-recipient-verify" }
rationale = "Loads one skill and returns parsed fields."

[[examples_bad]]
name = "missing_id"
args = {}
rationale = "id is required."
"#,
    r#"descriptor_version = 1
tool_name = "skills:create"
short_description = "Create or overwrite a skill file in 10_Topology/skills with strict validation."
when_to_use = "Use when the user explicitly asks to create a new skill file, and the workflow is reusable and procedural. Use overwrite=true only when replacing an existing skill intentionally."
when_not_to_use = "Do not use for one-off notes, arbitrary markdown dumping, or secret-bearing content. Do not overwrite existing skills unless explicitly requested."
routing_hints = ["create skill", "new skill", "write skill", "add skill", "author skill", "update skill with overwrite"]

[[examples_good]]
name = "create_skill"
args = { id = "team-email-safety", title = "Email safety", priority = "mandatory", triggers = ["mail:write"], body = "Never guess recipient addresses.", overwrite = false }
rationale = "Creates a validated new skill."

[[examples_good]]
name = "overwrite_skill"
args = { id = "team-email-safety", title = "Email safety v2", priority = "mandatory", triggers = ["mail:write", "vault:search"], body = "Verify recipients from vault first.", overwrite = true }
rationale = "Explicit replacement with overwrite flag."

[[examples_bad]]
name = "implicit_overwrite"
args = { id = "team-email-safety", title = "x", priority = "mandatory", triggers = ["mail:write"], body = "x" }
rationale = "If id exists, overwrite=false will fail; caller must be explicit."
"#,
    r#"descriptor_version = 1
tool_name = "moltbook:register"
short_description = "Register a new Moltbook agent and return claim credentials."
when_to_use = "Use only when the human explicitly asks to register/create a Moltbook agent identity. This creates an account and returns a secret API key."
when_not_to_use = "Do not use for normal Moltbook visits, status checks, feed reading, posting, or if the user has not explicitly approved registration."
routing_hints = ["register on moltbook", "create moltbook agent", "join moltbook", "claim moltbook account"]

[[examples_good]]
name = "register_agent"
args = { name = "Eris", description = "A local Rust-based assistant joining Moltbook with human oversight." }
rationale = "Explicit account creation request."

[[examples_bad]]
name = "missing_description"
args = { name = "Eris" }
rationale = "Registration requires both name and description."
"#,
    r#"descriptor_version = 1
tool_name = "moltbook:status"
short_description = "Check Moltbook claim/account status and profile."
when_to_use = "Use when the user asks whether the Moltbook account is claimed, active, authenticated, or what profile is configured."
when_not_to_use = "Do not use for browsing content; use moltbook:home first for an active visit."
routing_hints = ["moltbook status", "is moltbook claimed", "moltbook profile", "moltbook account active"]

[[examples_good]]
name = "status"
args = {}
rationale = "No arguments are needed."
"#,
    r#"descriptor_version = 1
tool_name = "moltbook:home"
short_description = "Fetch Moltbook home dashboard; first step for any user-controlled Moltbook visit."
when_to_use = "Use first when the user asks to check, visit, open, inspect, catch up on, browse, or explore Moltbook. Also the entry point for alarm-triggered browse sessions. After browsing, ALWAYS reschedule with agenda:remind_at (minutes=5 unless user chose another interval, description MUST contain 'moltbook' and the session expiry HH:MM). Each alarm is the next wake-up — without it the chain dies. Call clock:now each cycle; stop only when time >= expiry."
when_not_to_use = "Do not call without a user prompt or alarm trigger. Do not cascade into posting, DM approval, or marking notifications read unless the user requested or approves that follow-up."
routing_hints = ["check Moltbook", "visit Moltbook", "what is happening on Moltbook", "Moltbook heartbeat", "Moltbook home", "catch up on Moltbook", "look at Moltbook", "browse Moltbook", "explore Moltbook freely", "Moltbook session", "use Moltbook for a while", "moltbook browse session", "time to check moltbook", "moltbook check-in"]

[[examples_good]]
name = "home"
args = {}
rationale = "Starts an explicit Moltbook visit."
"#,
    r#"descriptor_version = 1
tool_name = "moltbook:feed"
short_description = "Read Moltbook feeds, global posts, or a submolt feed."
when_to_use = "Use after moltbook:home or when the user asks to browse the feed, posts, following feed, or a specific submolt. Supports sort/filter/cursor pagination. During timed browse sessions, treat this as a discovery step only — pair it with moltbook:comments on at least one chosen post_id per cycle. For source=submolt feeds that genuinely interest you, open comments on multiple distinct post_ids from that feed when time allows (stay curious across neighborhoods and revisit favorites)."
when_not_to_use = "Do not use as a background poll. Do not summarize threads or claim you understood a post from feed headlines alone; open moltbook:comments first. Do not vote or comment without having read the thread via moltbook:comments when replies exist."
routing_hints = ["Moltbook feed", "browse Moltbook posts", "read a submolt on Moltbook", "Moltbook posts list", "Moltbook following tab", "general submolt on Moltbook"]

[[examples_good]]
name = "following_feed"
args = { source = "personal", filter = "following", sort = "new", limit = 15 }
rationale = "Reads posts from followed accounts."

[[examples_good]]
name = "submolt_feed"
args = { source = "submolt", submolt = "general", sort = "new" }
rationale = "Reads one community feed."
"#,
    r#"descriptor_version = 1
tool_name = "moltbook:search"
short_description = "Semantic search on Moltbook — find posts and comments by meaning."
when_to_use = "Use when the user or browse session needs to discover threads by topic, question, or concept (not exact keywords). Good for research before commenting, finding discussions to join, or exploring a theme across submolts. Required JSON key: q (string, natural language, max 500 chars). Optional: type (all, posts, or comments), limit (default 20, max 50), cursor for pagination."
when_not_to_use = "Do not spam search as a substitute for reading threads you already have post_ids for. Do not claim you read a hit without following up with moltbook:comments on the relevant post_id."
routing_hints = ["search on Moltbook", "semantic search on Moltbook", "find Moltbook posts about", "what moltys say on Moltbook about", "discover discussions on Moltbook"]

[[examples_good]]
name = "natural_question"
args = { q = "How do agents handle long-term memory?" }
rationale = "Meaning-based query returns related posts and comments."

[[examples_good]]
name = "posts_only"
args = { q = "debugging tool calling failures", type = "posts", limit = 15 }
rationale = "Restricts hits to posts when comments are not needed."

[[examples_good]]
name = "next_page"
args = { q = "AI safety", type = "all", cursor = "eyJvZmZzZXQiOjIwfQ" }
rationale = "Continues a prior search using next_cursor from the API."
"#,
    r#"descriptor_version = 1
tool_name = "moltbook:comments"
short_description = "Read comments on a Moltbook post."
when_to_use = "Use when home/feed indicates activity on a post, or the user asks to read a Moltbook discussion thread. During alarm-driven browse sessions: call at least once per cycle with a real post_id from the latest feed/home — curiosity means reading what molts wrote, not recycling titles. Large threads: start with a modest limit (e.g. 15–25); if `data` includes a pagination cursor, fetch more pages with the same post_id/sort and `cursor` instead of one giant pull."
when_not_to_use = "Do not reply automatically; use moltbook:comment only when the user asked or approves a response."
routing_hints = ["Moltbook comments", "read Moltbook thread", "comments on my Moltbook post", "replies on Moltbook"]

[[examples_good]]
name = "read_comments_first_page"
args = { post_id = "post_123", sort = "new" }
rationale = "Opens the thread with the runtime default page size (moderate); omit cursor on the first request."

[[examples_good]]
name = "next_page"
args = { post_id = "post_123", sort = "new", cursor = "opaque_cursor_from_prior_response" }
rationale = "Continues the same thread using pagination from prior data; optional larger limit on continuation."

[[examples_good]]
name = "next_page_explicit_limit"
args = { post_id = "post_123", sort = "new", limit = 40, cursor = "opaque_cursor_from_prior_response" }
rationale = "Continuation requests may use a higher explicit limit than typical first-page pulls."
"#,
    r#"descriptor_version = 1
tool_name = "moltbook:comment"
short_description = "Create a Moltbook comment or reply."
when_to_use = "Use when the user explicitly asks to comment/reply, or after the user approves a drafted reply. Keep comments thoughtful and on-topic. Required JSON keys: post_id (string), content (string); optional parent_id."
when_not_to_use = "Do not comment for visibility, karma, low-effort reactions, controversial topics needing human input, or before reading the thread. Watch for verification_required and use moltbook:verify if needed."
routing_hints = ["comment on Moltbook", "reply on Moltbook", "answer that Moltbook comment", "leave a thoughtful comment"]

[[examples_good]]
name = "reply"
args = { post_id = "post_123", parent_id = "comment_456", content = "Thanks for the thoughtful note. Here is what I noticed..." }
rationale = "Posts an approved reply."
"#,
    r#"descriptor_version = 1
tool_name = "moltbook:vote"
short_description = "Vote on Moltbook posts or comments."
when_to_use = "Use when the user asks to upvote/downvote, or when they explicitly ask Eris to engage with content it genuinely evaluated and enjoyed. Required JSON keys: target (post or comment), id (string), direction (upvote or downvote)."
when_not_to_use = "Do not mass-vote, vote for politeness, vote without reading, or use for karma manipulation. Comment downvotes are not documented."
routing_hints = ["upvote Moltbook", "downvote Moltbook", "vote on post", "upvote that comment"]

[[examples_good]]
name = "upvote_post"
args = { target = "post", id = "post_123", direction = "upvote" }
rationale = "Rewards a post after evaluation."
"#,
    r#"descriptor_version = 1
tool_name = "moltbook:post"
short_description = "Create a Moltbook text or link post."
when_to_use = "Use only when the user explicitly asks to post or approves a drafted post. Good posts share a real question, discovery, experience, or useful thought."
when_not_to_use = "Do not post just because time passed, to chase karma, as autonomous heartbeat activity, or before checking for duplicate context when appropriate. Respect cooldowns and verification challenges."
routing_hints = ["post to Moltbook", "share on Moltbook", "create Moltbook post", "publish this to general"]

[[examples_good]]
name = "text_post"
args = { submolt_name = "general", title = "A small note on tool recovery", content = "I noticed that explicit recovery hints make agent tools easier to trust." }
rationale = "Creates an approved text post."
"#,
    r#"descriptor_version = 1
tool_name = "moltbook:verify"
short_description = "Submit an answer to a Moltbook AI verification challenge."
when_to_use = "Use after moltbook:post or moltbook:comment returns verification_required with a verification_code and challenge_text. Answer literally from the challenge text (correct units); numeric answers use two decimal places when decimals apply."
when_not_to_use = "Do not guess repeatedly or resubmit the same verification_code after already_answered/409. Failed/expired verification attempts can suspend the account; ask the human if unsure."
routing_hints = ["Moltbook verification", "verify Moltbook post", "solve verification challenge", "submit Moltbook answer"]

[[examples_good]]
name = "verify"
args = { verification_code = "moltbook_verify_abc123", answer = "15.00" }
rationale = "Submits a solved challenge."
"#,
    r#"descriptor_version = 1
tool_name = "moltbook:notifications_read"
short_description = "Mark Moltbook notifications read after handling them."
when_to_use = "Use after reading and responding to activity on a post, or when the user asks to clear Moltbook notifications."
when_not_to_use = "Do not mark notifications read before reading or handling them. Do not clear all unless the user explicitly asks."
routing_hints = ["mark Moltbook notifications read", "clear Moltbook notifications", "done with those Moltbook replies"]

[[examples_good]]
name = "post_read"
args = { scope = "post", post_id = "post_123" }
rationale = "Marks one handled post's notifications read."
"#,
    r#"descriptor_version = 1
tool_name = "moltbook:dm"
short_description = "Check and manage Moltbook direct messages."
when_to_use = "Use for explicit DM checks, listing requests/conversations, reading approved conversations, sending messages, or managing requests with human approval. Required JSON key: action — one of check, list_requests, list_conversations, read_conversation, send_request, send_message, approve_request, reject_request (not generic read)."
when_not_to_use = "Do not approve new DM requests without the human. Escalate sensitive topics, new requests, and messages with needs_human_input. Do not start DMs unless the user asks."
routing_hints = ["Moltbook DM", "Moltbook direct messages", "check Moltbook inbox", "reply to Moltbook message", "approve DM request"]

[[examples_good]]
name = "check_dm"
args = { action = "check" }
rationale = "Quick DM activity check."

[[examples_good]]
name = "send_message"
args = { action = "send_message", conversation_id = "conv_123", message = "Thanks, I will ask my human and follow up." }
rationale = "Sends into an existing approved conversation."

[[examples_good]]
name = "read_conversation"
args = { action = "read_conversation", conversation_id = "conv_123" }
rationale = "Loads messages from an approved conversation."
"#,
    r#"descriptor_version = 1
tool_name = "vision:see"
short_description = "Describe a normalized JPEG under the vault vision upload folder via the multimodal model."
when_to_use = "Use when the user attached an image in web or Discord chat, or asks about a file under the configured upload_dir (e.g. 99_USER_UPLOADED/images). Call with the exact relative_path from the attachment hint before answering visual questions."
when_not_to_use = "Do not use for text files (vault:read), URLs (web:fetch), or when vision is disabled. Do not guess paths — use the path from [Attached image at vault path: …] in the user message."
suggested_skills = ["media-catalog-workflow"]
routing_hints = ["describe image", "what is in this picture", "look at screenshot", "analyze photo", "attached image", "what do you see"]

[[examples_good]]
name = "attached_upload"
args = { relative_path = "99_USER_UPLOADED/images/550e8400-e29b-41d4-a716-446655440000.jpg", prompt = "What text and UI elements are visible?" }
rationale = "Uses vault path from web upload attachment."

[[examples_bad]]
name = "vault_text_file"
args = { relative_path = "10_Topology/skills/foo.md" }
rationale = "Not an image under upload_dir; use vault:read."
"#,
    r#"descriptor_version = 1
tool_name = "media:catalog"
short_description = "Create or update a 40_MEDIA catalog card for a user-uploaded blob (v1: images)."
when_to_use = "Use when the user asks to remember, save, or catalog an uploaded image — even if they only say 'remember this' without a title. First run vision:see on the attachment, then catalog with description (title optional: you invent a short label from what you saw; never use the user's command phrase as title). Requires exact relative_path under the vision upload dir."
when_not_to_use = "Do not catalog on every vision:see — only when the user explicitly wants the image remembered. Do not paste paths instead of cataloging when asked to remember. v1 supports images only."
suggested_skills = ["media-catalog-workflow"]
routing_hints = ["remember this image", "save this photo", "catalog this picture", "keep this image in memory", "remember the truck photo"]

[[examples_good]]
name = "remember_this_no_title"
args = { relative_path = "99_USER_UPLOADED/images/abc.jpg", description = "Artisan fish truck with chalkboard menu at an outdoor market." }
rationale = "User said remember this; agent vision:see'd first; title derived from description."

[[examples_good]]
name = "catalog_truck"
args = { relative_path = "99_USER_UPLOADED/images/abc.jpg", title = "Fisch Feinkost truck", tags = ["food", "zen"], description = "Artisan food truck.", user_notes = "Highly recommended." }
rationale = "Explicit remember request after vision:see."

[[examples_bad]]
name = "user_phrase_as_title"
args = { relative_path = "99_USER_UPLOADED/images/abc.jpg", title = "remember this", description = "Food truck." }
rationale = "Do not use the user's command as title; invent a descriptive short label."

[[examples_bad]]
name = "casual_mention"
args = { relative_path = "99_USER_UPLOADED/images/abc.jpg", title = "x" }
rationale = "Do not catalog unless the user asked to remember."
"#,
    r#"descriptor_version = 1
tool_name = "media:meta"
short_description = "Patch an existing 40_MEDIA catalog card (title, description, notes, tags)."
when_to_use = "Use when the user adds, corrects, or removes metadata on a cataloged image after it was shown or remembered."
when_not_to_use = "Do not use before media:catalog exists for that file. Do not use for first-time catalog — use media:catalog."
suggested_skills = ["media-catalog-workflow"]
routing_hints = ["update image notes", "add tag to photo", "correct image description", "append note to cataloged image"]

[[examples_good]]
name = "append_note"
args = { relative_path = "99_USER_UPLOADED/images/abc.jpg", user_notes_append = "Near Christa's office." }
rationale = "User adds context after display."

[[examples_bad]]
name = "no_card"
args = { relative_path = "99_USER_UPLOADED/images/missing.jpg", title = "x" }
rationale = "Card must exist; use media:catalog first."
"#,
    r#"descriptor_version = 1
tool_name = "vision:display"
short_description = "Show a vault image inline in the web UI for the operator."
when_to_use = "Use when the user asks to show, display, or pull up a known image path (from 40_MEDIA recall, memory, or prior upload). Pair with prose from the catalog card — do not only paste the path. Gatekeeper accepts path or file_path as aliases for relative_path."
when_not_to_use = "Do not use when vision is disabled. Do not use for visual analysis — use vision:see. Do not display without a validated upload_dir path."
suggested_skills = ["media-catalog-workflow"]
routing_hints = ["show me the image", "display the photo", "pull up that picture", "show the fish truck", "let me see it"]

[[examples_good]]
name = "show_truck"
args = { relative_path = "99_USER_UPLOADED/images/abc.jpg" }
rationale = "User asked to see a known cataloged image."

[[examples_bad]]
name = "analyze"
args = { relative_path = "99_USER_UPLOADED/images/abc.jpg" }
rationale = "Visual questions need vision:see; display is for showing pixels to the human."
"#,
    r#"descriptor_version = 1
tool_name = "doc:ingest"
short_description = "Ingest an uploaded PDF/Markdown/text file into the document RAG store."
when_to_use = "Use after a file lands in 99_USER_UPLOADED/files/ or when the user asks to index an uploaded report. Creates chunked vectors plus a 40_MEDIA discovery card for memory recall."
when_not_to_use = "Do not use for vault markdown notes (memory:commit / vault ingest). Do not use for ephemeral pasted text (content lens). Do not use for web fetch artifacts (web:find)."
routing_hints = ["ingest document", "index pdf", "index uploaded file", "parse report", "chunk document", "add document to search"]

[[examples_good]]
name = "ingest_upload"
args = { relative_path = "99_USER_UPLOADED/files/abc.pdf" }
rationale = "Indexes a vault-relative upload path."

[[examples_bad]]
name = "missing_path"
args = {}
rationale = "relative_path is required."
"#,
    r#"descriptor_version = 1
tool_name = "doc:read"
short_description = "Paginated sequential chunk reader for an ingested document."
when_to_use = "Use to read through a document page by page, e.g. to summarize, review, or extract information. Pass doc_id from doc:list or doc:ingest receipt. Optional start (default 0) and count (default 15) for pagination."
when_not_to_use = "Do not use for semantic search (doc:query). Do not use before doc:ingest."
routing_hints = ["read document", "read chunks", "page through document", "sequential read", "read uploaded file", "read the pdf"]

[[examples_good]]
name = "read_from_start"
args = { doc_id = "a1b2c3d4-e5f6-7890-abcd-ef1234567890" }
rationale = "Reads first page of chunks (default start=0, count=15)."

[[examples_good]]
name = "read_next_page"
args = { doc_id = "a1b2c3d4-e5f6-7890-abcd-ef1234567890", start = 15, count = 15 }
rationale = "Continues reading from chunk 15."

[[examples_bad]]
name = "missing_doc_id"
args = {}
rationale = "doc_id is required."
"#,
    r#"descriptor_version = 1
tool_name = "doc:query"
short_description = "Semantic search over ingested document chunks."
when_to_use = "Use when memory:query surfaced a document card (doc_id in type_fields) or the user asks what an uploaded PDF/report says. Returns cited passages."
when_not_to_use = "Do not use for vault markdown recall (memory:query). Do not use for lexical file grep (vault:search). Do not use before doc:ingest."
routing_hints = ["search document", "what does the pdf say", "find in uploaded report", "document passage", "query ingested file"]

[[examples_good]]
name = "scoped_query"
args = { query = "Q2 revenue", doc_id = "a1b2c3d4-e5f6-7890-abcd-ef1234567890" }
rationale = "Scopes search to one ingested document."

[[examples_bad]]
name = "empty_query"
args = { query = "" }
rationale = "query is required."
"#,
    r#"descriptor_version = 1
tool_name = "doc:list"
short_description = "List ingested documents in the document RAG store."
when_to_use = "Use to see which uploads are indexed (doc_id, chunk counts) before doc:query, doc:read, or doc:delete."
when_not_to_use = "Do not use for vault folder listings (vault:list) or memory recall (memory:query)."
routing_hints = ["list documents", "indexed uploads", "what documents are ingested"]

[[examples_good]]
name = "list_all"
args = {}
rationale = "No parameters required."
"#,
    r#"descriptor_version = 1
tool_name = "doc:delete"
short_description = "Remove an ingested document from the RAG store and memory discovery tier."
when_to_use = "Use when the user wants a document fully removed from search (chunks + 40_MEDIA card)."
when_not_to_use = "Do not use to delete raw upload bytes only — removes indexed chunks and catalog card."
routing_hints = ["delete document", "remove ingested pdf", "unindex document"]

[[examples_good]]
name = "delete_by_id"
args = { doc_id = "a1b2c3d4-e5f6-7890-abcd-ef1234567890" }
rationale = "doc_id from doc:list or memory card type_fields."

[[examples_bad]]
name = "missing_id"
args = {}
rationale = "doc_id is required."
"#,
];
