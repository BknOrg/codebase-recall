// tooltip-panel: highlight-set computation, the hover tooltip, and the
// selection side panel (callers / callees / files).

  // --- highlight helpers ------------------------------------------------
  const hit = (d, q) =>
    !!q && (d.label.toLowerCase().includes(q) || (d.path || "").toLowerCase().includes(q));

  function nodeIsRendered(id) {
    return simNodes.has(id) && nodes.indexOf(simNodes.get(id)) !== -1;
  }
  function neighbours1(id) {
    const s = new Set([id]);
    for (const x of adj.get(id) || []) s.add(x);
    return s;
  }
  // ids to keep bright, or null = everything bright
  function highlightSet() {
    if (hovered) return neighbours1(hovered);
    if (selected && nodeIsRendered(selected)) return neighbours1(selected);
    if (state.query && !state.isolate) {
      const s = new Set();
      for (const n of nodes) if (hit(n, state.query)) s.add(n.id);
      return s.size ? s : null;
    }
    return null;
  }

  // --- tooltip ---------------------------------------------------
  function showTooltip(d) {
    if (isDragging) return;
    let html = `<div class="tt-title"><span>${esc(d.label)}</span><span class="tt-kind">${esc(
      d.kind
    )}</span></div>`;
    if (d.path) html += `<div class="tt-path">${esc(d.path)}</div>`;
    const meta = [];
    if (d.language) meta.push(d.language);
    if (d.lines && Array.isArray(d.lines)) meta.push(`L${d.lines[0]}-${d.lines[1]}`);
    if (d.degree != null) meta.push(`${d.degree} links`);
    if (d.exported) meta.push("exported");
    if (d.kind === "dir") meta.push("click to zoom in");
    else if (d.kind === "file" && childrenOf.has(d.id))
      meta.push(expanded.has(d.id) ? "click to collapse" : "click to expand");
    meta.push("⌥-click to focus");
    if (meta.length) html += `<div class="tt-meta">${esc(meta.join(" • "))}</div>`;
    tooltip.innerHTML = html;
    tooltip.style.display = "block";
  }
  function updateTooltipPos(ev) {
    const r = stage.getBoundingClientRect();
    let left = ev.clientX - r.left + 14;
    let top = ev.clientY - r.top + 14;
    if (left + 260 > r.width) left = ev.clientX - r.left - 260;
    if (top + 90 > r.height) top = ev.clientY - r.top - 80;
    tooltip.style.left = Math.max(8, left) + "px";
    tooltip.style.top = Math.max(8, top) + "px";
  }
  function hideTooltip() {
    tooltip.style.display = "none";
  }
  function esc(s) {
    return String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
  }

  // --- side panel ------------------------------------------------
  function labelFor(id) {
    const m = nodeById.get(id);
    return m ? m.label : id;
  }
  function updatePanel() {
    if (!selected || !nodeById.has(selected) || !nodeIsRendered(selected)) {
      panel.hidden = true;
      return;
    }
    const n = nodeById.get(selected);
    let html = `<button class="sp-close" title="close">×</button><h2>${esc(n.label)}</h2><div class="sp-kind">${esc(
      n.kind
    )}</div>`;
    const groups = [];

    if (isSymId(selected)) {
      const home = fileOf(selected);
      if (home) groups.push(["defined in", [home]]);
      groups.push(["uses → (files)", [...(symUses.get(selected) || [])]]);
      groups.push(["← used by (files)", [...(symUsedBy.get(selected) || [])]]);
    } else {
      const out = { calls: new Set(), imports: new Set(), references: new Set() };
      const inc = { calls: new Set(), imports: new Set(), references: new Set() };
      for (const l of links) {
        if (l.kind === "contains") continue;
        const sid = l.source.id != null ? l.source.id : l.source;
        const tid = l.target.id != null ? l.target.id : l.target;
        if (sid === selected && out[l.kind]) out[l.kind].add(tid);
        if (tid === selected && inc[l.kind]) inc[l.kind].add(sid);
      }
      groups.push(["calls →", [...out.calls]]);
      groups.push(["← called by", [...inc.calls]]);
      groups.push(["imports →", [...out.imports]]);
      groups.push(["← imported by", [...inc.imports]]);
      groups.push(["references →", [...out.references]]);
    }

    let any = false;
    for (const [title, ids] of groups) {
      if (!ids.length) continue;
      any = true;
      html += `<div class="sp-group">${esc(title)} (${ids.length})</div>`;
      for (const id of ids.slice(0, 60))
        html += `<button class="row" data-id="${esc(id)}">${esc(labelFor(id))}</button>`;
    }
    if (!any) html += `<div class="sp-group">no visible relations</div>`;

    // While isolating, offer a one-click "code-rcl dump -r" context bundle for
    // this node (values live in `state` so they survive panel re-renders).
    if (state.isolate) {
      html +=
        `<div class="sp-group">context bundle · code-rcl dump -r</div>` +
        `<div class="sp-dump">` +
        `<label>depth <input type="number" id="spDepth" min="1" max="5" value="${state.dumpDepth}"></label>` +
        `<label>file <input type="text" id="spOut" value="${esc(state.dumpName)}"></label>` +
        `<button class="sp-btn" id="spDumpGo">generate</button>` +
        `<div class="sp-dump-result" id="spDumpResult"></div>` +
        `</div>`;
    }

    panel.innerHTML = html;
    panel.hidden = false;
  }
  panel.addEventListener("input", (ev) => {
    if (ev.target.id === "spDepth")
      state.dumpDepth = Math.min(5, Math.max(1, parseInt(ev.target.value, 10) || 2));
    else if (ev.target.id === "spOut") state.dumpName = ev.target.value;
  });
  panel.addEventListener("click", (ev) => {
    if (ev.target.id === "spDumpGo") {
      runDumpBundle();
      return;
    }
    if (ev.target.classList.contains("sp-close")) {
      selected = null;
      updatePanel();
      scheduleDraw();
      return;
    }
    const row = ev.target.closest(".row");
    if (!row) return;
    const id = row.dataset.id;
    // make sure the target is on screen: open its file if needed
    if (isSymId(id)) {
      const f = fileOf(id);
      if (f && !expanded.has(f)) {
        expanded.add(f);
        selected = id;
        rebuild(0.4);
        centerOn(nodeIsRendered(id) ? id : f);
        return;
      }
    }
    selected = id;
    centerOn(id);
    updatePanel();
    scheduleDraw();
  });
