//! bkndb-backed graph cache stored at `<project>/.code-rcl/cache.bkndb`.
//!
//! The cache is derived data (it is rebuilt from the source tree), so a schema
//! change simply drops and recreates every table; see [`schema`].

pub mod mappers;
pub mod models;
pub mod mutations;
pub mod queries;
pub mod schema;
pub mod utils;

#[allow(unused_imports)]
pub use utils::levenshtein;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
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
        let db = BknDb::open(&path).map_err(|e| match e {
            BknError::DatabaseLocked(_) => anyhow!(
                "{} is in use by another code-rcl process (serve, mcp or sync); \
                 wait for it to finish or stop it, then retry",
                path.display()
            ),
            other => anyhow::Error::new(other).context(format!("opening {}", path.display())),
        })?;

        let cache = Self {
            db,
            tables: Schemas::build()?,
        };
        cache.ensure_schema()?;
        Ok(cache)
    }

    /// Create the tables, or rebuild them from scratch when the file holds a
    /// different [`schema::SCHEMA_VERSION`] (including a file with no version,
    /// e.g. one left by an earlier layout).
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
