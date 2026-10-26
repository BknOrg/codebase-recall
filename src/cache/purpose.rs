//! Independently versioned Gemma purpose vectors. Base embedding rows stay intact.
use super::{CacheDb, models::FileRow, schema::Schemas};
use anyhow::{Result, ensure};
use bkndb::{
    BknError, StorageWriteTx,
    relational::{Query, RelBatchView},
    value::{PropValue, Properties},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Context {
    pub symbol_key: String,
    pub path: String,
    pub name: String,
    pub kind: String,
    pub source_hash: String,
    pub input_hash: String,
    pub text: String,
}

#[derive(Clone)]
pub struct Row {
    pub context: Context,
    pub vector: Vec<f32>,
    pub config_key: String,
}

fn text<'a>(p: &'a Properties, key: &str) -> std::result::Result<&'a str, BknError> {
    match p.get(key) {
        Some(PropValue::Str(v)) => Ok(v),
        _ => Err(BknError::Encoding(format!("invalid purpose {key}"))),
    }
}

pub(super) fn remove<W: StorageWriteTx>(
    rel: &mut RelBatchView<'_, W>,
    t: &Schemas,
    file_id: i64,
) -> std::result::Result<(), BknError> {
    rel.table(t.purpose_contexts.clone())
        .delete_where_eq("file_id", &file_id.into())?;
    rel.table(t.purpose_files.clone()).delete(&file_id.into())?;
    Ok(())
}

impl CacheDb {
    pub fn purpose_marker(&self, file_id: i64) -> Result<Option<String>> {
        Ok(self.db.read_tx(|tx| {
            Ok(tx
                .relational()
                .table(self.tables.purpose_files.clone())
                .get(&file_id.into())?
                .and_then(|r| {
                    r.values.get("marker").and_then(|v| {
                        if let PropValue::Str(v) = v {
                            Some(v.clone())
                        } else {
                            None
                        }
                    })
                }))
        })?)
    }
    pub fn purpose_rows(&self) -> Result<Vec<Row>> {
        self.rows(&self.tables.purpose_contexts, &Query::new())?
            .into_iter()
            .map(|r| {
                let context = serde_json::from_str(text(&r.values, "payload")?)?;
                let config_key = text(&r.values, "config_key")?.to_owned();
                let Some(PropValue::Bytes(bytes)) = r.values.get("vector") else {
                    anyhow::bail!("missing purpose vector")
                };
                ensure!(bytes.len() % 4 == 0, "invalid purpose vector bytes");
                let vector = bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| f32::from_le_bytes(*b))
                    .collect();
                Ok(Row {
                    context,
                    vector,
                    config_key,
                })
            })
            .collect()
    }
    pub fn purpose_vector(
        &self,
        input_hash: &str,
        config_key: &str,
        text_input: &str,
    ) -> Result<Option<Vec<f32>>> {
        for r in self.rows(
            &self.tables.purpose_contexts,
            &Query::new()
                .where_eq("input_hash", input_hash)
                .where_eq("config_key", config_key),
        )? {
            let context: Context = serde_json::from_str(text(&r.values, "payload")?)?;
            if context.text != text_input {
                continue;
            }
            if let Some(PropValue::Bytes(bytes)) = r.values.get("vector") {
                ensure!(bytes.len() % 4 == 0, "invalid cached purpose vector");
                return Ok(Some(
                    bytes
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|b| f32::from_le_bytes(*b))
                        .collect(),
                ));
            }
        }
        Ok(None)
    }
    pub fn commit_purpose_file(
        &self,
        file: &FileRow,
        config_key: &str,
        marker: &str,
        rows: &[(Context, Vec<f32>)],
    ) -> Result<()> {
        self.db.write_tx(|tx| {
            let mut rel = tx.relational();
            let current = rel
                .table(self.tables.files.clone())
                .get(&file.id.into())?
                .ok_or_else(|| BknError::Encoding("purpose source deleted".into()))?;
            if text(&current.values, "content_hash")? != file.content_hash {
                return Err(BknError::Encoding("purpose source changed".into()));
            }
            remove(&mut rel, &self.tables, file.id)?;
            for (context, vector) in rows {
                if context.path != file.path
                    || context.source_hash != file.content_hash
                    || ![256, 768].contains(&vector.len())
                    || vector.iter().any(|v| !v.is_finite())
                    || !vector.iter().any(|v| *v != 0.0)
                    || context.input_hash
                        != blake3::hash(format!("{config_key}\n{}", context.text).as_bytes())
                            .to_hex()
                            .as_str()
                {
                    return Err(BknError::Encoding("invalid purpose input or vector".into()));
                }
                let payload = serde_json::to_string(context)
                    .map_err(|e| BknError::Encoding(e.to_string()))?;
                rel.table(self.tables.purpose_contexts.clone())
                    .insert(Properties::from([
                        ("key".into(), context.symbol_key.clone().into()),
                        ("file_id".into(), file.id.into()),
                        ("config_key".into(), config_key.into()),
                        ("input_hash".into(), context.input_hash.clone().into()),
                        ("payload".into(), payload.into()),
                        (
                            "vector".into(),
                            PropValue::Bytes(vector.iter().flat_map(|v| v.to_le_bytes()).collect()),
                        ),
                    ]))?;
            }
            rel.table(self.tables.purpose_files.clone())
                .upsert(Properties::from([
                    ("file_id".into(), file.id.into()),
                    ("marker".into(), marker.into()),
                    ("contexts".into(), (rows.len() as i64).into()),
                ]))?;
            #[cfg(test)]
            if FAIL_COMMIT.with(|f| f.replace(false)) {
                return Err(BknError::Encoding("injected purpose commit failure".into()));
            }
            Ok(())
        })?;
        Ok(())
    }
}

#[cfg(test)]
thread_local! {pub(crate) static FAIL_COMMIT:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
