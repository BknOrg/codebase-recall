//! Bounded cross-file length scheduling; exactly one native inference call at a time.
use super::{Chunk, Embedder, chunk, normalize, scope, staging};
use crate::{
    cache::{CacheDb, models::FileRow},
    config::ProjectConfig,
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    fs::{self, File},
    io::{BufReader, BufWriter, Write},
    path::{Path, PathBuf},
};

const FILES: usize = 8;
const INPUTS: usize = 128;
const BUFFER: usize = 64 * 1024 * 1024;
const BUCKETS: [usize; 5] = [64, 128, 256, 384, 512];
struct Active {
    file: FileRow,
    dir: PathBuf,
    reader: BufReader<File>,
    waiting: usize,
    total: usize,
    read: usize,
}
struct Target {
    file: usize,
    ordinal: usize,
    chunk: Chunk,
}
struct Input {
    tokens: usize,
    age: usize,
    targets: Vec<Target>,
}
impl Input {
    fn chunk(&self) -> &Chunk {
        &self.targets[0].chunk
    }
    fn bucket(&self) -> usize {
        BUCKETS.iter().position(|b| self.tokens <= *b).unwrap_or(4)
    }
    fn bytes(&self) -> usize {
        self.targets.capacity() * std::mem::size_of::<Target>()
            + self
                .targets
                .iter()
                .map(|t| {
                    let c = &t.chunk;
                    // Include retained capacities, metadata and native-input copies/results.
                    [
                        &c.path,
                        &c.name,
                        &c.owner,
                        &c.kind,
                        &c.symbol_key,
                        &c.source_hash,
                        &c.input_hash,
                        &c.text,
                        &c.snippet,
                    ]
                    .iter()
                    .map(|s| s.capacity())
                    .sum::<usize>()
                        + 2 * c.text.len()
                        + 768 * 4 * 2
                })
                .sum::<usize>()
    }
}
fn select(pending: &[Input], batch: usize, size: usize) -> Vec<usize> {
    let bucket = if batch.is_multiple_of(16) {
        pending.iter().min_by_key(|p| p.age).unwrap().bucket()
    } else {
        pending.iter().map(Input::bucket).min().unwrap()
    };
    let mut candidates: Vec<_> = pending
        .iter()
        .enumerate()
        .filter(|(_, p)| p.bucket() == bucket)
        .map(|(i, _)| i)
        .collect();
    candidates.sort_by(|a, b| {
        let (a, b) = (&pending[*a], &pending[*b]);
        let (x, y) = (a.chunk(), b.chunk());
        (
            a.tokens,
            &x.path,
            &x.symbol_key,
            x.chunk_index,
            &x.input_hash,
        )
            .cmp(&(
                b.tokens,
                &y.path,
                &y.symbol_key,
                y.chunk_index,
                &y.input_hash,
            ))
    });
    candidates.truncate(size);
    candidates
}
#[cfg(all(test, feature = "embeddings"))]
pub(super) fn benchmark_order(
    chunks: &[Chunk],
    embedder: &dyn Embedder,
    batch_size: usize,
) -> Result<Vec<Vec<usize>>> {
    ensure!(
        chunks.len() <= INPUTS,
        "benchmark queue exceeds production limit"
    );
    let mut pending = chunks
        .iter()
        .enumerate()
        .map(|(ordinal, c)| {
            Ok(Input {
                tokens: embedder.input_tokens(&c.text)?.min(512),
                age: ordinal,
                targets: vec![Target {
                    file: 0,
                    ordinal,
                    chunk: c.clone(),
                }],
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut result = Vec::new();
    while !pending.is_empty() {
        let selected = select(&pending, result.len() + 1, batch_size);
        result.push(
            selected
                .iter()
                .map(|i| pending[*i].targets[0].ordinal)
                .collect(),
        );
        let mut selected = selected;
        selected.sort_by_key(|i| std::cmp::Reverse(*i));
        for i in selected {
            pending.remove(i);
        }
    }
    Ok(result)
}
fn verify(project: &Path, file: &FileRow, cfg: &ProjectConfig) -> Result<()> {
    crate::operation::check()?;
    ensure!(
        ProjectConfig::load(project)? == *cfg,
        "configuration changed during embedding; retry"
    );
    let bytes = fs::read(project.join(&file.path))?;
    ensure!(
        blake3::hash(&bytes).to_hex().as_str() == file.content_hash,
        "source changed during embedding: {}; retry",
        file.path
    );
    Ok(())
}
pub fn index(
    db: &CacheDb,
    project: &Path,
    full: bool,
    embedder: &mut dyn Embedder,
    cfg: &ProjectConfig,
    key: &str,
) -> Result<Value> {
    index_bounded(db, project, full, embedder, cfg, key, BUFFER)
}
pub(super) fn index_bounded(
    db: &CacheDb,
    project: &Path,
    full: bool,
    embedder: &mut dyn Embedder,
    cfg: &ProjectConfig,
    key: &str,
    buffer: usize,
) -> Result<Value> {
    let stage = staging::Staging::new(project)?;
    let mut files = scope(db, cfg)?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let total_files = files.len();
    let mut files = files.into_iter().enumerate();
    let mut active: Vec<Option<Active>> = (0..FILES).map(|_| None).collect();
    let mut pending: Vec<Input> = Vec::new();
    let (mut embedded, mut unique, mut dedup, mut reused, mut unchanged, mut done) =
        (0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
    let (mut batch, mut sequence, mut actual_tokens, mut padded_tokens, mut peak, mut staged_bytes) =
        (0usize, 0usize, 0usize, 0usize, 0usize, 0u64);
    let mut exhausted = false;
    crate::progress::update(|v| v["files_total"] = json!(total_files));
    loop {
        crate::operation::check()?;
        // At most eight source snapshots are staged, never eight in-memory result sets.
        for slot in &mut active {
            while slot.is_none() && !exhausted {
                let Some((number, file)) = files.next() else {
                    exhausted = true;
                    break;
                };
                if !file.parsed_ok {
                    crate::operation::warning(format!(
                        "embedding skipped parser-error file: {}",
                        file.path
                    ));
                    done += 1;
                    continue;
                }
                if !full
                    && db
                        .meta_get(&format!("embedding_file:{}", file.path))?
                        .as_deref()
                        == Some(format!("{key}:{}", file.content_hash).as_str())
                {
                    unchanged += 1;
                    done += 1;
                    continue;
                }
                verify(project, &file, cfg)?;
                crate::progress::phase("chunking");
                crate::progress::update(|v| v["file"] = json!(file.path));
                let source = db.source_snapshot(&file)?;
                let mut symbols = db.symbols_for_file(file.id)?;
                symbols.sort_by_key(|s| chunk::symbol_key(&file.path, s));
                let dir = stage.file_dir(number)?;
                let spool = dir.join("inputs.jsonl");
                let mut out = BufWriter::new(File::create_new(&spool)?);
                let mut total = 0;
                chunk::visit_profile(
                    &file.path,
                    &file.content_hash,
                    &source,
                    &symbols,
                    embedder,
                    key,
                    cfg.embeddings.chunk_profile,
                    &mut |c| {
                        crate::operation::check()?;
                        serde_json::to_writer(&mut out, &c)?;
                        out.write_all(b"\n")?;
                        total += 1;
                        Ok(())
                    },
                )?;
                out.flush()?;
                drop(out);
                staged_bytes += fs::metadata(&spool)?.len();
                *slot = Some(Active {
                    file,
                    dir,
                    reader: BufReader::new(File::open(spool)?),
                    waiting: 0,
                    total,
                    read: 0,
                });
            }
        }
        // Round-robin generators keep small files represented in mixed batches.
        loop {
            let mut advanced = false;
            for (slot, file) in active.iter_mut().enumerate() {
                if pending.iter().map(|p| p.targets.len()).sum::<usize>() >= INPUTS {
                    break;
                }
                // A 512-token UTF-8 input has bounded model size, but retain a large
                // reserve for tokenizers with unusually long single tokens.
                let resident: usize = pending.iter().map(Input::bytes).sum();
                if resident > buffer / 2 {
                    break;
                }
                let Some(file) = file.as_mut() else { continue };
                if file.read == file.total {
                    continue;
                }
                let c = staging::read_chunk(&mut file.reader)?.context("truncated chunk spool")?;
                let ordinal = file.read;
                file.read += 1;
                advanced = true;
                if let Some(vector) = db.embedding_vector(&c.input_hash, key, &c.text)? {
                    ensure!(
                        vector.len() == cfg.embeddings.dimensions
                            && vector.iter().all(|v| v.is_finite()),
                        "invalid reused vector; run embeddings sync --full"
                    );
                    staging::write_row(&file.dir, ordinal, &c, &vector)?;
                    reused += 1;
                    continue;
                }
                let tokens = embedder.input_tokens(&c.text)?;
                ensure!(tokens <= 512, "embedding input exceeds 512 tokens");
                file.waiting += 1;
                let target = Target {
                    file: slot,
                    ordinal,
                    chunk: c,
                };
                if let Some(input) = pending.iter_mut().find(|p| {
                    p.chunk().input_hash == target.chunk.input_hash
                        && p.chunk().text == target.chunk.text
                }) {
                    input.targets.push(target);
                    dedup += 1;
                } else {
                    sequence += 1;
                    pending.push(Input {
                        tokens,
                        age: sequence,
                        targets: vec![target],
                    });
                }
                let resident: usize =
                    pending.iter().map(Input::bytes).sum::<usize>() + FILES * 8192;
                ensure!(
                    resident <= buffer,
                    "single embedding input exceeds scheduler buffer; reduce source symbol size"
                );
                peak = peak.max(resident);
            }
            if !advanced
                || pending.iter().map(|p| p.targets.len()).sum::<usize>() >= INPUTS
                || pending.iter().map(Input::bytes).sum::<usize>() > buffer / 2
            {
                break;
            }
        }
        if !pending.is_empty() {
            batch += 1;
            let selected = select(&pending, batch, cfg.embeddings.batch_size);
            let texts = selected
                .iter()
                .map(|i| pending[*i].chunk().text.clone())
                .collect();
            actual_tokens += selected.iter().map(|i| pending[*i].tokens).sum::<usize>();
            padded_tokens +=
                selected.iter().map(|i| pending[*i].tokens).max().unwrap() * selected.len();
            crate::progress::phase("inference");
            let batch_files = selected
                .iter()
                .flat_map(|i| pending[*i].targets.iter().map(|t| t.chunk.path.clone()))
                .collect::<std::collections::BTreeSet<_>>();
            crate::progress::update(|v| {
                v["batch_files"] = json!(batch_files);
                v["file"] = if batch_files.len() == 1 {
                    json!(batch_files.first())
                } else {
                    Value::Null
                };
                v["batch"] = json!(batch);
                v["active_files"] = json!(active.iter().flatten().count());
                v["pending_inputs"] = json!(pending.len());
                v["scheduler_peak_bytes"] = json!(peak);
            });
            crate::operation::check()?;
            let vectors = embedder.embed(texts)?;
            ensure!(
                vectors.len() == selected.len(),
                "embedding batch returned wrong item count"
            );
            // Validate the entire batch before staging any of its outputs.
            let vectors = vectors
                .iter()
                .map(|v| normalize(v, cfg.embeddings.dimensions))
                .collect::<Result<Vec<_>>>()?;
            crate::operation::check()?;
            crate::progress::phase("staging");
            let mut finished = selected.into_iter().zip(vectors).collect::<Vec<_>>();
            finished.sort_by_key(|(i, _)| std::cmp::Reverse(*i));
            for (i, vector) in finished {
                let input = pending.remove(i);
                unique += 1;
                for target in input.targets {
                    let file = active[target.file].as_mut().unwrap();
                    staging::write_row(&file.dir, target.ordinal, &target.chunk, &vector)?;
                    file.waiting -= 1;
                    embedded += 1;
                }
            }
            crate::progress::completed();
        }
        for slot in &mut active {
            if slot
                .as_ref()
                .is_some_and(|f| f.read == f.total && f.waiting == 0)
            {
                let file = slot.take().unwrap();
                verify(project, &file.file, cfg)?;
                crate::progress::phase("commit");
                db.commit_embedding_stream(
                    &file.file.path,
                    &file.file.content_hash,
                    key,
                    file.total,
                    staging::rows(&file.dir, file.total),
                )?;
                done += 1;
                crate::progress::update(|v| {
                    v["committed_chunks"] =
                        json!(v["committed_chunks"].as_u64().unwrap_or(0) + file.total as u64)
                });
                drop(file.reader);
                fs::remove_dir_all(file.dir)?;
                crate::progress::completed();
            }
        }
        crate::progress::update(|v| {
            v["active_files"] = json!(active.iter().flatten().count());
            v["pending_inputs"] = json!(pending.len());
            v["files_done"] = json!(done);
            v["embedded_chunks"] = json!(embedded);
            v["unique_inferences"] = json!(unique);
            v["deduplicated_chunks"] = json!(dedup);
            v["reused_chunks"] = json!(reused);
            v["actual_tokens"] = json!(actual_tokens);
            v["padded_tokens"] = json!(padded_tokens);
            v["staging_bytes_written"] = json!(staged_bytes);
        });
        if exhausted && active.iter().all(Option::is_none) {
            break;
        }
    }
    Ok(
        json!({"batching":"length","embedded_chunks":embedded,"unique_inferences":unique,"deduplicated_chunks":dedup,"reused_chunks":reused,"unchanged_files":unchanged,"batches":batch,"actual_tokens":actual_tokens,"padded_tokens":padded_tokens,"batch_capacity":batch*cfg.embeddings.batch_size,"scheduler_peak_bytes":peak,"staging_input_bytes_written":staged_bytes}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input(tokens: usize, age: usize) -> Input {
        Input {
            tokens,
            age,
            targets: vec![Target {
                file: 0,
                ordinal: 0,
                chunk: Chunk {
                    path: format!("{age}.rs"),
                    name: "f".into(),
                    owner: "".into(),
                    kind: "function".into(),
                    is_exported: false,
                    symbol_key: age.to_string(),
                    chunk_index: 0,
                    start_byte: 0,
                    end_byte: 1,
                    start_line: 1,
                    end_line: 1,
                    source_hash: "h".into(),
                    input_hash: age.to_string(),
                    text: "t".into(),
                    snippet: "s".into(),
                    representation: None,
                    symbol_range: None,
                },
            }],
        }
    }
    #[test]
    fn shortest_first_and_every_sixteenth_oldest_bucket() {
        let p = vec![input(500, 1), input(20, 3), input(50, 2), input(100, 4)];
        assert_eq!(select(&p, 1, 4), vec![1, 2]);
        assert_eq!(select(&p, 16, 4), vec![0]);
        assert_eq!(select(&p, 17, 1), vec![1]);
    }
}
