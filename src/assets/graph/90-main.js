// main: localStorage persistence, the start-up sequence, and the
// window.__graphDebug introspection hook.

  // --- persistence ---------------------------------------------
  const LSKEY = "code-rcl-graph:" + (DATA.root || "");
  let persistT = 0;
  function persist() {
    clearTimeout(persistT);
    persistT = setTimeout(() => {
      try {
        const pos = {};
        for (const [id, s] of simNodes) if (s.x != null) pos[id] = [Math.round(s.x), Math.round(s.y)];
        localStorage.setItem(
          LSKEY,
          JSON.stringify({
            expanded: [...expanded],
            focusId,
            focusDepth,
            pos,
            tf: [transform.x, transform.y, transform.k],
          })
        );
      } catch (e) {}
    }, 400);
  }
  function restore() {
    try {
      const raw = localStorage.getItem(LSKEY);
      if (!raw) return false;
      const st = JSON.parse(raw);
      if (Array.isArray(st.expanded))
        for (const id of st.expanded) if (nodeById.has(id)) expanded.add(id);
      if (st.focusId && nodeById.has(st.focusId)) focusId = st.focusId;
      if (st.focusDepth) focusDepth = st.focusDepth;
      if (st.pos)
        for (const id in st.pos)
          if (nodeById.has(id)) {
            const s = asSimNode(nodeById.get(id));
            s.x = st.pos[id][0];
            s.y = st.pos[id][1];
          }
      if (Array.isArray(st.tf)) {
        transform = d3.zoomIdentity.translate(st.tf[0], st.tf[1]).scale(st.tf[2]);
        inFileRegime = transform.k >= DIR_IN;
      }
      return true;
    } catch (e) {
      return false;
    }
  }

  // --- start ----------------------------------------------------
  resize();
  // For a very large repo, open zoomed out on the directory roll-up.
  const fileCount = DATA.nodes.reduce((a, n) => a + (n.kind === "file" ? 1 : 0), 0);
  inFileRegime = fileCount <= 400;
  const restored = restore();
  warmedUp = restored;
  autoFitted = restored;
  d3.select(canvas).call(zoom.transform, transform);
  rebuild(restored ? 0.3 : 0.9);

  window.__graphDebug = () => {
    const hist = {};
    for (const n of nodes) hist[n.kind] = (hist[n.kind] || 0) + 1;
    return {
      regime: inFileRegime ? "file" : "dir",
      nodes: nodes.length,
      links: links.length,
      expanded: expanded.size,
      focusId,
      selected,
      hist,
      transform: [transform.x, transform.y, transform.k],
      screenPos: (id) => {
        const d = simNodes.get(id);
        return d && d.x != null
          ? [d.x * transform.k + transform.x, d.y * transform.k + transform.y]
          : null;
      },
    };
  };
