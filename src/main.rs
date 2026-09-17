mod analysis;
mod assets;
mod cache;
mod cli;
mod commands;
mod dump;
mod graph;
mod precise;
mod server;

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

    match cli.command {
        Command::Dump(args) => commands::dump::run(args),
        Command::Init(args) => commands::init::run(args),
        Command::Sync(args) => commands::sync::run(args),
        Command::Graph(args) => commands::graph::run(args),
        Command::Serve(args) => commands::serve::run(args),
        Command::Impact(args) => commands::impact::run(args),
        Command::Digest(args) => commands::digest::run(args),
        Command::Mcp(args) => commands::mcp::run(args),
        Command::Setup(args) => commands::setup::run(args),
    }
}
