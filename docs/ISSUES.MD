check on te usage of config

enable_reasoning_fsm flag
we use it for ollama, but later also for
Tag-stripping lives in ReasoningRouter in `src/engine/router.rs `
both serve different purposes, so we need to distinct this

---

65536 num ctx for gemma4
