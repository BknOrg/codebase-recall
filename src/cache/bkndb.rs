#![allow(dead_code)]
//! Native BknDb backend for codebase-recall (.bkndb single-file LSM format).

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use bkndb::db::Db;
use bkndb::graph::{Direction, NodeId, Properties};
use bkndb::relational::{ColumnDef, ColumnKind, RelSchema};
use bkndb::value::PropValue;
use bkndb::{LsmStorageBackend, MemoryStorageBackend};
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

pub static FILES_SCHEMA: RelSchema = RelSchema {
    name: "files",
    columns: &[
        ColumnDef { name: "id", kind: ColumnKind::Int },
        ColumnDef { name: "path", kind: ColumnKind::Str },
        ColumnDef { name: "hash", kind: ColumnKind::Str },
        ColumnDef { name: "lang", kind: ColumnKind::Str },
        ColumnDef { name: "synced_at", kind: ColumnKind::Int },
    ],
    primary_key: "id",
    auto_increment_pk: false,
    indexed_columns: &["path"],
};

pub static SYMBOLS_SCHEMA: RelSchema = RelSchema {
    name: "symbols_meta",
    columns: &[
        ColumnDef { name: "id", kind: ColumnKind::Int },
        ColumnDef { name: "file_id", kind: ColumnKind::Int },
        ColumnDef { name: "file_path", kind: ColumnKind::Str },
        ColumnDef { name: "name", kind: ColumnKind::Str },
        ColumnDef { name: "kind", kind: ColumnKind::Str },
        ColumnDef { name: "signature", kind: ColumnKind::Str },
        ColumnDef { name: "docstring", kind: ColumnKind::Str },
        ColumnDef { name: "line_start", kind: ColumnKind::Int },
        ColumnDef { name: "line_end", kind: ColumnKind::Int },
    ],
    primary_key: "id",
    auto_increment_pk: false,
    indexed_columns: &["name", "file_id"],
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileRecord {
    pub id: u64,
    pub path: String,
    pub hash: String,
    pub language: String,
    pub ast_synced_at: i64,
    pub precise_synced_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SymbolRecord {
    pub id: u64,
    pub file_id: u64,
    pub name: String,
    pub kind: String,
    pub signature: Option<String>,
    pub docstring: Option<String>,
    pub line_start: usize,
    pub line_end: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RefRecord {
    pub id: u64,
    pub file_id: u64,
    pub caller_symbol_id: Option<u64>,
    pub resolved_symbol_id: Option<u64>,
    pub name: String,
    pub kind: String,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportRecord {
    pub id: u64,
    pub file_id: u64,
    pub raw_specifier: String,
    pub imported_name: Option<String>,
    pub alias: Option<String>,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SymbolSearchResult {
    pub symbol_id: u64,
    pub file_path: String,
    pub name: String,
    pub kind: String,
    pub line_start: u32,
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImpactNode {
    pub symbol_name: String,
    pub symbol_kind: String,
    pub file_path: String,
    pub line_start: u32,
    pub depth: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImpactResult {
    pub root_symbol: String,
    pub affected_callers: Vec<ImpactNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CallPathStep {
    pub from_symbol: String,
    pub to_symbol: String,
    pub edge_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CallPathResult {
    pub distance: usize,
    pub steps: Vec<CallPathStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HubItem {
    pub symbol_name: String,
    pub symbol_kind: String,
    pub file_path: String,
    pub incoming_calls: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DigestResult {
    pub total_files: usize,
    pub total_symbols: usize,
    pub top_hubs: Vec<HubItem>,
}

enum EngineWrapper {
    Lsm(Db<LsmStorageBackend>),
    Mem(Db<MemoryStorageBackend>),
}

pub struct BknDbCodeStore {
    engine: EngineWrapper,
}

impl BknDbCodeStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let backend = LsmStorageBackend::open(path)?;
        Ok(Self {
            engine: EngineWrapper::Lsm(Db::new(backend)),
        })
    }

    pub fn in_memory() -> Self {
        let backend = MemoryStorageBackend::new();
        Self {
            engine: EngineWrapper::Mem(Db::new(backend)),
        }
    }
}

macro_rules! dispatch_store {
    ($self:expr, $store:ident => $body:expr) => {
        match &$self.engine {
            EngineWrapper::Lsm($store) => $body,
            EngineWrapper::Mem($store) => $body,
        }
    };
}

impl BknDbCodeStore {
    pub fn sync_file(
        &mut self,
        file: FileRecord,
        symbols: Vec<SymbolRecord>,
        refs: Vec<RefRecord>,
        _imports: Vec<ImportRecord>,
    ) -> Result<()> {
        dispatch_store!(self, db => {
            db.write_tx(|batch| {
                // 1. Delete previous file records if existing
                let prev_file = {
                    let mut rel = batch.relational();
                    let files = rel.table(&FILES_SCHEMA).select_eq("path", &PropValue::Str(file.path.clone()))?;
                    files.first().map(|r| r.pk.clone())
                };

                if let Some(prev_pk) = prev_file {
                    let prev_file_id = match prev_pk {
                        PropValue::Int(n) => n as u64,
                        _ => 0,
                    };
                    {
                        let mut rel = batch.relational();
                        let _ = rel.table(&FILES_SCHEMA).delete(&prev_pk);
                    }
                    {
                        let mut g = batch.graph();
                        let _ = g.cascade_delete(NodeId(prev_file_id), "CONTAINS");
                        let _ = g.delete_node(NodeId(prev_file_id));
                    }
                    {
                        let mut rel = batch.relational();
                        let prev_syms = rel.table(&SYMBOLS_SCHEMA).select_eq("file_id", &PropValue::Int(prev_file_id as i64))?;
                        for sym in prev_syms {
                            let _ = rel.table(&SYMBOLS_SCHEMA).delete(&sym.pk);
                        }
                    }
                }

                // 2. Insert into files table
                let mut file_props = Properties::new();
                file_props.insert("path".to_string(), PropValue::Str(file.path.clone()));
                file_props.insert("hash".to_string(), PropValue::Str(file.hash.clone()));
                file_props.insert("lang".to_string(), PropValue::Str(file.language.clone()));
                file_props.insert("synced_at".to_string(), PropValue::Int(file.ast_synced_at));
                {
                    let mut rel = batch.relational();
                    rel.table(&FILES_SCHEMA).insert_with_pk(PropValue::Int(file.id as i64), file_props.clone())?;
                }

                // 3. Create File node in graph
                let file_node_id = {
                    let mut g = batch.graph();
                    g.create_node("File", file_props)?
                };

                // 4. Create Symbol rows and nodes
                let mut symbol_id_to_node_id = HashMap::new();
                for sym in &symbols {
                    let mut props = Properties::new();
                    props.insert("name".to_string(), PropValue::Str(sym.name.clone()));
                    props.insert("kind".to_string(), PropValue::Str(sym.kind.clone()));
                    props.insert("file_path".to_string(), PropValue::Str(file.path.clone()));
                    props.insert("line_start".to_string(), PropValue::Int(sym.line_start as i64));

                    let node_id = {
                        let mut g = batch.graph();
                        let nid = g.create_node(&sym.kind, props)?;
                        g.create_edge(file_node_id, "CONTAINS", nid, Properties::new())?;
                        nid
                    };
                    symbol_id_to_node_id.insert(sym.id, node_id);

                    let mut row_props = Properties::new();
                    row_props.insert("file_id".to_string(), PropValue::Int(file.id as i64));
                    row_props.insert("file_path".to_string(), PropValue::Str(file.path.clone()));
                    row_props.insert("name".to_string(), PropValue::Str(sym.name.clone()));
                    row_props.insert("kind".to_string(), PropValue::Str(sym.kind.clone()));
                    if let Some(sig) = &sym.signature {
                        row_props.insert("signature".to_string(), PropValue::Str(sig.clone()));
                    }
                    if let Some(doc) = &sym.docstring {
                        row_props.insert("docstring".to_string(), PropValue::Str(doc.clone()));
                    }
                    row_props.insert("line_start".to_string(), PropValue::Int(sym.line_start as i64));
                    row_props.insert("line_end".to_string(), PropValue::Int(sym.line_end as i64));

                    let mut rel = batch.relational();
                    rel.table(&SYMBOLS_SCHEMA).insert_with_pk(PropValue::Int(node_id.0 as i64), row_props)?;
                }

                // 5. Connect Calls / Refs edges
                for r in &refs {
                    if let (Some(caller_id), Some(resolved_id)) = (r.caller_symbol_id, r.resolved_symbol_id) {
                        let caller_node = symbol_id_to_node_id.get(&caller_id).copied();
                        let target_node = symbol_id_to_node_id.get(&resolved_id).copied().or_else(|| {
                            let mut rel = batch.relational();
                            let rows = rel.table(&SYMBOLS_SCHEMA).select_eq("name", &PropValue::Str(r.name.clone())).ok()?;
                            rows.first().and_then(|row| match row.pk {
                                PropValue::Int(n) => Some(NodeId(n as u64)),
                                _ => None,
                            })
                        });

                        if let (Some(from), Some(to)) = (caller_node, target_node) {
                            let mut edge_props = Properties::new();
                            edge_props.insert("line".to_string(), PropValue::Int(r.line as i64));
                            let edge_type = if r.kind == "call" { "CALLS" } else { "REFERENCES" };
                            let mut g = batch.graph();
                            g.create_edge(from, edge_type, to, edge_props)?;
                        }
                    }
                }

                Ok(())
            })?;

            Ok(())
        })
    }

    pub fn delete_file(&mut self, path: &str) -> Result<bool> {
        dispatch_store!(self, db => {
            db.write_tx(|batch| {
                let prev_file = {
                    let mut rel = batch.relational();
                    let files = rel.table(&FILES_SCHEMA).select_eq("path", &PropValue::Str(path.to_string()))?;
                    files.first().map(|r| r.pk.clone())
                };

                match prev_file {
                    Some(prev_pk) => {
                        let prev_file_id = match prev_pk {
                            PropValue::Int(n) => n as u64,
                            _ => 0,
                        };
                        {
                            let mut rel = batch.relational();
                            let _ = rel.table(&FILES_SCHEMA).delete(&prev_pk);
                        }
                        {
                            let mut g = batch.graph();
                            let _ = g.cascade_delete(NodeId(prev_file_id), "CONTAINS");
                            let _ = g.delete_node(NodeId(prev_file_id));
                        }
                        {
                            let mut rel = batch.relational();
                            let prev_syms = rel.table(&SYMBOLS_SCHEMA).select_eq("file_id", &PropValue::Int(prev_file_id as i64))?;
                            for sym in prev_syms {
                                let _ = rel.table(&SYMBOLS_SCHEMA).delete(&sym.pk);
                            }
                        }
                        Ok(true)
                    }
                    None => Ok(false),
                }
            })
            .map_err(|e| anyhow!("{e}"))
        })
    }

    pub fn search_symbols(&self, query: &str, limit: usize) -> Result<Vec<SymbolSearchResult>> {
        dispatch_store!(self, db => {
            let rel = db.relational();
            let syms_tbl = rel.table(&SYMBOLS_SCHEMA);
            let rows = syms_tbl.select_prefix("name", query)?;

            let mut results = Vec::new();
            for r in rows.into_iter().take(limit) {
                let name = match r.get(&SYMBOLS_SCHEMA, "name") {
                    Some(PropValue::Str(s)) => s.clone(),
                    _ => continue,
                };
                let kind = match r.get(&SYMBOLS_SCHEMA, "kind") {
                    Some(PropValue::Str(s)) => s.clone(),
                    _ => "symbol".to_string(),
                };
                let file_path = match r.get(&SYMBOLS_SCHEMA, "file_path") {
                    Some(PropValue::Str(s)) => s.clone(),
                    _ => String::new(),
                };
                let line_start = match r.get(&SYMBOLS_SCHEMA, "line_start") {
                    Some(PropValue::Int(n)) => *n as u32,
                    _ => 0,
                };
                let signature = match r.get(&SYMBOLS_SCHEMA, "signature") {
                    Some(PropValue::Str(s)) => Some(s.clone()),
                    _ => None,
                };

                let symbol_id = match r.pk {
                    PropValue::Int(n) => n as u64,
                    _ => 0,
                };

                results.push(SymbolSearchResult {
                    symbol_id,
                    file_path,
                    name,
                    kind,
                    line_start,
                    signature,
                });
            }
            Ok(results)
        })
    }

    pub fn find_call_path(&self, from_symbol: &str, to_symbol: &str) -> Result<Option<CallPathResult>> {
        dispatch_store!(self, db => {
            let rel = db.relational();
            let syms_tbl = rel.table(&SYMBOLS_SCHEMA);
            let graph = db.graph();

            let from_rows = syms_tbl.select().where_eq("name", PropValue::Str(from_symbol.to_string())).limit(1).run()?;
            let to_rows = syms_tbl.select().where_eq("name", PropValue::Str(to_symbol.to_string())).limit(1).run()?;

            let from_node_id = match from_rows.first() {
                Some(r) => match r.pk {
                    PropValue::Int(n) => NodeId(n as u64),
                    _ => return Ok(None),
                },
                None => return Ok(None),
            };

            let to_node_id = match to_rows.first() {
                Some(r) => match r.pk {
                    PropValue::Int(n) => NodeId(n as u64),
                    _ => return Ok(None),
                },
                None => return Ok(None),
            };

            let filters = ["CALLS", "REFERENCES"];
            let path_opt = graph.find_shortest_path(from_node_id, to_node_id, Direction::Out, Some(&filters))?;

            match path_opt {
                Some(path) => {
                    let mut steps = Vec::new();
                    for window in path.steps.windows(2) {
                        let from_step = &window[0];
                        let to_step = &window[1];

                        let from_node = graph.get_node(from_step.node)?.ok_or_else(|| {
                            anyhow!("Node {}", from_step.node.0)
                        })?;
                        let to_node = graph.get_node(to_step.node)?.ok_or_else(|| {
                            anyhow!("Node {}", to_step.node.0)
                        })?;

                        let from_name = match from_node.properties.get("name") {
                            Some(PropValue::Str(s)) => s.clone(),
                            _ => format!("node_{}", from_step.node.0),
                        };
                        let to_name = match to_node.properties.get("name") {
                            Some(PropValue::Str(s)) => s.clone(),
                            _ => format!("node_{}", to_step.node.0),
                        };
                        let edge_type = to_step.edge_type.clone().unwrap_or_else(|| "CALLS".to_string());

                        steps.push(CallPathStep {
                            from_symbol: from_name,
                            to_symbol: to_name,
                            edge_type,
                        });
                    }

                    let distance = steps.len();
                    Ok(Some(CallPathResult {
                        distance,
                        steps,
                    }))
                }
                None => Ok(None),
            }
        })
    }

    pub fn analyze_impact(&self, symbol_name: &str, max_depth: usize) -> Result<ImpactResult> {
        dispatch_store!(self, db => {
            let rel = db.relational();
            let syms_tbl = rel.table(&SYMBOLS_SCHEMA);
            let graph = db.graph();

            let rows = syms_tbl.select().where_eq("name", PropValue::Str(symbol_name.to_string())).limit(1).run()?;
            let root_node_id = match rows.first() {
                Some(r) => match r.pk {
                    PropValue::Int(n) => NodeId(n as u64),
                    _ => return Ok(ImpactResult {
                        root_symbol: symbol_name.to_string(),
                        affected_callers: vec![],
                    }),
                },
                None => return Ok(ImpactResult {
                    root_symbol: symbol_name.to_string(),
                    affected_callers: vec![],
                }),
            };

            let mut visited = HashSet::new();
            let mut queue = VecDeque::new();
            let mut callers = Vec::new();

            visited.insert(root_node_id);
            queue.push_back((root_node_id, 0));

            while let Some((curr, depth)) = queue.pop_front() {
                if depth >= max_depth {
                    continue;
                }

                let neighbors = graph.neighbors_in_any(curr)?;
                for (edge_type, caller_node_id, _edge_id) in neighbors {
                    if edge_type == "CALLS" || edge_type == "REFERENCES" {
                        if visited.insert(caller_node_id) {
                            if let Some(node) = graph.get_node(caller_node_id)? {
                                let name = match node.properties.get("name") {
                                    Some(PropValue::Str(s)) => s.clone(),
                                    _ => format!("node_{}", caller_node_id.0),
                                };
                                let kind = match node.properties.get("kind") {
                                    Some(PropValue::Str(s)) => s.clone(),
                                    _ => node.label.clone(),
                                };
                                let file_path = match node.properties.get("file_path") {
                                    Some(PropValue::Str(s)) => s.clone(),
                                    _ => String::new(),
                                };
                                let line_start = match node.properties.get("line_start") {
                                    Some(PropValue::Int(n)) => *n as u32,
                                    _ => 0,
                                };

                                callers.push(ImpactNode {
                                    symbol_name: name,
                                    symbol_kind: kind,
                                    file_path,
                                    line_start,
                                    depth: depth + 1,
                                });

                                queue.push_back((caller_node_id, depth + 1));
                            }
                        }
                    }
                }
            }

            Ok(ImpactResult {
                root_symbol: symbol_name.to_string(),
                affected_callers: callers,
            })
        })
    }

    pub fn architecture_digest(&self, top_k: usize) -> Result<DigestResult> {
        dispatch_store!(self, db => {
            let rel = db.relational();
            let files_tbl = rel.table(&FILES_SCHEMA);
            let syms_tbl = rel.table(&SYMBOLS_SCHEMA);
            let graph = db.graph();

            let hubs = graph.top_hubs(top_k, Direction::In, None)?;

            let mut top_hubs = Vec::with_capacity(hubs.len());
            for (node_id, degree) in hubs {
                if let Some(node) = graph.get_node(node_id)? {
                    let name = match node.properties.get("name") {
                        Some(PropValue::Str(s)) => s.clone(),
                        _ => format!("node_{}", node_id.0),
                    };
                    let kind = match node.properties.get("kind") {
                        Some(PropValue::Str(s)) => s.clone(),
                        _ => node.label.clone(),
                    };
                    let file_path = match node.properties.get("file_path") {
                        Some(PropValue::Str(s)) => s.clone(),
                        _ => String::new(),
                    };

                    top_hubs.push(HubItem {
                        symbol_name: name,
                        symbol_kind: kind,
                        file_path,
                        incoming_calls: degree as u64,
                    });
                }
            }

            let all_files = files_tbl.select().limit(100_000).run()?;
            let all_syms = syms_tbl.select().limit(100_000).run()?;

            Ok(DigestResult {
                total_files: all_files.len(),
                total_symbols: all_syms.len(),
                top_hubs,
            })
        })
    }

    pub fn get_file(&self, path: &str) -> Result<Option<FileRecord>> {
        dispatch_store!(self, db => {
            let rel = db.relational();
            let files_tbl = rel.table(&FILES_SCHEMA);
            let rows = files_tbl.select().where_eq("path", PropValue::Str(path.to_string())).limit(1).run()?;
            match rows.first() {
                Some(r) => {
                    let id = match r.pk {
                        PropValue::Int(n) => n as u64,
                        _ => 0,
                    };
                    let file_path = match r.get(&FILES_SCHEMA, "path") {
                        Some(PropValue::Str(s)) => s.clone(),
                        _ => path.to_string(),
                    };
                    let hash = match r.get(&FILES_SCHEMA, "hash") {
                        Some(PropValue::Str(s)) => s.clone(),
                        _ => String::new(),
                    };
                    let language = match r.get(&FILES_SCHEMA, "lang") {
                        Some(PropValue::Str(s)) => s.clone(),
                        _ => "unknown".to_string(),
                    };
                    let ast_synced_at = match r.get(&FILES_SCHEMA, "synced_at") {
                        Some(PropValue::Int(n)) => *n,
                        _ => 0,
                    };

                    Ok(Some(FileRecord {
                        id,
                        path: file_path,
                        hash,
                        language,
                        ast_synced_at,
                        precise_synced_at: None,
                    }))
                }
                None => Ok(None),
            }
        })
    }

    pub fn list_files(&self) -> Result<Vec<FileRecord>> {
        dispatch_store!(self, db => {
            let rel = db.relational();
            let files_tbl = rel.table(&FILES_SCHEMA);
            let rows = files_tbl.select().limit(100_000).run()?;
            let mut files = Vec::with_capacity(rows.len());
            for r in rows {
                let id = match r.pk {
                    PropValue::Int(n) => n as u64,
                    _ => 0,
                };
                let path = match r.get(&FILES_SCHEMA, "path") {
                    Some(PropValue::Str(s)) => s.clone(),
                    _ => continue,
                };
                let hash = match r.get(&FILES_SCHEMA, "hash") {
                    Some(PropValue::Str(s)) => s.clone(),
                    _ => String::new(),
                };
                let language = match r.get(&FILES_SCHEMA, "lang") {
                    Some(PropValue::Str(s)) => s.clone(),
                    _ => "unknown".to_string(),
                };
                let ast_synced_at = match r.get(&FILES_SCHEMA, "synced_at") {
                    Some(PropValue::Int(n)) => *n,
                    _ => 0,
                };

                files.push(FileRecord {
                    id,
                    path,
                    hash,
                    language,
                    ast_synced_at,
                    precise_synced_at: None,
                });
            }
            Ok(files)
        })
    }
}
