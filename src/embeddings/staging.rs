//! Operation-owned disk staging. No incomplete file is published to the index.
use super::Chunk;
use anyhow::{Context, Result, ensure};
use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

const MARKER: &str = "code-rcl-embedding-staging-v1\n";
pub struct Staging {
    pub dir: tempfile::TempDir,
    _lock: Option<File>,
}
impl Staging {
    #[cfg(test)]
    pub fn abandon(self) -> PathBuf {
        self.dir.keep()
    }
    pub fn new(project: &Path) -> Result<Self> {
        let lock = crate::progress::operation_lock(project)?;
        let root = project.join(".code-rcl/embedding-staging");
        ensure!(
            root.parent()
                .unwrap()
                .canonicalize()?
                .starts_with(project.canonicalize()?),
            "staging parent escapes project"
        );
        if root.exists() {
            ensure!(
                !fs::symlink_metadata(&root)?.file_type().is_symlink(),
                "staging root is a symlink"
            );
        }
        fs::create_dir_all(&root)?;
        let root = root.canonicalize()?;
        ensure!(
            root.starts_with(project.canonicalize()?),
            "staging escapes project"
        );
        for entry in fs::read_dir(&root)? {
            let entry = entry?;
            if !entry
                .file_name()
                .to_string_lossy()
                .starts_with("operation-")
                || !entry.file_type()?.is_dir()
            {
                continue;
            }
            let path = entry.path();
            let marker = path.join("owner");
            if fs::symlink_metadata(&marker).is_ok_and(|m| m.file_type().is_file())
                && fs::read_to_string(&marker).is_ok_and(|s| s == MARKER)
            {
                // The shared operation lock proves that a previous owner is gone.
                ensure!(
                    path.canonicalize()?.starts_with(&root),
                    "foreign staging directory"
                );
                fs::remove_dir_all(path)?;
            }
        }
        let dir = tempfile::Builder::new()
            .prefix("operation-")
            .tempdir_in(root)?;
        fs::write(dir.path().join("owner"), MARKER)?;
        Ok(Self { dir, _lock: lock })
    }
    pub fn file_dir(&self, number: usize) -> Result<PathBuf> {
        let path = self.dir.path().join(number.to_string());
        fs::create_dir(&path)?;
        Ok(path)
    }
}
pub fn write_row(dir: &Path, ordinal: usize, chunk: &Chunk, vector: &[f32]) -> Result<()> {
    let _timer = crate::operation::time("embedding_staging");
    let file = File::create_new(dir.join(format!("{ordinal}.json")))?;
    let mut out = std::io::BufWriter::new(file);
    serde_json::to_writer(&mut out, &(chunk, vector))?;
    out.flush()?;
    Ok(())
}
pub fn rows(dir: &Path, count: usize) -> impl Iterator<Item = Result<(Chunk, Vec<f32>)>> + '_ {
    (0..count).map(move |i| {
        let file = File::open(dir.join(format!("{i}.json")))
            .context("incomplete staged embedding file")?;
        Ok(serde_json::from_reader(BufReader::new(file))?)
    })
}
pub fn read_chunk(reader: &mut BufReader<File>) -> Result<Option<Chunk>> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    Ok(Some(serde_json::from_str(&line)?))
}
