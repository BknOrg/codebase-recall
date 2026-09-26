use anyhow::Result;

use crate::cli::ServeArgs;
use crate::commands::graph::build_graph;
use crate::server::{self, ServeOptions};

pub fn run(mut args: ServeArgs) -> Result<()> {
    crate::config::ProjectConfig::load(&args.query.project)?
        .apply_precise_default(&mut args.query.precise);
    let graph = build_graph(&args.query)?;
    server::serve(
        &graph,
        ServeOptions {
            port: args.port,
            open: !args.no_open,
            project: args.query.project.clone(),
        },
    )
}
