// legend: a live colour key — rebuilt on every rebuild() from the node kinds (or, in
// "color by subsystem" mode, the subsystems) actually on the canvas, so e.g. the "directory" row shows only in the folder
// roll-up view. The "type" row label follows the graph's languages.

  const legendEl = document.getElementById("legend");
  let legendSig = "";
  const MAX_LEGEND_COMMUNITIES = 12;

  // Languages span the whole graph, so the type-row label stays stable while
  // you expand / collapse files.
  const ALL_LANGS = new Set();
  for (const n of DATA.nodes) if (n.language) ALL_LANGS.add(n.language);

  function buildLegend() {
    if (!legendEl) return;
    const kinds = new Set();
    for (const d of nodes) kinds.add(d.kind); // only what's rendered right now

    const hasFunc = ["function", "method", "impl", "module", "macro"].some((k) => kinds.has(k));
    const hasType = [...TYPE_KINDS].some((k) => kinds.has(k));

    const rows = [];
    if (state.colorMode === "community" && COMMUNITIES.length) {
      // One row per subsystem actually on the canvas, biggest first (COMMUNITIES is size-sorted).
      const present = new Set();
      let unassigned = false;
      for (const d of nodes) {
        if (d.kind === "dir" || d.kind === "external") continue;
        if (d.community != null) present.add(d.community);
        else unassigned = true;
      }
      for (const c of COMMUNITIES) {
        if (!present.has(c.id)) continue;
        if (rows.length >= MAX_LEGEND_COMMUNITIES) break;
        rows.push([communityColor(c.id), `${c.label} (${c.size})`, false]);
      }
      const hidden = present.size - rows.length;
      if (hidden > 0) rows.push(["transparent", `+${hidden} more subsystems`, false]);
      if (unassigned) rows.push(["var(--ext)", "no subsystem", false]);
    } else {
      if (kinds.has("dir")) rows.push(["var(--dir)", "directory", true]);
      if (kinds.has("file")) rows.push(["var(--file)", "file", false]);
      if (hasFunc) rows.push(["var(--func)", "function / method", false]);
      if (hasType) rows.push(["var(--type)", typeLabel(ALL_LANGS), false]);
      if (kinds.has("variable")) rows.push(["var(--var)", "variable", false]);
      if (kinds.has("external")) rows.push(["var(--ext)", "external module", false]);
    }

    const sig = state.colorMode + "|" + rows.map((r) => r[0] + r[1]).join("|");
    if (sig === legendSig) return; // rebuild() is frequent — skip idle DOM writes
    legendSig = sig;

    legendEl.innerHTML = rows
      .map(
        ([v, label, sq]) =>
          `<div><span class="dot${sq ? " sq" : ""}" style="background:${v}"></span>${label}</div>`
      )
      .join("");
    legendEl.hidden = rows.length === 0;
  }

  // The `--type` colour covers struct/enum/trait/interface/type/class/component;
  // name it after whatever languages are in play.
  function typeLabel(langs) {
    const has = (l) => langs.has(l);
    const rusty = has("rust");
    const oop = has("java") || has("kotlin") || has("javascript") || has("typescript");
    if (rusty && !oop && langs.size === 1) return "struct / enum / trait";
    if (has("python") && langs.size === 1) return "class";
    if (oop && !rusty) return "class / interface";
    if (rusty && oop) return "type · class · struct";
    return "type";
  }
