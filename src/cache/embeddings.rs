//! Durable symbol chunks live beside the AST cache, never keyed by transient symbol IDs.
use super::CacheDb;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub path: String,
    pub name: String,
    pub owner: String,
    pub kind: String,
    pub is_exported: bool,
    pub symbol_key: String,
    pub chunk_index: usize,
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: usize,
    pub end_line: usize,
    pub source_hash: String,
    pub input_hash: String,
    pub text: String,
    pub snippet: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub representation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol_range: Option<[usize; 2]>,
}

use anyhow::Result;
use bkndb::{
    relational::Query,
    value::{PropValue, Properties},
};

#[derive(Clone)]
pub struct StoredChunk {
    pub chunk: Chunk,
    pub vector: Vec<f32>,
    pub valid: bool,
    pub config_key: String,
}
pub struct EmbeddingHeader {
    pub path: String,
    pub source_hash: String,
    pub config_key: String,
    pub valid: bool,
}
impl CacheDb {
    /// Reuse only exact inputs in the same model/dimension/pipeline namespace.
    pub fn embedding_vector(
        &self,
        input_hash: &str,
        key: &str,
        text: &str,
    ) -> Result<Option<Vec<f32>>> {
        for row in self.rows(
            &self.tables.embedding_chunks,
            &Query::new()
                .where_eq("input_hash", input_hash)
                .where_eq("config_key", key)
                .limit(1),
        )? {
            if row.values.get("config_key") != Some(&PropValue::from(key)) {
                continue;
            }
            let Some(PropValue::Str(payload)) = row.values.get("payload") else {
                anyhow::bail!("invalid embedding payload");
            };
            let chunk: Chunk = serde_json::from_str(payload)?;
            if chunk.text != text {
                continue;
            }
            let Some(PropValue::Bytes(bytes)) = row.values.get("vector") else {
                anyhow::bail!("invalid embedding vector");
            };
            anyhow::ensure!(bytes.len() % 4 == 0, "invalid embedding vector bytes");
            return Ok(Some(
                bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| f32::from_le_bytes(*b))
                    .collect(),
            ));
        }
        Ok(None)
    }
    /// Coverage needs four small columns, not source payloads or decoded float vectors.
    pub fn embedding_headers(&self) -> Result<Vec<EmbeddingHeader>> {
        let _timer = crate::operation::time("embedding_headers");
        self.rows(
            &self.tables.embedding_chunks,
            &Query::new().columns(["path", "source_hash", "config_key", "valid"]),
        )?
        .into_iter()
        .map(|row| {
            let text = |key: &str| match row.values.get(key) {
                Some(PropValue::Str(v)) => Ok(v.clone()),
                _ => anyhow::bail!("invalid embedding header: {key}"),
            };
            Ok(EmbeddingHeader {
                path: text("path")?,
                source_hash: text("source_hash")?,
                config_key: text("config_key")?,
                valid: row.values.get("valid") == Some(&PropValue::Bool(true)),
            })
        })
        .collect()
    }
    /// Upgrade derived counters without inference; publish the format marker last.
    pub fn ensure_embedding_counts(&self) -> Result<()> {
        if self.meta_get("embedding_count_format")?.as_deref() == Some("1") {
            return Ok(());
        }
        let mut counts: std::collections::BTreeMap<String, (String, String, usize, usize)> =
            Default::default();
        for row in self.embedding_headers()? {
            let count = counts.entry(row.path).or_insert((
                row.source_hash.clone(),
                row.config_key.clone(),
                0,
                0,
            ));
            anyhow::ensure!(
                count.0 == row.source_hash && count.1 == row.config_key,
                "mixed legacy embedding snapshots; rebuild with embeddings sync --full"
            );
            count.2 += 1;
            count.3 += usize::from(row.valid);
        }
        self.db.write_tx(|b| {
            let mut rel=b.relational();
            for (path,(hash,key,total,valid)) in &counts {
                let mut row=Properties::new();
                row.insert("key".into(),format!("embedding_count:{path}").into());
                row.insert("value".into(),serde_json::json!({"source_hash":hash,"config_key":key,"total":total,"valid":valid}).to_string().into());
                rel.table(self.tables.meta.clone()).upsert(row)?;
            }
            let mut row=Properties::new();row.insert("key".into(),"embedding_count_format".into());row.insert("value".into(),"1".into());rel.table(self.tables.meta.clone()).upsert(row)?;
            Ok(())
        })?;
        Ok(())
    }
    pub fn embedding_chunks(&self, path: Option<&str>) -> Result<Vec<StoredChunk>> {
        let _timer = crate::operation::time("embedding_rows_decode");
        let query = path.map_or_else(Query::new, |p| Query::new().where_eq("path", p));
        self.rows(&self.tables.embedding_chunks, &query)?
            .into_iter()
            .map(|row| {
                let text = |k: &str| match row.values.get(k) {
                    Some(PropValue::Str(s)) => Ok(s.clone()),
                    _ => anyhow::bail!("invalid embedding column: {k}"),
                };
                let chunk = serde_json::from_str(&text("payload")?)?;
                let vector = match row.values.get("vector") {
                    Some(PropValue::Bytes(bytes)) if bytes.len() % 4 == 0 => bytes
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|b| f32::from_le_bytes(*b))
                        .collect(),
                    _ => anyhow::bail!("invalid embedding vector bytes"),
                };
                Ok(StoredChunk {
                    chunk,
                    vector,
                    valid: row.values.get("valid") == Some(&PropValue::Bool(true)),
                    config_key: text("config_key")?,
                })
            })
            .collect()
    }
    pub fn invalidate_embedding_file(&self, path: &str) -> Result<()> {
        self.db.write_tx(|b| {
            let mut rel = b.relational();
            for r in rel
                .table(self.tables.embedding_chunks.clone())
                .select_eq("path", &PropValue::from(path))?
            {
                rel.table(self.tables.embedding_chunks.clone())
                    .update(&r.pk, |v| {
                        v.insert("valid".into(), false.into());
                    })?;
            }
            rel.table(self.tables.meta.clone())
                .delete(&PropValue::from(format!("embedding_file:{path}")))?;
            let mut excluded = Properties::new();
            excluded.insert("key".into(), format!("embedding_excluded:{path}").into());
            excluded.insert("value".into(), "true".into());
            rel.table(self.tables.meta.clone()).upsert(excluded)?;
            Ok(())
        })?;
        Ok(())
    }
    pub fn clear_embedding_index(&self) -> Result<()> {
        self.db.write_tx(|b| {
            let mut rel = b.relational();
            for table in [&self.tables.purpose_contexts, &self.tables.purpose_files] {
                for row in rel.table(table.clone()).find(&Query::new())? {
                    rel.table(table.clone()).delete(&row.pk)?;
                }
            }
            for r in rel
                .table(self.tables.embedding_chunks.clone())
                .find(&Query::new())?
            {
                rel.table(self.tables.embedding_chunks.clone())
                    .delete(&r.pk)?;
            }
            for r in rel.table(self.tables.meta.clone()).find(&Query::new())? {
                if matches!(&r.pk, PropValue::Str(k) if k.starts_with("embedding_file:") || k.starts_with("embedding_count:")) {
                    rel.table(self.tables.meta.clone()).delete(&r.pk)?;
                }
            }
            Ok(())
        })?;
        Ok(())
    }
    pub fn commit_embedding_file(
        &self,
        path: &str,
        source_hash: &str,
        config_key: &str,
        chunks: &[(Chunk, Vec<f32>)],
    ) -> Result<()> {
        self.commit_embedding_stream(
            path,
            source_hash,
            config_key,
            chunks.len(),
            chunks.iter().cloned().map(Ok),
        )
    }
    /// Read staged rows inside one transaction: decoding/I/O failure rolls it all back.
    pub fn commit_embedding_stream(
        &self,
        path: &str,
        source_hash: &str,
        config_key: &str,
        count: usize,
        chunks: impl IntoIterator<Item = Result<(Chunk, Vec<f32>)>>,
    ) -> Result<()> {
        let _timer = crate::operation::time("embedding_commit");
        let generation = self
            .meta_get("embedding_generation")?
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0)
            + 1;
        self.db.write_tx(|b| {
            let mut rel = b.relational();
            rel.table(self.tables.embedding_chunks.clone())
                .delete_where_eq("path", &PropValue::from(path))?;
            let mut received = 0;
            for row in chunks {
                let (chunk, vector) = row.map_err(|e| bkndb::BknError::Encoding(e.to_string()))?;
                if chunk.path != path || chunk.source_hash != source_hash
                    || blake3::hash(format!("{config_key}\n{}",chunk.text).as_bytes()).to_hex().as_str()!=chunk.input_hash
                    || vector.iter().any(|v|!v.is_finite())
                {
                    return Err(bkndb::BknError::Encoding("invalid staged chunk identity or vector".into()));
                }
                received += 1;
                let payload = serde_json::to_string(&chunk)
                    .map_err(|e| bkndb::BknError::Encoding(e.to_string()))?;
                let mut row = Properties::new();
                for (k, v) in [
                    ("path", path),
                    ("source_hash", source_hash),
                    ("input_hash", chunk.input_hash.as_str()),
                    ("config_key", config_key),
                    ("payload", payload.as_str()),
                ] {
                    row.insert(k.into(), v.into());
                }
                row.insert("valid".into(), true.into());
                row.insert(
                    "vector".into(),
                    PropValue::Bytes(vector.iter().flat_map(|v| v.to_le_bytes()).collect()),
                );
                rel.table(self.tables.embedding_chunks.clone())
                    .insert(row)?;
            }
            if received != count {
                return Err(bkndb::BknError::Encoding("incomplete embedding staging".into()));
            }
            for (key, value) in [
                (format!("embedding_count:{path}"),serde_json::json!({"source_hash":source_hash,"config_key":config_key,"total":count,"valid":count}).to_string()),
                (
                    format!("embedding_file:{path}"),
                    format!("{config_key}:{source_hash}"),
                ),
                ("embedding_generation".into(), generation.to_string()),
            ] {
                let mut row = Properties::new();
                row.insert("key".into(), key.into());
                row.insert("value".into(), value.into());
                rel.table(self.tables.meta.clone()).upsert(row)?;
            }
            Ok(())
        })?;
        Ok(())
    }
}
