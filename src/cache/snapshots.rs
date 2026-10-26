//! Original source bytes, independent of the embedding runtime and tokenizer.
use super::{CacheDb, models::FileRow};
use anyhow::{Context, Result, ensure};
use bkndb::{
    BknError,
    value::{PropValue, Properties},
};

#[cfg(test)]
thread_local! {
    pub(crate) static FAIL_ANALYSIS_COMMIT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    pub(crate) static FAIL_MIGRATION_COMMIT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(super) fn properties(file_id: i64, hash: &str, bytes: &[u8]) -> Properties {
    Properties::from([
        ("file_id".into(), file_id.into()),
        ("source_hash".into(), hash.into()),
        ("content".into(), PropValue::Bytes(bytes.to_vec())),
    ])
}

impl CacheDb {
    /// Fill/repair derived source bytes without invalidating unchanged analysis or vectors.
    /// The caller supplies the exact buffer it just hashed during source sync.
    pub fn ensure_source_snapshot(&self, file: &FileRow, bytes: &[u8]) -> Result<bool> {
        let _timer = crate::operation::time("source_snapshot_fill");
        ensure!(
            blake3::hash(bytes).to_hex().as_str() == file.content_hash,
            "source changed while filling snapshot for {}; run source sync",
            file.path
        );
        let pk = PropValue::Int(file.id);
        let expected = properties(file.id, &file.content_hash, bytes);
        let (current, matches) = self.db.read_tx(|r| {
            let rel = r.relational();
            Ok((
                rel.table(self.tables.files.clone()).get(&pk)?,
                rel.table(self.tables.source_snapshots.clone())
                    .get(&pk)?
                    .is_some_and(|row| {
                        row.values.get("source_hash") == expected.get("source_hash")
                            && row.values.get("content") == expected.get("content")
                    }),
            ))
        })?;
        ensure!(
            current.is_some_and(|r| r.values.get("content_hash")
                == Some(&PropValue::from(file.content_hash.as_str()))
                && r.values.get("path") == Some(&PropValue::from(file.path.as_str()))),
            "file changed while filling source snapshot for {}; run source sync",
            file.path
        );
        if matches {
            return Ok(false);
        }
        self.db.write_tx(|b| {
            let mut rel = b.relational();
            let current = rel.table(self.tables.files.clone()).get(&pk)?;
            if !current.is_some_and(|r| {
                r.values.get("content_hash") == Some(&PropValue::from(file.content_hash.as_str()))
                    && r.values.get("path") == Some(&PropValue::from(file.path.as_str()))
            }) {
                return Err(BknError::Encoding(
                    "file changed while filling source snapshot; retry source sync".into(),
                ));
            }
            rel.table(self.tables.source_snapshots.clone())
                .upsert(expected.clone())?;
            Ok(())
        })?;
        Ok(true)
    }

    /// Load input only from the database. Repo reads elsewhere are freshness checks.
    pub fn source_snapshot(&self, file: &FileRow) -> Result<String> {
        let _timer = crate::operation::time("source_snapshot_read");
        let (current, row) = self.db.read_tx(|r| {
            let rel = r.relational();
            let pk = PropValue::Int(file.id);
            Ok((
                rel.table(self.tables.files.clone()).get(&pk)?,
                rel.table(self.tables.source_snapshots.clone()).get(&pk)?,
            ))
        })?;
        ensure!(
            current.is_some_and(|r| r.values.get("content_hash")
                == Some(&PropValue::from(file.content_hash.as_str()))
                && r.values.get("path") == Some(&PropValue::from(file.path.as_str()))),
            "source snapshot file identity changed for {}; run source sync",
            file.path
        );
        let row = row.with_context(|| {
            format!(
                "source snapshot missing for {}; run source sync (code-rcl sync)",
                file.path
            )
        })?;
        ensure!(
            row.values.get("source_hash") == Some(&PropValue::from(file.content_hash.as_str())),
            "source snapshot metadata mismatch for {}; run source sync",
            file.path
        );
        let Some(PropValue::Bytes(bytes)) = row.values.get("content") else {
            anyhow::bail!(
                "source snapshot payload invalid for {}; run source sync",
                file.path
            );
        };
        ensure!(
            blake3::hash(bytes).to_hex().as_str() == file.content_hash,
            "source snapshot content hash mismatch for {}; run source sync",
            file.path
        );
        String::from_utf8(bytes.clone())
            .with_context(|| format!("embedding requires UTF-8 source snapshot: {}", file.path))
    }
}
