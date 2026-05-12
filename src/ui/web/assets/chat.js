(function () {
  const transcript = document.getElementById("transcript");
  const thoughtPane = document.getElementById("thought-pane");
  const toolsActivity = document.getElementById("tools-activity");
  const telemetryLog = document.getElementById("telemetry-log");
  const statusState = document.getElementById("status-state");
  const statusMetrics = document.getElementById("status-metrics");
  const statusCore = document.getElementById("status-core");
  const statusTiming = document.getElementById("status-timing");
  const statusTokens = document.getElementById("status-tokens");
  const statusSpeed = document.getElementById("status-speed");
  const form = document.getElementById("chat-form");
  const input = document.getElementById("message-input");
  const btnExit = document.getElementById("btn-exit");
  const btnSend = form ? form.querySelector('button[type="submit"]') : null;
  const shutdownOverlay = document.getElementById("shutdown-overlay");
  const shutdownDetail = document.getElementById("shutdown-overlay-detail");

  const TELEMETRY_MAX_LINES = 80;
  const THOUGHT_MAX = 5;
  const thoughtHistory = [];
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

  function escapeHtml(s) {
    return String(s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;");
  }

  /** Minimal safe markdown: inline code, **bold**, newlines. */
  function renderAssistantMarkdown(raw) {
    var x = escapeHtml(raw);
    x = x.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
    x = x.replace(/`([^`]+)`/g, "<code>$1</code>");
    x = x.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
    x = x.replace(/\n/g, "<br>");
    return x;
  }

  function agentHue(name) {
    var h = 0;
    var i;
    var s = String(name);
    for (i = 0; i < s.length; i++) {
      h = (h * 33 + s.charCodeAt(i)) | 0;
    }
    return Math.abs(h) % 360;
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

  function appendAssistantTranscript(raw) {
    const norm = normalizeLatexArrowsForDisplay(String(raw));
    const m = norm.match(/^\[([^\]]+)\]:\s*([\s\S]*)$/);
    const row = document.createElement("div");
    row.className = "msg assistant";
    if (m) {
      const label = document.createElement("span");
      label.className = "agent-label";
      label.textContent = "[" + m[1] + "]:";
      const hue = agentHue(m[1]);
      label.style.color = "hsl(" + hue + ", 70%, 72%)";
      label.style.borderLeftColor = "hsl(" + hue + ", 58%, 48%)";
      const body = document.createElement("div");
      body.className = "agent-body markdown-body";
      body.innerHTML = renderAssistantMarkdown(m[2]);
      row.appendChild(label);
      row.appendChild(body);
    } else {
      const body = document.createElement("div");
      body.className = "markdown-body";
      body.innerHTML = renderAssistantMarkdown(norm);
      row.appendChild(body);
    }
    transcript.appendChild(row);
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

  function setStatusPlain(text) {
    if (statusMetrics) statusMetrics.style.display = "none";
    if (statusState) {
      statusState.textContent = text;
      statusState.className = "status-pill state-plain";
    }
  }

  function showStatusMetricsRow() {
    if (statusMetrics) statusMetrics.style.display = "flex";
  }

  function normalizeAgentState(raw) {
    if (raw == null) return "?";
    if (typeof raw === "string") return raw;
    if (typeof raw === "object") {
      const keys = Object.keys(raw);
      if (keys.length === 1) return keys[0];
    }
    return String(raw);
  }

  function statePillClass(st) {
    const s = String(st || "").toLowerCase();
    if (s === "idle") return "state-idle";
    if (s === "chat") return "state-chat";
    if (s === "reflect") return "state-reflect";
    if (s === "recover") return "state-recover";
    return "state-unknown";
  }

  function isBusyState(st) {
    return normalizeAgentState(st) !== "Idle";
  }

  function setToolsActivity(text) {
    const t = (text || "").trim();
    toolsActivity.textContent = t;
  }

  function telemetryLineClass(text) {
    const t = String(text);
    if (t.indexOf("[SYSTEM]") === 0) return "telemetry-line telemetry-sys";
    if (t.indexOf("[tool]") === 0) return "telemetry-line telemetry-tool";
    if (t.indexOf("[ui]") === 0) return "telemetry-line telemetry-ui";
    if (t.indexOf("[alarm]") === 0) return "telemetry-line telemetry-alarm";
    if (t.indexOf("[fcp]") === 0) return "telemetry-line telemetry-fcp";
    return "telemetry-line telemetry-def";
  }

  function appendTelemetry(text) {
    const line = document.createElement("div");
    line.className = telemetryLineClass(text);
    line.textContent = text;
    telemetryLog.appendChild(line);
    while (telemetryLog.children.length > TELEMETRY_MAX_LINES) {
      telemetryLog.removeChild(telemetryLog.firstChild);
    }
    telemetryLog.scrollTop = telemetryLog.scrollHeight;
  }

  function clearThoughtHistory() {
    thoughtHistory.length = 0;
    if (thoughtPane) thoughtPane.textContent = "";
  }

  function pushThought(text) {
    if (!thoughtPane) return;
    const t = normalizeLatexArrowsForDisplay(String(text));
    thoughtHistory.push(t);
    while (thoughtHistory.length > THOUGHT_MAX) {
      thoughtHistory.shift();
    }
    thoughtPane.textContent = "";
    for (let i = 0; i < thoughtHistory.length; i++) {
      const el = document.createElement("div");
      el.className = "thought-entry";
      el.textContent = thoughtHistory[i];
      thoughtPane.appendChild(el);
    }
    thoughtPane.scrollTop = thoughtPane.scrollHeight;
  }

  /** `llm_*_tps_milli` is completion tok/s × 1000 (fixed-point). */
  function fmtTpsMilli(milli) {
    const n = Number(milli) || 0;
    if (n <= 0) return "-";
    return (n / 1000).toFixed(2);
  }

  function fmtInferMs(ms) {
    var n = Number(ms) || 0;
    if (n <= 0) return "-";
    if (n >= 1000) return (n / 1000).toFixed(1) + "s";
    return n + "ms";
  }

  function chipPair(k, v) {
    return (
      '<span class="k">' +
      escapeHtml(k) +
      '</span> <span class="v">' +
      escapeHtml(v) +
      "</span>"
    );
  }

  /** @param {object} u state update */
  function applyStateUpdate(u) {
    const st = normalizeAgentState(u.state);
    showStatusMetricsRow();
    if (statusState) {
      statusState.textContent = st;
      const busy = isBusyState(st);
      statusState.className =
        "status-pill " + statePillClass(st) + (busy ? " status-pill--busy" : "");
    }
    const q = u.queued_inputs || 0;
    if (statusCore) {
      statusCore.innerHTML =
        chipPair("Rounds", u.tool_rounds + "/" + u.max_tool_rounds) +
        ' <span class="k">·</span> ' +
        chipPair("Recovery", u.recovery_count + "/" + u.max_recovery_attempts) +
        ' <span class="k">·</span> ' +
        chipPair("Queued", String(q));
    }
    if (statusTiming) {
      statusTiming.innerHTML =
        chipPair("Routing", u.router_ms + "ms") +
        ' <span class="k">·</span> ' +
        chipPair("LLM", u.llm_ms + "ms") +
        ' <span class="k">·</span> ' +
        chipPair("Tools", u.tool_ms + "ms") +
        ' <span class="k">·</span> ' +
        chipPair("Total", u.total_ms + "ms");
    }
    const pt = u.llm_prompt_tokens || 0;
    const ct = u.llm_completion_tokens || 0;
    const tot = pt + ct;
    if (statusTokens) {
      statusTokens.innerHTML =
        chipPair("Prompt", pt + " tok") +
        ' <span class="k">·</span> ' +
        chipPair("Completion", ct + " tok") +
        ' <span class="k">·</span> ' +
        chipPair("Total", tot + " tok");
    }
    if (statusSpeed) {
      statusSpeed.innerHTML =
        chipPair("Infer", fmtInferMs(u.llm_last_generation_ms)) +
        ' <span class="k">·</span> ' +
        chipPair("This reply", fmtTpsMilli(u.llm_last_tps_milli) + " tok/s") +
        ' <span class="k">·</span> ' +
        chipPair("Avg", fmtTpsMilli(u.llm_tps_ewma_milli) + " tok/s");
    }
    setToolsActivity(u.activity_line || "");
  }

  async function requestShutdown() {
    if (shuttingDown) return;
    shuttingDown = true;
    if (btnExit) btnExit.disabled = true;
    if (btnSend) btnSend.disabled = true;
    input.disabled = true;
    appendTelemetry("[ui] Stopping server (clean shutdown)…");
    setStatusPlain("Shutting down…");
    showShutdownOverlay(
      "Stopping Eris, managed sidecars, and asking Ollama to unload this session’s models. This can take a few seconds after the page closes."
    );
    try {
      const res = await fetch("/api/shutdown", { method: "POST" });
      if (res.ok) {
        setStatusPlain("Server stop requested. Terminal will finish cleanup.");
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
        setStatusPlain("Shutdown failed — use Ctrl+C in the terminal");
      }
    } catch (err) {
      setStatusPlain("Server unreachable (may already have exited).");
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
      appendAssistantTranscript(data.IncomingMessage);
      return;
    }
    if (data.UserTranscriptLine) {
      const u = data.UserTranscriptLine;
      appendUserTranscriptLine(u.source, u.body);
      return;
    }
    if (data.ModelThought) {
      pushThought(data.ModelThought);
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
    setStatusPlain("SSE disconnected — refresh the page");
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
    clearThoughtHistory();
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

  setStatusPlain("Connecting…");
})();
