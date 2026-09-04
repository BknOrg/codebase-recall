//! Front-end assets for the graph view, embedded into the binary at compile time.
//!
//! The same markup + view script drives two delivery modes:
//! * [`Delivery::Inline`] — one self-contained `.html` file (`code-ctx graph`).
//! * [`Delivery::Server`] — assets fetched from the local `code-ctx serve`
//!   process, plus a tiny script that keeps the server alive only while the tab is.

/// Vendored d3 v7 (UMD). Pinned; see README for the exact version and source.
pub const D3_JS: &str = include_str!("d3.min.js");
pub const GRAPH_CSS: &str = include_str!("graph.css");
pub const GRAPH_VIEW_JS: &str = include_str!("graph-view.js");
/// Heartbeat client used only by `code-ctx serve`.
pub const LIVE_JS: &str = include_str!("live.js");

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
    <label><input type="checkbox" data-kind="references" checked> references</label>
    <label><input type="checkbox" data-kind="contains"> contains</label>
    <label><input type="checkbox" id="expandAll"> expand all files</label>
    <input type="search" id="search" placeholder="filter nodes&hellip;">
    <button id="fitBtn" type="button">fit</button>
  </header>
  <div id="stage">
    <svg id="svg"><g id="view"><g id="edges"></g><g id="nodes"></g></g></svg>
    <div class="legend">
      <div><span class="dot" style="background:var(--file)"></span>file</div>
      <div><span class="dot" style="background:var(--func)"></span>function / method</div>
      <div><span class="dot" style="background:var(--type)"></span>type</div>
      <div><span class="dot" style="background:var(--var)"></span>variable</div>
      <div><span class="dot" style="background:var(--ext)"></span>external</div>
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
            format!("<script>{D3_JS}</script>\n<script>{GRAPH_VIEW_JS}</script>"),
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
