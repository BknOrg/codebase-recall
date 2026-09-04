use anyhow::Result;

use crate::cli::ServeArgs;
use crate::commands::graph::build_graph;
use crate::server::{self, ServeOptions};

pub fn run(args: ServeArgs) -> Result<()> {
    let graph = build_graph(&args.query)?;
    server::serve(
        &graph,
        ServeOptions {
            port: args.port,
            open: !args.no_open,
        },
    )
}
