// model: turn the static graph + view state into the node/link arrays the
// simulation and renderer consume (visibility, edge roll-up, BFS).

  // --- physics node reuse (positions survive rebuilds) ------------------
  const simNodes = new Map();
  function asSimNode(n) {
    let s = simNodes.get(n.id);
    if (!s) {
      s = Object.assign({}, n);
      simNodes.set(n.id, s);
    } else {
      s.kind = n.kind;
      s.label = n.label;
      s.degree = n.degree;
    }
    return s;
  }

  // --- what is visible --------------------------------------------------
  function visibleNodeList() {
    if (!inFileRegime) return DATA.nodes.filter((n) => n.kind === "dir");
    const out = [];
    for (const n of DATA.nodes) {
      if (n.kind === "dir") continue;
      if (n.kind === "file" || n.kind === "external") {
        out.push(n);
        continue;
      }
      const f = fileOf(n.id);
      if (f && expanded.has(f)) out.push(n);
    }
    return out;
  }

  // Map a file/external id onto the node that currently represents it.
  function repFile(id) {
    return inFileRegime ? id : dirOf(id);
  }

  function buildLinks() {
    const out = new Map();
    const add = (s, t, kind) => {
      if (!s || !t || s === t) return;
      const key = s + "\x1f" + t + "\x1f" + kind;
      let l = out.get(key);
      if (!l) out.set(key, (l = { id: key, source: s, target: t, kind, count: 0 }));
      l.count++;
    };

    if (state.kinds.has("imports"))
      for (const e of importEdges) add(repFile(e.source), repFile(e.target), "imports");

    for (const kind of ["calls", "references"]) {
      if (!state.kinds.has(kind)) continue;
      for (const a of callAgg[kind]) {
        if (!inFileRegime) {
          add(dirOf(a.srcFile), dirOf(a.tgtFile), kind);
          continue;
        }
        const aExp = expanded.has(a.srcFile);
        const bExp = expanded.has(a.tgtFile);
        if (!aExp && !bExp) continue; // calls stay hidden until a file is opened
        const L = aExp && a.srcSyms.size ? [...a.srcSyms] : [a.srcFile];
        const R = bExp && a.tgtSyms.size ? [...a.tgtSyms] : [a.tgtFile];
        for (const l of L)
          for (const r of R) {
            if (isSymId(l) && isSymId(r)) {
              // never symbol↔symbol: route each end through the other file
              add(l, a.tgtFile, kind);
              add(a.srcFile, r, kind);
            } else {
              add(l, r, kind);
            }
          }
      }
    }

    // containment tethers
    if (inFileRegime) {
      for (const f of expanded) {
        const stack = [...(childrenOf.get(f) || [])];
        while (stack.length) {
          const c = stack.pop();
          if (!nodeById.has(c)) continue;
          add(parentOf.get(c), c, "contains");
          for (const g of childrenOf.get(c) || []) stack.push(g);
        }
      }
    } else {
      for (const n of DATA.nodes) {
        if (n.kind !== "dir") continue;
        const p = parentOf.get(n.id);
        if (p && p.startsWith("dir:")) add(p, n.id, "contains");
      }
    }

    return [...out.values()];
  }

  function neighbourhoodByDepth(seed, depth, linkList) {
    const nb = new Map();
    const push = (a, b) => {
      let arr = nb.get(a);
      if (!arr) nb.set(a, (arr = []));
      arr.push(b);
    };
    for (const l of linkList) {
      if (l.kind === "contains") continue;
      const s = l.source.id != null ? l.source.id : l.source;
      const t = l.target.id != null ? l.target.id : l.target;
      push(s, t);
      push(t, s);
    }
    const keep = new Set([seed]);
    let frontier = [seed];
    for (let d = 0; d < depth && frontier.length; d++) {
      const next = [];
      for (const id of frontier)
        for (const n of nb.get(id) || [])
          if (!keep.has(n)) {
            keep.add(n);
            next.push(n);
          }
      frontier = next;
    }
    return keep;
  }
