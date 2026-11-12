//! Incremental source-content retrieval. All writes share the source/AST transaction.
use super::{CacheDb, models::NewSymbol, schema::Schemas};
use anyhow::{Context, Result};
use bkndb::{
    BknError, StorageWriteTx,
    relational::{Query, RelBatchView},
    value::{PropValue, Properties},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

pub const VERSION: &str = "content-v3";
pub(crate) fn doc_start(source: &str, start: usize) -> usize {
    let line_start = source[..start].rfind('\n').map_or(0, |i| i + 1);
    let mut at = line_start;
    let mut attached = start;
    while at > 0 {
        let end = at - 1;
        let prev = source[..end].rfind('\n').map_or(0, |i| i + 1);
        let line = source[prev..end].trim();
        if line.is_empty() {
            at = prev;
            continue;
        }
        if line.starts_with("//")
            || line.starts_with("/**")
            || line.starts_with('*')
            || line.starts_with('#')
            || line.ends_with("*/")
        {
            at = prev;
            attached = prev;
        } else {
            break;
        }
    }
    attached
}
pub fn terms(text: &str) -> Vec<String> {
    let mut spaced = String::with_capacity(text.len());
    let mut previous = None::<char>;
    for c in text.chars() {
        if c.is_uppercase() && previous.is_some_and(|p| p.is_lowercase() || p.is_numeric()) {
            spaced.push(' ');
        }
        spaced.push(c);
        previous = Some(c);
    }
    spaced
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.chars().count() > 1 && s.len() <= 256)
        .map(str::to_lowercase)
        .filter(|s| {
            ![
                "the", "and", "or", "with", "from", "into", "when", "where", "that", "of", "are",
                "how", "for", "in", "to", "is", "on", "by", "as", "at", "be", "this", "it", "yang",
                "dan", "atau", "di", "ke", "dari", "untuk", "pada", "saat", "dengan", "ini", "itu",
            ]
            .contains(&s.as_str())
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    pub key: String,
    pub path: String,
    pub name: String,
    pub kind: String,
    pub is_exported: bool,
    pub start_byte: usize,
    pub end_byte: usize,
    pub line: usize,
    pub end_line: usize,
    pub source_hash: String,
    pub length: usize,
}
pub struct Prepared<'a> {
    identities: HashMap<String, (Document, &'a str)>,
    documents: Vec<Properties>,
    postings: Vec<Properties>,
    length: usize,
}
pub fn prepare<'a>(
    path: &str,
    hash: &str,
    source: Option<&'a [u8]>,
    symbols: &[NewSymbol],
) -> Result<Prepared<'a>> {
    let mut batch = Prepared {
        identities: HashMap::new(),
        documents: vec![],
        postings: vec![],
        length: 0,
    };
    let Some(source) = source.and_then(|s| std::str::from_utf8(s).ok()) else {
        return Ok(batch);
    };
    let mut ranges = Vec::new();
    for s in symbols {
        crate::operation::check()?;
        let (Ok(lo), Ok(hi)) = (usize::try_from(s.start_byte), usize::try_from(s.end_byte)) else {
            continue;
        };
        if lo >= hi || source.get(lo..hi).is_none() {
            continue;
        }
        let start = doc_start(source, lo);
        ranges.push((start, hi));
        let key = format!("{path}:{}:{}:{lo}:{hi}", s.kind, s.name);
        batch.add(
            Document {
                key,
                path: path.into(),
                name: s.name.clone(),
                kind: s.kind.to_string(),
                is_exported: s.is_exported,
                start_byte: start,
                end_byte: hi,
                line: source[..start].bytes().filter(|b| *b == b'\n').count() + 1,
                end_line: source[..hi].bytes().filter(|b| *b == b'\n').count() + 1,
                source_hash: hash.into(),
                length: 0,
            },
            &source[start..hi],
        )?;
    }
    ranges.sort_unstable();
    let mut at = 0;
    for (lo, hi) in ranges
        .into_iter()
        .chain(std::iter::once((source.len(), source.len())))
    {
        if lo > at && !terms(&source[at..lo]).is_empty() {
            let name = std::path::Path::new(path)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            batch.add(
                Document {
                    key: format!("{path}:file:{name}:{at}:{lo}"),
                    path: path.into(),
                    name,
                    kind: "file".into(),
                    is_exported: false,
                    start_byte: at,
                    end_byte: lo,
                    line: source[..at].bytes().filter(|b| *b == b'\n').count() + 1,
                    end_line: source[..lo].bytes().filter(|b| *b == b'\n').count() + 1,
                    source_hash: hash.into(),
                    length: 0,
                },
                &source[at..lo],
            )?;
        }
        at = at.max(hi);
    }
    Ok(batch)
}
impl<'a> Prepared<'a> {
    fn add(&mut self, mut doc: Document, source: &'a str) -> Result<()> {
        if let Some((previous, text)) = self.identities.get(&doc.key) {
            anyhow::ensure!(
                previous == &doc && *text == source,
                "conflicting search document identity {:?} in {} (kind {}, bytes {}..{}); analysis was not committed",
                doc.key,
                doc.path,
                doc.kind,
                doc.start_byte,
                doc.end_byte
            );
            return Ok(());
        }
        self.identities
            .insert(doc.key.clone(), (doc.clone(), source));
        let mut counts = BTreeMap::<String, usize>::new();
        for term in terms(&format!("{} {} {source}", doc.path, doc.name)) {
            *counts.entry(term).or_default() += 1;
        }
        doc.length = counts.values().sum();
        self.length += doc.length;
        for (term, tf) in counts {
            let key = blake3::hash(format!("{term}\n{}", doc.key).as_bytes())
                .to_hex()
                .to_string();
            self.postings.push(Properties::from([
                ("key".into(), key.into()),
                ("term".into(), term.into()),
                ("document".into(), doc.key.clone().into()),
                ("frequency".into(), (tf as i64).into()),
            ]));
        }
        self.documents.push(Properties::from([
            ("key".into(), doc.key.clone().into()),
            ("length".into(), (doc.length as i64).into()),
            ("payload".into(), serde_json::to_string(&doc)?.into()),
        ]));
        Ok(())
    }
    pub(super) fn write<W: StorageWriteTx>(
        &self,
        rel: &mut RelBatchView<'_, W>,
        t: &Schemas,
        file_id: i64,
        hash: &str,
    ) -> std::result::Result<(), BknError> {
        remove(rel, t, file_id)?;
        let rows = self.documents.iter().chain(&self.postings).count();
        if rows > 0 {
            let stamp = |row: &Properties| {
                let mut r = row.clone();
                r.insert("file_id".into(), file_id.into());
                r
            };
            if !self.documents.is_empty() {
                rel.table(t.search_documents.clone())
                    .insert_bulk(self.documents.iter().map(stamp))?;
            }
            if !self.postings.is_empty() {
                rel.table(t.search_postings.clone())
                    .insert_bulk(self.postings.iter().map(stamp))?;
            }
        }
        rel.table(t.search_files.clone()).upsert(Properties::from([
            ("file_id".into(), file_id.into()),
            ("marker".into(), format!("{VERSION}:{hash}").into()),
            ("documents".into(), (self.documents.len() as i64).into()),
            ("length".into(), (self.length as i64).into()),
        ]))?;
        Ok(())
    }
}
pub(super) fn remove<W: StorageWriteTx>(
    rel: &mut RelBatchView<'_, W>,
    t: &Schemas,
    file_id: i64,
) -> std::result::Result<(), BknError> {
    for table in [&t.search_documents, &t.search_postings] {
        rel.table(table.clone())
            .delete_where_eq("file_id", &file_id.into())?;
    }
    rel.table(t.search_files.clone()).delete(&file_id.into())?;
    Ok(())
}
fn int(values: &Properties, key: &str) -> i64 {
    match values.get(key) {
        Some(PropValue::Int(v)) => *v,
        _ => 0,
    }
}
fn marker_id(row: &bkndb::relational::Row) -> i64 {
    match &row.pk {
        PropValue::Int(id) => *id,
        _ => 0,
    }
}
fn string<'a>(values: &'a Properties, key: &str) -> &'a str {
    match values.get(key) {
        Some(PropValue::Str(v)) => v,
        _ => "",
    }
}
impl CacheDb {
    pub fn ensure_content_index(
        &self,
        file: &super::models::FileRow,
        bytes: &[u8],
    ) -> Result<bool> {
        let marker = format!("{VERSION}:{}", file.content_hash);
        let current = self.db.read_tx(|r| {
            r.relational()
                .table(self.tables.search_files.clone())
                .get(&file.id.into())
        })?;
        anyhow::ensure!(
            blake3::hash(bytes).to_hex().as_str() == file.content_hash,
            "source changed while indexing {}",
            file.path
        );
        if current.is_some_and(|r| string(&r.values, "marker") == marker) {
            return Ok(false);
        }
        let symbols = self
            .symbols_for_file(file.id)?
            .into_iter()
            .map(|s| NewSymbol {
                semantic: Default::default(),
                name: s.name,
                kind: s.kind,
                is_exported: s.is_exported,
                start_byte: s.start_byte.unwrap_or(-1),
                end_byte: s.end_byte.unwrap_or(-1),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let prepared = prepare(&file.path, &file.content_hash, Some(bytes), &symbols)?;
        self.db.write_tx(|b| {
            let mut rel = b.relational();
            let row = rel
                .table(self.tables.files.clone())
                .get(&file.id.into())?
                .ok_or_else(|| BknError::Encoding("source file disappeared".into()))?;
            if string(&row.values, "content_hash") != file.content_hash {
                return Err(BknError::Encoding(
                    "source changed during content backfill".into(),
                ));
            }
            prepared.write(&mut rel, &self.tables, file.id, &file.content_hash)
        })?;
        Ok(true)
    }
    pub fn content_candidates(
        &self,
        query: &str,
        accepts: impl Fn(&Document) -> bool,
        limit: usize,
    ) -> Result<Vec<(Document, f64)>> {
        let _timer = crate::operation::time("content_retrieval");
        let mut query_terms = terms(query);
        query_terms.sort();
        query_terms.dedup();
        let files = self
            .all_files()?
            .into_iter()
            .map(|f| (f.id, f))
            .collect::<HashMap<_, _>>();
        self.db
            .read_tx(|r| {
                let rel = r.relational();
                let stamps = rel
                    .table(self.tables.search_files.clone())
                    .find(&Query::new())?;
                let valid: HashSet<_> = stamps
                    .iter()
                    .filter(|row| {
                        files.get(&marker_id(row)).is_some_and(|f| {
                            string(&row.values, "marker") == format!("{VERSION}:{}", f.content_hash)
                        })
                    })
                    .map(marker_id)
                    .collect();
                if valid.len()<files.len() {
                    crate::operation::partial();
                    crate::operation::reason("content_coverage",serde_json::json!({"valid_files":valid.len(),"total_files":files.len(),"recovery":"run code-rcl sync"}));
                }
                let n = stamps
                    .iter()
                    .filter(|row| valid.contains(&marker_id(row)))
                    .map(|row| int(&row.values, "documents") as f64)
                    .sum::<f64>();
                let length = stamps
                    .iter()
                    .filter(|row| valid.contains(&marker_id(row)))
                    .map(|row| int(&row.values, "length") as f64)
                    .sum::<f64>();
                let average = (length / n.max(1.0)).max(1.0);
                let mut scores = HashMap::<String, (Document, f64)>::new();
                for term in &query_terms {
                    if crate::operation::cancelled() {
                        return Err(BknError::Encoding("operation cancelled".into()));
                    }
                    let postings = rel
                        .table(self.tables.search_postings.clone())
                        .select_eq("term", &term.as_str().into())?;
                    let df = postings
                        .iter()
                        .filter(|p| valid.contains(&int(&p.values, "file_id")))
                        .count() as f64;
                    let idf = (1.0 + (n - df + 0.5).max(0.0) / (df + 0.5)).ln();
                    for p in postings {
                        let file_id = int(&p.values, "file_id");
                        if !valid.contains(&file_id) {
                            continue;
                        }
                        let key = string(&p.values, "document");
                        if !scores.contains_key(key) {
                            let Some(row) = rel
                                .table(self.tables.search_documents.clone())
                                .get(&key.into())?
                            else {
                                return Err(BknError::Encoding(
                                    "content posting has no document; run source sync".into(),
                                ));
                            };
                            let doc: Document =
                                serde_json::from_str(string(&row.values, "payload"))
                                    .map_err(|e| BknError::Encoding(e.to_string()))?;
                            if !accepts(&doc) {
                                continue;
                            }
                            scores.insert(key.to_string(), (doc, 0.0));
                        }
                        let (doc, score) = scores.get_mut(key).unwrap();
                        let tf = int(&p.values, "frequency") as f64;
                        *score += idf * tf * 2.2
                            / (tf + 1.2 * (0.25 + 0.75 * doc.length as f64 / average));
                    }
                }
                let mut hits = scores.into_values().collect::<Vec<_>>();
                hits.sort_by(|a, b| {
                    b.1.total_cmp(&a.1)
                        .then(a.0.path.cmp(&b.0.path))
                        .then(a.0.start_byte.cmp(&b.0.start_byte))
                        .then(a.0.key.cmp(&b.0.key))
                });
                crate::operation::omitted_for(
                    hits.len().saturating_sub(limit),
                    "content_candidate_limit",
                );
                hits.truncate(limit);
                Ok(hits)
            })
            .context("searching source content index")
    }
}

#[cfg(test)]
mod duplicate_tests {
    use super::*;
    #[test]
    fn identical_documents_are_counted_once_but_conflicts_are_rejected() {
        let source = "pub fn known() {}";
        let parsed = crate::analysis::parse_file(crate::analysis::Language::Rust, source);
        let one = prepare("lib.rs", "hash", Some(source.as_bytes()), &parsed.symbols).unwrap();
        let mut symbols = parsed.symbols.clone();
        symbols.extend(parsed.symbols.clone());
        let duplicate = prepare("lib.rs", "hash", Some(source.as_bytes()), &symbols).unwrap();
        assert_eq!(one.documents, duplicate.documents);
        assert_eq!(one.postings, duplicate.postings);
        assert_eq!(one.length, duplicate.length);
        symbols.last_mut().unwrap().is_exported = false;
        assert!(
            prepare("lib.rs", "hash", Some(source.as_bytes()), &symbols)
                .err()
                .unwrap()
                .to_string()
                .contains("conflicting search document identity")
        );
        let doc = Document {
            key: "test".into(),
            path: "lib.rs".into(),
            name: "known".into(),
            kind: "function".into(),
            is_exported: false,
            start_byte: 0,
            end_byte: 1,
            line: 1,
            end_line: 1,
            source_hash: "hash".into(),
            length: 0,
        };
        let mut batch = Prepared {
            identities: HashMap::new(),
            documents: vec![],
            postings: vec![],
            length: 0,
        };
        batch.add(doc.clone(), "alpha").unwrap();
        assert!(batch.add(doc, "bravo").is_err());
    }
}
