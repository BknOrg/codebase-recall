//! Serializers for a [`CodeGraph`].

pub mod dot;
pub mod html;
pub mod json;

use anyhow::{bail, Result};

use crate::graph::CodeGraph;

/// Output format selected on the CLI.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Html,
    Json,
    Dot,
}

impl Format {
    pub fn parse(token: &str) -> Result<Format> {
        Ok(match token.trim().to_ascii_lowercase().as_str() {
            "html" => Format::Html,
            "json" => Format::Json,
            "dot" | "graphviz" | "gv" => Format::Dot,
            other => bail!("unknown graph format `{other}` (expected html, json, or dot)"),
        })
    }

    pub fn extension(&self) -> &'static str {
        match self {
            Format::Html => "html",
            Format::Json => "json",
            Format::Dot => "dot",
        }
    }
}

pub fn render(graph: &CodeGraph, format: Format) -> Result<String> {
    Ok(match format {
        Format::Html => html::render(graph),
        Format::Json => json::render(graph)?,
        Format::Dot => dot::render(graph),
    })
}
