//! Shared reference indexes, independent of graph filtering and rendering.
use super::{
    imports::resolve_import,
    refs::{self, ResolveCtx, Target},
};
use crate::cache::models::{BindingRow, FileRow, ImportRow, ScopeRow, SymbolRow};
use anyhow::Result;
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};
pub(crate) struct ReferenceIndex<'a> {
    pub file_by_id: HashMap<i64, &'a FileRow>,
    pub sym_by_id: HashMap<i64, &'a SymbolRow>,
    pub syms_by_file: HashMap<i64, Vec<&'a SymbolRow>>,
    pub defs_by_name: HashMap<&'a str, Vec<&'a SymbolRow>>,
    binding: HashMap<(i64, String), Target>,
    type_methods: HashMap<String, HashMap<String, Vec<i64>>>,
    type_fields: HashMap<String, HashMap<String, String>>,
    local_types: refs::LocalTypes,
    pub imports_files: HashMap<i64, HashSet<i64>>,
    pub resolved_imports: HashMap<i64, Option<String>>,
}
impl<'a> ReferenceIndex<'a> {
    pub fn new(
        project: &Path,
        files: &'a [FileRow],
        symbols: &'a [SymbolRow],
        imports: &[ImportRow],
        scopes: &[ScopeRow],
        bindings: &[BindingRow],
    ) -> Result<Self> {
        let c_context = files
            .iter()
            .any(|f| matches!(f.language.as_str(), "c" | "cpp"))
            .then(|| {
                crate::config::ProjectConfig::load(project)
                    .and_then(|cfg| crate::c_family::BuildContext::load(project, &cfg))
            })
            .transpose()?;
        let file_by_id: HashMap<i64, &FileRow> = files.iter().map(|f| (f.id, f)).collect();
        let path_set: HashSet<String> = files.iter().map(|f| f.path.clone()).collect();
        let resolve_file = |importer: &FileRow, im: &ImportRow| {
            if matches!(importer.language.as_str(), "c" | "cpp") {
                let context = c_context.as_ref()?;
                let abs = project.join(&importer.path);
                let target = crate::c_family::resolve_include(
                    &abs,
                    &im.raw_specifier,
                    &context.include_paths(&abs),
                )?;
                let rel = target
                    .strip_prefix(project)
                    .ok()?
                    .to_string_lossy()
                    .replace('\\', "/");
                path_set.contains(&rel).then_some(rel)
            } else {
                resolve_import(&importer.language, &importer.path, im, &path_set)
            }
        };
        let frow_by_path: HashMap<&str, &FileRow> =
            files.iter().map(|f| (f.path.as_str(), f)).collect();

        let sym_by_id: HashMap<i64, &SymbolRow> = symbols.iter().map(|s| (s.id, s)).collect();
        let mut syms_by_file: HashMap<i64, Vec<&SymbolRow>> = HashMap::new();
        for s in symbols {
            crate::operation::check()?;
            syms_by_file.entry(s.file_id).or_default().push(s);
        }
        let import_by_id: HashMap<i64, &ImportRow> = imports.iter().map(|i| (i.id, i)).collect();
        let scope_owner: HashMap<i64, Option<i64>> =
            scopes.iter().map(|s| (s.id, s.owner_symbol_id)).collect();

        let resolved_imports: HashMap<i64, Option<String>> = imports
            .iter()
            .map(|im| {
                (
                    im.id,
                    file_by_id
                        .get(&im.file_id)
                        .and_then(|f| resolve_file(f, im)),
                )
            })
            .collect();
        let mut imports_files: HashMap<i64, HashSet<i64>> = HashMap::new();
        for im in imports {
            if let Some(path) = resolved_imports.get(&im.id).and_then(|p| p.as_ref())
                && let Some(file) = frow_by_path.get(path.as_str())
                && file.id != im.file_id
            {
                imports_files.entry(im.file_id).or_default().insert(file.id);
            }
        }
        for file in files
            .iter()
            .filter(|f| matches!(f.language.as_str(), "c" | "cpp"))
        {
            let mut reached = HashSet::new();
            let mut stack = imports_files
                .get(&file.id)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .collect::<Vec<_>>();
            while let Some(id) = stack.pop() {
                if reached.insert(id) {
                    stack.extend(imports_files.get(&id).into_iter().flatten().copied());
                }
            }
            imports_files.entry(file.id).or_default().extend(reached);
        }
        let mut defs_by_name: HashMap<&str, Vec<&SymbolRow>> = HashMap::new();
        for s in symbols {
            crate::operation::check()?;
            defs_by_name.entry(s.name.as_str()).or_default().push(s);
        }

        // Locally-visible name -> cross-file target, from the `bindings` table.
        let mut binding: HashMap<(i64, String), Target> = HashMap::new();
        for b in bindings {
            let Some(imp) = b.import_id.and_then(|i| import_by_id.get(&i)) else {
                continue;
            };
            let Some(importer) = file_by_id.get(&b.file_id) else {
                continue;
            };
            let Some(tpath) = resolve_file(importer, imp) else {
                continue;
            };
            let Some(tf) = frow_by_path.get(tpath.as_str()) else {
                continue;
            };
            match b.binding_kind.as_str() {
                "import" => {
                    let want = imp.imported_name.as_deref().unwrap_or(b.name.as_str());
                    let ids: Vec<_> = syms_by_file
                        .get(&tf.id)
                        .into_iter()
                        .flatten()
                        .filter(|s| s.name == want)
                        .map(|s| s.id)
                        .collect();
                    if ids.is_empty() {
                        binding.insert((b.file_id, b.name.clone()), Target::Module(tf.id));
                    } else {
                        binding.insert((b.file_id, b.name.clone()), Target::Symbols(ids));
                    }
                }
                "namespace" => {
                    binding.insert((b.file_id, b.name.clone()), Target::Module(tf.id));
                }
                _ => {}
            }
        }

        // type simple name -> { method name -> symbol ids }. Several unrelated types
        // can share a simple name (one `Walker` per analyzer), so every definition is
        // kept, in symbol order, and the resolver picks the one nearest the caller.
        let mut type_methods: HashMap<String, HashMap<String, Vec<i64>>> = HashMap::new();
        for s in symbols {
            crate::operation::check()?;
            if s.semantic.kind.callable()
                && let Some(t) = s.semantic.owner.as_ref().or(s.type_name.as_ref())
            {
                type_methods
                    .entry(t.clone())
                    .or_default()
                    .entry(s.name.clone())
                    .or_default()
                    .push(s.id);
            }
        }

        // type simple name -> { field name -> field type }, and
        // (enclosing symbol id, param/local name) -> declared type.
        let mut type_fields: HashMap<String, HashMap<String, String>> = HashMap::new();
        let mut local_types: refs::LocalTypes = HashMap::new();
        let scope_by_id: HashMap<_, _> = scopes.iter().map(|s| (s.id, s)).collect();
        for b in bindings {
            let Some(ty) = b.type_expr.clone() else {
                continue;
            };
            let owner = scope_owner.get(&b.scope_id).copied().flatten();
            match b.binding_kind.as_str() {
                "field" => {
                    if let Some(owner_sym) = owner.and_then(|id| sym_by_id.get(&id)) {
                        type_fields
                            .entry(owner_sym.semantic.qualified_name.clone())
                            .or_default()
                            .insert(b.name.clone(), ty);
                    }
                }
                "param" | "local" => {
                    if let Some(oid) = owner
                        && let Some(scope) = scope_by_id.get(&b.scope_id)
                    {
                        local_types.entry((oid, b.name.clone())).or_default().push((
                            scope.start_byte,
                            scope.end_byte,
                            b.declaration_byte,
                            ty,
                        ));
                    }
                }
                _ => {}
            }
        }

        Ok(Self {
            file_by_id,
            sym_by_id,
            syms_by_file,
            defs_by_name,
            binding,
            type_methods,
            type_fields,
            local_types,
            imports_files,
            resolved_imports,
        })
    }
    pub(super) fn context(&self) -> ResolveCtx<'_> {
        ResolveCtx {
            file_by_id: &self.file_by_id,
            sym_by_id: &self.sym_by_id,
            syms_by_file: &self.syms_by_file,
            defs_by_name: &self.defs_by_name,
            binding: &self.binding,
            type_methods: &self.type_methods,
            type_fields: &self.type_fields,
            local_types: &self.local_types,
            imports_files: &self.imports_files,
        }
    }
    pub fn heuristic(&self, reference: &crate::cache::models::RefRow) -> Option<refs::Resolution> {
        let mut reference = reference.clone();
        reference.precise_status = None;
        reference.precise_symbol_id = None;
        reference.precise_confidence = None;
        refs::resolve_ref(
            &reference,
            self.file_by_id.get(&reference.file_id)?,
            &self.context(),
        )
    }
}
