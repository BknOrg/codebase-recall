// interaction: zoom/pan, canvas hit-testing, hover + node drag + click,
// and the fit / centre camera moves.

  // --- zoom -----------------------------------------------------------
  let transform = d3.zoomIdentity;
  const zoom = d3
    .zoom()
    .scaleExtent([0.02, 6])
    .on("zoom", (ev) => {
      transform = ev.transform;
      if (applyingRegimeFit) {
        scheduleDraw();
        return;
      }
      const fileReg = inFileRegime
        ? transform.k >= DIR_IN // stay in file view until you zoom well out
        : transform.k > DIR_OUT; // stay in dir view until you zoom well in
      if (fileReg !== inFileRegime) {
        inFileRegime = fileReg;
        pendingRegimeFit = true;
        rebuild(0.35);
      } else {
        scheduleDraw();
      }
    })
    .on("end", persist);
  d3.select(canvas).call(zoom).on("dblclick.zoom", null);

  // --- hit testing --------------------------------------------------
  function quadtree() {
    if (!quad) quad = d3.quadtree().x((d) => d.x).y((d) => d.y).addAll(nodes);
    return quad;
  }
  function nodeAt(clientX, clientY) {
    if (!nodes.length) return null;
    const r = canvas.getBoundingClientRect();
    const wx = (clientX - r.left - transform.x) / transform.k;
    const wy = (clientY - r.top - transform.y) / transform.k;
    const searchR = Math.max(16, 24 / transform.k);
    const found = quadtree().find(wx, wy, searchR);
    if (!found) return null;
    const rr = Math.max(nodeRadius(found) + 6 / transform.k, 10 / transform.k);
    return Math.hypot(found.x - wx, found.y - wy) <= rr ? found : null;
  }

  // --- pointer: hover + drag + click ------------------------------
  d3.select(canvas)
    .on("pointermove", (ev) => {
      if (isDragging) return;
      const n = nodeAt(ev.clientX, ev.clientY);
      const id = n ? n.id : null;
      if (id !== hovered) {
        hovered = id;
        scheduleDraw();
      }
      if (n) {
        canvas.style.cursor = "pointer";
        showTooltip(n);
        updateTooltipPos(ev);
      } else {
        canvas.style.cursor = "";
        hideTooltip();
      }
    })
    .on("pointerleave", () => {
      hovered = null;
      hideTooltip();
      scheduleDraw();
    });

  canvas.addEventListener(
    "pointerdown",
    (ev) => {
      if (ev.button !== 0) return;
      const n = nodeAt(ev.clientX, ev.clientY);
      if (!n) {
        // A plain click on empty space (not the start of a pan) drops isolate
        // and any selection; a drag still pans as usual.
        const sx = ev.clientX;
        const sy = ev.clientY;
        const up = (e) => {
          canvas.removeEventListener("pointerup", up);
          if (Math.abs(e.clientX - sx) + Math.abs(e.clientY - sy) <= 3) onEmptyClick();
        };
        canvas.addEventListener("pointerup", up);
        return;
      }
      ev.stopImmediatePropagation();
      ev.preventDefault();
      try {
        canvas.setPointerCapture(ev.pointerId);
      } catch (e) {}
      isDragging = true;
      hideTooltip();
      let moved = false;
      sim.alphaTarget(0.2).restart();
      n.fx = n.x;
      n.fy = n.y;
      const move = (e) => {
        const r = canvas.getBoundingClientRect();
        n.fx = (e.clientX - r.left - transform.x) / transform.k;
        n.fy = (e.clientY - r.top - transform.y) / transform.k;
        if (Math.abs(e.clientX - ev.clientX) + Math.abs(e.clientY - ev.clientY) > 3) moved = true;
        scheduleDraw();
      };
      const up = (e) => {
        canvas.removeEventListener("pointermove", move);
        canvas.removeEventListener("pointerup", up);
        try {
          canvas.releasePointerCapture(ev.pointerId);
        } catch (er) {}
        sim.alphaTarget(0);
        n.fx = null;
        n.fy = null;
        isDragging = false;
        if (!moved) onNodeClick(n, e);
        persist();
      };
      canvas.addEventListener("pointermove", move);
      canvas.addEventListener("pointerup", up);
    },
    true
  );

  canvas.addEventListener("dblclick", (ev) => {
    if (!nodeAt(ev.clientX, ev.clientY)) fit(true);
  });

  function onEmptyClick() {
    if (state.isolate) {
      state.query = "";
      state.isolate = false;
      const s = document.getElementById("search");
      if (s) s.value = "";
      selected = null;
      rebuild(0.4);
      return;
    }
    if (selected) {
      selected = null;
      updatePanel();
      scheduleDraw();
    }
  }

  function onNodeClick(n, ev) {
    if (ev && (ev.altKey || ev.metaKey)) {
      focusId = focusId === n.id ? null : n.id;
      selected = n.id;
      rebuild(0.5);
      return;
    }
    if (n.kind === "dir") {
      // zoom past DIR_OUT so the view flips to files inside this directory
      const k = Math.max(DIR_OUT + 0.15, transform.k * 3);
      const tf = d3.zoomIdentity
        .translate(width / 2 - k * n.x, height / 2 - k * n.y)
        .scale(k);
      d3.select(canvas).transition().duration(450).call(zoom.transform, tf);
      return;
    }
    if (n.kind === "file" && childrenOf.has(n.id)) {
      const before = nodes.length;
      expanded.has(n.id) ? expanded.delete(n.id) : expanded.add(n.id);
      selected = n.id;
      rebuild(0.5);
      if (nodes.length < 8 && nodes.length < before) fit(true);
      return;
    }
    selected = selected === n.id ? null : n.id;
    updatePanel();
    scheduleDraw();
  }

  // --- fit / center -----------------------------------------------
  const fit = () => fitRegimeAware(true);
  // Fit, but keep the zoom on the correct side of DIR_ZOOM so a regime switch
  // doesn't immediately bounce back.
  function fitRegimeAware(animate) {
    if (!nodes.length) return;
    let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
    for (const n of nodes) {
      if (n.x == null) continue;
      x0 = Math.min(x0, n.x);
      y0 = Math.min(y0, n.y);
      x1 = Math.max(x1, n.x);
      y1 = Math.max(y1, n.y);
    }
    if (!isFinite(x0)) return;
    const pad = 90;
    let k = Math.max(
      0.02,
      Math.min((width - pad) / Math.max(x1 - x0, 1), (height - pad) / Math.max(y1 - y0, 1), 2.5)
    );
    k = inFileRegime ? Math.max(k, DIR_IN + 0.03) : Math.min(k, DIR_OUT - 0.05);
    const tf = d3.zoomIdentity
      .translate(width / 2 - (k * (x0 + x1)) / 2, height / 2 - (k * (y0 + y1)) / 2)
      .scale(k);
    applyingRegimeFit = true;
    const sel = d3.select(canvas);
    if (animate) sel.transition().duration(420).call(zoom.transform, tf);
    else sel.call(zoom.transform, tf);
    applyingRegimeFit = false;
    autoFitted = true;
  }
  function centerOn(id) {
    const d = simNodes.get(id);
    if (!d || d.x == null) return;
    const k = transform.k;
    const tf = d3.zoomIdentity.translate(width / 2 - k * d.x, height / 2 - k * d.y).scale(k);
    d3.select(canvas).transition().duration(350).call(zoom.transform, tf);
  }
