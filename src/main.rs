mod analysis;
mod assets;
mod cache;
mod cli;
mod commands;
mod config;
#[cfg(test)]
#[path = "cache/content_tests.rs"]
mod content_tests;
mod dump;
mod embeddings;
mod graph;
mod health;
mod operation;
mod output;
mod precise;
mod progress;
mod selection;
#[cfg(feature = "viewer")]
mod server;
mod service;
#[cfg(test)]
#[path = "cache/snapshot_tests.rs"]
mod snapshot_tests;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Command};

fn main() -> Result<()> {
    // Normalize command line arguments: if user runs `code-rcl <cmd> help`,
    // treat `help` as `--help` so clap prints the detailed help and examples.
    let raw_args: Vec<String> = std::env::args().collect();
    let normalized_args: Vec<String> = if raw_args.len() >= 2 && raw_args[1] != "help" {
        raw_args
            .into_iter()
            .enumerate()
            .map(|(i, arg)| {
                if i > 1 && arg == "help" {
                    "--help".to_string()
                } else {
                    arg
                }
            })
            .collect()
    } else {
        raw_args
    };

    let cli = Cli::parse_from(normalized_args);

    let result = match cli.command {
        Command::Dump(args) => commands::dump::run(args),
        Command::Init(args) => commands::init::run(args),
        Command::Sync(args) => commands::sync::run(args),
        Command::Graph(args) => commands::graph::run(args),
        Command::Serve(args) => commands::serve::run(args),
        Command::Impact(args) => commands::impact::run(args),
        Command::Path(args) => commands::path::run(args),
        Command::Explain(args) => commands::explain::run(args),
        Command::Report(args) => commands::report::run(args),
        Command::Search(args) => commands::search::run(args),
        Command::Embeddings(args) => embeddings::run(args),
        Command::Digest(args) => commands::digest::run(args),
        Command::Mcp(args) => commands::mcp::run(args),
        Command::Setup(args) => commands::setup::run(args),
        Command::Uninstall(args) => commands::setup::uninstall(args),
    };
    if operation::cancelled() {
        std::process::exit(130);
    }
    result
}
