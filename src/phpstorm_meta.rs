//! JetBrains PhpStorm advanced metadata (`.phpstorm.meta.php`) indexing.
//!
//! Supports `override()` with `type()`, `elementType()`, and `map()` directives
//! inside `namespace PHPSTORM_META { ... }`, for return-type inference at call sites.
//!
//! See <https://www.jetbrains.com/help/phpstorm/ide-advanced-metadata.html>.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bumpalo::Bump;
use mago_span::HasSpan;
use mago_syntax::ast::*;
use mago_syntax::parser::parse_file_content;

use crate::names::OwnedResolvedNames;
use crate::Backend;

/// Parsed `override()` directive (second argument to `override(...)`).
#[derive(Debug, Clone)]
pub enum PhpStormOverrideDirective {
    /// Return type is the type of the n-th (0-based) call argument.
    Type { arg_index: usize },
    /// Return type is the array element type of the n-th argument.
    ElementType { arg_index: usize },
    /// Map from call argument to a return type string (parsed as PHPDoc-like type).
    Map { entries: Vec<(MapLookupKey, String)> },
}

/// Key in a `map([ ... ])` entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MapLookupKey {
    /// Default / fallback (`''` in PhpStorm metadata).
    Default,
    String(String),
    Int(i64),
    ClassConst { class_fqn: String, constant: String },
}

/// Index of PhpStorm metadata merged from all discovered files.
#[derive(Debug, Clone, Default)]
pub struct PhpStormMetaIndex {
    /// (`Class\FQN`, `methodName`) → directive.
    pub(crate) method_overrides: HashMap<(String, String), PhpStormOverrideDirective>,
    /// Function FQN (no leading `\`) → directive.
    pub(crate) function_overrides: HashMap<String, PhpStormOverrideDirective>,
}

pub(crate) fn normalize_fqn(s: &str) -> String {
    s.trim_start_matches('\\').to_string()
}

/// Discover `.phpstorm.meta.php` files and `*.php` inside `.phpstorm.meta.php/` directories.
pub fn discover_phpstorm_meta_files(root: &Path, vendor_dir_paths: &[PathBuf]) -> Vec<PathBuf> {
    use ignore::WalkBuilder;

    let mut result = Vec::new();
    let vendor_paths_owned: Vec<PathBuf> = vendor_dir_paths.to_vec();

    let walker = WalkBuilder::new(root)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        // JetBrains meta lives in dotfiles (`.phpstorm.meta.php` and `.phpstorm.meta.php/`).
        .hidden(false)
        .parents(true)
        .ignore(true)
        .filter_entry(move |entry| {
            if entry.file_type().is_some_and(|ft| ft.is_dir()) {
                let path = entry.path();
                if vendor_paths_owned.iter().any(|vp| vp == path) {
                    return false;
                }
            }
            true
        })
        .build();

    for entry in walker.flatten() {
        let path = entry.path();
        if is_phpstorm_meta_related(path) {
            result.push(path.to_path_buf());
        }
    }

    result.sort();
    result.dedup();
    result
}

pub(crate) fn is_phpstorm_meta_related(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    if ext != "php" {
        return false;
    }
    if path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n == ".phpstorm.meta.php")
    {
        return true;
    }
    path.parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .is_some_and(|n| n == ".phpstorm.meta.php")
}

/// Build an index by parsing every path (sorted; later paths override keys).
pub fn build_index_from_paths(paths: &[PathBuf]) -> PhpStormMetaIndex {
    let mut index = PhpStormMetaIndex::default();
    for path in paths {
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        merge_content_into_index(&mut index, &content);
    }
    index
}

fn merge_content_into_index(index: &mut PhpStormMetaIndex, content: &str) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        parse_meta_content(content)
    }));
    match result {
        Ok(parsed) => {
            for (k, v) in parsed.method_overrides {
                index.method_overrides.insert(k, v);
            }
            for (k, v) in parsed.function_overrides {
                index.function_overrides.insert(k, v);
            }
        }
        Err(_) => {
            tracing::warn!("PHPantom: failed to parse PhpStorm meta file (parser panic)");
        }
    }
}

fn parse_meta_content(content: &str) -> PhpStormMetaIndex {
    let arena = Bump::new();
    let file_id = mago_database::file::FileId::new(".phpstorm.meta.php");
    let program = parse_file_content(&arena, file_id, content);
    let name_resolver = mago_names::resolver::NameResolver::new(&arena);
    let mago_resolved = name_resolver.resolve(program);
    let resolved = OwnedResolvedNames::from_resolved(&mago_resolved);

    let mut index = PhpStormMetaIndex::default();
    for statement in program.statements.iter() {
        if let Statement::Namespace(ns) = statement {
            let is_meta_ns = ns
                .name
                .as_ref()
                .map(|ident| ident.value() == "PHPSTORM_META")
                .unwrap_or(false);
            if !is_meta_ns {
                continue;
            }
            extract_from_statements(ns.statements().iter(), content, &resolved, &mut index);
        }
    }
    index
}

fn extract_from_statements<'a>(
    stmts: impl Iterator<Item = &'a Statement<'a>>,
    content: &str,
    resolved: &OwnedResolvedNames,
    index: &mut PhpStormMetaIndex,
) {
    for stmt in stmts {
        match stmt {
            Statement::Expression(expr_stmt) => {
                try_extract_override(&expr_stmt.expression, content, resolved, index);
            }
            Statement::Namespace(inner) => {
                extract_from_statements(inner.statements().iter(), content, resolved, index);
            }
            _ => {}
        }
    }
}

fn try_extract_override(
    expr: &Expression<'_>,
    content: &str,
    resolved: &OwnedResolvedNames,
    index: &mut PhpStormMetaIndex,
) {
    let expr = unwrap_parens(expr);
    let Expression::Call(Call::Function(fc)) = expr else {
        return;
    };
    if !callee_is_named(fc.function, "override", resolved) {
        return;
    }
    let mut pos_args = Vec::new();
    for arg in fc.argument_list.arguments.iter() {
        match arg {
            Argument::Positional(p) => pos_args.push(p.value),
            Argument::Named(_) => {}
        }
    }
    if pos_args.len() < 2 {
        return;
    }
    let target = pos_args[0];
    let directive_expr = pos_args[1];
    let Some(directive) = parse_directive(directive_expr, content, resolved) else {
        return;
    };
    match parse_override_target(target, resolved) {
        Some(OverrideTarget::Method {
            class_fqn,
            method,
        }) => {
            index
                .method_overrides
                .insert((normalize_fqn(&class_fqn), method), directive);
        }
        Some(OverrideTarget::Function { fqn }) => {
            index.function_overrides.insert(normalize_fqn(&fqn), directive);
        }
        None => {}
    }
}

enum OverrideTarget {
    Method { class_fqn: String, method: String },
    Function { fqn: String },
}

fn parse_override_target(expr: &Expression<'_>, resolved: &OwnedResolvedNames) -> Option<OverrideTarget> {
    let expr = unwrap_parens(expr);
    match expr {
        Expression::Call(Call::StaticMethod(sc)) => {
            let class_fqn = resolved
                .get(sc.class.span().start.offset)
                .map(|s| s.to_string())?;
            let method = match &sc.method {
                ClassLikeMemberSelector::Identifier(id) => id.value.to_string(),
                _ => return None,
            };
            Some(OverrideTarget::Method { class_fqn, method })
        }
        Expression::Call(Call::Function(fc)) => {
            let fqn = resolved
                .get(fc.function.span().start.offset)
                .map(|s| s.to_string())?;
            Some(OverrideTarget::Function { fqn })
        }
        _ => None,
    }
}

fn parse_directive(
    expr: &Expression<'_>,
    content: &str,
    resolved: &OwnedResolvedNames,
) -> Option<PhpStormOverrideDirective> {
    let expr = unwrap_parens(expr);
    let Expression::Call(Call::Function(fc)) = expr else {
        return None;
    };
    let name = callee_simple_name(fc.function, resolved)?;
    let mut pos = Vec::new();
    for arg in fc.argument_list.arguments.iter() {
        if let Argument::Positional(p) = arg {
            pos.push(p.value);
        }
    }
    match name.as_str() {
        "type" => {
            let idx = pos.first().and_then(|e| literal_usize(e))?;
            Some(PhpStormOverrideDirective::Type { arg_index: idx })
        }
        "elementType" => {
            let idx = pos.first().and_then(|e| literal_usize(e))?;
            Some(PhpStormOverrideDirective::ElementType { arg_index: idx })
        }
        "map" => {
            let arr = pos.first().copied()?;
            let entries = parse_map_array(arr, content, resolved)?;
            Some(PhpStormOverrideDirective::Map { entries })
        }
        _ => None,
    }
}

fn parse_map_array(
    expr: &Expression<'_>,
    content: &str,
    resolved: &OwnedResolvedNames,
) -> Option<Vec<(MapLookupKey, String)>> {
    let expr = unwrap_parens(expr);
    let elements = match expr {
        Expression::Array(a) => &a.elements,
        Expression::LegacyArray(a) => &a.elements,
        _ => return None,
    };
    let mut out = Vec::new();
    for el in elements.iter() {
        if let ArrayElement::KeyValue(kv) = el {
            let key = map_key_from_expr(&kv.key, content, resolved)?;
            let val = map_value_to_type_string(&kv.value, content, resolved)?;
            out.push((key, val));
        }
    }
    Some(out)
}

fn map_key_from_expr(
    expr: &Expression<'_>,
    content: &str,
    resolved: &OwnedResolvedNames,
) -> Option<MapLookupKey> {
    let expr = unwrap_parens(expr);
    match expr {
        Expression::Literal(Literal::String(s)) => {
            let v = s
                .value
                .map(|x| x.to_string())
                .or_else(|| {
                    crate::util::unquote_php_string(s.raw).map(|x| x.to_string())
                })?;
            if v.is_empty() {
                return Some(MapLookupKey::Default);
            }
            Some(MapLookupKey::String(v))
        }
        Expression::Literal(Literal::Integer(i)) => {
            let n: i64 = i.raw.parse().ok()?;
            Some(MapLookupKey::Int(n))
        }
        Expression::Access(Access::ClassConstant(cca)) => {
            let ClassLikeConstantSelector::Identifier(id) = &cca.constant else {
                return None;
            };
            let class_fqn = resolved
                .get(cca.class.span().start.offset)
                .map(|s| normalize_fqn(s))?;
            Some(MapLookupKey::ClassConst {
                class_fqn,
                constant: id.value.to_string(),
            })
        }
        Expression::Identifier(id) => {
            let fqn = resolved.get(id.span().start.offset)?;
            Some(MapLookupKey::String(normalize_fqn(fqn)))
        }
        _ => {
            let start = expr.span().start.offset as usize;
            let end = expr.span().end.offset as usize;
            let slice = content.get(start..end)?;
            Some(MapLookupKey::String(slice.trim().to_string()))
        }
    }
}

fn map_value_to_type_string(
    expr: &Expression<'_>,
    _content: &str,
    resolved: &OwnedResolvedNames,
) -> Option<String> {
    let expr = unwrap_parens(expr);
    match expr {
        Expression::Literal(Literal::String(s)) => s
            .value
            .map(|x| x.to_string())
            .or_else(|| {
                crate::util::unquote_php_string(s.raw).map(|x| x.to_string())
            }),
        Expression::Access(Access::ClassConstant(cca)) => {
            if let ClassLikeConstantSelector::Identifier(id) = &cca.constant {
                if id.value == "class" {
                    return resolved
                        .get(cca.class.span().start.offset)
                        .map(|n| normalize_fqn(n));
                }
            }
            None
        }
        Expression::Identifier(id) => resolved
            .get(id.span().start.offset)
            .map(|n| normalize_fqn(n)),
        _ => None,
    }
}

fn literal_usize(expr: &Expression<'_>) -> Option<usize> {
    let expr = unwrap_parens(expr);
    match expr {
        Expression::Literal(Literal::Integer(i)) => i.raw.parse().ok(),
        _ => None,
    }
}

fn callee_simple_name(expr: &Expression<'_>, resolved: &OwnedResolvedNames) -> Option<String> {
    let expr = unwrap_parens(expr);
    match expr {
        Expression::Identifier(id) => Some(id.value().to_string()),
        _ => {
            let off = expr.span().start.offset;
            let fqn = resolved.get(off)?;
            Some(
                fqn.rsplit('\\')
                    .next()
                    .unwrap_or(fqn)
                    .to_string(),
            )
        }
    }
}

fn callee_is_named(expr: &Expression<'_>, want: &str, resolved: &OwnedResolvedNames) -> bool {
    callee_simple_name(expr, resolved)
        .as_deref()
        .is_some_and(|n| n == want)
}

fn unwrap_parens<'a>(expr: &'a Expression<'a>) -> &'a Expression<'a> {
    match expr {
        Expression::Parenthesized(p) => unwrap_parens(p.expression),
        _ => expr,
    }
}

/// Whether opening or editing this document URI should reload `.phpstorm.meta.php` data.
pub(crate) fn should_refresh_index_for_uri(uri: &str) -> bool {
    tower_lsp::lsp_types::Url::parse(uri)
        .ok()
        .and_then(|u| u.to_file_path().ok())
        .is_some_and(|p| is_phpstorm_meta_related(&p))
}

/// Refresh the workspace PhpStorm meta index from disk and clear the resolved-class cache.
pub(crate) fn refresh_index(backend: &Backend) {
    let Some(root) = backend.workspace_root.read().clone() else {
        *backend.phpstorm_meta.write() = PhpStormMetaIndex::default();
        return;
    };
    let vendor = backend.vendor_dir_paths.lock().clone();
    let paths = discover_phpstorm_meta_files(&root, &vendor);
    let index = build_index_from_paths(&paths);
    *backend.phpstorm_meta.write() = index;
    backend.resolved_class_cache.lock().clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_override_type_on_static_method_with_dummy_arg() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".phpstorm.meta.php");
        std::fs::write(
            &path,
            concat!(
                "<?php\n",
                "namespace PHPSTORM_META {\n",
                "    override(\\Demo\\Factory::get(0), type(0));\n",
                "}\n",
            ),
        )
        .unwrap();
        let idx = build_index_from_paths(&[path]);
        let key = (normalize_fqn("Demo\\Factory"), "get".to_string());
        assert!(
            idx.method_overrides.contains_key(&key),
            "expected override key {:?}, have {:?}",
            key,
            idx.method_overrides.keys().collect::<Vec<_>>()
        );
    }
}
