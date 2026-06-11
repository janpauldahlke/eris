//! Fallback "typical phrasing" strings for tool routing embeddings and phrase compendiums.
//! Descriptor `routing_hints` in specs take precedence when present ([`ToolRouter::enrich_for_routing`] / compendium).

/// Lexical triggers used when a tool has no descriptor `routing_hints` in the registry.
pub fn fallback_triggers(tool_name: &str) -> &'static str {
    match tool_name {
        "vault:read" => {
            "reading files, checking notes, looking at documents, show me, what is in my vault, review notes, open file, read my notes"
        }
        "vault:write" => {
            "writing files, saving notes, creating documents, write this down, save to vault, take a note, jot down, record"
        }
        "vault:list" => {
            "listing files, what files do I have, show directory, browse vault, what is in my folder, list notes"
        }
        "memory:query" => {
            "remembering, recalling, do you remember, what did I say, past conversations, search memory, who am I, what is my name, user name, my identity, preferences, facts about the user, recall, recognize me, history, look up contact, stored email address, email in memory, before sending email, before I mail, find their email, multi-step reminder, after the alarm, synthesis notes, 30_Synthesis, long-term recall, vector memory, where we left off, pick up where we left off, latest notes, most recent memory, last saved, resume session"
        }
        "memory:commit" => {
            "save staged entry to vault by staged_id, persist to disk when user asked to keep forever or save permanently"
        }
        "memory:commit_all" => {
            "flush all staged memories, persist all staged entries, bulk commit staged memory"
        }
        "memory:staged_list" => {
            "show staged memory ids, list staged entries, what is currently staged before commit"
        }
        "memory:stage" => {
            "ephemeral staging with ttl, no vault write until commit, hold fact until user wants disk save"
        }
        "agenda:push" => {
            "adding tasks, to-do list, add to agenda queue, schedule, plan, new task without setting a time, queue work, background task, add to my list, track this for later"
        }
        "agenda:list" => {
            "show tasks, what is on my list, pending items, show agenda, my schedule, what do I have to do"
        }
        "agenda:remove" => {
            "remove task, cancel agenda item, delete from list, drop task, never mind that reminder, scratch that task"
        }
        "agenda:remind_at" => {
            "remind me at, remind me in, remind me about, remind me tomorrow, remember to, nudge me at, ping me at, todo reminder, snooze task, alarm for my task, at 3pm for this, on my agenda, on my todo list, task_id reminder, in two minutes, in 2 minutes, multi-step task, several steps later, then send email, remind yourself to, delayed checklist"
        }
        "agenda:complete" => {
            "finishing tasks, mark done, complete task, check off, task finished, I did it"
        }
        "web:fetch" => {
            "fetching URLs, open this link, check this website, browse this page, read this article, get content from URL"
        }
        "web:search" => {
            "search the web, google this, look up online, find on the internet, web search query, duckduckgo, search for bundesliga, latest news search, who won yesterday"
        }
        "news:today" => {
            "todays headlines, top stories, morning briefing, news digest, breaking news, front page news, what is in the news today, latest headlines from homepage, politics news science business economics technology sport world uk health section"
        }
        "web:find" => {
            "query fetched web artifact by artifact id, search fetched page snippets, web find mission chunks"
        }
        "vision:see" => {
            "describe image, what is in this picture, look at screenshot, analyze photo, read image text, attached image, what do you see"
        }
        "system:health" => {
            "system status, Ollama endpoint, LLM model, CPU usage, RAM memory usage, GPU usage NVIDIA nvidia-smi, disk space, health check, diagnostics, how is the system, performance, resources"
        }
        "clock:now" => "what time is it, current time, timezone, date now, local time",
        "clock:timer" => {
            "generic timer in 30 minutes, countdown, stretch break, ping me in, not tied to agenda list, label-only reminder"
        }
        "clock:alarm" => {
            "wake me up, wake alarm, alarm clock only, no task just alarm, not on my todo list, standalone alarm no agenda, no errand, bell only"
        }
        "weather:current" => {
            "weather now, temperature outside, is it raining, rainfall, sunny or cloudy, conditions today, current conditions, what's the weather like"
        }
        "weather:forecast" => {
            "weather forecast, hourly temperature, next days weather, will it rain tomorrow, rain outlook, sun or clouds, upcoming weather"
        }
        "wiki:summary" => {
            "wikipedia, encyclopedia, what is X, who was, summary of topic, general knowledge, what does wikipedia say, define concept, historical figure, science topic overview"
        }
        "db:find_connections" => {
            "train connection, Zugverbindung, ICE schedule, next train from to, Deutsche Bahn, transit between cities, departure arrival platform delay, Hamburg to Berlin by rail"
        }
        "mail:check" => {
            "check email, new mail, inbox, unread messages, check gmail, any new emails, email summary, recent emails, list messages, gmail search, list threads, search inbox, from filter, mail listing"
        }
        "mail:read" => {
            "read email, open message, show email, email details, message content, full email, read message, opens mail, full gmail body, message id, open thread body"
        }
        "mail:write" => {
            "send email, compose mail, write email, reply, email to, send a message, draft email, dispatch mail, send the greeting, send mail"
        }
        "mail:digest" => {
            "summarize email, today's mail, digest inbox, what came in today, recap messages, overview of recent gmail, batch list mail"
        }
        "mail:delete" => {
            "delete email, trash message, remove email, get rid of mail, discard message"
        }
        "mail:move" => {
            "move email to folder, label message, file under label, move to spam, archive to label, put mail in ebay folder"
        }
        "calendar:list" => {
            "google calendar, what meetings, schedule today, events this week, appointments, show my calendar, list events, am I free, what's on tomorrow"
        }
        "calendar:get" => {
            "open calendar event, event details by id, read meeting, full calendar event json"
        }
        "calendar:create" => {
            "add calendar event, schedule meeting, block time, create google calendar appointment, invite on calendar"
        }
        "calendar:update" => {
            "reschedule meeting, change event time, edit calendar event, move appointment, update meeting title"
        }
        "calendar:delete" => {
            "cancel calendar event, remove from google calendar, delete meeting, clear calendar block"
        }
        "vision:display" => {
            "show me the image, display the photo, pull up that picture, let me see it, show the truck"
        }
        "media:catalog" => {
            "remember this image, save this photo, catalog this picture, keep in media memory"
        }
        "media:meta" => {
            "update image notes, add tag to photo, correct image description, append catalog note"
        }
        _ => "",
    }
}
