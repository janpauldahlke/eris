(function () {
  const transcript = document.getElementById("transcript");
  const thoughtPane = document.getElementById("thought-pane");
  const toolsActivity = document.getElementById("tools-activity");
  const telemetryLog = document.getElementById("telemetry-log");
  const statusLine = document.getElementById("status-line");
  const form = document.getElementById("chat-form");
  const input = document.getElementById("message-input");

  const TELEMETRY_MAX_LINES = 80;

  function appendLine(text, className) {
    const div = document.createElement("div");
    div.className = "msg " + (className || "");
    div.textContent = text;
    transcript.appendChild(div);
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
        " · T:" +
        u.tool_rounds +
        "/" +
        u.max_tool_rounds +
        " R:" +
        u.recovery_count +
        "/" +
        u.max_recovery_attempts +
        " Q:" +
        q +
        " · " +
        u.router_ms +
        "/" +
        u.llm_ms +
        "/" +
        u.tool_ms +
        " ms (Σ " +
        u.total_ms +
        ")"
    );
    setToolsActivity(u.activity_line || "");
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
    if (data.ModelThought) {
      thoughtPane.textContent = data.ModelThought;
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

  form.addEventListener("submit", async function (e) {
    e.preventDefault();
    const text = input.value;
    if (!text.trim()) return;
    input.value = "";
    appendLine("You: " + text.trim(), "user");
    thoughtPane.textContent = "";
    try {
      const res = await fetch("/api/action", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ Submit: text }),
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
