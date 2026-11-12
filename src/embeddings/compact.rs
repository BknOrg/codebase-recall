//! Quality-gated experiment: synthetic container text never becomes a source snippet.
use super::Embedder;
use crate::{
    cache::models::SymbolRow,
    config::{EmbeddingChunkProfile, EmbeddingsConfig},
};
use anyhow::{Result, ensure};
pub fn check_profile(cfg: &EmbeddingsConfig) -> Result<()> {
    ensure!(
        cfg.chunk_profile == EmbeddingChunkProfile::Full
            || cfg!(test)
            || std::env::var_os("CODE_RCL_EXPERIMENTAL_COMPACT").as_deref()
                == Some(std::ffi::OsStr::new("1")),
        "compact profile has not passed quality gates; available only in the experimental benchmark harness (CODE_RCL_EXPERIMENTAL_COMPACT=1)"
    );
    Ok(())
}
pub fn container(kind: &str) -> bool {
    matches!(
        kind,
        "impl" | "class" | "interface" | "trait" | "struct" | "enum" | "module" | "object"
    )
}
/// None means full fallback, not a silently incomplete container summary.
pub fn summary(
    s: &SymbolRow,
    symbols: &[SymbolRow],
    source: &str,
    tokenizer: &dyn Embedder,
    bodies: Option<&crate::analysis::embedding_bodies::Bodies>,
) -> Result<Option<String>> {
    let Some(bodies) = bodies else {
        return Ok(None);
    };
    let (Some(start), Some(end)) = (s.start_byte, s.end_byte) else {
        return Ok(None);
    };
    let (Ok(start), Ok(end)) = (usize::try_from(start), usize::try_from(end)) else {
        return Ok(None);
    };
    if source.get(start..end).is_none() {
        return Ok(None);
    }
    if !bodies.contains_key(&(start, end)) {
        return Ok(None);
    }
    let mut remove = Vec::new();
    for child in symbols {
        if !container(&child.kind)
            && !matches!(
                child.kind.as_str(),
                "function" | "method" | "constructor" | "destructor"
            )
        {
            continue;
        }
        let (Some(lo), Some(hi)) = (child.start_byte, child.end_byte) else {
            continue;
        };
        let (Ok(lo), Ok(hi)) = (usize::try_from(lo), usize::try_from(hi)) else {
            return Ok(None);
        };
        if lo < start || hi > end || (lo == start && hi == end) {
            continue;
        }
        let Some(text) = source.get(lo..hi) else {
            return Ok(None);
        };
        // A removed child must independently produce a nonempty source chunk.
        if tokenizer.offsets(text)?.is_empty() {
            return Ok(None);
        }
        let Some(spans) = bodies.get(&(lo, hi)) else {
            return Ok(None);
        };
        if spans.len() > 1 {
            return Ok(None);
        }
        if let Some(&(a, b)) = spans.first() {
            if a < lo
                || b > hi
                || a >= b
                || !source.is_char_boundary(a)
                || !source.is_char_boundary(b)
            {
                return Ok(None);
            }
            remove.push((a, b));
        }
    }
    remove.sort_by_key(|&(a, b)| (a, std::cmp::Reverse(b)));
    let mut summary = String::new();
    let mut cursor = start;
    for (a, b) in remove {
        if b <= cursor {
            continue;
        } // body of a child already delegated as a whole
        if a < cursor {
            return Ok(None);
        } // overlapping AST mappings are ambiguous
        summary.push_str(&source[cursor..a]);
        summary.push_str(" /* independently indexed body */ ");
        cursor = b;
    }
    summary.push_str(&source[cursor..end]);
    Ok(Some(summary))
}
