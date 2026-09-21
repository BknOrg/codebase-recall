//! Calls hiding inside macro arguments.
//!
//! tree-sitter hands back a macro's arguments as a `token_tree`: raw tokens,
//! not an expression tree. So `futures::try_join!(async { run_inner(..) })`
//! records exactly one reference — the macro name — and every call written
//! inside it is invisible. On a codebase like Jujutsu that is ~18k macro
//! invocations' worth of blind spot.
//!
//! The fix is to take the token text, re-parse it as a Rust fragment, walk the
//! result with the ordinary walker, and rebase the offsets back onto the real
//! file.

use std::cell::RefCell;

use tree_sitter::{Node, Parser, Tree};

use crate::analysis::ParsedFile;
use crate::analysis::scope::ScopeStack;

use super::Walker;

/// How far to follow macros nested in macros (`assert!(matches!(..))`).
const MAX_MACRO_DEPTH: u32 = 3;

/// Wrappers that turn token soup into something the grammar accepts.
///
/// The first reads the tokens as a comma-separated expression list, which is
/// what most macros hold (`try_join!(a, b)`, `write!(f, "{}", v)`, `vec![1,2]`).
/// The second reads them as statements, for the block-bodied ones
/// (`select! { a = f() => {} }`).
///
/// Neither prefix may contain a newline: line rebasing assumes fragment line 1
/// sits on the same source line as the opening delimiter.
const WRAPPERS: [(&str, &str); 2] = [("fn __m(){(", ")}"), ("fn __m(){", "}")];

thread_local! {
    /// One parser per thread, reused across every macro in the run. Building a
    /// fresh `Parser` for each of ~18k macros would dominate a sync.
    static MACRO_PARSER: RefCell<Option<Parser>> = const { RefCell::new(None) };
}

/// Run `f` with the shared fragment parser, or do nothing if the Rust grammar
/// cannot be loaded.
fn with_parser<T>(f: impl FnOnce(&mut Parser) -> T) -> Option<T> {
    MACRO_PARSER.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            let mut p = Parser::new();
            let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
            p.set_language(&language).ok()?;
            *slot = Some(p);
        }
        slot.as_mut().map(f)
    })
}

impl<'a> Walker<'a> {
    /// Parse a macro's `token_tree` as Rust and merge the references found
    /// inside it into the enclosing file's results.
    ///
    /// Only references are taken. Symbols and scopes are deliberately dropped:
    /// a macro body defines nothing the enclosing file should inherit, and
    /// feeding its scopes to `resolve_locals` would corrupt the real ones.
    /// String literals are left to [`Walker::collect_macro`]'s own scan, which
    /// attributes them to the macro rather than to an inner callee.
    pub fn collect_macro_body(&mut self, token_tree: Node) {
        if self.macro_depth >= MAX_MACRO_DEPTH {
            return;
        }
        // Strip the delimiters; each is one byte in Rust.
        let range = token_tree.byte_range();
        let (inner_start, inner_end) = (range.start + 1, range.end.saturating_sub(1));
        if inner_end <= inner_start {
            return;
        }
        let Ok(content) = std::str::from_utf8(&self.src[inner_start..inner_end]) else {
            return;
        };
        if content.trim().is_empty() {
            return;
        }

        let Some((fragment, tree, prefix_len)) = parse_fragment(content) else {
            return;
        };

        let mut sub = Walker {
            src: fragment.as_bytes(),
            out: ParsedFile::default(),
            stack: Vec::new(),
            sc: ScopeStack::new(fragment.len() as i64),
            macro_depth: self.macro_depth + 1,
        };
        sub.walk(tree.root_node());

        // Fragment line 1 is the line the opening delimiter sits on, because no
        // wrapper prefix contains a newline.
        let line_base = token_tree.start_position().row as i64;
        let content_span = prefix_len..prefix_len + content.len();

        for mut rf in sub.out.refs {
            let start = rf.start_byte as usize;
            if !content_span.contains(&start) {
                continue; // a node belonging to the wrapper itself
            }
            let shift = |b: i64| inner_start as i64 + (b - prefix_len as i64);
            rf.start_byte = shift(rf.start_byte);
            // `--precise` sends this offset to the language server, so it has
            // to land on the real token in the real file.
            rf.name_start_byte = rf
                .name_start_byte
                .filter(|b| content_span.contains(&(*b as usize)))
                .map(shift);
            rf.start_line += line_base;
            // The fragment has its own scope tree, so any local resolution done
            // inside it is meaningless here; the parent file's pass redoes it.
            rf.local_only = false;
            rf.resolved_local_symbol_index = None;
            self.out.refs.push(rf);
        }
    }
}

/// Wrap `content` so the Rust grammar accepts it, returning the wrapped text,
/// its tree, and the prefix length used. Picks whichever wrapper parses with
/// the fewest errors; gives up if a macro is genuinely not Rust.
fn parse_fragment(content: &str) -> Option<(String, Tree, usize)> {
    let mut best: Option<(String, Tree, usize, usize)> = None;
    for (prefix, suffix) in WRAPPERS {
        let text = format!("{prefix}{content}{suffix}");
        let Some(Some(tree)) = with_parser(|p| p.parse(&text, None)) else {
            continue;
        };
        if !tree.root_node().has_error() {
            return Some((text, tree, prefix.len()));
        }
        let errors = count_errors(tree.root_node());
        if best.as_ref().is_none_or(|(_, _, _, b)| errors < *b) {
            best = Some((text, tree, prefix.len(), errors));
        }
    }
    // A DSL macro can be arbitrarily un-Rust-like. Past a certain error density
    // whatever we extracted would be noise, so nothing is better than guessing.
    let (text, tree, prefix, errors) = best?;
    (errors <= MAX_FRAGMENT_ERRORS).then_some((text, tree, prefix))
}

/// Above this many error nodes the fragment is treated as not-Rust.
const MAX_FRAGMENT_ERRORS: usize = 4;

fn count_errors(node: Node) -> usize {
    if !node.has_error() {
        return 0;
    }
    let mut total = usize::from(node.is_error() || node.is_missing());
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        total += count_errors(child);
    }
    total
}
