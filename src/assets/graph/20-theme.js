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
      dir: g("--dir", "#475569"),
      file: g("--file", "#2563eb"),
      func: g("--func", "#16a34a"),
      type: g("--type", "#d97706"),
      variable: g("--var", "#9333ea"),
      ext: g("--ext", "#64748b"),
    };
  }
  readColors();
  const mq = window.matchMedia("(prefers-color-scheme: dark)");
  (mq.addEventListener || mq.addListener).call(mq, "change", () => {
    readColors();
    scheduleDraw();
  });

  function kindColor(k) {
    if (k === "dir") return COL.dir;
    if (k === "file") return COL.file;
    if (k === "external") return COL.ext;
    if (k === "variable") return COL.variable;
    if (TYPE_KINDS.has(k)) return COL.type;
    return COL.func;
  }
  function nodeRadius(d) {
    if (d.kind === "dir") return 9;
    if (d.kind === "file") return 6.5;
    return 4 + Math.min(6, Math.sqrt(d.degree || 0));
  }
