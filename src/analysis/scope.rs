//! Shared lexical-scope machinery used by every language analyzer.
//!
//! An analyzer builds a [`ScopeStack`] as it walks the AST: `push` on entering a
//! function / class / block, `bind` on every name it declares, `pop` on the way
//! out. After the walk, [`resolve_locals`] does a same-file name resolution pass
//! so a reference that binds to a local, a parameter, or a sibling top-level
//! symbol is settled before the cross-file resolver ever sees it.

use crate::analysis::ParsedFile;
use crate::cache::models::{NewBinding, NewScope};

/// Builds the scope tree and its bindings during an AST walk.
pub struct ScopeStack {
    pub scopes: Vec<NewScope>,
    pub bindings: Vec<NewBinding>,
    /// Indices into `scopes`: root -> ... -> current.
    stack: Vec<usize>,
}

impl ScopeStack {
    /// Start with a single module scope spanning the whole file.
    pub fn new(file_len: i64) -> Self {
        let mut s = Self {
            scopes: Vec::new(),
            bindings: Vec::new(),
            stack: Vec::new(),
        };
        s.scopes.push(NewScope {
            parent_index: None,
            owner_symbol_index: None,
            kind: "module".to_string(),
            start_byte: 0,
            end_byte: file_len.max(0),
        });
        s.stack.push(0);
        s
    }

    /// Index of the scope currently on top of the stack.
    pub fn current(&self) -> usize {
        *self.stack.last().unwrap_or(&0)
    }

    /// Enter a new child scope.
    pub fn push(
        &mut self,
        kind: &str,
        owner_symbol_index: Option<usize>,
        start_byte: i64,
        end_byte: i64,
    ) {
        let idx = self.scopes.len();
        self.scopes.push(NewScope {
            parent_index: Some(self.current()),
            owner_symbol_index,
            kind: kind.to_string(),
            start_byte,
            end_byte,
        });
        self.stack.push(idx);
    }

    /// Leave the current scope (the root module scope is never popped).
    pub fn pop(&mut self) {
        if self.stack.len() > 1 {
            self.stack.pop();
        }
    }

    /// Record a name introduced in the current scope.
    pub fn bind(
        &mut self,
        name: &str,
        kind: &str,
        symbol_index: Option<usize>,
        import_index: Option<usize>,
        type_expr: Option<String>,
    ) {
        if name.is_empty() {
            return;
        }
        self.bindings.push(NewBinding {
            scope_index: self.current(),
            name: name.to_string(),
            binding_kind: kind.to_string(),
            symbol_index,
            import_index,
            type_expr,
        });
    }

    /// Move the built tree into a [`ParsedFile`].
    pub fn finish_into(self, out: &mut ParsedFile) {
        out.scopes = self.scopes;
        out.bindings = self.bindings;
    }
}

/// Same-file resolution pass. For every **bare** reference (`foo()`, no
/// receiver), walk the scope chain outward from the reference's position and
/// settle it against the first matching binding:
///
/// * `local` / `param` -> `local_only = true` (never a cross-symbol edge)
/// * `symbol`          -> `resolved_local_symbol_index` (a same-file edge)
/// * `import` / `namespace` -> left for the cross-file resolver
///
/// `field` bindings are ignored here: a bare name never resolves to a field
/// (that needs `self.` / `this.`), and the field's type is consumed elsewhere.
pub fn resolve_locals(parsed: &mut ParsedFile) {
    if parsed.scopes.is_empty() {
        return;
    }
    for ri in 0..parsed.refs.len() {
        if parsed.refs[ri].receiver.is_some() {
            continue; // only bare calls resolve to locals
        }
        let at = parsed.refs[ri].start_byte;
        let name = parsed.refs[ri].name.clone();

        // innermost (narrowest) scope containing `at`
        let mut best: Option<(i64, usize)> = None;
        for (i, sc) in parsed.scopes.iter().enumerate() {
            if at >= sc.start_byte && at < sc.end_byte {
                let w = sc.end_byte - sc.start_byte;
                if best.map_or(true, |(bw, _)| w < bw) {
                    best = Some((w, i));
                }
            }
        }
        let Some((_, mut scope_idx)) = best else {
            continue;
        };

        loop {
            let hit = parsed.bindings.iter().find(|b| {
                b.scope_index == scope_idx && b.name == name && b.binding_kind != "field"
            });
            if let Some(b) = hit {
                match b.binding_kind.as_str() {
                    "local" | "param" => parsed.refs[ri].local_only = true,
                    "symbol" => parsed.refs[ri].resolved_local_symbol_index = b.symbol_index,
                    _ => {}
                }
                break;
            }
            match parsed.scopes[scope_idx].parent_index {
                Some(p) => scope_idx = p,
                None => break,
            }
        }
    }
}
