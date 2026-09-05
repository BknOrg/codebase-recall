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
}

#[derive(Parser, Debug)]
pub struct DumpArgs {
    #[arg(default_value = ".")]
    pub path: PathBuf,

    #[arg(short = 'f', long = "file", default_value = "codebase-context.md")]
    pub output: PathBuf,

    #[arg(long, default_value_t = 50)]
    pub max_size_kb: u64,
}

#[derive(Parser, Debug)]
pub struct InitArgs {
    /// Project directory to initialize
    #[arg(long, default_value = ".")]
    pub project: PathBuf,

    /// Overwrite an existing config.toml
    #[arg(long)]
    pub force: bool,
}

#[derive(Parser, Debug)]
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
    #[arg(
        long,
        value_delimiter = ',',
        default_value = "imports,calls,contains"
    )]
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
}

#[derive(Parser, Debug)]
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
