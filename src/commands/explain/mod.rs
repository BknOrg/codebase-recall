pub mod models;
pub mod render;

pub use models::*;
pub use render::*;

use anyhow::Result;

use crate::cache::CacheDb;
use crate::cache::models::{FileRow, SymbolRow};
use crate::cli::{ExplainArgs, ImpactArgs};
use crate::commands::digest::{extract_docs_and_diagram, extract_signature};
use crate::commands::impact::{self, ImpactReport};

const DOC_LINES: usize = 5;

pub fn run(args: ExplainArgs) -> Result<()> {
    let reports = generate_reports(&args)?;
    if args.json {
        println!(
            "{}",
            crate::output::json(&args.project, args.no_sync, &reports)?
        );
    } else {
        print!("{}", render_ascii(&reports));
    }
    Ok(())
}

/// One report per node matching `args.symbol`: its declaration details from the
/// cache plus one hop of callers/callees from the `impact` engine.
pub fn generate_reports(args: &ExplainArgs) -> Result<Vec<ExplainReport>> {
    let impact_args = ImpactArgs {
        symbol: Some(args.symbol.clone()),
        diff: false,
        project: args.project.clone(),
        depth: 1,
        direction: "both".to_string(),
        kinds: vec![
            "calls".to_string(),
            "imports".to_string(),
            "references".to_string(),
        ],
        json: false,
        no_sync: args.no_sync,
        precise: args.precise.clone(),
    };
    let relations = impact::generate_reports(&impact_args)?;

    let db = CacheDb::open(&args.project)?;

    let mut out = Vec::with_capacity(relations.len());
    for report in relations {
        out.push(build_report(args, &db, report)?);
    }
    Ok(out)
}

fn build_report(
    args: &ExplainArgs,
    db: &CacheDb,
    relations: ImpactReport,
) -> Result<ExplainReport> {
    let located = locate_symbol(db, &relations)?;

    let (mut signature, mut doc) = (None, None);
    let mut members = Vec::new();
    let (mut start_line, mut end_line, mut language) = (None, None, None);
    let mut is_exported = true;

    if let Some((file, sym)) = &located {
        language = Some(file.language.clone());
        start_line = sym.start_line;
        end_line = sym.end_line;
        is_exported = sym.is_exported;

        let source = std::fs::read_to_string(args.project.join(&file.path)).unwrap_or_default();
        let lines: Vec<String> = source.lines().map(String::from).collect();

        let sig = extract_signature(&lines, sym.start_line, sym.end_line);
        if !sig.is_empty() {
            signature = sig.into();
        }
        let (d, _) = extract_docs_and_diagram(
            &lines,
            sym.start_line,
            sym.end_line,
            &file.language,
            DOC_LINES,
        );
        doc = d;

        members = db
            .symbols_for_file(file.id)?
            .into_iter()
            .filter(|s| {
                s.parent_symbol_id == Some(sym.id)
                    || (s.kind == "method"
                        && s.file_id == sym.file_id
                        && s.type_name.as_deref() == Some(sym.name.as_str()))
            })
            .map(|s| MemberInfo {
                semantic: Some(s.semantic.clone()),
                name: s.name,
                kind: s.kind.to_string(),
                line: s.start_line,
                is_exported: s.is_exported,
            })
            .collect();
        members.sort_by_key(|m: &MemberInfo| m.line);
    }

    Ok(ExplainReport {
        semantic: located.as_ref().map(|(_, s)| s.semantic.clone()),
        symbol: relations.target_symbol.clone(),
        id: relations.target_id.clone(),
        kind: relations.target_kind.clone(),
        path: relations.target_path.clone(),
        language,
        start_line,
        end_line,
        is_exported,
        community: relations.target_community.clone(),
        signature,
        doc,
        members,
        relations,
    })
}

/// Find the cache rows behind a symbol node. `None` for file nodes or when the
/// cache no longer has the symbol.
fn locate_symbol(db: &CacheDb, report: &ImpactReport) -> Result<Option<(FileRow, SymbolRow)>> {
    if !report.target_id.starts_with("sym:") {
        return Ok(None);
    }
    let Some(path) = report.target_path.as_deref() else {
        return Ok(None);
    };
    let Some(file) = db.file_by_path(path)? else {
        return Ok(None);
    };
    let sym = db.symbols_for_file(file.id)?.into_iter().find(|s| {
        crate::graph::declaration_id(
            path,
            &s.name,
            s.start_byte.unwrap_or(0),
            s.end_byte.unwrap_or(0),
            &s.kind,
        ) == report.target_id
    });
    Ok(sym.map(|s| (file, s)))
}
