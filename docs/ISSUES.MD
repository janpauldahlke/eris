- about the tools, am thinking about describing each individual tool in a seperate json to give the SML an example for the usage.
- the web:search does kill context condensation, it is simply to large, we should move this out of the scope for the process of fetching, then create a buffer in mom witht the paginated outcome, and then run a serch over this to parser it with the give prompt. \* user says "read www.zeit.de and dtell be about the news" - propmpt with task needs to be "saved" -> websearch triggers and reads fornt page, which will be huge. keeps out in mem, then the prompt is running on this- we CANT inject the web_search directly into our context, is overkill

- on sigkill and /exit we need to have a good tear down
  -> when i hardquit, the models are still running, on eris chat we need to check with like ollama ps which models run and start them!, also tear down lifecycle maybe adding a pending SHUTTING down to the TUI
- persistency to the pyhsical vault.
  -> we need to analyse this, atm block are saved in the semantic brain, but when staged we need them to be moved to the vault into markdowns, this
