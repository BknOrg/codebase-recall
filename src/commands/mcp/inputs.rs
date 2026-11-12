//! Input schemas and decoding share these definitions.
use anyhow::{Result, bail, ensure};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

fn budget() -> u32 {
    256
}
fn two() -> u32 {
    2
}
fn eight() -> u32 {
    8
}
fn file_size() -> u64 {
    50
}
fn limit() -> usize {
    25
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum MatchMode {
    Exact,
    Prefix,
    Substring,
}
fn node_limit() -> usize {
    2000
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ImpactDirection {
    Both,
    Reverse,
    Forward,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PathDirection {
    Forward,
    Reverse,
    Any,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    File,
    Symbol,
    Both,
}
fn impact_direction() -> ImpactDirection {
    ImpactDirection::Both
}
fn path_direction() -> PathDirection {
    PathDirection::Forward
}
fn scope() -> Scope {
    Scope::Both
}
fn kinds() -> Vec<String> {
    vec!["calls".into(), "imports".into(), "references".into()]
}
const EDGE_KINDS: &[&str] = &[
    "calls",
    "imports",
    "references",
    "contains",
    "implements",
    "inherits",
    "overrides",
];
fn graph_kinds() -> Vec<String> {
    vec!["imports".into(), "calls".into(), "contains".into()]
}

macro_rules! input {
    ($name:ident { $($field:tt)* }) => {
        #[derive(Deserialize, Serialize, JsonSchema)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            pub project: Option<String>,
            #[serde(default = "budget")]
            #[schemars(range(min = 1, max = 2048))]
            pub max_output_kb: u32,
            #[serde(default)] pub no_sync: bool,
            $($field)*
        }
    }
}
input!(Digest {
    #[serde(default)] pub path: Option<String>,
    #[serde(default)] pub all: bool,
    #[serde(default)] pub doc_lines: usize,
    #[serde(default)] pub with_docs: bool,
});
input!(Impact {
    pub symbol: Option<String>,
    #[serde(default)] pub diff: bool,
    #[serde(default = "two")] #[schemars(range(min = 1, max = 8))] pub depth: u32,
    #[serde(default = "impact_direction")] pub direction: ImpactDirection,
    #[serde(default = "kinds")] pub kinds: Vec<String>,
    #[serde(default)] pub precise: bool,
    #[serde(default)] pub precise_selective: bool,
    #[serde(default)] pub allow_heuristic_fallback: bool,
});
input!(PathInput {
    pub from: String, pub to: String,
    #[serde(default = "eight")] #[schemars(range(min = 1, max = 20))] pub max_depth: u32,
    #[serde(default = "path_direction")] pub direction: PathDirection,
    #[serde(default = "kinds")] pub kinds: Vec<String>,
    #[serde(default)] pub precise: bool,
    #[serde(default)] pub precise_selective: bool,
    #[serde(default)] pub allow_heuristic_fallback: bool,
});
input!(Explain {
    pub symbol: String,
    #[serde(default)] pub precise: bool,
    #[serde(default)] pub precise_selective: bool,
    #[serde(default)] pub allow_heuristic_fallback: bool,
});
input!(Report {
    #[serde(default)] pub languages: Vec<String>,
    #[serde(default)] pub paths: Vec<String>,
    #[serde(default)] pub exclude_paths: Vec<String>,
    #[serde(default)] pub categories: Vec<String>,
});
input!(Dump {
    pub target: String,
    #[serde(default = "two")] #[schemars(range(min = 1, max = 5))] pub depth: u32,
    #[serde(default = "file_size")] #[schemars(range(min = 1, max = 10240))] pub max_size_kb: u64,
});
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Retrieval {
    Lexical,
    Auto,
    Semantic,
    Hybrid,
}
impl Default for Retrieval {
    fn default() -> Self {
        if crate::commands::search::retrieval::profile().promoted {
            Self::Auto
        } else {
            Self::Lexical
        }
    }
}
input!(Search {
    #[serde(default)] pub languages: Vec<String>,
    #[serde(default)] pub paths: Vec<String>,
    #[serde(default)] pub exclude_paths: Vec<String>,
    #[serde(default)] pub categories: Vec<String>,

    #[serde(default)] pub retrieval: Retrieval,
    #[serde(default)] pub match_mode: Option<MatchMode>,
    pub query: String, pub kind: Option<String>,
    #[serde(default)] pub exported_only: bool,
    #[serde(default)] pub strings: bool,
    #[serde(default = "limit")] #[schemars(range(min = 1, max = 100))] pub limit: usize,
});
input!(Sync {
    #[serde(default)] pub embeddings: bool,
    #[serde(default)] pub precise: bool,
    #[serde(default)] pub precise_selective: bool,
    #[serde(default)] pub allow_heuristic_fallback: bool,
    #[serde(default)] pub languages: Vec<String>,
    #[serde(default)] pub full: bool,
});
input!(Graph {
    #[serde(default = "scope")] pub scope: Scope,
    /// Select edge kinds, including opt-in inherits/implements/overrides.
    #[serde(default = "graph_kinds")] pub kinds: Vec<String>,
    /// Heuristic guesses are capped at 0.35; raise this to exclude them.
    #[serde(default)] #[schemars(range(min = 0.0, max = 1.0))] pub min_confidence: f32,
    pub focus: Option<String>,
    #[serde(default = "two")] #[schemars(range(min = 1, max = 5))] pub depth: u32,
    #[serde(default = "node_limit")] #[schemars(range(min = 1, max = 100000))] pub max_nodes: usize,
    #[serde(default)] pub precise: bool,
    #[serde(default)] pub precise_selective: bool,
    #[serde(default)] pub allow_heuristic_fallback: bool,
});

pub const TOOLS: [(&str, &str); 9] = [
    (
        "code_rcl_digest",
        "Outline modules and typed declarations, including type members and other file declarations. Semantic facts distinguish owners, constructors, fields and properties. Use report for an architecture overview; inspect source before editing.",
    ),
    (
        "code_rcl_impact",
        "Analyze callers and callees. Supply symbol OR diff=true; use qualified names or returned declaration IDs to disambiguate. Inspect relation.resolution.ambiguous and alternatives: guesses are capped at 0.35. Select inherits/implements/overrides in kinds for structural traversal. Diff compares tracked HEAD and working-tree code, including deletions; excludes untracked files. direction: both, reverse, forward.",
    ),
    (
        "code_rcl_path",
        "Find a connection between symbols. direction: forward, reverse, any. Inspect each hop's resolution for ambiguous guesses; kinds explicitly selects structural traversal. A missing path is not proof of no dependency.",
    ),
    (
        "code_rcl_explain",
        "Explain a symbol's semantic kind, qualified owner, signature, members and direct relationships. Use returned declaration IDs for overloads. Inspect resolution evidence and ambiguity; unknown semantic facts are not negative facts.",
    ),
    (
        "code_rcl_report",
        "Start here for project architecture, hubs and subsystems. Inspect resolution_summary and ambiguous_edges before interpreting dependency coverage; counts describe the selected scope.",
    ),
    (
        "code_rcl_dump",
        "Return related files as a context bundle. max_size_kb limits each file; max_output_kb limits the complete response.",
    ),
    (
        "code_rcl_search",
        "Search declarations using retrieval=lexical, semantic, hybrid, or auto. Default stays lexical until automatic feature retrieval passes release gates; auto task queries currently report that gate. Prepare local models explicitly. Normal semantic/hybrid requests refresh pending in-scope embeddings; no_sync uses unverified snapshots. Inspect metadata.embeddings and metadata.retrieval for coverage and candidate-pool counts.",
    ),
    (
        "code_rcl_sync",
        "Hash source and reparse changed contents or stale analyzer revisions. Cache 10-12 migrates to 13 while preserving source snapshots and invalidating derived analysis/vectors; no_sync cannot read stale semantics. embeddings=true updates the prepared vector index without downloading a model. full=true rebuilds selected embeddings and/or precise backends; at least one is required. languages filters scope. precise_selective=true verifies uncertain/risky sites with installed LSPs under a shared budget (default 30s); missing servers/errors return partial heuristics. Inspect metadata.precision; policy completion is not full verification. precise=true requests strict full checks unless allow_heuristic_fallback=true; the modes are exclusive.",
    ),
    (
        "code_rcl_graph",
        "Inspect graph v3 semantic nodes and resolution evidence. scope: file, symbol, both. kinds selects edges; inherits/implements/overrides are opt-in. min_confidence defaults to 0 and filters guesses capped at 0.35. Declaration IDs contain byte ranges, not line numbers. resolution_summary counts reference sites before node caps; alternatives are not extra edges. Limits and omitted counts are reported in metadata. Optional precise_selective verifies uncertain/risky reference sites independently of display filters; inspect metadata.precision coverage.",
    ),
];

macro_rules! select {
    ($name:expr, $action:ident $(, $args:expr)?) => {
        match $name {
            "code_rcl_digest" => $action::<Digest>($($args)?),
            "code_rcl_impact" => $action::<Impact>($($args)?),
            "code_rcl_path" => $action::<PathInput>($($args)?),
            "code_rcl_explain" => $action::<Explain>($($args)?),
            "code_rcl_report" => $action::<Report>($($args)?),
            "code_rcl_dump" => $action::<Dump>($($args)?),
            "code_rcl_search" => $action::<Search>($($args)?),
            "code_rcl_sync" => $action::<Sync>($($args)?),
            "code_rcl_graph" => $action::<Graph>($($args)?),
            _ => bail!("unknown tool: {}", $name),
        }
    }
}
fn decode<T: serde::de::DeserializeOwned + Serialize>(args: &Value) -> Result<Value> {
    Ok(serde_json::to_value(serde_json::from_value::<T>(
        args.clone(),
    )?)?)
}
fn schema<T: JsonSchema>() -> Result<Value> {
    Ok(serde_json::to_value(schemars::schema_for!(T))?)
}
pub fn input_schema(name: &str) -> Result<Value> {
    let mut value = select!(name, schema)?;
    if value["properties"].get("kinds").is_some() {
        value["properties"]["kinds"]["items"]["enum"] = json!(EDGE_KINDS);
        value["properties"]["kinds"]["minItems"] = json!(1);
    }
    if value["properties"].get("precise_selective").is_some() {
        value["properties"]["precise_selective"]["description"] = json!(
            "Opt in to LSP verification of uncertain/risky reference sites after heuristic analysis. Shared operation budget is [precise].selective_budget_secs (default 30). Reuses valid per-reference cache; missing servers/errors retain remaining heuristics with partial coverage. Mutually exclusive with precise; inspect metadata.precision, not just complete/status. Display filters do not limit verification work."
        );
        value["properties"]["precise"]["description"] = json!(
            "Request full verification of pending reference sites through installed language servers. Strict: failures are errors unless allow_heuristic_fallback=true. Mutually exclusive with precise_selective."
        );
        value["properties"]["allow_heuristic_fallback"]["description"] = json!(
            "Explicitly permit partial heuristic results when strict precise verification fails. Selective mode already degrades gracefully; do not combine precise and precise_selective."
        );
        value["allOf"] = json!([{"not":{"required":["precise","precise_selective"],"properties":{"precise":{"const":true},"precise_selective":{"const":true}}}}]);
    }
    if name == "code_rcl_sync" {
        value["properties"]["full"]["description"] = json!(
            "Force rechecks in the selected precise mode and/or rebuild selected prepared embeddings. Use precise_selective=true or precise=true and/or embeddings=true (sync config can supply a mode). Selective mode still selects uncertain/risky sites and respects its budget. This is not a full source reparse, model download, or the CLI-only precise_full parameter."
        );
    }
    if name == "code_rcl_impact" {
        value["oneOf"] = json!([
            {"required":["symbol"],"properties":{"symbol":{"type":"string","minLength":1},"diff":{"const":false}}},
            {"required":["diff"],"properties":{"diff":{"const":true},"symbol":{"type":"null"}}}
        ]);
    }
    Ok(value)
}
pub fn validate(name: &str, args: &Value) -> Result<Value> {
    if name == "code_rcl_search"
        && args["retrieval"] == "semantic"
        && args.get("match_mode").is_some()
    {
        bail!("match_mode applies to lexical/hybrid retrieval, not semantic");
    }
    let value = select!(name, decode, args)?;
    if let Some(confidence) = value.get("min_confidence").and_then(Value::as_f64) {
        ensure!(
            (0.0..=1.0).contains(&confidence),
            "min_confidence must be in 0..=1"
        );
    }
    if matches!(name, "code_rcl_search" | "code_rcl_report") {
        crate::selection::Selection::validate(&crate::selection::Filters::from_mcp(&value))?;
    }
    for (key, max) in [
        ("depth", if name == "code_rcl_impact" { 8 } else { 5 }),
        ("max_depth", 20),
        ("limit", 100),
        ("max_nodes", 100000),
        ("max_size_kb", 10240),
        ("max_output_kb", 2048),
    ] {
        if let Some(number) = value.get(key).and_then(Value::as_u64) {
            ensure!((1..=max).contains(&number), "{key} must be in 1..={max}");
        }
    }
    for key in ["symbol", "from", "to", "query", "target", "project"] {
        if let Some(text) = value.get(key).and_then(Value::as_str) {
            ensure!(!text.trim().is_empty(), "{key} must not be empty");
        }
    }
    if name == "code_rcl_sync" {
        ensure!(
            value["embeddings"] != true || value["no_sync"] != true,
            "embeddings indexing cannot use no_sync"
        );
    }
    if name == "code_rcl_search" {
        ensure!(
            matches!(value["retrieval"].as_str(), Some("lexical" | "auto"))
                || value["strings"] != true,
            "strings is lexical-only"
        );
        ensure!(
            ["exact", "prefix", "substring"]
                .contains(&value["match_mode"].as_str().unwrap_or("substring")),
            "invalid match_mode"
        );
    }
    if name == "code_rcl_impact" {
        ensure!(
            value["symbol"].is_string() != value["diff"].as_bool().unwrap_or(false),
            "supply exactly one of symbol or diff=true"
        );
    }
    if let Some(direction) = value.get("direction").and_then(Value::as_str) {
        let choices: &[&str] = if name == "code_rcl_path" {
            &["forward", "reverse", "any"]
        } else {
            &["both", "forward", "reverse"]
        };
        ensure!(
            choices.contains(&direction),
            "invalid direction: {direction}"
        );
    }
    if let Some(scope) = value.get("scope").and_then(Value::as_str) {
        ensure!(["file", "symbol", "both"].contains(&scope), "invalid scope");
    }
    if let Some(kinds) = value.get("kinds").and_then(Value::as_array) {
        ensure!(
            !kinds.is_empty()
                && kinds
                    .iter()
                    .all(|k| EDGE_KINDS.contains(&k.as_str().unwrap_or(""))),
            "invalid edge kinds"
        );
    }
    ensure!(
        !(value["precise"] == true && value["precise_selective"] == true),
        "precise and precise_selective are mutually exclusive"
    );
    if let Some(languages) = value.get("languages").and_then(Value::as_array) {
        ensure!(
            languages.iter().all(|l| [
                "rust",
                "rs",
                "python",
                "py",
                "javascript",
                "js",
                "jsx",
                "typescript",
                "ts",
                "tsx",
                "java",
                "kotlin",
                "kt",
                "vue",
                "svelte",
                "go",
                "c",
                "cpp",
                "c++",
                "toml"
            ]
            .contains(&l.as_str().unwrap_or(""))),
            "unsupported language filter"
        );
    }
    ensure!(
        !value["full"].as_bool().unwrap_or(false)
            || value["precise"] == true
            || value["precise_selective"] == true
            || value["embeddings"] == true,
        "full requires precise=true or embeddings=true"
    );
    ensure!(
        !value["allow_heuristic_fallback"].as_bool().unwrap_or(false)
            || value["precise"] == true
            || value["precise_selective"] == true,
        "allow_heuristic_fallback requires precise or precise_selective"
    );
    ensure!(
        !(value["no_sync"] == true
            && (value["precise"] == true
                || value["precise_selective"] == true
                || name == "code_rcl_sync")),
        "no_sync cannot be combined with sync or precise"
    );
    Ok(value)
}

pub fn list_tools() -> Value {
    json!(TOOLS.iter().map(|(name, description)| json!({
        "name": name, "description": description,
        "inputSchema": input_schema(name).expect("known schema"),
        "outputSchema": {"type":"object","required":["schema_version","status","data","metadata"],"properties":{"schema_version":{"type":"integer","const":2},"status":{"type":"string"},"data":{},"metadata":{"type":"object"}}}
    })).collect::<Vec<_>>())
}

#[cfg(test)]
mod rebuild_tests {
    use super::*;

    #[test]
    fn graph_schema_and_runtime_share_kinds_and_confidence_contract() {
        let defaults = validate("code_rcl_graph", &json!({})).unwrap();
        assert_eq!(defaults["kinds"], json!(["imports", "calls", "contains"]));
        assert_eq!(defaults["min_confidence"], 0.0);
        let schema = input_schema("code_rcl_graph").unwrap();
        for kind in EDGE_KINDS {
            assert!(
                schema["properties"]["kinds"]["items"]["enum"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(kind))
            );
            assert!(validate("code_rcl_graph", &json!({"kinds":[kind]})).is_ok());
        }
        for confidence in [0.0, 0.35, 1.0] {
            assert!(validate("code_rcl_graph", &json!({"min_confidence":confidence})).is_ok());
        }
        for args in [
            json!({"kinds":[]}),
            json!({"kinds":["typo"]}),
            json!({"min_confidence":-0.1}),
            json!({"min_confidence":1.1}),
        ] {
            assert!(validate("code_rcl_graph", &args).is_err(), "{args}");
        }
    }

    #[test]
    fn full_sync_validates_all_backend_combinations() {
        for full in [false, true] {
            for embeddings in [false, true] {
                for precise in [false, true] {
                    let args = json!({"full": full, "embeddings": embeddings, "precise": precise});
                    assert_eq!(
                        validate("code_rcl_sync", &args).is_ok(),
                        !full || embeddings || precise,
                        "{args}"
                    );
                    let mut no_sync = args;
                    no_sync["no_sync"] = json!(true);
                    assert!(validate("code_rcl_sync", &no_sync).is_err());
                }
            }
        }
    }
}
