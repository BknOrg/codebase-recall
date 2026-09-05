//! Front-end assets for the graph view, embedded into the binary at compile time.
//!
//! The same markup + view script drives two delivery modes:
//! * [`Delivery::Inline`] — one self-contained `.html` file (`code-ctx graph`).
//! * [`Delivery::Server`] — assets fetched from the local `code-ctx serve`
//!   process, plus a tiny script that keeps the server alive only while the tab is.

use std::sync::LazyLock;

/// Vendored d3 v7 (UMD). Pinned; see README for the exact version and source.
pub const D3_JS: &str = include_str!("d3.min.js");
pub const GRAPH_CSS: &str = include_str!("graph.css");
/// Heartbeat client used only by `code-ctx serve`.
pub const LIVE_JS: &str = include_str!("live.js");

/// The graph view script, kept as small single-responsibility source files under
/// `graph/` and stitched together — in this order — inside one IIFE at first use.
/// Delivered as a single `<script>` (inline mode) or one `/assets/graph-view.js`
/// route (serve mode), so there is no browser module loader involved.
const GRAPH_VIEW_PARTS: &[&str] = &[
    include_str!("graph/00-data.js"),
    include_str!("graph/10-state.js"),
    include_str!("graph/20-theme.js"),
    include_str!("graph/30-model.js"),
    include_str!("graph/40-sim.js"),
    include_str!("graph/50-render.js"),
    include_str!("graph/60-interaction.js"),
    include_str!("graph/70-tooltip-panel.js"),
    include_str!("graph/80-controls.js"),
    include_str!("graph/90-main.js"),
];

pub static GRAPH_VIEW_JS: LazyLock<String> = LazyLock::new(|| {
    format!(
        "(function () {{\n\"use strict\";\n\n{}\n}})();\n",
        GRAPH_VIEW_PARTS.join("\n")
    )
});

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Delivery {
    Inline,
    Server,
}

const BODY: &str = r#"<div id="app">
  <header>
    <h1>code-ctx graph</h1>
    <span class="stat">__STAT__</span>
    <label><input type="checkbox" data-kind="imports" checked> imports</label>
    <label><input type="checkbox" data-kind="calls" checked> calls</label>
    <label><input type="checkbox" data-kind="references"> references</label>
    <label><input type="checkbox" id="expandAll"> expand all</label>
    <button id="collapseAll" type="button">collapse</button>
    <input type="search" id="search" placeholder="filter nodes&hellip;">
    <label><input type="checkbox" id="isolate"> isolate</label>
    <button id="fitBtn" type="button">fit</button>
    <span id="focusCtl" hidden>
      <span id="focusLabel"></span>
      <button id="focusShallower" type="button" title="shallower">&minus;</button>
      <span id="focusDepth">2</span>
      <button id="focusDeeper" type="button" title="deeper">+</button>
      <button id="focusClear" type="button">clear focus</button>
    </span>
  </header>
  <div id="stage">
    <canvas id="scene"></canvas>
    <aside id="sidePanel" hidden></aside>
    <div class="legend">
      <div><span class="dot sq" style="background:var(--dir)"></span>directory</div>
      <div><span class="dot" style="background:var(--file)"></span>file</div>
      <div><span class="dot" style="background:var(--func)"></span>function / method</div>
      <div><span class="dot" style="background:var(--type)"></span>type</div>
      <div><span class="dot" style="background:var(--var)"></span>variable</div>
    </div>
    <div id="status"></div>
  </div>
</div>"#;

/// Build the full HTML document. `data_json` is the serialized `CodeGraph`;
/// `stat` is the short "N nodes / M edges" line shown in the header.
pub fn graph_page(data_json: &str, stat: &str, delivery: Delivery) -> String {
    // `<` only ever occurs inside JSON string values, so this keeps the blob
    // valid JSON while making it impossible to break out of the <script> tag.
    let safe = data_json.replace('<', "\\u003c");
    let body = BODY.replace("__STAT__", stat);

    let (head, tail) = match delivery {
        Delivery::Inline => (
            format!("<style>{GRAPH_CSS}</style>"),
            format!(
                "<script>{D3_JS}</script>\n<script>{}</script>",
                &*GRAPH_VIEW_JS
            ),
        ),
        Delivery::Server => (
            "<link rel=\"stylesheet\" href=\"/assets/graph.css\">".to_string(),
            "<script src=\"/assets/d3.min.js\"></script>\n\
             <script src=\"/assets/graph-view.js\"></script>\n\
             <script src=\"/assets/live.js\"></script>"
                .to_string(),
        ),
    };

    format!(
        "<!doctype html>\n\
<html lang=\"en\">\n\
<head>\n\
<meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
<title>code-ctx graph</title>\n\
{head}\n\
</head>\n\
<body>\n\
{body}\n\
<script id=\"graph-data\" type=\"application/json\">{safe}</script>\n\
{tail}\n\
</body>\n\
</html>\n"
    )
}
