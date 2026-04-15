(function () {
  const transcript = document.getElementById("transcript");
  const thoughtPane = document.getElementById("thought-pane");
  const toolsActivity = document.getElementById("tools-activity");
  const telemetryLog = document.getElementById("telemetry-log");
  const statusLine = document.getElementById("status-line");
  const form = document.getElementById("chat-form");
  const input = document.getElementById("message-input");
  const btnExit = document.getElementById("btn-exit");
  const btnSend = form.querySelector('button[type="submit"]');
  const shutdownOverlay = document.getElementById("shutdown-overlay");
  const shutdownDetail = document.getElementById("shutdown-overlay-detail");

  const TELEMETRY_MAX_LINES = 80;
  let shuttingDown = false;

  function showShutdownOverlay(detail) {
    if (!shutdownOverlay) return;
    if (shutdownDetail && detail) shutdownDetail.textContent = detail;
    shutdownOverlay.classList.remove("hidden");
    shutdownOverlay.setAttribute("aria-hidden", "false");
  }

  function hideShutdownOverlay() {
    if (!shutdownOverlay) return;
    shutdownOverlay.classList.add("hidden");
    shutdownOverlay.setAttribute("aria-hidden", "true");
  }

  /**
   * Some models emit LaTeX math for arrows (e.g. `$\rightarrow$`), which is not rendered in plain text.
   * Normalize common patterns to Unicode for the transcript and thought pane.
   */
  function normalizeLatexArrowsForDisplay(s) {
    if (s == null || typeof s !== "string") return s;
    return s
      .replace(/\$\s*\\+rightarrow\s*\$/gi, "\u2192")
      .replace(/\$\s*\\+Rightarrow\s*\$/gi, "\u21D2")
      .replace(/\$\s*\\+leftarrow\s*\$/gi, "\u2190")
      .replace(/\$\s*\\+to\s*\$/gi, "\u2192");
  }

  /** Browsers only honor this for script-opened windows; OS-opened tabs usually stay open. */
  function tryCloseTab() {
    window.close();
    window.setTimeout(function () {
      try {
        if (document.body) {
          appendTelemetry(
            "[ui] Browser blocked auto-close (normal if this tab was opened from the dock or URL bar). Close the tab yourself."
          );
        }
      } catch (e) {
        /* tab may be tearing down */
      }
    }, 500);
  }

  function appendLine(text, className) {
    const div = document.createElement("div");
    div.className = "msg " + (className || "");
    div.textContent = normalizeLatexArrowsForDisplay(String(text));
    transcript.appendChild(div);
    transcript.scrollTop = transcript.scrollHeight;
  }

  /** @param {string} source — `web` | `cli` | `discord` */
  function appendUserTranscriptLine(source, body) {
    const row = document.createElement("div");
    row.className = "msg user";
    const badge = document.createElement("span");
    badge.className = "src-badge src-" + String(source || "").toLowerCase();
    badge.textContent = String(source || "local");
    const span = document.createElement("span");
    span.className = "user-body";
    span.textContent = normalizeLatexArrowsForDisplay(String(body));
    row.appendChild(badge);
    row.appendChild(span);
    transcript.appendChild(row);
    transcript.scrollTop = transcript.scrollHeight;
  }

  function setStatus(text) {
    statusLine.textContent = text;
  }

  function setToolsActivity(text) {
    const t = (text || "").trim();
    toolsActivity.textContent = t;
  }

  function appendTelemetry(text) {
    const line = document.createElement("div");
    line.className = "telemetry-line";
    line.textContent = text;
    telemetryLog.appendChild(line);
    while (telemetryLog.children.length > TELEMETRY_MAX_LINES) {
      telemetryLog.removeChild(telemetryLog.firstChild);
    }
    telemetryLog.scrollTop = telemetryLog.scrollHeight;
  }

  function applyStateUpdate(u) {
    const st = u.state;
    const q = u.queued_inputs || 0;
    setStatus(
      st +
        " · tool rounds " +
        u.tool_rounds +
        "/" +
        u.max_tool_rounds +
        " · recovery " +
        u.recovery_count +
        "/" +
        u.max_recovery_attempts +
        " · queued " +
        q +
        " · routing " +
        u.router_ms +
        " ms · LLM " +
        u.llm_ms +
        " ms · tools " +
        u.tool_ms +
        " ms · total " +
        u.total_ms +
        " ms"
    );
    setToolsActivity(u.activity_line || "");
  }

  async function requestShutdown() {
    if (shuttingDown) return;
    shuttingDown = true;
    if (btnExit) btnExit.disabled = true;
    if (btnSend) btnSend.disabled = true;
    input.disabled = true;
    appendTelemetry("[ui] Stopping server (clean shutdown)…");
    setStatus("Shutting down…");
    showShutdownOverlay(
      "Stopping Eris, managed sidecars, and asking Ollama to unload this session’s models. This can take a few seconds after the page closes."
    );
    try {
      const res = await fetch("/api/shutdown", { method: "POST" });
      if (res.ok) {
        setStatus("Server stop requested. Terminal will finish cleanup.");
        appendTelemetry(
          "[ui] Goodbye — Ollama model RAM should drop shortly (check Activity Monitor). Closing tab if the browser allows…"
        );
        if (shutdownDetail) {
          shutdownDetail.textContent =
            "Stop signal sent. The terminal process will unload models and exit; this tab will close if the browser allows.";
        }
        es.close();
        await new Promise(function (resolve) {
          window.setTimeout(resolve, 1100);
        });
        tryCloseTab();
      } else {
        shuttingDown = false;
        hideShutdownOverlay();
        if (btnExit) btnExit.disabled = false;
        if (btnSend) btnSend.disabled = false;
        input.disabled = false;
        appendTelemetry(
          "[ui] Shutdown failed (" + res.status + "). Press Ctrl+C in the terminal."
        );
        setStatus("Shutdown failed — use Ctrl+C in the terminal");
      }
    } catch (err) {
      setStatus("Server unreachable (may already have exited).");
      appendTelemetry("[ui] Connection lost — if the terminal is back at a prompt, you are done.");
      if (shutdownDetail) {
        shutdownDetail.textContent =
          "Could not reach the server. If your shell is back at a prompt, cleanup may already be running.";
      }
      es.close();
      await new Promise(function (resolve) {
        window.setTimeout(resolve, 800);
      });
      tryCloseTab();
    }
  }

  function handleSessionEvent(data) {
    if (data.StateUpdate) {
      applyStateUpdate(data.StateUpdate);
      return;
    }
    if (data.IncomingMessage) {
      appendLine(data.IncomingMessage, "assistant");
      return;
    }
    if (data.UserTranscriptLine) {
      const u = data.UserTranscriptLine;
      appendUserTranscriptLine(u.source, u.body);
      return;
    }
    if (data.ModelThought) {
      thoughtPane.textContent = normalizeLatexArrowsForDisplay(data.ModelThought);
      return;
    }
    if (data.SystemError) {
      appendTelemetry(data.SystemError);
      return;
    }
    if (data.SystemAlarm) {
      appendTelemetry("[alarm] relayed to core (scheduler)");
      return;
    }
  }

  const es = new EventSource("/api/events");
  es.onmessage = function (ev) {
    try {
      const data = JSON.parse(ev.data);
      handleSessionEvent(data);
    } catch (e) {
      appendLine("[ui] bad SSE payload", "system");
    }
  };
  es.onerror = function () {
    setStatus("SSE disconnected — refresh the page");
  };

  if (btnExit) {
    btnExit.addEventListener("click", function () {
      requestShutdown();
    });
  }

  input.addEventListener("keydown", function (e) {
    if (e.key !== "Enter" || e.shiftKey) return;
    if (e.isComposing) return;
    e.preventDefault();
    if (shuttingDown) return;
    form.requestSubmit();
  });

  form.addEventListener("submit", async function (e) {
    e.preventDefault();
    const text = input.value;
    if (!text.trim()) return;
    const trimmed = text.trim();
    const norm = trimmed.toLowerCase();
    if (norm === "/exit" || norm === "/quit") {
      input.value = "";
      requestShutdown();
      return;
    }
    input.value = "";
    thoughtPane.textContent = "";
    try {
      const res = await fetch("/api/action", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          SubmitIngress: {
            source: "web",
            display: text,
            for_model: null,
          },
        }),
      });
      if (!res.ok) {
        appendLine("[ui] could not send message (channel busy?)", "system");
      }
    } catch (err) {
      appendLine("[ui] network error sending message", "system");
    }
  });

  window.addEventListener("keydown", function (e) {
    if (e.key === "Escape") {
      fetch("/api/action", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ CancelCurrentTurn: null }),
      }).catch(function () {});
    }
  });

  setStatus("Connecting…");
})();
