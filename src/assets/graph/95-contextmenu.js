// context menu: right-click a node/edge to run "dump -r" or "impact" and
// write the result next to that node/edge's own file (in `code-rcl serve`
// mode), or copy the equivalent CLI command (in static/offline HTML mode).

  const ctxMenu = document.createElement("div");
  ctxMenu.className = "ctx-menu";
  ctxMenu.hidden = true;
  stage.appendChild(ctxMenu);

  function hideCtxMenu() {
    ctxMenu.hidden = true;
  }

  function dirname(p) {
    if (!p) return "";
    const idx = Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\"));
    return idx === -1 ? "" : p.slice(0, idx);
  }

  // Same target derivation as the side panel's "dump -r" button
  // (runDumpBundle in 80-controls.js), plus the owning file's directory via
  // the fileOf()/nodeById helpers already built in 00-data.js.
  function resolveTargetAndDir(id) {
    const n = nodeById.get(id);
    if (!n) return { target: null, dir: "" };
    const target = n.kind === "file" ? n.path || n.label : isSymId(id) ? n.label : n.path || n.label;
    const fileId = fileOf(id);
    const fileNode = fileId ? nodeById.get(fileId) : null;
    const filePath = fileNode ? fileNode.path || fileId.replace("file:", "") : null;
    return { target, dir: dirname(filePath) };
  }

  function runCtxAction(kind, target, dir, format) {
    hideCtxMenu();
    const depth = state.dumpDepth || 2;

    if (location.protocol.startsWith("http")) {
      showStatus(kind === "dump" ? "generating dump…" : "running impact…");
      const params = new URLSearchParams({ target, depth: String(depth), dir });
      if (kind === "impact") params.set("format", format);
      fetch((kind === "dump" ? "/dump" : "/impact") + "?" + params.toString(), { method: "POST" })
        .then((r) => r.json())
        .then((d) => {
          if (d && d.ok) showStatus("✓ wrote → " + d.path, 8000);
          else showStatus("✗ " + ((d && d.error) || "failed"), 5000);
        })
        .catch((e) => showStatus("✗ " + e, 5000));
      return;
    }

    // Static HTML / offline mode: no server to write a file, so hand back
    // the equivalent CLI command (same fallback the side panel dump button
    // already uses).
    const cmd =
      kind === "dump"
        ? 'code-rcl dump -r "' + target + '" --depth ' + depth
        : 'code-rcl impact "' + target + '" --depth ' + depth + (format === "json" ? " --json" : "");
    const ok = () => showStatus("✓ command copied — paste in a terminal", 4000);
    if (navigator.clipboard && navigator.clipboard.writeText) navigator.clipboard.writeText(cmd).then(ok, () => showStatus(cmd, 6000));
    else showStatus(cmd, 6000);
  }

  function showCtxMenu(id, clientX, clientY) {
    const { target, dir } = resolveTargetAndDir(id);
    if (!target) {
      hideCtxMenu();
      return;
    }

    ctxMenu.innerHTML =
      `<div class="ctx-menu-title" title="${esc(target)}">${esc(target)}</div>` +
      `<button type="button" data-action="dump">Dump context (-r)</button>` +
      `<div class="ctx-menu-label">Impact analysis</div>` +
      `<button type="button" data-action="impact-ascii">as text</button>` +
      `<button type="button" data-action="impact-json">as JSON</button>`;

    ctxMenu.hidden = false;
    const r = stage.getBoundingClientRect();
    let left = clientX - r.left;
    let top = clientY - r.top;
    left = Math.max(0, Math.min(left, r.width - ctxMenu.offsetWidth - 4));
    top = Math.max(0, Math.min(top, r.height - ctxMenu.offsetHeight - 4));
    ctxMenu.style.left = left + "px";
    ctxMenu.style.top = top + "px";

    ctxMenu.querySelectorAll("button[data-action]").forEach((btn) => {
      btn.addEventListener("click", () => {
        const action = btn.dataset.action;
        if (action === "dump") runCtxAction("dump", target, dir);
        else runCtxAction("impact", target, dir, action === "impact-json" ? "json" : "ascii");
      });
    });
  }

  canvas.addEventListener("contextmenu", (ev) => {
    ev.preventDefault();
    const n = nodeAt(ev.clientX, ev.clientY);
    if (n) {
      showCtxMenu(n.id, ev.clientX, ev.clientY);
      return;
    }
    const edge = edgeAt(ev.clientX, ev.clientY);
    if (edge) {
      const sid = edge.source.id || edge.source;
      showCtxMenu(sid, ev.clientX, ev.clientY);
      return;
    }
    hideCtxMenu();
  });

  document.addEventListener("pointerdown", (ev) => {
    if (!ctxMenu.hidden && !ctxMenu.contains(ev.target)) hideCtxMenu();
  });
  document.addEventListener("keydown", (ev) => {
    if (ev.key === "Escape") hideCtxMenu();
  });
