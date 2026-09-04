/* code-ctx graph view — d3-force layout with incremental data-joins.
   Expects a global `d3` (v7) and a <script id="graph-data"> JSON blob. */
(function () {
  "use strict";
  const DATA = JSON.parse(document.getElementById("graph-data").textContent);

  const KIND_COLOR = (k) => ({
    file: "--file", external: "--ext", variable: "--var",
    struct: "--type", enum: "--type", trait: "--type", interface: "--type",
    type: "--type", class: "--type",
  }[k] || "--func");
  const cssVar = (v) =>
    getComputedStyle(document.documentElement).getPropertyValue(v).trim();
  const radius = (n) => (n.kind === "file" ? 7 : 5);

  // --- indexes ----------------------------------------------------------------
  const fileOf = new Map(); // symbol id -> owning file id
  for (const n of DATA.nodes)
    if (n.kind !== "file" && n.kind !== "external" && n.path)
      fileOf.set(n.id, "file:" + n.path);

  const expanded = new Set();
  const state = { kinds: new Set(["imports", "calls", "references"]), query: "" };
  let pressed = null; // id of the node currently held down, or null

  // Physics objects are kept across rebuilds so positions & velocity survive
  // a filter toggle or a file expand (no teardown, no re-scatter).
  const simNodes = new Map();
  const asSimNode = (n) => {
    let s = simNodes.get(n.id);
    if (!s) { s = Object.assign({}, n); simNodes.set(n.id, s); }
    return s;
  };

  // --- visibility -----------------------------------------------------------
  function visibleNodes() {
    return DATA.nodes
      .filter((n) => {
        if (n.kind === "file" || n.kind === "external") return true;
        const f = fileOf.get(n.id);
        return f ? expanded.has(f) : true;
      })
      .map(asSimNode);
  }
  function endpoint(id, vset) {
    if (vset.has(id)) return id;
    const f = fileOf.get(id); // collapse a hidden symbol onto its file
    return f && vset.has(f) ? f : null;
  }
  function visibleLinks(vset) {
    const seen = new Set();
    const out = [];
    for (const e of DATA.edges) {
      if (!state.kinds.has(e.kind)) continue;
      const s = endpoint(e.source, vset);
      const t = endpoint(e.target, vset);
      if (!s || !t || s === t) continue;
      const key = s + "\x1f" + t + "\x1f" + e.kind;
      if (seen.has(key)) continue;
      seen.add(key);
      out.push({ id: key, source: s, target: t, kind: e.kind });
    }
    return out;
  }

  // --- svg + zoom ---------------------------------------------------------
  const svg = d3.select("#svg");
  const viewG = d3.select("#view");
  const gEdges = d3.select("#edges");
  const gNodes = d3.select("#nodes");

  const zoom = d3
    .zoom()
    .scaleExtent([0.12, 4])
    .on("zoom", (ev) => viewG.attr("transform", ev.transform))
    .on("start", () => svg.classed("grabbing", true))
    .on("end", () => svg.classed("grabbing", false));
  svg.call(zoom).on("dblclick.zoom", null);

  // --- force simulation --------------------------------------------------
  const linkForce = d3
    .forceLink()
    .id((d) => d.id)
    .distance((l) => (l.kind === "contains" ? 34 : 92))
    .strength((l) => (l.kind === "contains" ? 0.55 : 0.14));

  const sim = d3
    .forceSimulation()
    .force("link", linkForce)
    .force("charge", d3.forceManyBody().strength(-240).distanceMax(560).theta(0.9))
    .force("collide", d3.forceCollide().radius((d) => radius(d) + 5))
    .force("x", d3.forceX(0).strength(0.04))
    .force("y", d3.forceY(0).strength(0.04))
    .velocityDecay(0.32)
    .alphaDecay(0.021)
    .on("tick", ticked);

  let linkSel = gEdges.selectAll("line");
  let nodeSel = gNodes.selectAll("g");

  function ticked() {
    linkSel
      .attr("x1", (d) => d.source.x)
      .attr("y1", (d) => d.source.y)
      .attr("x2", (d) => d.target.x)
      .attr("y2", (d) => d.target.y);
    nodeSel.attr("transform", (d) => `translate(${d.x},${d.y})`);
  }

  const drag = d3
    .drag()
    .on("start", (ev, d) => {
      if (!ev.active) sim.alphaTarget(0.25).restart();
      d.fx = d.x;
      d.fy = d.y;
      const g = ev.sourceEvent.target.closest(".node");
      if (g) g.classList.add("dragging");
      // Press-and-hold spotlights this node's neighbourhood; d3-drag fires
      // "start" on pointerdown even without any movement.
      pressed = d.id;
      applyHighlight();
    })
    .on("drag", (ev, d) => {
      d.fx = ev.x;
      d.fy = ev.y;
    })
    .on("end", (ev, d) => {
      if (!ev.active) sim.alphaTarget(0);
      d.fx = null;
      d.fy = null;
      const g = ev.sourceEvent.target.closest(".node");
      if (g) g.classList.remove("dragging");
      pressed = null;
      applyHighlight();
    });

  // --- (re)build through a keyed data-join ------------------------------
  function rebuild(reheat = 0.6) {
    const vnodes = visibleNodes();
    const vset = new Set(vnodes.map((n) => n.id));
    const vlinks = visibleLinks(vset);

    // seed newcomers next to their file so they ease in instead of flying from origin
    for (const n of vnodes) {
      if (n.x == null) {
        const anchor = simNodes.get(fileOf.get(n.id));
        n.x = (anchor ? anchor.x : 0) + (Math.random() - 0.5) * 40;
        n.y = (anchor ? anchor.y : 0) + (Math.random() - 0.5) * 40;
      }
    }

    // Enter/exit fade uses an inline `opacity` on the <g>/<line>. The `.dim`
    // rule (press-to-spotlight, search) therefore dims via *other* properties —
    // child `circle`/`text` opacity for nodes, `stroke-opacity` for edges — so
    // it composes with the fade instead of being shadowed by the inline style.
    nodeSel = gNodes
      .selectAll("g")
      .data(vnodes, (d) => d.id)
      .join(
        (enter) => {
          const g = enter
            .append("g")
            .attr("class", (d) => "node " + d.kind)
            .style("opacity", 0)
            .call(drag);
          g.append("circle")
            .attr("r", radius)
            .attr("fill", (d) => cssVar(KIND_COLOR(d.kind)));
          g.append("text")
            .attr("x", (d) => radius(d) + 3)
            .attr("y", 3)
            .text((d) => (d.label.length > 40 ? d.label.slice(0, 39) + "…" : d.label));
          g.filter((d) => d.kind === "file").on("click", (ev, d) => {
            expanded.has(d.id) ? expanded.delete(d.id) : expanded.add(d.id);
            rebuild();
          });
          g.transition().duration(220).style("opacity", 1);
          return g;
        },
        (update) => update,
        (exit) => exit.transition().duration(160).style("opacity", 0).remove()
      );

    linkSel = gEdges
      .selectAll("line")
      .data(vlinks, (d) => d.id)
      .join(
        (enter) =>
          enter
            .append("line")
            .attr("class", (d) => "edge " + d.kind)
            .style("opacity", 0)
            .call((e) => e.transition().duration(220).style("opacity", 1)),
        (update) => update,
        (exit) => exit.transition().duration(140).style("opacity", 0).remove()
      );

    sim.nodes(vnodes);
    linkForce.links(vlinks);
    sim.alpha(Math.max(sim.alpha(), reheat)).restart();
    applyHighlight();
  }

  // --- highlight: press-to-spotlight, then the search filter ----------------
  const hit = (d, q) =>
    d.label.toLowerCase().includes(q) || (d.path || "").toLowerCase().includes(q);
  const endId = (e) => (typeof e === "object" && e ? e.id : e);

  // ids of `id` plus every node one visible edge away
  function neighbourhood(id) {
    const near = new Set([id]);
    linkSel.each((d) => {
      const s = endId(d.source);
      const t = endId(d.target);
      if (s === id) near.add(t);
      else if (t === id) near.add(s);
    });
    return near;
  }

  // Single source of truth for the dim/match classes. A held-down node wins;
  // otherwise fall back to the search box.
  function applyHighlight() {
    if (pressed != null) {
      const near = neighbourhood(pressed);
      nodeSel.classed("match", (d) => d.id === pressed);
      nodeSel.classed("dim", (d) => !near.has(d.id));
      linkSel.classed("dim", (d) => endId(d.source) !== pressed && endId(d.target) !== pressed);
      return;
    }
    const q = state.query;
    nodeSel.classed("match", (d) => !!q && hit(d, q));
    nodeSel.classed("dim", (d) => !!q && !hit(d, q));
    linkSel.classed("dim", () => !!q);
  }

  // --- fit to view -------------------------------------------------------
  function fit() {
    const ns = sim.nodes();
    if (!ns.length) return;
    let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
    for (const n of ns) {
      x0 = Math.min(x0, n.x); y0 = Math.min(y0, n.y);
      x1 = Math.max(x1, n.x); y1 = Math.max(y1, n.y);
    }
    const r = svg.node().getBoundingClientRect();
    const pad = 70;
    const k = Math.min(
      (r.width - pad) / Math.max(x1 - x0, 1),
      (r.height - pad) / Math.max(y1 - y0, 1),
      2
    );
    const tx = r.width / 2 - (k * (x0 + x1)) / 2;
    const ty = r.height / 2 - (k * (y0 + y1)) / 2;
    svg
      .transition()
      .duration(450)
      .call(zoom.transform, d3.zoomIdentity.translate(tx, ty).scale(k));
  }

  // --- controls ---------------------------------------------------------
  for (const cb of document.querySelectorAll("header input[data-kind]"))
    cb.addEventListener("change", () => {
      cb.checked ? state.kinds.add(cb.dataset.kind) : state.kinds.delete(cb.dataset.kind);
      rebuild(0.5);
    });
  document.getElementById("expandAll").addEventListener("change", (e) => {
    expanded.clear();
    if (e.target.checked)
      for (const n of DATA.nodes) if (n.kind === "file") expanded.add(n.id);
    rebuild();
  });
  const search = document.getElementById("search");
  search.addEventListener("input", () => {
    state.query = search.value.trim().toLowerCase();
    applyHighlight();
  });
  document.getElementById("fitBtn").addEventListener("click", fit);
  svg.on("dblclick", (ev) => {
    if (!ev.target.closest(".node")) fit();
  });

  // Safety net: the spotlight lasts only while a node is held. d3-drag's "end"
  // normally clears it; this also catches a lost pointer / focus.
  for (const evt of ["pointerup", "mouseup", "pointercancel", "blur"])
    addEventListener(evt, () => {
      if (pressed != null) {
        pressed = null;
        applyHighlight();
      }
    });

  // --- start ----------------------------------------------------------------
  {
    const r = svg.node().getBoundingClientRect();
    svg.call(zoom.transform, d3.zoomIdentity.translate(r.width / 2, r.height / 2));
  }
  rebuild();
  let fitted = false;
  sim.on("tick.autofit", () => {
    if (!fitted && sim.alpha() < 0.12) {
      fitted = true;
      sim.on("tick.autofit", null);
      fit();
    }
  });
})();
