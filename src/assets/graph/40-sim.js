// sim: d3-force configuration and rebuild() — the single entry point that
// recomputes nodes/links, warms the layout, and restarts the simulation.

  // --- force simulation -----------------------------------------------
  const linkForce = d3
    .forceLink()
    .id((d) => d.id)
    .distance((l) => (l.kind === "contains" ? 30 : l.kind === "imports" ? 90 : 60))
    .strength((l) => (l.kind === "contains" ? 0.55 : 0.12));

  const sim = d3
    .forceSimulation()
    .force("link", linkForce)
    .force("charge", d3.forceManyBody().strength(-260).distanceMax(700).theta(1.1))
    .force("x", d3.forceX(0).strength(0.05))
    .force("y", d3.forceY(0).strength(0.05))
    .velocityDecay(0.35)
    .alphaDecay(0.028)
    .alphaMin(0.02)
    .stop();

  let collideOn = false;
  function updateCollide(n) {
    if (n <= 1500 && !collideOn) {
      sim.force("collide", d3.forceCollide().radius((d) => nodeRadius(d) + 6));
      collideOn = true;
    } else if (n > 1500 && collideOn) {
      sim.force("collide", null);
      collideOn = false;
    }
  }

  sim.on("tick", () => {
    quad = null;
    scheduleDraw();
  });
  sim.on("end", () => {
    quad = null;
    if (!autoFitted) {
      autoFitted = true;
      fit(true);
    }
    scheduleDraw();
    persist();
  });

  // --- current frame data -------------------------------------------
  let nodes = [];
  let links = [];
  let adj = new Map(); // includes contains, for hover highlight
  let quad = null;
  let warmedUp = false;
  let autoFitted = false;
  let pendingRegimeFit = false;
  let applyingRegimeFit = false;

  function rebuild(reheat) {
    reheat = reheat == null ? 0.7 : reheat;

    let vnodes = visibleNodeList();
    if (!inFileRegime && vnodes.length === 0) {
      inFileRegime = true; // repo has no sub-directories to roll up into
      vnodes = visibleNodeList();
    }
    let vset = new Set(vnodes.map((n) => n.id));
    let vlinks = buildLinks().filter((l) => vset.has(l.source) && vset.has(l.target));

    if (focusId && vset.has(focusId)) {
      const near = neighbourhoodByDepth(focusId, focusDepth, vlinks);
      vnodes = vnodes.filter((n) => near.has(n.id));
      vset = new Set(vnodes.map((n) => n.id));
      vlinks = vlinks.filter((l) => vset.has(l.source) && vset.has(l.target));
    }

    if (state.isolate && state.query) {
      const hitset = new Set(vnodes.filter((n) => hit(n, state.query)).map((n) => n.id));
      for (const l of vlinks) {
        if (hitset.has(l.source)) hitset.add(l.target);
        else if (hitset.has(l.target)) hitset.add(l.source);
      }
      vnodes = vnodes.filter((n) => hitset.has(n.id));
      vset = new Set(vnodes.map((n) => n.id));
      vlinks = vlinks.filter((l) => vset.has(l.source) && vset.has(l.target));
    }

    const simList = vnodes.map(asSimNode);
    let fresh = 0;
    for (const n of simList) {
      if (n.x == null) {
        fresh++;
        const anchor = simNodes.get(parentOf.get(n.id)) || simNodes.get(fileOf(n.id));
        n.x = (anchor ? anchor.x : 0) + (Math.random() - 0.5) * 60;
        n.y = (anchor ? anchor.y : 0) + (Math.random() - 0.5) * 60;
      }
    }

    adj = new Map();
    const addAdj = (a, b) => {
      let s = adj.get(a);
      if (!s) adj.set(a, (s = new Set()));
      s.add(b);
    };
    for (const l of vlinks) {
      addAdj(l.source, l.target);
      addAdj(l.target, l.source);
    }

    nodes = simList;
    links = vlinks;

    updateCollide(nodes.length);
    sim.nodes(nodes);
    linkForce.links(links);
    quad = null;

    // Warm up headlessly when the layout is mostly new (first load, regime
    // switch, or a big expand) so it doesn't visibly explode into place.
    if (!warmedUp || fresh > 0.3 * Math.max(simList.length, 1)) {
      warmedUp = true;
      sim.alpha(1);
      const ticks = Math.min(200, 60 + Math.round(nodes.length / 40));
      for (let i = 0; i < ticks; i++) sim.tick();
      if (!autoFitted || pendingRegimeFit) fitRegimeAware();
      pendingRegimeFit = false;
    }
    sim.alpha(Math.max(sim.alpha(), reheat)).restart();

    buildLegend();
    updateHeader();
    updatePanel();
    scheduleDraw();
    persist();
  }
