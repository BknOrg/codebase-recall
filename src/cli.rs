use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "codebase-recall",
    version,
    about = "CLI codebase context dumper for LLMs and code relation grapher",
    propagate_version = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Dump the codebase into a single Markdown context file
    Dump(DumpArgs),
    /// Create .code-rcl/ (graph cache DB + config) in the target project
    Init(InitArgs),
    /// Parse changed source files into the graph cache
    Sync(SyncArgs),
    /// Render a relation graph to a file (html, json, dot)
    Graph(GraphArgs),
    /// Serve the relation graph in the browser; the server exits when you close the tab
    Serve(ServeArgs),
    /// Analyze blast radius and downstream/upstream callers affected by modifying a symbol
    Impact(ImpactArgs),
    /// Generate an architecture outline and public API digest of the codebase
    Digest(DigestArgs),
    /// Run Model Context Protocol (MCP) server over stdio for AI agent integration
    Mcp(McpArgs),
    /// Self-install the AI agent skill and configure MCP servers (Antigravity/Gemini, Claude Code)
    Setup(SetupArgs),
}

#[derive(Parser, Debug)]
#[command(
    about = "Dump the codebase into a single Markdown context file",
    after_help = "Examples:\n  code-rcl dump\n  code-rcl dump path/to/project -o my-context.md\n  code-rcl dump -r handle_request --depth 2\n  code-rcl dump --max-size-kb 100"
)]
pub struct DumpArgs {
    /// Target project directory or root path to dump
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Output markdown file path stem or full filename
    #[arg(short = 'o', long = "output", default_value = "codebase-context")]
    pub output: PathBuf,

    /// Skip source files larger than this many KB
    #[arg(long, default_value_t = 50)]
    pub max_size_kb: u64,

    /// Focus on a symbol or file and dump only its connected neighborhood
    #[arg(short = 'r', long = "relation")]
    pub relation: Option<String>,

    /// Hop depth for relation neighborhood extraction
    #[arg(long, default_value_t = 2)]
    pub depth: u32,

    /// Skip auto-syncing changed files before dumping
    #[arg(long)]
    pub no_sync: bool,
}

#[derive(Parser, Debug)]
#[command(
    about = "Create .code-rcl/ (graph cache DB + config) in the target project",
    after_help = "Examples:\n  code-rcl init\n  code-rcl init --project path/to/project\n  code-rcl init --force"
)]
pub struct InitArgs {
    /// Project directory to initialize
    #[arg(long, default_value = ".")]
    pub project: PathBuf,

    /// Overwrite an existing config.toml
    #[arg(long)]
    pub force: bool,
}

#[derive(Parser, Debug)]
#[command(
    about = "Parse changed source files into the graph cache",
    after_help = "Examples:\n  code-rcl sync\n  code-rcl sync --language rust,py\n  code-rcl sync --precise\n  code-rcl sync --precise --precise-full"
)]
pub struct SyncArgs {
    /// Project directory to sync
    #[arg(long, default_value = ".")]
    pub project: PathBuf,

    /// Skip source files larger than this many KB
    #[arg(long, default_value_t = 512)]
    pub max_file_kb: u64,

    /// Restrict to a subset of languages (e.g. rust,js,py)
    #[arg(long, value_delimiter = ',')]
    pub language: Vec<String>,

    #[command(flatten)]
    pub precise: PreciseArgs,
}

/// Opt-in compiler-grade resolution, shared by `sync`, `graph`, `serve`, and `impact`.
#[derive(Parser, Debug, Clone)]
pub struct PreciseArgs {
    /// Resolve references through the real language server for each language
    /// (rust-analyzer, pyright, jdtls, kotlin-language-server, typescript-language-server) instead of
    /// guessing from the AST. Needs those servers installed; any that are
    /// missing are reported and their language keeps its heuristic edges.
    #[arg(long)]
    pub precise: bool,

    /// Re-ask the language servers about every file, not just the ones with no
    /// answer yet. Use after edits whose effects reach other files.
    #[arg(long, requires = "precise")]
    pub precise_full: bool,

    /// Seconds to wait for a single language-server answer
    #[arg(
        long,
        default_value_t = 15,
        value_name = "SECONDS",
        requires = "precise"
    )]
    pub precise_timeout: u64,
}

impl Default for PreciseArgs {
    fn default() -> Self {
        Self {
            precise: false,
            precise_full: false,
            precise_timeout: 15,
        }
    }
}

/// Filters shared by `graph` and `serve`: they select which nodes and edges the
/// resolved [`crate::graph::CodeGraph`] ends up containing.
#[derive(Parser, Debug)]
pub struct GraphQuery {
    /// Project directory to graph
    #[arg(long, default_value = ".")]
    pub project: PathBuf,

    /// Graph scope: file, symbol, or both
    #[arg(long, default_value = "both")]
    pub scope: String,

    /// Edge kinds to include, comma-separated: imports, calls, references, contains
    /// (`references` is noisy on large graphs, so it is off by default)
    #[arg(long, value_delimiter = ',', default_value = "imports,calls,contains")]
    pub kinds: Vec<String>,

    /// Only include files matching this glob
    #[arg(long)]
    pub path: Option<String>,

    /// Restrict the graph to the neighborhood of this symbol name
    #[arg(long)]
    pub focus: Option<String>,

    /// BFS depth around --focus
    #[arg(long, default_value_t = 2)]
    pub depth: u32,

    /// Drop edges below this confidence
    #[arg(long, default_value_t = 0.4)]
    pub min_confidence: f32,

    /// Include edges to external modules (npm/pypi/crate deps)
    #[arg(long)]
    pub include_external: bool,

    /// Cap on total graph nodes; past this the lowest-degree symbols are dropped
    /// (files, dirs and externals are always kept). 0 disables the cap.
    #[arg(long, default_value_t = 4000)]
    pub max_nodes: usize,

    /// Do not auto-sync changed files before rendering
    #[arg(long)]
    pub no_sync: bool,

    #[command(flatten)]
    pub precise: PreciseArgs,
}

#[derive(Parser, Debug)]
#[command(
    about = "Render a relation graph to a file (html, json, dot)",
    after_help = "Examples:\n  code-rcl graph\n  code-rcl graph --format json -o graph.json\n  code-rcl graph --focus calculate_total --depth 2\n  code-rcl graph --kinds calls --path \"src/**/*.rs\""
)]
pub struct GraphArgs {
    #[command(flatten)]
    pub query: GraphQuery,

    /// Output formats, comma-separated: html, json, dot
    #[arg(long, value_delimiter = ',', default_value = "html")]
    pub format: Vec<String>,

    /// Output file or path stem. Defaults to <project>/.code-ctx/code-graph.<ext>
    #[arg(short = 'o', long)]
    pub output: Option<PathBuf>,
}

#[derive(Parser, Debug)]
#[command(
    about = "Serve the relation graph in the browser; the server exits when you close the tab",
    after_help = "Examples:\n  code-rcl serve\n  code-rcl serve --port 8080 --no-open\n  code-rcl serve --focus execute_query"
)]
pub struct ServeArgs {
    #[command(flatten)]
    pub query: GraphQuery,

    /// Port to bind on 127.0.0.1 (0 picks a free port)
    #[arg(long, default_value_t = 0)]
    pub port: u16,

    /// Do not open a browser window automatically
    #[arg(long)]
    pub no_open: bool,
}

#[derive(Parser, Debug)]
#[command(
    about = "Analyze blast radius and downstream/upstream callers affected by modifying a symbol",
    after_help = "Examples:\n  code-rcl impact parse_config\n  code-rcl impact execute_query --depth 10\n  code-rcl impact validate_input --kinds calls\n  code-rcl impact delete_user --json"
)]
pub struct ImpactArgs {
    /// Target symbol name or identifier to analyze blast radius for
    pub symbol: String,

    /// Project directory to inspect
    #[arg(long, default_value = ".")]
    pub project: PathBuf,

    /// Maximum upstream caller traversal depth
    #[arg(long, default_value_t = 5)]
    pub depth: u32,

    /// Edge kinds to traverse in reverse, comma-separated (e.g. calls,imports)
    #[arg(long, value_delimiter = ',', default_value = "calls,imports")]
    pub kinds: Vec<String>,

    /// Output result as JSON instead of ASCII tree
    #[arg(long)]
    pub json: bool,

    /// Skip auto-syncing changed files before analyzing
    #[arg(long)]
    pub no_sync: bool,

    #[command(flatten)]
    pub precise: PreciseArgs,
}

#[derive(Parser, Debug)]
#[command(
    about = "Generate an architecture outline and public API digest of the codebase",
    after_help = "Examples:\n  code-rcl digest\n  code-rcl digest src/analysis\n  code-rcl digest -o architecture.md\n  code-rcl digest --all\n  code-rcl digest --json"
)]
pub struct DigestArgs {
    /// Target project directory or sub-path to outline [default: .]
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Explicit project root — overrides PATH's auto-detection (walking up
    /// for .code-rcl/Cargo.toml/package.json/pyproject.toml). PATH, if also
    /// given, is then read as a sub-path filter within this root instead of
    /// a location to search from.
    #[arg(long)]
    pub project: Option<PathBuf>,

    /// Output file path for the generated markdown digest
    #[arg(short = 'o', long = "output")]
    pub output: Option<PathBuf>,

    /// Include private/internal functions and types (default: public only)
    #[arg(long)]
    pub all: bool,

    /// Output result as structured JSON instead of Markdown
    #[arg(long)]
    pub json: bool,

    /// Skip auto-syncing changed files before generating digest
    #[arg(long)]
    pub no_sync: bool,
}

#[derive(Parser, Debug, Clone)]
#[command(
    about = "Run Model Context Protocol (MCP) server over stdio for AI agent integration",
    after_help = "Examples:\n  code-rcl mcp\n  code-rcl mcp --project /path/to/repo"
)]
pub struct McpArgs {
    /// Default project directory to analyze [default: .]
    #[arg(long, default_value = ".")]
    pub project: PathBuf,
}

#[derive(Parser, Debug, Clone)]
#[command(
    about = "Self-install the AI agent skill and configure MCP servers (Antigravity/Gemini, Claude Code)",
    after_help = "Examples:\n  code-rcl setup\n  code-rcl setup --workspace\n  code-rcl setup --global\n  code-rcl setup --target claude\n  code-rcl setup --print-skill"
)]
pub struct SetupArgs {
    /// Install globally to user profile config (~/.gemini/config and ~/.claude.json)
    #[arg(long)]
    pub global: bool,

    /// Install locally into the current workspace (.agents/ and .mcp.json)
    #[arg(long)]
    pub workspace: bool,

    /// Target AI agent environment: gemini, claude, or all [default: all]
    #[arg(long, default_value = "all")]
    pub target: String,

    /// Print the embedded SKILL.md to stdout and exit
    #[arg(long)]
    pub print_skill: bool,

    /// Only install the skill file, do not modify MCP configs
    #[arg(long)]
    pub skill_only: bool,

    /// Only configure MCP server, do not install skill file
    #[arg(long)]
    pub mcp_only: bool,

    /// Custom target directory for the skill (overrides defaults)
    #[arg(long)]
    pub skill_dir: Option<PathBuf>,
}
