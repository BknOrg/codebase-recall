// controls: wire the header checkboxes, buttons, search box and focus
// depth controls to view state; keep the focus control strip in sync.

  // --- header controls ---------------------------------------------
  const $ = (id) => document.getElementById(id);

  // Grey out an edge-kind toggle when the graph carries no such edge (e.g.
  // `references` unless the graph was built with `--kinds references`).
  const edgeKinds = new Set(DATA.edges.map((e) => e.kind));
  for (const cb of document.querySelectorAll("header input[data-kind]")) {
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
