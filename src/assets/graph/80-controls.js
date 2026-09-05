// controls: wire the header checkboxes, buttons, search box and focus
// depth controls to view state; keep the focus control strip in sync.

  // --- header controls ---------------------------------------------
  const $ = (id) => document.getElementById(id);
  for (const cb of document.querySelectorAll("header input[data-kind]"))
    cb.addEventListener("change", () => {
      cb.checked ? state.kinds.add(cb.dataset.kind) : state.kinds.delete(cb.dataset.kind);
      rebuild(0.5);
    });
  $("expandAll").addEventListener("change", (e) => {
    if (e.target.checked) {
      for (const n of DATA.nodes) if (n.kind === "file" && childrenOf.has(n.id)) expanded.add(n.id);
    } else {
      expanded.clear();
    }
    rebuild(0.7);
  });
  const collapseBtn = $("collapseAll");
  if (collapseBtn)
    collapseBtn.addEventListener("click", () => {
      expanded.clear();
      focusId = null;
      const ea = $("expandAll");
      if (ea) ea.checked = false;
      rebuild(0.7);
    });
  const search = $("search");
  search.addEventListener("input", () => {
    state.query = search.value.trim().toLowerCase();
    if (state.isolate) rebuild(0.4);
    else scheduleDraw();
  });
  const isolate = $("isolate");
  if (isolate)
    isolate.addEventListener("change", () => {
      state.isolate = isolate.checked;
      rebuild(0.4);
      if (state.isolate && state.query) fit(true);
    });
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
