//! SQLite-backed graph cache stored at `<project>/.code-rcl/cache.db`.

pub mod mappers;
pub mod models;
pub mod mutations;
pub mod queries;
pub mod schema;
pub mod utils;

#[allow(unused_imports)]
pub use utils::levenshtein;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::Connection;

/// Directory that holds all code-rcl project state.
pub const CODE_CTX_DIR: &str = ".code-rcl";
/// Cache database file name inside [`CODE_CTX_DIR`].
pub const DB_FILE: &str = "cache.db";

pub struct CacheDb {
    pub(crate) conn: Connection,
}

/// Absolute path to `<project>/.code-rcl`.
pub fn ctx_dir(project_root: &Path) -> PathBuf {
    project_root.join(CODE_CTX_DIR)
}

/// Absolute path to `<project>/.code-rcl/cache.db`.
pub fn db_path(project_root: &Path) -> PathBuf {
    ctx_dir(project_root).join(DB_FILE)
}

impl CacheDb {
    /// Open (creating `.code-rcl/` and the database if needed) and migrate.
    pub fn open(project_root: &Path) -> Result<Self> {
        let dir = ctx_dir(project_root);
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;

        let conn = Connection::open(db_path(project_root))
            .with_context(|| format!("opening {}", db_path(project_root).display()))?;
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;

        let mut db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&mut self) -> Result<()> {
        let mut current: i64 = self
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))?;

        while (current as usize) < schema::MIGRATIONS.len() {
            let script = schema::MIGRATIONS[current as usize];
            let tx = self.conn.transaction()?;
            tx.execute_batch(script)
                .with_context(|| format!("applying migration to v{}", current + 1))?;
            tx.commit()?;
            current += 1;
            self.conn.pragma_update(None, "user_version", current)?;
        }

        debug_assert_eq!(current, schema::SCHEMA_VERSION);
        Ok(())
    }
}
