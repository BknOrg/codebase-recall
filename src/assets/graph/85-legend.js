// Live legend: colors identify families or subsystems; glyphs identify exact kinds.
const legendEl = document.getElementById("legend");
let legendSig = "";
const MAX_LEGEND_COMMUNITIES = 12;
const RELATION_MEANINGS = {
  calls: "caller → callable",
  imports: "importer → module / file",
  references: "user → referenced symbol",
  contains: "owner — member (no arrow)",
  inherits: "derived type → base type",
  implements: "implementing type → interface / trait",
  overrides: "overriding method → base method",
};
function buildLegend() {
  if (!legendEl) return;
  const counts = new Map();
  for (const n of nodes) counts.set(n.kind, (counts.get(n.kind) || 0) + 1);
  const subsystemMode = state.colorMode === "community" && COMMUNITIES.length > 0;
  const rows = [`<p class="legend-note">Visible on canvas · ${nodes.length} nodes · ${links.length} links</p>`];
  const swatch = (color, shape = "circle") =>
    `<span aria-hidden="true" class="dot ${shape === "square" ? "sq" : shape}" style="background:${color}"></span>`;
  const row = (color, label, count, shape = "circle") =>
    `<div class="legend-row">${swatch(color, shape)}<span>${esc(label)}</span><span class="legend-count">${count}</span></div>`;
  rows.push(`<h3>${subsystemMode ? "Subsystem colors" : "Node families"}</h3>`);
  if (counts.has("dir")) rows.push(row(COL.dir, "Directory", counts.get("dir"), "square"));
  if (subsystemMode) {
    const present = new Map();
    let unassigned = 0;
    for (const n of nodes) {
      if (n.kind === "dir" || n.kind === "external") continue;
      if (n.community == null) unassigned++;
      else present.set(n.community, (present.get(n.community) || 0) + 1);
    }
    let shown = 0;
    for (const c of COMMUNITIES) {
      if (!present.has(c.id) || shown >= MAX_LEGEND_COMMUNITIES) continue;
      rows.push(row(communityColor(c.id), c.label, present.get(c.id)));
      shown++;
    }
    if (present.size > shown) rows.push(`<p class="legend-note">+${present.size - shown} more subsystems</p>`);
    if (unassigned) rows.push(row(COL.ext, "No subsystem", unassigned));
    rows.push('<h3>Symbol shapes and glyphs</h3><p class="legend-note">Fill color indicates subsystem. Shape and glyph still identify symbols.</p>');
  } else if (counts.has("file")) rows.push(row(COL.file, "File", counts.get("file"), "square"));
  for (const [key, family] of Object.entries(NODE_FAMILIES)) {
    const present = [...counts.keys()].filter(k => !["dir", "file", "external"].includes(k) && nodeFamily(k) === key).sort();
    if (!present.length) continue;
    const count = present.reduce((sum, kind) => sum + counts.get(kind), 0);
    const color = subsystemMode ? COL.muted : COL[family.color];
    rows.push(`<details class="legend-family" data-family="${esc(key)}"><summary>${swatch(color, family.shape)}<span>${esc(family.label)}</span><span class="legend-count">${count}</span></summary>`);
    for (const kind of present) {
      rows.push(`<div class="legend-kind"><code>${esc(KIND_GLYPHS[kind] || "?")}</code><span>${esc(kind.replaceAll("_", " "))}</span><span class="legend-count">${counts.get(kind)}</span></div>`);
    }
    rows.push('</details>');
  }
  if (counts.has("external")) rows.push(row(COL.ext, "External module", counts.get("external")));
  if (nodes.some(n => !["dir", "file", "external"].includes(n.kind))) rows.push('<p class="legend-note">Expand a family for symbol abbreviations. Zoom in to see glyphs inside nodes.</p>');
  const visibleKinds = new Set(links.map(l => l.kind));
  if (visibleKinds.size) rows.push('<h3>Relation colors and direction</h3>');
  for (const [kind, style] of Object.entries(EDGE_STYLES)) {
    if (!visibleKinds.has(kind)) continue;
    rows.push(`<div class="legend-relation"><span aria-hidden="true" class="legend-line ${kind === "contains" ? "" : "arrow"}" style="color:${edgeColor(kind)}"></span><span><strong>${esc(style.label)}</strong><small>${esc(RELATION_MEANINGS[kind])}</small></span></div>`);
  }
  if (visibleKinds.size) {
    rows.push('<h3>Resolution certainty</h3>');
    rows.push('<div class="legend-row"><span aria-hidden="true" class="legend-line"></span><span>No ambiguity marked</span></div>');
    if (links.some(l => l.ambiguous)) rows.push('<div class="legend-row"><span aria-hidden="true" class="legend-line ambiguous"></span><span>Ambiguous guess included</span></div>');
    rows.push('<p class="legend-note">A solid line is not proof of precise resolution. Dashed aggregates may also contain resolved links. Alternatives are not extra edges.</p>');
  }
  const precision = DATA.precision;
  if (precision && precision.requested_mode !== "heuristic") {
    const coverage = precision.languages || [];
    const count = key => coverage.reduce((n, l) => n + (l[key] || 0), 0);
    rows.push(`<h3>LSP verification coverage</h3><p class="legend-note">${esc(precision.requested_mode)} · ${precision.complete ? "policy completed" : "partial"}. ${count("verified")} verified, ${count("unresolved")} unresolved, ${count("skipped")} skipped, ${count("pending")} pending. Selective completion does not verify every reference.</p>`);
  }
  if (nodes.length) rows.push('<h3>Selection</h3><div class="legend-row"><span aria-hidden="true" class="legend-focus"></span><span>Hovered or selected node</span></div><p class="legend-note">Selection preserves relation colors. Select a node for evidence, reasons, heuristic scores and alternatives. Use Filters to hide families, relations or ambiguous guesses.</p>');
  if (!nodes.length) rows.push('<p class="legend-note">No visible nodes. Clear filters or expand a file to inspect symbols.</p>');
  const html = rows.join("");
  if (html === legendSig) return;
  // Preserve expanded family keys when counts or the current view change.
  const openFamilies = new Set([...legendEl.querySelectorAll?.('.legend-family[open]') || []].map(el => el.dataset.family));
  legendSig = html;
  legendEl.innerHTML = html;
  for (const el of legendEl.querySelectorAll?.('.legend-family') || []) {
    if (openFamilies.has(el.dataset.family)) el.open = true;
  }
}
