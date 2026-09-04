mod analysis;
mod assets;
mod cache;
mod cli;
mod commands;
mod formatter;
mod graph;
mod server;
mod walker;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Command};

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Dump(args) => commands::dump::run(args),
        Command::Init(args) => commands::init::run(args),
        Command::Sync(args) => commands::sync::run(args),
        Command::Graph(args) => commands::graph::run(args),
        Command::Serve(args) => commands::serve::run(args),
    }
}
