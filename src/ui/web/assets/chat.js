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
  /** Rolling buffer: newest at bottom; oldest dropped when this many are exceeded. */
  const THOUGHT_MAX = 4;
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
  function appendUserTranscriptLine(source, body, image, audio) {
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
    if (image && image.preview_url) {
      const img = document.createElement("img");
      img.className = "user-image";
      img.src = image.preview_url;
      img.alt = "Attached image";
      row.appendChild(img);
    }
    if (audio && audio.preview_url) {
      const aud = document.createElement("audio");
      aud.className = "user-audio";
      aud.controls = true;
      aud.src = audio.preview_url;
      aud.preload = "metadata";
      row.appendChild(aud);
    }
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
      appendUserTranscriptLine(u.source, u.body, u.image || null, u.audio || null);
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

  let pendingAttachment = null;
  let pendingAudioAttachment = null;
  let mediaRecorder = null;
  let mediaStream = null;
  let recordedChunks = [];

  function clearPendingAudioAttachment() {
    pendingAudioAttachment = null;
    const chip = document.getElementById("audio-chip");
    const preview = document.getElementById("audio-preview");
    const label = document.getElementById("audio-chip-label");
    if (chip) chip.classList.remove("visible");
    if (preview) {
      preview.removeAttribute("src");
      preview.load();
    }
    if (label) label.textContent = "Voice clip";
  }

  function setPendingAudioAttachment(data) {
    pendingAudioAttachment = {
      relative_path: data.relative_path,
      preview_url: data.preview_url,
      duration_secs: data.duration_secs,
    };
    const chip = document.getElementById("audio-chip");
    const preview = document.getElementById("audio-preview");
    const label = document.getElementById("audio-chip-label");
    if (preview) preview.src = data.preview_url;
    if (label) {
      const d = Number(data.duration_secs);
      label.textContent =
        Number.isFinite(d) && d > 0
          ? "Voice " + d.toFixed(1) + "s"
          : "Voice clip";
    }
    if (chip) chip.classList.add("visible");
  }

  function clearPendingAttachment() {
    pendingAttachment = null;
    const chip = document.getElementById("attachment-chip");
    const preview = document.getElementById("attachment-preview");
    if (chip) chip.classList.remove("visible");
    if (preview) preview.removeAttribute("src");
  }

  function setPendingAttachment(data) {
    pendingAttachment = {
      relative_path: data.relative_path,
      preview_url: data.preview_url,
      width: data.width,
      height: data.height,
    };
    const chip = document.getElementById("attachment-chip");
    const preview = document.getElementById("attachment-preview");
    if (preview) preview.src = data.preview_url;
    if (chip) chip.classList.add("visible");
  }

  const PRECOMPRESS_MAX_EDGE = 2048;
  const PRECOMPRESS_JPEG_QUALITY = 0.85;
  const PRECOMPRESS_IF_BYTES_OVER = 512 * 1024;

  function isHeicFile(file) {
    const t = String(file.type || "").toLowerCase();
    const n = String(file.name || "").toLowerCase();
    return (
      t === "image/heic" ||
      t === "image/heif" ||
      n.endsWith(".heic") ||
      n.endsWith(".heif")
    );
  }

  /** Downscale large camera JPEGs/PNG in-browser before upload (server still normalizes to 896px). */
  async function compressImageFile(file) {
    if (!file || !file.type.startsWith("image/")) return file;
    if (file.type === "image/gif") return file;
    if (file.size <= PRECOMPRESS_IF_BYTES_OVER) return file;

    let bitmap;
    try {
      bitmap = await createImageBitmap(file);
    } catch (_e) {
      return file;
    }

    let tw = bitmap.width;
    let th = bitmap.height;
    const maxEdge = Math.max(tw, th);
    if (maxEdge > PRECOMPRESS_MAX_EDGE) {
      const scale = PRECOMPRESS_MAX_EDGE / maxEdge;
      tw = Math.max(1, Math.round(tw * scale));
      th = Math.max(1, Math.round(th * scale));
    }

    const canvas = document.createElement("canvas");
    canvas.width = tw;
    canvas.height = th;
    const ctx = canvas.getContext("2d");
    if (!ctx) {
      bitmap.close();
      return file;
    }
    ctx.drawImage(bitmap, 0, 0, tw, th);
    bitmap.close();

    const blob = await new Promise(function (resolve, reject) {
      canvas.toBlob(
        function (b) {
          if (b) resolve(b);
          else reject(new Error("compress failed"));
        },
        "image/jpeg",
        PRECOMPRESS_JPEG_QUALITY
      );
    });

    const base = String(file.name || "image").replace(/\.[^.]+$/, "") || "image";
    return new File([blob], base + ".jpg", { type: "image/jpeg" });
  }

  async function uploadVisionImage(file) {
    if (isHeicFile(file)) {
      appendLine(
        "[ui] HEIC/HEIF not supported in web upload — export as JPEG first",
        "system"
      );
      return;
    }
    let uploadFile = file;
    try {
      uploadFile = await compressImageFile(file);
    } catch (_e) {
      uploadFile = file;
    }
    const fd = new FormData();
    fd.append("file", uploadFile);
    const res = await fetch("/api/vision/upload", { method: "POST", body: fd });
    if (!res.ok) {
      const err = await res.json().catch(function () {
        return {};
      });
      appendLine(
        "[ui] image upload failed: " + (err.error || res.status),
        "system"
      );
      return;
    }
    const data = await res.json();
    setPendingAttachment(data);
  }

  const visionEnabled =
    document.body &&
    document.body.getAttribute("data-vision-enabled") === "true";

  const audioEnabled =
    document.body &&
    document.body.getAttribute("data-audio-enabled") === "true";

  async function submitIngress(ingress) {
    try {
      const res = await fetch("/api/action", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ SubmitIngress: ingress }),
      });
      if (!res.ok) {
        appendLine("[ui] could not send message (channel busy?)", "system");
      }
    } catch (_err) {
      appendLine("[ui] network error sending message", "system");
    }
  }

  async function uploadAudioFile(file, autoSend) {
    const fd = new FormData();
    fd.append("file", file);
    const res = await fetch("/api/audio/upload", { method: "POST", body: fd });
    if (!res.ok) {
      let err = {};
      try {
        err = await res.json();
      } catch (_e) {}
      appendLine(
        "[ui] audio upload failed: " + (err.error || res.status),
        "system"
      );
      return null;
    }
    const data = await res.json();
    const attachment = {
      relative_path: data.relative_path,
      preview_url: data.preview_url,
      duration_secs: data.duration_secs,
    };
    if (autoSend) {
      const caption = input ? input.value : "";
      if (input) input.value = "";
      await submitIngress({
        source: "web",
        display: caption,
        for_model: null,
        audio: attachment,
      });
      return attachment;
    }
    setPendingAudioAttachment(data);
    return attachment;
  }

  function stopMediaCapture() {
    if (mediaRecorder && mediaRecorder.state !== "inactive") {
      mediaRecorder.stop();
    }
    if (mediaStream) {
      mediaStream.getTracks().forEach(function (t) {
        t.stop();
      });
      mediaStream = null;
    }
  }

  const composeStack = document.querySelector(".compose-stack");

  if (audioEnabled) {
    const toolbar = document.getElementById("compose-toolbar");
    if (toolbar) toolbar.hidden = false;

    const audioRemove = document.getElementById("audio-remove");
    if (audioRemove) {
      audioRemove.addEventListener("click", function () {
        clearPendingAudioAttachment();
      });
    }

    const micBtn = document.getElementById("audio-mic-btn");
    if (micBtn) {
      micBtn.addEventListener("click", async function () {
        if (mediaRecorder && mediaRecorder.state === "recording") {
          stopMediaCapture();
          micBtn.classList.remove("recording");
          micBtn.setAttribute("aria-label", "Record voice");
          return;
        }
        if (!navigator.mediaDevices || !navigator.mediaDevices.getUserMedia) {
          appendLine("[ui] microphone not supported in this browser", "system");
          return;
        }
        try {
          mediaStream = await navigator.mediaDevices.getUserMedia({ audio: true });
          recordedChunks = [];
          mediaRecorder = new MediaRecorder(mediaStream);
          mediaRecorder.ondataavailable = function (e) {
            if (e.data && e.data.size > 0) recordedChunks.push(e.data);
          };
          mediaRecorder.onstop = async function () {
            const mimeType =
              (mediaRecorder && mediaRecorder.mimeType) || "audio/webm";
            const blob = new Blob(recordedChunks, { type: mimeType });
            recordedChunks = [];
            mediaRecorder = null;
            if (mediaStream) {
              mediaStream.getTracks().forEach(function (t) {
                t.stop();
              });
              mediaStream = null;
            }
            if (blob.size === 0) {
              appendLine("[ui] empty recording", "system");
              return;
            }
            const file = new File([blob], "recording.webm", {
              type: blob.type || "audio/webm",
            });
            try {
              await uploadAudioFile(file, true);
            } catch (_err) {
              appendLine("[ui] network error uploading recording", "system");
            }
          };
          mediaRecorder.start();
          micBtn.classList.add("recording");
          micBtn.setAttribute("aria-label", "Stop recording and send");
        } catch (_err) {
          appendLine("[ui] microphone permission denied or unavailable", "system");
        }
      });
    }
  }

  const composeStackVision = composeStack;

  if (visionEnabled && form) {
    const removeBtn = document.getElementById("attachment-remove");
    if (removeBtn) {
      removeBtn.addEventListener("click", function () {
        clearPendingAttachment();
      });
    }

    function preventDefaults(e) {
      e.preventDefault();
      e.stopPropagation();
    }

    const dropTarget = composeStackVision || form;
    ["dragenter", "dragover", "dragleave", "drop"].forEach(function (evName) {
      dropTarget.addEventListener(evName, preventDefaults, false);
    });

    dropTarget.addEventListener("dragenter", function () {
      form.classList.add("vision-drop-active");
    });
    dropTarget.addEventListener("dragleave", function () {
      form.classList.remove("vision-drop-active");
    });
    dropTarget.addEventListener("drop", async function (e) {
      form.classList.remove("vision-drop-active");
      const dt = e.dataTransfer;
      if (!dt || !dt.files || !dt.files.length) return;
      const file = dt.files[0];
      if (!file || (!file.type.startsWith("image/") && !isHeicFile(file))) {
        appendLine("[ui] drop an image file (png, jpg, webp, gif)", "system");
        return;
      }
      try {
        await uploadVisionImage(file);
      } catch (_err) {
        appendLine("[ui] network error uploading image", "system");
      }
    });
  }

  form.addEventListener("submit", async function (e) {
    e.preventDefault();
    const text = input.value;
    if (!text.trim() && !pendingAudioAttachment) return;
    const trimmed = text.trim();
    const norm = trimmed.toLowerCase();
    if (norm === "/exit" || norm === "/quit") {
      input.value = "";
      requestShutdown();
      return;
    }
    input.value = "";
    const ingress = {
      source: "web",
      display: text,
      for_model: null,
    };
    if (pendingAttachment) {
      ingress.image = pendingAttachment;
      clearPendingAttachment();
    }
    if (pendingAudioAttachment) {
      ingress.audio = pendingAudioAttachment;
      clearPendingAudioAttachment();
    }
    await submitIngress(ingress);
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
