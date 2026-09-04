mod analysis;
mod assets;
mod cache;
mod cli;
mod commands;
mod dump;
mod graph;
mod server;

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
