(function () {
  const modal = document.getElementById("console-modal");
  const modalTitle = document.getElementById("console-modal-title");
  const modalBody = document.getElementById("console-modal-body");
  const modalClose = document.getElementById("console-modal-close");
  const toastEl = document.getElementById("console-toast");
  const rail = document.getElementById("left-rail");
  const railToggle = document.getElementById("rail-toggle");

  let toastTimer = null;
  let activePanel = null;
  let memoryCards = [];
  let activeTagFilter = null;
  let uploadsPollTimer = null;

  const THEME_KEY = "eris_theme";
  const RAIL_KEY = "eris_rail_expanded";

  function escapeHtml(s) {
    return String(s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;");
  }

  function showToast(msg, ms) {
    if (!toastEl) return;
    toastEl.textContent = msg;
    toastEl.classList.remove("hidden");
    if (toastTimer) window.clearTimeout(toastTimer);
    toastTimer = window.setTimeout(function () {
      toastEl.classList.add("hidden");
    }, ms || 4000);
  }

  function isModalOpen() {
    return modal && !modal.classList.contains("hidden");
  }

  function closeModal() {
    if (!modal) return;
    modal.classList.add("hidden");
    modal.setAttribute("aria-hidden", "true");
    activePanel = null;
    document.querySelectorAll(".rail-btn").forEach(function (b) {
      b.classList.toggle("active", b.getAttribute("data-panel") === "chat");
    });
    if (uploadsPollTimer) {
      window.clearInterval(uploadsPollTimer);
      uploadsPollTimer = null;
    }
  }

  function openModal(title, html) {
    if (!modal || !modalTitle || !modalBody) return;
    modalTitle.textContent = title;
    modalBody.innerHTML = html;
    modal.classList.remove("hidden");
    modal.setAttribute("aria-hidden", "false");
  }

  async function fetchJson(url, opts) {
    const res = await fetch(url, opts);
    const data = await res.json().catch(function () {
      return {};
    });
    if (!res.ok) {
      throw new Error(data.error || "request failed (" + res.status + ")");
    }
    return data;
  }

  function applyTheme(name) {
    document.body.setAttribute("data-theme", name);
    try {
      localStorage.setItem(THEME_KEY, name);
    } catch (_e) {}
  }

  function initTheme() {
    let saved = "dark";
    try {
      saved = localStorage.getItem(THEME_KEY) || "dark";
    } catch (_e) {}
    applyTheme(saved);
  }

  function initRail() {
    if (!rail || !railToggle) return;
    let expanded = false;
    try {
      expanded = localStorage.getItem(RAIL_KEY) === "1";
    } catch (_e) {}
    function setRailExpanded(expanded) {
      rail.classList.toggle("expanded", expanded);
      const path = railToggle.querySelector("svg path");
      if (path) {
        path.setAttribute("d", expanded ? "M15 6l-6 6 6 6" : "M9 6l6 6-6 6");
      }
      railToggle.setAttribute(
        "title",
        expanded ? "Collapse sidebar" : "Expand sidebar"
      );
      railToggle.setAttribute(
        "aria-label",
        expanded ? "Collapse sidebar" : "Expand sidebar"
      );
    }
    if (expanded) setRailExpanded(true);
    railToggle.addEventListener("click", function () {
      const now = !rail.classList.contains("expanded");
      setRailExpanded(now);
      try {
        localStorage.setItem(RAIL_KEY, now ? "1" : "0");
      } catch (_e) {}
    });
  }

  async function renderIdentity() {
    openModal("Identity", "<p class='hint'>Loading…</p>");
    try {
      const data = await fetchJson("/api/console/identity");
      openModal(
        "Identity",
        "<p class='hint'>Persona and RPG instructions for the agent. Changes hot-reload into the running session (no restart).</p>" +
          "<p class='hint'>Path: <code>" +
          escapeHtml(data.path) +
          "</code></p>" +
          "<div class='console-field'><label for='identity-editor'>Identity.md</label>" +
          "<textarea id='identity-editor'>" +
          escapeHtml(data.content) +
          "</textarea></div>" +
          "<div class='console-actions'>" +
          "<button type='button' class='console-btn' id='identity-save'>Save</button>" +
          "<button type='button' class='console-btn secondary' id='identity-discard'>Discard</button>" +
          "</div>"
      );
      document.getElementById("identity-save").addEventListener("click", async function () {
        const content = document.getElementById("identity-editor").value;
        try {
          await fetchJson("/api/console/identity", {
            method: "PUT",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ content: content }),
          });
          showToast("Identity saved — agent context reloads automatically.");
        } catch (e) {
          showToast(String(e.message || e));
        }
      });
      document.getElementById("identity-discard").addEventListener("click", closeModal);
    } catch (e) {
      modalBody.innerHTML =
        "<p class='console-warn'>" + escapeHtml(String(e.message || e)) + "</p>";
    }
  }

  function renderSettingsField(f, schema) {
    const id = "setting-" + f.key.replace(/\./g, "-");
    let input = "";
    if (!f.editable) {
      input =
        "<input type='text' id='" +
        id +
        "' disabled value='" +
        escapeHtml(String(f.value)) +
        "' />";
    } else if (typeof f.value === "boolean") {
      input =
        "<select id='" +
        id +
        "'><option value='true'" +
        (f.value ? " selected" : "") +
        ">true</option><option value='false'" +
        (!f.value ? " selected" : "") +
        ">false</option></select>";
    } else if (f.key === "num_ctx") {
      input =
        "<input type='number' id='" +
        id +
        "' min='1024' max='" +
        schema.num_ctx_max +
        "' value='" +
        escapeHtml(String(f.value)) +
        "' />";
      if (schema.num_ctx_warn_above) {
        input +=
          "<div class='console-warn' id='num-ctx-warn'>Values above " +
          schema.num_ctx_warn_above +
          " may cause VRAM OOM.</div>";
      }
    } else if (typeof f.value === "number") {
      input =
        "<input type='number' id='" +
        id +
        "' value='" +
        escapeHtml(String(f.value)) +
        "' step='any' />";
    } else {
      input =
        "<input type='text' id='" +
        id +
        "' value='" +
        escapeHtml(String(f.value)) +
        "' />";
    }
    return (
      "<div class='console-field' data-key='" +
      escapeHtml(f.key) +
      "' data-editable='" +
      (f.editable ? "1" : "0") +
      "'><label for='" +
      id +
      "'>" +
      escapeHtml(f.label) +
      "</label>" +
      input +
      "<div class='hint'>" +
      escapeHtml(f.description) +
      "</div>" +
      "<div class='impact'>" +
      escapeHtml(f.impact) +
      "</div></div>"
    );
  }

  async function renderSettings() {
    openModal("Settings", "<p class='hint'>Loading…</p>");
    try {
      const schema = await fetchJson("/api/console/settings");
      let html =
        "<div id='settings-restart-banner' class='console-banner hidden'>Saved. Restart Eris to apply changes.</div>";
      schema.fields.forEach(function (f) {
        html += renderSettingsField(f, schema);
      });
      html +=
        "<div class='console-actions'><button type='button' class='console-btn' id='settings-save'>Save</button></div>";
      openModal("Settings", html);

      document.getElementById("settings-save").addEventListener("click", async function () {
        const values = {};
        modalBody.querySelectorAll(".console-field[data-editable='1']").forEach(function (el) {
          const key = el.getAttribute("data-key");
          const input = el.querySelector("input, select");
          if (!input || !key) return;
          if (input.tagName === "SELECT") {
            values[key] = input.value === "true";
          } else if (input.type === "number") {
            values[key] = Number(input.value);
          } else {
            values[key] = input.value;
          }
        });
        try {
          await fetchJson("/api/console/settings", {
            method: "PUT",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ values: values }),
          });
          const banner = document.getElementById("settings-restart-banner");
          if (banner) banner.classList.remove("hidden");
          showToast("Settings saved — restart required.");
        } catch (e) {
          showToast(String(e.message || e));
        }
      });
    } catch (e) {
      modalBody.innerHTML =
        "<p class='console-warn'>" + escapeHtml(String(e.message || e)) + "</p>";
    }
  }

  async function renderSkills() {
    openModal("Skills", "<p class='hint'>Loading…</p>");
    try {
      const skills = await fetchJson("/api/console/skills");
      let html =
        "<p class='hint'>Topology skills — read-only. These guide mandatory agent workflows.</p><div class='chip-grid'>";
      if (!skills.length) {
        html += "<span class='hint'>No skills found under 10_Topology/skills/</span>";
      }
      skills.forEach(function (s) {
        const label = s.title || s.filename;
        html +=
          "<button type='button' class='skill-chip' data-file='" +
          escapeHtml(s.filename) +
          "'>" +
          escapeHtml(label) +
          "</button>";
      });
      html += "</div>";
      openModal("Skills", html);
      modalBody.querySelectorAll(".skill-chip").forEach(function (btn) {
        btn.addEventListener("click", function () {
          openSkillDetail(btn.getAttribute("data-file"));
        });
      });
    } catch (e) {
      modalBody.innerHTML =
        "<p class='console-warn'>" + escapeHtml(String(e.message || e)) + "</p>";
    }
  }

  async function openSkillDetail(filename) {
    try {
      const data = await fetchJson(
        "/api/console/skills/" + encodeURIComponent(filename)
      );
      const s = data.summary;
      openModal(
        s.title || s.filename,
        "<div class='note-meta'>" +
          (s.priority ? "Priority: " + escapeHtml(s.priority) + " · " : "") +
          (s.triggers ? "Triggers: " + escapeHtml(s.triggers) : "") +
          "</div><div class='note-body'>" +
          escapeHtml(data.body) +
          "</div>" +
          "<div class='console-actions'><button type='button' class='console-btn secondary' id='skill-back'>Back to list</button></div>"
      );
      document.getElementById("skill-back").addEventListener("click", renderSkills);
    } catch (e) {
      showToast(String(e.message || e));
    }
  }

  function renderMemoryChips() {
    const filtered = activeTagFilter
      ? memoryCards.filter(function (c) {
          return c.tags && c.tags.indexOf(activeTagFilter) >= 0;
        })
      : memoryCards;
    let html = "<div class='tag-filter-bar'><button type='button' class='tag-filter" +
      (activeTagFilter ? "" : " active") +
      "' data-tag=''>All</button>";
    const tags = {};
    memoryCards.forEach(function (c) {
      (c.tags || []).forEach(function (t) {
        tags[t] = true;
      });
    });
    Object.keys(tags)
      .sort()
      .forEach(function (t) {
        html +=
          "<button type='button' class='tag-filter" +
          (activeTagFilter === t ? " active" : "") +
          "' data-tag='" +
          escapeHtml(t) +
          "'>" +
          escapeHtml(t) +
          "</button>";
      });
    html += "</div><div class='chip-grid'>";
    if (!filtered.length) {
      html += "<span class='hint'>No synthesis notes yet.</span>";
    }
    filtered.forEach(function (c) {
      html +=
        "<button type='button' class='memory-chip' data-path='" +
        escapeHtml(c.head_path) +
        "'>" +
        escapeHtml(c.title) +
        "</button>";
    });
    html += "</div>";
    modalBody.innerHTML =
      "<p class='hint'>Synthesis memory — titles only (UUID folders hidden). Read-only.</p>" +
      html;
    modalBody.querySelectorAll(".tag-filter").forEach(function (btn) {
      btn.addEventListener("click", function () {
        activeTagFilter = btn.getAttribute("data-tag") || null;
        renderMemoryChips();
      });
    });
    modalBody.querySelectorAll(".memory-chip").forEach(function (btn) {
      btn.addEventListener("click", function () {
        openMemoryNote(btn.getAttribute("data-path"));
      });
    });
  }

  async function renderMemory() {
    openModal("Memory", "<p class='hint'>Loading…</p>");
    try {
      const data = await fetchJson("/api/console/memory");
      memoryCards = data.cards || [];
      activeTagFilter = null;
      openModal("Memory", "");
      renderMemoryChips();
    } catch (e) {
      modalBody.innerHTML =
        "<p class='console-warn'>" + escapeHtml(String(e.message || e)) + "</p>";
    }
  }

  async function openMemoryNote(path) {
    try {
      const data = await fetchJson(
        "/api/console/memory/note?path=" + encodeURIComponent(path)
      );
      const tags = (data.tags || []).join(", ");
      openModal(
        data.title || "Note",
        "<div class='note-meta'>" +
          (tags ? "Tags: " + escapeHtml(tags) + " · " : "") +
          (data.epistemic_status
            ? "Status: " + escapeHtml(data.epistemic_status) + " · "
            : "") +
          "<code>" +
          escapeHtml(data.path) +
          "</code></div>" +
          "<div class='note-body'>" +
          escapeHtml(data.body) +
          "</div>" +
          "<div class='console-actions'><button type='button' class='console-btn secondary' id='memory-back'>Back</button></div>"
      );
      document.getElementById("memory-back").addEventListener("click", renderMemory);
    } catch (e) {
      showToast(String(e.message || e));
    }
  }

  function attachImageToChat(entry) {
    if (!entry.preview_url || !window.erisAttach) {
      showToast("Vision attach unavailable.");
      return;
    }
    window.erisAttach.setPendingAttachment({
      relative_path: entry.relative_path,
      preview_url: entry.preview_url,
      width: 0,
      height: 0,
    });
    closeModal();
    showToast("Image attached to next message.");
  }

  async function renderUploads() {
    openModal("Uploads", "<p class='hint'>Loading…</p>");
    try {
      const data = await fetchJson("/api/console/uploads");
      let html = "";

      html += "<div class='upload-section'><h3>Images</h3><div class='upload-grid'>";
      if (!data.images.length) {
        html += "<span class='hint'>No images</span>";
      }
      data.images.forEach(function (img) {
        html +=
          "<img class='upload-thumb' src='" +
          escapeHtml(img.preview_url) +
          "' alt='" +
          escapeHtml(img.filename) +
          "' data-path='" +
          escapeHtml(img.relative_path) +
          "' data-preview='" +
          escapeHtml(img.preview_url) +
          "' title='Click to attach' />";
      });
      html += "</div></div>";

      html += "<div class='upload-section'><h3>Audio</h3>";
      if (!data.audio.length) {
        html += "<span class='hint'>No audio clips</span>";
      }
      data.audio.forEach(function (a) {
        html +=
          "<div style='margin-bottom:0.35rem'><audio controls src='" +
          escapeHtml(a.preview_url) +
          "'></audio> <span class='hint'>" +
          escapeHtml(a.filename) +
          "</span></div>";
      });
      html += "</div>";

      html +=
        "<div class='upload-section'><h3>Files</h3>" +
        "<div class='drop-zone' id='file-drop-zone'>Drop PDF or Markdown here</div>" +
        "<input type='file' id='file-drop-input' accept='.pdf,.md,.markdown' hidden />";
      if (!data.files.length) {
        html += "<p class='hint'>No files uploaded yet</p>";
      } else {
        html += "<ul style='margin-top:0.5rem;padding-left:1.2rem;font-size:12px'>";
        data.files.forEach(function (f) {
          html +=
            "<li>" +
            escapeHtml(f.filename) +
            " (" +
            Math.round(f.size_bytes / 1024) +
            " KB)</li>";
        });
        html += "</ul>";
      }
      html += "</div>";

      openModal("Uploads", html);

      modalBody.querySelectorAll(".upload-thumb").forEach(function (img) {
        img.addEventListener("click", function () {
          attachImageToChat({
            relative_path: img.getAttribute("data-path"),
            preview_url: img.getAttribute("data-preview"),
          });
        });
      });

      const dropZone = document.getElementById("file-drop-zone");
      const fileInput = document.getElementById("file-drop-input");
      if (dropZone && fileInput) {
        dropZone.addEventListener("click", function () {
          fileInput.click();
        });
        ["dragenter", "dragover"].forEach(function (ev) {
          dropZone.addEventListener(ev, function (e) {
            e.preventDefault();
            dropZone.classList.add("active");
          });
        });
        dropZone.addEventListener("dragleave", function () {
          dropZone.classList.remove("active");
        });
        dropZone.addEventListener("drop", async function (e) {
          e.preventDefault();
          dropZone.classList.remove("active");
          const file = e.dataTransfer && e.dataTransfer.files && e.dataTransfer.files[0];
          if (file) await uploadFile(file);
        });
        fileInput.addEventListener("change", async function () {
          const file = fileInput.files && fileInput.files[0];
          if (file) await uploadFile(file);
          fileInput.value = "";
        });
      }

      if (uploadsPollTimer) window.clearInterval(uploadsPollTimer);
      uploadsPollTimer = window.setInterval(function () {
        if (isModalOpen() && activePanel === "uploads") {
          renderUploads();
        }
      }, 5000);
    } catch (e) {
      modalBody.innerHTML =
        "<p class='console-warn'>" + escapeHtml(String(e.message || e)) + "</p>";
    }
  }

  async function uploadFile(file) {
    const fd = new FormData();
    fd.append("file", file);
    try {
      await fetchJson("/api/console/uploads/files", { method: "POST", body: fd });
      showToast("File uploaded.");
      renderUploads();
    } catch (e) {
      showToast(String(e.message || e));
    }
  }

  function renderTheme() {
    let current = "dark";
    try {
      current = localStorage.getItem(THEME_KEY) || "dark";
    } catch (_e) {}
    const themes = [
      { id: "dark", bg: "#080a12", accent: "#5ce5be" },
      { id: "light", bg: "#eef1f8", accent: "#0d7a62" },
      { id: "warm", bg: "#14100c", accent: "#e8b86d" },
    ];
    let html = "<p class='hint'>Appearance preset (saved in this browser).</p><div class='theme-picker'>";
    themes.forEach(function (t) {
      html +=
        "<button type='button' class='theme-swatch" +
        (current === t.id ? " selected" : "") +
        "' data-theme='" +
        t.id +
        "' style='background:linear-gradient(135deg," +
        t.bg +
        " 60%," +
        t.accent +
        " 60%)' title='" +
        t.id +
        "'></button>";
    });
    html += "</div>";
    openModal("Theme", html);
    modalBody.querySelectorAll(".theme-swatch").forEach(function (btn) {
      btn.addEventListener("click", function () {
        applyTheme(btn.getAttribute("data-theme"));
        renderTheme();
      });
    });
  }

  function openPanel(name) {
    if (name === "chat") {
      closeModal();
      return;
    }
    activePanel = name;
    document.querySelectorAll(".rail-btn").forEach(function (b) {
      b.classList.toggle("active", b.getAttribute("data-panel") === name);
    });
    if (name === "identity") renderIdentity();
    else if (name === "settings") renderSettings();
    else if (name === "skills") renderSkills();
    else if (name === "memory") renderMemory();
    else if (name === "uploads") renderUploads();
    else if (name === "theme") renderTheme();
  }

  document.querySelectorAll(".rail-btn").forEach(function (btn) {
    btn.addEventListener("click", function () {
      openPanel(btn.getAttribute("data-panel"));
    });
  });

  if (modalClose) modalClose.addEventListener("click", closeModal);
  if (modal) {
    modal.addEventListener("click", function (e) {
      if (e.target === modal) closeModal();
    });
  }

  window.erisConsole = {
    isModalOpen: isModalOpen,
    closeModal: closeModal,
  };

  initTheme();
  initRail();
})();
