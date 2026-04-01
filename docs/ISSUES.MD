- about the tools, am thinking about describing each individual tool in a seperate json, to give the SML an example for the usage. - by this i want to sepeate actual code and jsons formats

- on sigkill and /exit we need to have a good tear down
  -> when i hardquit, the models are still running, on eris chat we need to check with like ollama ps which models run and start them!, also tear down lifecycle maybe adding a pending SHUTTING down to the TUI
- persistency to the pyhsical vault.
  -> we need to analyse this, atm block are saved in the semantic brain, but when staged we need them to be moved to the vault into markdowns, this
