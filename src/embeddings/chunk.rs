use super::{Chunk, Embedder};
use crate::cache::models::SymbolRow;
use crate::config::EmbeddingChunkProfile;
use anyhow::{Result, ensure};
use std::{borrow::Cow, collections::HashSet};
pub fn symbol_key(path: &str, s: &SymbolRow) -> String {
    format!(
        "{}:{}:{}:{}:{}",
        path,
        s.kind,
        s.name,
        s.start_byte.unwrap_or(0),
        s.end_byte.unwrap_or(0)
    )
}
fn owner(s: &SymbolRow, symbols: &[SymbolRow]) -> String {
    let mut id = s.parent_symbol_id;
    let mut names = vec![];
    let mut seen = HashSet::new();
    while let Some(i) = id {
        if !seen.insert(i) {
            break;
        }
        let Some(p) = symbols.iter().find(|p| p.id == i) else {
            break;
        };
        names.push(p.name.clone());
        id = p.parent_symbol_id;
    }
    names.reverse();
    names.join("::")
}
// Keep the existing document embedding input unchanged; scoring/content
// retrieval attach broader documentation independently.
fn doc_start(source: &str, start: usize) -> usize {
    let line_start = source[..start].rfind('\n').map_or(0, |i| i + 1);
    let mut at = line_start;
    while at > 0 {
        let prev_end = at.saturating_sub(1);
        let prev = source[..prev_end].rfind('\n').map_or(0, |i| i + 1);
        let line = source[prev..prev_end].trim();
        if line.starts_with("///")
            || line.starts_with("//!")
            || line.starts_with("/**")
            || line.starts_with('*')
            || line.starts_with("//")
            || line.starts_with('#')
            || line.ends_with("*/")
        {
            at = prev;
        } else {
            break;
        }
    }
    if at < line_start { at } else { start }
}
#[cfg(test)]
pub fn chunks(
    path: &str,
    source_hash: &str,
    source: &str,
    symbols: &[SymbolRow],
    tokenizer: &dyn Embedder,
    key: &str,
) -> Result<Vec<Chunk>> {
    profile_chunks(
        path,
        source_hash,
        source,
        symbols,
        tokenizer,
        key,
        EmbeddingChunkProfile::Full,
    )
}
pub fn profile_chunks(
    path: &str,
    source_hash: &str,
    source: &str,
    symbols: &[SymbolRow],
    tokenizer: &dyn Embedder,
    key: &str,
    profile: EmbeddingChunkProfile,
) -> Result<Vec<Chunk>> {
    let mut result = Vec::new();
    visit_profile(
        path,
        source_hash,
        source,
        symbols,
        tokenizer,
        key,
        profile,
        &mut |chunk| {
            result.push(chunk);
            Ok(())
        },
    )?;
    Ok(result)
}
#[allow(clippy::too_many_arguments)]
pub fn visit_profile(
    path: &str,
    source_hash: &str,
    source: &str,
    symbols: &[SymbolRow],
    tokenizer: &dyn Embedder,
    key: &str,
    profile: EmbeddingChunkProfile,
    emit: &mut dyn FnMut(Chunk) -> Result<()>,
) -> Result<()> {
    let _timer = crate::operation::time("chunking_tokenization");
    let bodies = if profile == EmbeddingChunkProfile::Compact {
        crate::analysis::embedding_bodies::parse(path, source)
    } else {
        None
    };
    for s in symbols {
        crate::operation::check()?;
        let (Some(start), Some(end)) = (s.start_byte, s.end_byte) else {
            continue;
        };
        let (Ok(start), Ok(end)) = (usize::try_from(start), usize::try_from(end)) else {
            continue;
        };
        if start >= end
            || end > source.len()
            || !source.is_char_boundary(start)
            || !source.is_char_boundary(end)
        {
            continue;
        }
        let declaration_start = start;
        let start = doc_start(source, start);
        let summary = if profile == EmbeddingChunkProfile::Compact
            && super::compact::container(&s.kind)
        {
            let value = super::compact::summary(s, symbols, source, tokenizer, bodies.as_ref())?;
            let original = tokenizer.input_tokens(&source[start..end])?;
            let actual = if let Some(text) = &value {
                // Adjacent documentation is retained in the synthetic body too.
                tokenizer.input_tokens(&format!("{}{text}", &source[start..declaration_start]))?
            } else {
                original
            };
            crate::operation::container_tokens(original, actual, value.is_none());
            crate::progress::update(|v| v["containers"] = crate::operation::container_stats());
            value
        } else {
            None
        };
        let body: Cow<'_, str> = match &summary {
            Some(text) => Cow::Owned(format!("{}{text}", &source[start..declaration_start])),
            None => Cow::Borrowed(&source[start..end]),
        };
        let owner = owner(s, symbols);
        let signature = s
            .signature
            .as_deref()
            .unwrap_or_else(|| source[declaration_start..end].lines().next().unwrap_or(""));
        let mut header = format!(
            "title: {path} | text: {} {} {}\n{}\n",
            owner, s.name, s.kind, signature
        );
        let prefix_tokens = tokenizer.offsets(&header)?;
        if prefix_tokens.len() > 128 {
            header.truncate(prefix_tokens[127].1);
            header.push('\n');
        }
        let budget = 510usize.saturating_sub(tokenizer.offsets(&header)?.len());
        ensure!(budget > 64, "embedding metadata leaves no room for source");
        let tokens = tokenizer.offsets(&body)?;
        let mut at = 0;
        let mut ordinal = 0;
        while at < tokens.len() {
            let mut until = (at + budget).min(tokens.len());
            let begin = tokens[at].0;
            let text = loop {
                let finish = tokens[until - 1].1;
                ensure!(
                    body.is_char_boundary(begin) && body.is_char_boundary(finish),
                    "tokenizer returned non-UTF8 source boundary"
                );
                let text = format!("{header}{}", &body[begin..finish]);
                if tokenizer.offsets(&text)?.len() <= 510 {
                    break text;
                }
                ensure!(until > at + 1, "source token exceeds chunk budget");
                until -= 1;
            };
            let finish = tokens[until - 1].1;
            let (lo, hi) = if summary.is_some() {
                // Generated-text offsets cannot identify source bytes. Return an
                // explicitly marked, bounded excerpt from the original symbol.
                let limit = source[start..end]
                    .char_indices()
                    .nth(1600)
                    .map_or(end, |(i, _)| start + i);
                (start, limit)
            } else {
                (start + begin, start + finish)
            };
            let input_hash = blake3::hash(format!("{key}\n{text}").as_bytes())
                .to_hex()
                .to_string();
            emit(Chunk {
                path: path.into(),
                name: s.name.clone(),
                owner: owner.clone(),
                kind: s.kind.to_string(),
                is_exported: s.is_exported,
                symbol_key: symbol_key(path, s),
                chunk_index: ordinal,
                start_byte: lo,
                end_byte: hi,
                start_line: source[..lo].bytes().filter(|b| *b == b'\n').count() + 1,
                end_line: source[..hi].bytes().filter(|b| *b == b'\n').count() + 1,
                source_hash: source_hash.into(),
                input_hash,
                text,
                snippet: source[lo..hi].into(),
                representation: summary.as_ref().map(|_| "container_summary".into()),
                symbol_range: summary.as_ref().map(|_| [declaration_start, end]),
            })?;
            if until == tokens.len() {
                break;
            }
            at = until.saturating_sub(64).max(at + 1);
            ordinal += 1;
        }
    }
    Ok(())
}
