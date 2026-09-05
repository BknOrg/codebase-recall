// data: parse the graph blob, build id indexes (parent/child, file-of,
// dir-of), and pre-aggregate call edges to file pairs.

  const DATA = JSON.parse(document.getElementById("graph-data").textContent);
  const nodeById = new Map(DATA.nodes.map((n) => [n.id, n]));

  const TYPE_KINDS = new Set(["struct", "enum", "trait", "interface", "type", "class"]);
  const isSymId = (id) => typeof id === "string" && id.startsWith("sym:");

  // parent/child from `contains` edges: dir->dir, dir->file, file->symbol, sym->sym
  const parentOf = new Map();
  const childrenOf = new Map();
  for (const e of DATA.edges) {
    if (e.kind !== "contains") continue;
    parentOf.set(e.target, e.source);
    let kids = childrenOf.get(e.source);
    if (!kids) childrenOf.set(e.source, (kids = []));
    kids.push(e.target);
  }

  // owning file id for any node id (memoised)
  const _fileOf = new Map();
  function fileOf(id) {
    if (id == null) return null;
    if (id.startsWith("file:")) return id;
    if (_fileOf.has(id)) return _fileOf.get(id);
    let c = id;
    while (c != null && !c.startsWith("file:")) c = parentOf.get(c);
    c = c || null;
    _fileOf.set(id, c);
    return c;
  }
  function dirOf(id) {
    let c = id;
    while (c != null && !c.startsWith("dir:")) c = parentOf.get(c);
    return c || null;
  }

  // import edges stay file->file; call/reference edges get aggregated to the
  // (srcFile, tgtFile) pair, remembering which symbols contributed each end.
  const importEdges = [];
  const callAgg = { calls: [], references: [] };
  const symUses = new Map(); // symId -> Set(fileId it calls into)
  const symUsedBy = new Map(); // symId -> Set(fileId that calls it)
  {
    const m = new Map();
    const bump = (map, k, v) => {
      let s = map.get(k);
      if (!s) map.set(k, (s = new Set()));
      s.add(v);
    };
    for (const e of DATA.edges) {
      if (e.kind === "imports") {
        importEdges.push({ source: e.source, target: e.target });
        continue;
      }
      if (e.kind !== "calls" && e.kind !== "references") continue;
      const sf = fileOf(e.source);
      const tf = fileOf(e.target);
      if (!sf || !tf || sf === tf) continue; // intra-file calls add nothing at this altitude
      const key = sf + "\x1f" + tf + "\x1f" + e.kind;
      let a = m.get(key);
      if (!a)
        m.set(
          key,
          (a = { srcFile: sf, tgtFile: tf, kind: e.kind, srcSyms: new Set(), tgtSyms: new Set(), count: 0 })
        );
      a.count++;
      if (isSymId(e.source) && nodeById.has(e.source)) {
        a.srcSyms.add(e.source);
        bump(symUses, e.source, tf);
      }
      if (isSymId(e.target) && nodeById.has(e.target)) {
        a.tgtSyms.add(e.target);
        bump(symUsedBy, e.target, sf);
      }
    }
    for (const a of m.values()) callAgg[a.kind].push(a);
  }
