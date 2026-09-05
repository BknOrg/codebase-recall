// state: zoom-regime thresholds, view state (expanded / hovered / selected /
// focus), DOM handles, and canvas sizing.

  // --- state --------------------------------------------------------------
  // Zoom hysteresis: drop into the directory roll-up when you zoom out past
  // DIR_IN, climb back to files only once you zoom in past DIR_OUT. The gap
  // lets you frame the directory view comfortably without it flipping back.
  const DIR_IN = 0.32;
  const DIR_OUT = 0.95;
  const expanded = new Set(); // file ids whose symbols are shown
  const state = { kinds: new Set(["imports", "calls"]), query: "", isolate: false };
  let inFileRegime = true;
  let hovered = null;
  let selected = null;
  let focusId = null;
  let focusDepth = 2;
  let isDragging = false;

  // --- DOM ---------------------------------------------------------------
  const stage = document.getElementById("stage");
  const canvas = document.getElementById("scene");
  const ctx = canvas.getContext("2d");
  const tooltip = document.createElement("div");
  tooltip.className = "graph-tooltip";
  stage.appendChild(tooltip);
  const panel = document.getElementById("sidePanel");

  let dpr = Math.max(1, window.devicePixelRatio || 1);
  let width = 0;
  let height = 0;

  function resize() {
    dpr = Math.max(1, window.devicePixelRatio || 1);
    width = stage.clientWidth;
    height = stage.clientHeight;
    canvas.width = Math.round(width * dpr);
    canvas.height = Math.round(height * dpr);
    canvas.style.width = width + "px";
    canvas.style.height = height + "px";
    scheduleDraw();
  }
  window.addEventListener("resize", resize);
