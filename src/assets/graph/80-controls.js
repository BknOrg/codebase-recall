// controls: wire the header checkboxes, buttons, search box and focus
// depth controls to view state; keep the focus control strip in sync.

  // --- header controls ---------------------------------------------
  const $ = (id) => document.getElementById(id);

  // Grey out an edge-kind toggle when the graph carries no such edge (e.g.
  // `references` unless the graph was built with `--kinds references`).
  const edgeKinds = new Set(DATA.edges.map((e) => e.kind));
  for (const cb of document.querySelectorAll("#filtersView input[data-kind]")) {
    const kind = cb.dataset.kind;
    if (!edgeKinds.has(kind)) {
      cb.disabled = true;
      cb.checked = false;
      state.kinds.delete(kind);
      const label = cb.closest("label");
      if (label) {
        label.title = 'no "' + kind + '" edges in this graph — re-render with --kinds ' + kind;
        label.style.opacity = 0.5;
      }
      continue;
    }
    cb.addEventListener("change", () => {
      cb.checked ? state.kinds.add(kind) : state.kinds.delete(kind);
      // `calls` edges only surface once a file is opened (see buildLinks).
      if (kind === "calls" && cb.checked && expanded.size === 0)
        showStatus("expand a file to see call edges");
      rebuild(0.5);
    });
  }

  // The "expand all" checkbox is the only expand/collapse control: checking it
  // opens every file, unchecking it collapses everything (and drops any focus).
  $("expandAll").addEventListener("change", (e) => {
    if (e.target.checked) {
      for (const n of DATA.nodes) if (n.kind === "file" && childrenOf.has(n.id)) expanded.add(n.id);
    } else {
      expanded.clear();
      focusId = null;
    }
    rebuild(0.7);
    fitRegimeAware(true);
  });

  // "Color by subsystem" recolours nodes by detected community; it is only offered when the
  // graph carries communities (files with no cross-file links produce none).
  const colorToggle = $("colorByCommunity");
  if (colorToggle) {
    if (COMMUNITIES.length === 0) {
      colorToggle.disabled = true;
      const label = colorToggle.closest("label");
      if (label) {
        label.title = "no subsystems detected: files have no cross-file imports or calls";
        label.style.opacity = 0.5;
      }
    } else {
      colorToggle.addEventListener("change", () => {
        state.colorMode = colorToggle.checked ? "community" : "kind";
        buildLegend();
        scheduleDraw();
        persist();
      });
    }
  }

  // --- search doubles as isolate ---------------------------------
  // A non-empty query keeps only matching nodes + their 1-hop neighbours
  // (see rebuild); clearing it brings the whole graph back.
  const search = $("search");
  let searchT = 0;
  search.addEventListener("input", () => {
    state.query = search.value.trim().toLowerCase();
    state.isolate = state.query.length > 0;
    clearTimeout(searchT);
    searchT = setTimeout(() => {
      rebuild(0.4);
      if (state.isolate) fitRegimeAware(true);
    }, 200);
  });

  // --- context bundle ("code-rcl dump -r") for the selected node ----------
  // Triggered from the side panel's "generate" button (only shown while
  // isolating). Writes its result into #spDumpResult in that panel.
  function runDumpBundle() {
    const resultEl = $("spDumpResult");
    const say = (msg, cls) => {
      if (resultEl) resultEl.className = "sp-dump-result " + (cls || "");
      if (resultEl) resultEl.textContent = msg;
    };
    if (!selected || !nodeById.has(selected)) {
      say("select a node first", "err");
      return;
    }
    const n = nodeById.get(selected);
    const target =
      n.kind === "file" ? n.path || n.label : isSymId(selected) ? n.label : n.path || n.label;
    const depth = state.dumpDepth || 2;
    const name = (state.dumpName || "").trim() || "codebase-context.md";

    if (location.protocol.startsWith("http")) {
      say("generating…", "");
      fetch(
        "/dump?target=" + encodeURIComponent(target) + "&depth=" + depth + "&name=" + encodeURIComponent(name),
        { method: "POST" }
      )
        .then((r) => r.json())
        .then((d) => {
          if (d && d.ok) {
            say("✓ wrote " + d.files + " files → " + d.path, "ok");
            showStatus("context bundle written → " + d.path, 8000);
          } else {
            say("✗ " + ((d && d.error) || "failed"), "err");
          }
        })
        .catch((e) => say("✗ " + e, "err"));
    } else {
      const cmd = 'code-rcl dump -r "' + target + '" --depth ' + depth + ' -f "' + name + '"';
      const ok = () => say("✓ command copied — paste in a terminal", "ok");
      if (navigator.clipboard && navigator.clipboard.writeText)
        navigator.clipboard.writeText(cmd).then(ok, () => say(cmd, ""));
      else say(cmd, "");
    }
  }

  $("fitBtn").addEventListener("click", () => fitRegimeAware());
  const clearBtn = $("focusClear");
  if (clearBtn)
    clearBtn.addEventListener("click", () => {
      focusId = null;
      rebuild(0.4);
    });
  const deeper = $("focusDeeper");
  const shallower = $("focusShallower");
  if (deeper)
    deeper.addEventListener("click", () => {
      focusDepth = Math.min(5, focusDepth + 1);
      rebuild(0.4);
    });
  if (shallower)
    shallower.addEventListener("click", () => {
      focusDepth = Math.max(1, focusDepth - 1);
      rebuild(0.4);
    });

  function updateHeader() {
    const ctl = $("focusCtl");
    if (!ctl) return;
    if (focusId && nodeById.has(focusId)) {
      ctl.hidden = false;
      $("focusLabel").textContent = "focus: " + nodeById.get(focusId).label;
      $("focusDepth").textContent = String(focusDepth);
    } else {
      ctl.hidden = true;
    }
  }

  // --- left panel toggle & icon-rail panels -------------------------
  const toggleLeftBtn = $("toggleLeftPane");
  const leftPane = $("leftPane");
  if (toggleLeftBtn && leftPane) {
    toggleLeftBtn.addEventListener("click", () => {
      leftPane.classList.toggle("collapsed");
      resize();
    });
  }

  const railBtns = document.querySelectorAll(".rail-btn");
  const treeView = $("treeView");
  const codeView = $("codeView");
  const filtersView = $("filtersView");
  const panels = { tree: treeView, code: codeView, filters: filtersView };
  function switchTab(panel) {
    railBtns.forEach((b) => b.classList.toggle("active", b.dataset.panel === panel));
    state.leftTab = panel;
    for (const [key, el] of Object.entries(panels)) {
      if (el) el.hidden = key !== panel;
    }
  }
  railBtns.forEach((btn) => {
    btn.addEventListener("click", () => switchTab(btn.dataset.panel));
  });

  // --- project name (shown in the sidebar header / floating pill) --
  const rootPath = (DATA.root || "").replace(/[\\/]+$/, "");
  const projectName = rootPath.split(/[\\/]/).filter(Boolean).pop() || "codebase";
  const projectNameEl = $("projectName");
  if (projectNameEl) projectNameEl.textContent = projectName;

  // --- code viewer & source fetching -------------------------------
  const fileCache = new Map();

  function openInCodeViewer(filePath, targetLine, range, highlightKind) {
    const codeBarEl = $("codeBar");
    const codePathEl = $("codePath");
    const codeLineEl = $("codeLine");
    const codeContentEl = $("codeContent");
    if (!codePathEl || !codeContentEl) return;

    switchTab("code");
    if (codeBarEl) codeBarEl.hidden = false;
    codePathEl.textContent = filePath;
    codeLineEl.textContent = targetLine ? "L" + targetLine : "";

    state.activeFile = filePath;
    state.activeLine = targetLine;
    state.activeRange = range;

    const renderLines = (content) => {
      const lines = content.split("\n");
      let html = "";
      for (let i = 0; i < lines.length; i++) {
        const lineNum = i + 1;
        let hlCls = "";
        if (range && lineNum >= range[0] && lineNum <= range[1]) {
          hlCls = highlightKind === "call" ? " highlight-call" : " highlight-def";
        } else if (lineNum === targetLine) {
          hlCls = highlightKind === "call" ? " highlight-call" : " highlight-def";
        }
        html += `<div class="code-row${hlCls}" id="cline-${lineNum}" data-line="${lineNum}"><span class="code-num">${lineNum}</span><span class="code-text">${esc(lines[i])}</span></div>`;
      }
      codeContentEl.innerHTML = html;

      if (targetLine) {
        requestAnimationFrame(() => {
          const row = document.getElementById("cline-" + targetLine);
          if (row) {
            row.scrollIntoView({ block: "center", behavior: "smooth" });
          }
        });
      }
    };

    if (fileCache.has(filePath)) {
      renderLines(fileCache.get(filePath));
      return;
    }

    if (location.protocol.startsWith("http")) {
      codeContentEl.innerHTML = `<div style="padding: 10px; color: var(--muted); font-size: 11px;">loading ${esc(filePath)}…</div>`;
      fetch("/source?path=" + encodeURIComponent(filePath))
        .then((r) => r.json())
        .then((d) => {
          if (d && d.ok && d.content != null) {
            fileCache.set(filePath, d.content);
            renderLines(d.content);
          } else {
            codeContentEl.innerHTML = `<div style="padding: 10px; color: var(--accent); font-size: 11px;">error: ${esc((d && d.error) || "cannot load source")}</div>`;
          }
        })
        .catch((err) => {
          codeContentEl.innerHTML = `<div style="padding: 10px; color: var(--accent); font-size: 11px;">fetch error: ${esc(err.message)}</div>`;
        });
    } else {
      codeContentEl.innerHTML = `<div style="padding: 10px; color: var(--muted); font-size: 11px;">source view requires <code>code-rcl serve</code></div>`;
    }
  }

  // --- expandable hierarchical file tree ---------------------------
  // Same path data as src/assets/icons/folder.svg & code.svg (rail buttons) —
  // duplicated inline here since these are small, static, rarely-changing
  // icons and not worth a Rust->JS asset-sharing path for just two glyphs.
  const ICON_FOLDER_SVG =
    '<svg class="tree-icon" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg"><path fill-rule="evenodd" clip-rule="evenodd" d="M4 4C3.73478 4 3.48043 4.10536 3.29289 4.29289C3.10536 4.48043 3 4.73478 3 5V19C3 19.2652 3.10536 19.5196 3.29289 19.7071C3.48043 19.8946 3.73478 20 4 20H20C20.2652 20 20.5196 19.8946 20.7071 19.7071C20.8946 19.5196 21 19.2652 21 19V8C21 7.73478 20.8946 7.48043 20.7071 7.29289C20.5196 7.10536 20.2652 7 20 7H11.5352C10.8665 7 10.242 6.6658 9.87108 6.1094L8.46482 4H4ZM1.87868 2.87868C2.44129 2.31607 3.20435 2 4 2H8.46482C9.13352 2 9.75799 2.3342 10.1289 2.8906L11.5352 5H20C20.7957 5 21.5587 5.31607 22.1213 5.87868C22.6839 6.44129 23 7.20435 23 8V19C23 19.7957 22.6839 20.5587 22.1213 21.1213C21.5587 21.6839 20.7957 22 20 22H4C3.20435 22 2.44129 21.6839 1.87868 21.1213C1.31607 20.5587 1 19.7957 1 19V5C1 4.20435 1.31607 3.44129 1.87868 2.87868Z" fill="currentColor"/></svg>';
  const ICON_FILE_SVG =
    '<svg class="tree-icon" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg"><path d="M7 8L3 11.6923L7 16M17 8L21 11.6923L17 16M14 4L10 20" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>';

  function buildFileTree() {
    const treeList = $("treeList");
    if (!treeList) return;
    const fileNodes = DATA.nodes.filter((n) => n.kind === "file" && (n.path || n.label));

    // Build directory hierarchy
    const root = { name: "", children: new Map(), files: [] };
    for (const n of fileNodes) {
      const fullPath = (n.path || n.label).replace(/\\/g, "/");
      const parts = fullPath.split("/");
      const fileName = parts.pop();
      let curr = root;
      for (const part of parts) {
        if (!curr.children.has(part)) {
          curr.children.set(part, { name: part, children: new Map(), files: [] });
        }
        curr = curr.children.get(part);
      }
      curr.files.push({ node: n, fileName, fullPath });
    }

    function renderDir(dirNode) {
      let html = "";
      const sortedDirs = [...dirNode.children.keys()].sort();
      for (const dirName of sortedDirs) {
        const sub = dirNode.children.get(dirName);
        html += `<div class="tree-node tree-folder">` +
          `<div class="tree-row folder-row">` +
          `<span class="tree-caret">▾</span>` +
          ICON_FOLDER_SVG +
          `<span class="tree-name">${esc(dirName)}</span>` +
          `</div>` +
          `<div class="tree-children">` +
          renderDir(sub) +
          `</div>` +
          `</div>`;
      }
      dirNode.files.sort((a, b) => a.fileName.localeCompare(b.fileName));
      for (const f of dirNode.files) {
        html += `<div class="tree-node tree-file">` +
          `<div class="tree-row file-row" data-id="${esc(f.node.id)}" data-path="${esc(f.fullPath)}">` +
          `<span class="tree-caret"></span>` +
          ICON_FILE_SVG +
          `<span class="tree-name" title="${esc(f.fullPath)}">${esc(f.fileName)}</span>` +
          `</div>` +
          `</div>`;
      }
      return html;
    }

    treeList.innerHTML = renderDir(root);

    treeList.addEventListener("click", (ev) => {
      // Toggle folder expansion
      const folderRow = ev.target.closest(".folder-row");
      if (folderRow) {
        const folderNode = folderRow.closest(".tree-folder");
        const children = folderNode ? folderNode.querySelector(".tree-children") : null;
        const caret = folderRow.querySelector(".tree-caret");
        if (children && caret) {
          const isCollapsed = children.classList.toggle("collapsed");
          caret.textContent = isCollapsed ? "▸" : "▾";
        }
        return;
      }

      // File selection
      const fileRow = ev.target.closest(".file-row");
      if (!fileRow) return;

      document.querySelectorAll(".file-row").forEach((r) => r.classList.remove("active"));
      fileRow.classList.add("active");

      const id = fileRow.dataset.id;
      const path = fileRow.dataset.path;

      // Focus on file node and its relations in the graph
      if (id && nodeById.has(id)) {
        expanded.add(id);
        selected = id;
        rebuild(0.4);
        centerOn(id);
        updatePanel();
        scheduleDraw();
      }

      // If code viewer tab is active, open it
      openInCodeViewer(path, 1, [1, 1], "def");
    });
  }

  buildFileTree();
