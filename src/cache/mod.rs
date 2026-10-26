//! bkndb-backed graph cache stored at `<project>/.code-rcl/cache.bkndb`.
//!
//! The cache is derived data (it is rebuilt from the source tree), so a schema
//! change can migrate existing rows or rebuild incompatible layouts; see [`schema`].

pub mod content;
pub mod embeddings;
pub mod mappers;
pub mod models;
pub mod mutations;
pub(crate) mod purpose;
pub mod queries;
pub mod schema;
pub mod snapshots;
pub mod utils;

#[allow(unused_imports)]
pub use utils::levenshtein;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use bkndb::value::{PropValue, Properties};
use bkndb::{BknDb, BknError};

use schema::Schemas;

/// Directory that holds all code-rcl project state.
pub const CODE_CTX_DIR: &str = ".code-rcl";
/// Database file name inside [`CODE_CTX_DIR`].
pub const BKNDB_FILE: &str = "cache.bkndb";

pub struct CacheDb {
    pub(crate) db: BknDb,
    pub(crate) tables: Schemas,
}

/// Absolute path to `<project>/.code-rcl`.
pub fn ctx_dir(project_root: &Path) -> PathBuf {
    project_root.join(CODE_CTX_DIR)
}

/// Absolute path to `<project>/.code-rcl/cache.bkndb`.
pub fn bkndb_path(project_root: &Path) -> PathBuf {
    ctx_dir(project_root).join(BKNDB_FILE)
}

impl CacheDb {
    /// Open (creating `.code-rcl/` and the database if needed) and make sure
    /// the schema is current.
    ///
    /// bkndb locks the file exclusively, so a second code-rcl process opening
    /// the same project while this one is alive fails with a clear message.
    pub fn open(project_root: &Path) -> Result<Self> {
        let dir = ctx_dir(project_root);
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;

        let path = bkndb_path(project_root);
        let db = match BknDb::open(&path) {
            Ok(db) => db,
            Err(BknError::DatabaseLocked(_)) => {
                anyhow::bail!(
                    "{} is in use by another code-rcl process (serve, mcp or sync); \
                     wait for it to finish or stop it, then retry",
                    path.display()
                );
            }
            Err(e @ BknError::Corruption(_)) => {
                // If the cache file has an incompatible format (e.g. upgrading between bkndb versions)
                // or corrupted engine data, discard the cache and recreate it fresh.
                eprintln!(
                    "cache database at {} has incompatible or corrupted format ({e}); recreating fresh cache...",
                    path.display()
                );
                let quarantine = path.with_extension(format!(
                    "corrupt-{}-{}",
                    utils::unix_now(),
                    std::process::id()
                ));
                std::fs::rename(&path, &quarantine)
                    .with_context(|| format!("quarantining {}", path.display()))?;
                BknDb::open(&path).map_err(|e2| {
                    anyhow::Error::new(e2).context(format!("recreating {}", path.display()))
                })?
            }
            Err(e) => {
                return Err(anyhow::Error::new(e).context(format!("opening {}", path.display())));
            }
        };

        let cache = Self {
            db,
            tables: Schemas::build()?,
        };
        cache.ensure_schema()?;
        Ok(cache)
    }

    /// Migrate schemas 10/11 additively; rebuild other incompatible derived layouts
    /// (including a cache with no recorded version).
    fn ensure_schema(&self) -> Result<()> {
        let want = schema::SCHEMA_VERSION.to_string();

        let current = self
            .db
            .read_tx(|r| {
                let rel = r.relational();
                if rel.table_schema(schema::META)?.is_none() {
                    return Ok(None);
                }
                let row = rel
                    .table_named(schema::META)?
                    .get(&PropValue::from(schema::SCHEMA_VERSION_KEY))?;
                Ok(row.and_then(|r| match r.values.get("value") {
                    Some(PropValue::Str(v)) => Some(v.clone()),
                    _ => None,
                }))
            })
            .context("reading cache schema version")?;
        if current.as_deref() == Some(want.as_str()) {
            self.db
                .write_tx(|tx| {
                    let mut rel = tx.relational();
                    for table in [&self.tables.purpose_contexts, &self.tables.purpose_files] {
                        if rel.table_schema(table.name())?.is_none() {
                            rel.create_table(table.clone())?;
                        }
                    }
                    Ok(())
                })
                .context("adding purpose-context cache without rebuilding existing indexes")?;
            return Ok(());
        }

        if matches!(current.as_deref(), Some("10" | "11")) {
            self.db
                .write_tx(|b| {
                    let mut rel = b.relational();
                    if current.as_deref() == Some("10")
                        && rel.table_schema(schema::SOURCE_SNAPSHOTS)?.is_some()
                    {
                        return Err(BknError::Encoding(
                            "schema 10 has an unexpected source_snapshots table; migration aborted"
                                .into(),
                        ));
                    }
                    if current.as_deref() == Some("10") {
                        rel.create_table(self.tables.source_snapshots.clone())?;
                    }
                    for table in [
                        &self.tables.search_documents,
                        &self.tables.search_postings,
                        &self.tables.search_files,
                        &self.tables.purpose_contexts,
                        &self.tables.purpose_files,
                    ] {
                        // A conflicting derived table is not safe to overwrite.
                        if rel.table_schema(table.name())?.is_some() {
                            return Err(BknError::Encoding(
                                "unexpected search table; migration aborted".into(),
                            ));
                        }
                        rel.create_table(table.clone())?;
                    }
                    #[cfg(test)]
                    if snapshots::FAIL_MIGRATION_COMMIT.with(|flag| flag.replace(false)) {
                        return Err(BknError::Encoding(
                            "injected migration commit failure".into(),
                        ));
                    }
                    let mut row = Properties::new();
                    row.insert("key".into(), schema::SCHEMA_VERSION_KEY.into());
                    row.insert("value".into(), want.clone().into());
                    rel.table(self.tables.meta.clone()).upsert(row)?;
                    Ok(())
                })
                .context("migrating cache schema to 12 without rebuilding existing indexes")?;
            return Ok(());
        }

        self.db
            .write_tx(|b| {
                let mut rel = b.relational();
                for t in rel.list_tables()? {
                    rel.drop_table(t.name())?;
                }
                for s in self.tables.all() {
                    rel.create_table(s.clone())?;
                }
                let mut row = Properties::new();
                row.insert("key".into(), schema::SCHEMA_VERSION_KEY.into());
                row.insert("value".into(), want.clone().into());
                rel.table(self.tables.meta.clone()).upsert(row)?;
                Ok(())
            })
            .context("initializing cache schema")?;
        Ok(())
    }
}
