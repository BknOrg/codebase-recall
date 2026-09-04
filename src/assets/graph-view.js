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
  const radius = (n) => (n.kind === "file" ? 8 : 5.5);

  // --- indexes ----------------------------------------------------------------
  const fileOf = new Map(); // symbol id -> owning file id
  for (const n of DATA.nodes)
    if (n.kind !== "file" && n.kind !== "external" && n.path)
      fileOf.set(n.id, "file:" + n.path);

  const expanded = new Set();
  const state = { kinds: new Set(["imports", "calls", "references"]), query: "" };
  let hovered = null; // id of node currently hovered by cursor
  let isDragging = false;

  // Tooltip element
  const stage = document.getElementById("stage");
  const tooltip = document.createElement("div");
  tooltip.className = "graph-tooltip";
  stage.appendChild(tooltip);

  function showTooltip(ev, d) {
    if (isDragging) return;
    let html = `<div class="tt-title"><span>${d.label}</span><span class="tt-kind">${d.kind}</span></div>`;
    if (d.path) {
      html += `<div class="tt-path">${d.path}</div>`;
    }
    const meta = [];
    if (d.language) meta.push(d.language);
    if (d.lines && Array.isArray(d.lines)) {
      meta.push(`L${d.lines[0]}-${d.lines[1]}`);
    }
    if (d.exported) meta.push("exported");
    if (meta.length) {
      html += `<div class="tt-meta">${meta.join(" &bull; ")}</div>`;
    }
    tooltip.innerHTML = html;
    tooltip.style.display = "block";
    updateTooltipPos(ev);
  }

  function updateTooltipPos(ev) {
    const stageRect = stage.getBoundingClientRect();
    let left = ev.clientX - stageRect.left + 14;
    let top = ev.clientY - stageRect.top + 14;
    if (left + 240 > stageRect.width) {
      left = ev.clientX - stageRect.left - 240;
    }
    if (top + 80 > stageRect.height) {
      top = ev.clientY - stageRect.top - 70;
    }
    tooltip.style.left = `${Math.max(8, left)}px`;
    tooltip.style.top = `${Math.max(8, top)}px`;
  }

  function hideTooltip() {
    tooltip.style.display = "none";
  }

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
    .distance((l) => (l.kind === "contains" ? 40 : 110))
    .strength((l) => (l.kind === "contains" ? 0.55 : 0.14));

  const sim = d3
    .forceSimulation()
    .force("link", linkForce)
    .force("charge", d3.forceManyBody().strength(-280).distanceMax(650).theta(0.9))
    .force("collide", d3.forceCollide().radius((d) => radius(d) + 8))
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
      isDragging = true;
      hideTooltip();
      if (!ev.active) sim.alphaTarget(0.25).restart();
      d.fx = d.x;
      d.fy = d.y;
      const g = ev.sourceEvent.target.closest(".node");
      if (g) g.classList.add("dragging");
      hovered = d.id;
      applyHighlight();
    })
    .on("drag", (ev, d) => {
      d.fx = ev.x;
      d.fy = ev.y;
    })
    .on("end", (ev, d) => {
      isDragging = false;
      if (!ev.active) sim.alphaTarget(0);
      d.fx = null;
      d.fy = null;
      const g = ev.sourceEvent.target.closest(".node");
      if (g) g.classList.remove("dragging");
      // Cek apakah kursor masih di atas node
      const currentHover = document.elementFromPoint(ev.sourceEvent.clientX, ev.sourceEvent.clientY)?.closest(".node");
      if (!currentHover) {
        hovered = null;
      }
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
    // rule therefore dims via child opacity / stroke-opacity so it composes
    // cleanly with the fade.
    nodeSel = gNodes
      .selectAll("g")
      .data(vnodes, (d) => d.id)
      .join(
        (enter) => {
          const g = enter
            .append("g")
            .attr("class", (d) => "node " + d.kind)
            .style("opacity", 0)
            .call(drag)
            .on("pointerenter", (ev, d) => {
              hovered = d.id;
              applyHighlight();
              showTooltip(ev, d);
            })
            .on("pointermove", (ev) => {
              updateTooltipPos(ev);
            })
            .on("pointerleave", () => {
              if (!isDragging) {
                hovered = null;
                applyHighlight();
                hideTooltip();
              }
            });

          g.append("circle")
            .attr("r", radius)
            .attr("fill", (d) => cssVar(KIND_COLOR(d.kind)));

          g.append("text")
            .attr("x", (d) => radius(d) + 4)
            .attr("y", 3.5)
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

  // --- highlight: hover-to-spotlight, then search filter --------------------
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

  // Single source of truth for the dim/hovered/connected/match classes.
  function applyHighlight() {
    if (hovered != null) {
      const near = neighbourhood(hovered);
      nodeSel.classed("hovered", (d) => d.id === hovered);
      nodeSel.classed("connected", (d) => d.id !== hovered && near.has(d.id));
      nodeSel.classed("match", false);
      nodeSel.classed("dim", (d) => !near.has(d.id));

      linkSel.classed("highlighted", (d) => endId(d.source) === hovered || endId(d.target) === hovered);
      linkSel.classed("dim", (d) => endId(d.source) !== hovered && endId(d.target) !== hovered);
      return;
    }

    // Reset hover classes jika tidak ada node yang di-hover
    nodeSel.classed("hovered", false);
    nodeSel.classed("connected", false);
    linkSel.classed("highlighted", false);

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

  // Clear highlight on mouseleave svg container
  svg.on("mouseleave", () => {
    if (!isDragging && hovered != null) {
      hovered = null;
      applyHighlight();
      hideTooltip();
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
