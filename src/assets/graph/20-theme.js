// theme: CSS-variable colour cache (refreshed on scheme change) plus the
// per-kind colour and radius helpers.

// --- theme colours (read once, refresh on scheme change) ---------------
let COL = {};
function readColors() {
  const s = getComputedStyle(document.documentElement);
  const g = (v, fb) => s.getPropertyValue(v).trim() || fb;
  COL = {
    bg: g("--bg", "#fff"),
    fg: g("--fg", "#111"),
    muted: g("--muted", "#666"),
    edge: g("--edge", "#ccc"),
    accent: g("--accent", "#ef4444"),
    focus: g("--graph-focus", "#3b82f6"),
    outbound: g("--graph-outbound", "#ef4444"),
    inbound: g("--graph-inbound", "#0ea5e9"),
    dir: g("--dir", "#475569"),
    file: g("--file", "#2563eb"),
    func: g("--func", "#7c3aed"),
    type: g("--type", "#2563eb"),
    variable: g("--var", "#b45309"),
    organization: g("--organization", "#526579"),
    component: g("--component", "#0f766e"),
    macro: g("--macro", "#be185d"),
    glyph: g("--node-glyph", "#fff"),
    ext: g("--ext", "#64748b"),
    edges: Object.fromEntries(Object.keys(EDGE_STYLES).map(kind =>
      [kind, g("--edge-" + kind, "#64748b")])),
  };
}
// Shared by the canvas, legend and filters. Unknown kinds stay neutral.
const NODE_FAMILIES = {
  type: { label: "Types", color: "type", shape: "square" },
  callable: { label: "Functions / methods / constructors / destructors", color: "func", shape: "circle" },
  data: { label: "Fields / properties / data", color: "variable", shape: "diamond" },
  organization: { label: "Modules / namespaces / impls", color: "organization", shape: "hexagon" },
  component: { label: "Components", color: "component", shape: "square" },
  macro: { label: "Macros", color: "macro", shape: "hexagon" },
  other: { label: "Other / unknown", color: "ext", shape: "circle" },
};
const EDGE_STYLES = {
  calls: { label: "calls" }, imports: { label: "imports" },
  references: { label: "references" }, contains: { label: "contains" },
  inherits: { label: "inherits" }, implements: { label: "implements" },
  overrides: { label: "overrides" },
};
const KIND_GLYPHS = {
  function: "fn", method: "m", constructor: "new", destructor: "~", concept: "Co", class: "C", struct: "S",
  union: "U", interface: "I", trait: "Tr", enum: "E", enum_member: "e",
  type: "T", type_alias: "=", record: "R", object: "O", field: "f",
  property: "p", variable: "v", constant: "k", parameter: "a", module: "M",
  namespace: "N", package: "Pkg", impl: "im", macro: "!", component: "<>",
};
function nodeFamily(kind) {
  if (kind === "component") return "component";
  if (TYPE_KINDS.has(kind)) return "type";
  if (["function", "method", "constructor", "destructor"].includes(kind)) return "callable";
  if (["field", "property", "variable", "constant", "parameter", "enum_member"].includes(kind)) return "data";
  if (["module", "namespace", "package", "impl"].includes(kind)) return "organization";
  return kind === "macro" ? "macro" : "other";
}
function nodeShape(kind) {
  if (kind === "dir" || kind === "file") return "square";
  return NODE_FAMILIES[nodeFamily(kind)].shape;
}
function edgeColor(kind) { return COL.edges[kind] || COL.edge; }
function edgeDash(ambiguous, zoom) { return ambiguous ? [5 / zoom, 4 / zoom] : []; }
readColors();
const mq = window.matchMedia("(prefers-color-scheme: dark)");
const onSchemeChange = () => {
  readColors();
  legendSig = "";
  buildLegend();
  scheduleDraw();
};
if (mq.addEventListener) mq.addEventListener("change", onSchemeChange);
else mq.addListener(onSchemeChange);

function kindColor(k) {
  if (k === "dir") return COL.dir;
  if (k === "file") return COL.file;
  if (k === "external") return COL.ext;
  return COL[NODE_FAMILIES[nodeFamily(k)].color];
}
// Subsystems (communities): a stable hue per community id, spread by the golden angle so
// neighbouring ids stay visually distinct however many there are.
const COMMUNITIES = DATA.communities || [];
const communityInfo = new Map(COMMUNITIES.map((c) => [c.id, c]));
function communityColor(id) {
  const hue = Math.round((id * 137.508) % 360);
  return `hsl(${hue}, 62%, ${mq.matches ? 62 : 44}%)`;
}
// What a node is filled with. In subsystem mode: its subsystem colour, or neutral grey when it
// has none (matching the legend's "no subsystem" row); directory roll-ups keep their own colour.
function nodeFill(d) {
  if (state.colorMode !== "community" || d.kind === "dir" || d.kind === "external") return kindColor(d.kind);
  return d.community != null ? communityColor(d.community) : COL.ext;
}
function nodeRadius(d) {
  if (d.kind === "dir") return 16;
  if (d.kind === "file") return 6.5;
  return 4 + Math.min(6, Math.sqrt(d.degree || 0));
}
