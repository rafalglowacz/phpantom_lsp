//! Centralized stub patch system for phpstorm-stubs deficiencies.
//!
//! The embedded [phpstorm-stubs](https://github.com/JetBrains/phpstorm-stubs)
//! sometimes lack `@template` annotations or have overly broad return types
//! (e.g. `mixed`) for functions whose return type actually depends on an
//! argument.  PHPStan solves this with dynamic return type extensions written
//! in PHP; we solve it by patching the parsed [`FunctionInfo`] / [`ClassInfo`]
//! at load time.
//!
//! This module provides two entry points:
//!
//! - [`apply_function_stub_patches`]: patches a freshly-parsed `FunctionInfo`
//!   (called from `find_or_load_function` after stub parsing).
//! - [`apply_class_stub_patches`]: patches a freshly-parsed `ClassInfo`
//!   (called from `parse_and_cache_content_versioned` for stub URIs).
//!
//! All stub-deficiency workarounds for built-in PHP symbols live here,
//! making it easy to audit the full inventory and push fixes upstream.
//!
//! ## When to add a patch here vs. hardcoded logic elsewhere
//!
//! If the correct behaviour can be expressed with `@template` / `@return` /
//! `@implements` annotations (i.e. PHPStan's own stubs already have the
//! fix), it belongs here as a `FunctionInfo` or `ClassInfo` patch.  If the
//! behaviour requires inspecting call-site argument *values* at resolution
//! time (e.g. `array_map`'s callback return type), it must stay as hardcoded
//! logic in `rhs_resolution.rs` / `raw_type_inference.rs`.
//!
//! ## Patch inventory
//!
//! ### Function patches
//!
//! 1. **`array_reduce`** — phpstorm-stubs declare `mixed` return type.
//!    The actual return type is the type of the initial value (3rd argument).
//!    PHPStan expresses this as `@template TReturn` + `@param TReturn $initial`
//!    \+ `@return TReturn`.  We patch the same template/binding/return onto
//!    the parsed `FunctionInfo`.
//!    PHPStan ref: `stubs/arrayFunctions.stub`
//!
//! ### Class patches
//!
//! 1. **`ArrayIterator`** — phpstorm-stubs declare no `@template` parameters.
//!    PHPStan adds `@template TKey of array-key`, `@template TValue`,
//!    `@implements SeekableIterator<TKey, TValue>`,
//!    `@implements ArrayAccess<TKey, TValue>`.  Without these, generic usage
//!    like `ArrayIterator<int, Rule>` cannot substitute template parameters
//!    through the interface chain, so `->current()` returns `mixed` instead
//!    of `Rule`.
//!    PHPStan ref: `stubs/iterable.stub`
//!
//! 2. **`ArrayObject`** — phpstorm-stubs declare no `@template` parameters.
//!    PHPStan adds `@template TKey of array-key`, `@template TValue`,
//!    `@implements IteratorAggregate<TKey, TValue>`,
//!    `@implements ArrayAccess<TKey, TValue>`.
//!    PHPStan ref: `stubs/iterable.stub`
//!
//! 3. **`SplDoublyLinkedList`** — phpstorm-stubs declare no `@template`
//!    parameters.  PHPStan adds `@template TKey of int`,
//!    `@template TValue`, `@implements Iterator<TKey, TValue>`,
//!    `@implements ArrayAccess<TKey, TValue>`.
//!    PHPStan ref: `stubs/iterable.stub`
//!
//! 4. **`SplQueue`** — extends `SplDoublyLinkedList` but phpstorm-stubs
//!    lack `@template` and `@extends`.  PHPStan adds
//!    `@template TKey of int`, `@template TValue`,
//!    `@extends SplDoublyLinkedList<TKey, TValue>`.
//!    PHPStan ref: `stubs/iterable.stub`
//!
//! 5. **`SplStack`** — same pattern as `SplQueue`.
//!    PHPStan ref: `stubs/iterable.stub`
//!
//! 6. **`SplPriorityQueue`** — phpstorm-stubs lack `@template`.
//!    PHPStan adds `@template TPriority`, `@template TValue`,
//!    `@implements Iterator<int, TValue>`.
//!    PHPStan ref: `stubs/iterable.stub`
//!
//! 7. **`SplFixedArray`** — phpstorm-stubs lack `@template`.
//!    PHPStan adds `@template TValue`,
//!    `@implements Iterator<int, TValue>`,
//!    `@implements ArrayAccess<int, TValue>`.
//!    PHPStan ref: `stubs/iterable.stub`
//!
//! 8. **`SplObjectStorage`** — phpstorm-stubs lack `@template`.
//!    PHPStan adds `@template TObject of object`,
//!    `@template TData`,
//!    `@implements Iterator<int, TObject>`,
//!    `@implements ArrayAccess<TObject, TData>`.
//!    PHPStan ref: `stubs/iterable.stub`
//!
//! 9. **`WeakMap`** — phpstorm-stubs lack `@template`.
//!    PHPStan adds `@template TKey of object`,
//!    `@template TValue`,
//!    `@implements Iterator<TKey, TValue>`,
//!    `@implements ArrayAccess<TKey, TValue>`.
//!    PHPStan ref: `stubs/WeakMap.stub`
//!
//! ## Removing patches
//!
//! When phpstorm-stubs gains proper annotations for a patched symbol,
//! delete the corresponding patch function here and remove its dispatch
//! from the entry point.  Run the test suite to verify that the stub's
//! own annotations produce the same result.

use crate::atom::atom;
use crate::php_type::PhpType;
use crate::types::{ClassInfo, FunctionInfo};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Function patches
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Apply all registered stub patches to a freshly-parsed function.
///
/// Called from [`find_or_load_function`](crate::resolution) after a
/// `FunctionInfo` is parsed from embedded phpstorm-stubs, before it is
/// cached in `global_functions`.  Only functions with known deficiencies
/// are patched; all others pass through unchanged.
pub fn apply_function_stub_patches(func: &mut FunctionInfo) {
    if func.name.as_str() == "array_reduce" {
        patch_array_reduce(func);
    }
}

/// Patch `array_reduce` to use template-based return type inference.
///
/// phpstorm-stubs signature:
/// ```text
/// function array_reduce(array $array, callable $callback, mixed $initial = null): mixed {}
/// ```
///
/// PHPStan's corrected signature (from `stubs/arrayFunctions.stub`):
/// ```text
/// @template TIn of mixed
/// @template TReturn of mixed
/// @param array<TIn> $array
/// @param callable(TReturn, TIn): TReturn $callback
/// @param TReturn $initial
/// @return TReturn
/// ```
///
/// We only need the `TReturn` template (bound to `$initial`) and the
/// return type override.  `TIn` doesn't affect the return type so we
/// skip it — the existing callable param type from the stub is adequate.
fn patch_array_reduce(func: &mut FunctionInfo) {
    // Only patch if the return type is still the deficient `mixed`.
    let dominated_by_mixed = func.return_type.as_ref().is_some_and(|rt| rt.is_mixed());
    if !dominated_by_mixed {
        return;
    }

    let tpl_name = atom("TReturn");

    // Add template parameter if not already present.
    if !func.template_params.iter().any(|t| t == &tpl_name) {
        func.template_params.push(tpl_name);
    }

    // Bind TReturn to the $initial parameter (3rd positional arg).
    let param_name = atom("$initial");
    if !func.template_bindings.iter().any(|(t, _)| t == &tpl_name) {
        func.template_bindings.push((tpl_name, param_name));
    }

    // Override return type from `mixed` to `TReturn`.
    func.return_type = Some(PhpType::Named(tpl_name.to_string()));
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Class patches
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Apply all registered stub patches to a freshly-parsed class.
///
/// Called from [`parse_and_cache_content_versioned`](crate::resolution)
/// after a `ClassInfo` is parsed from embedded phpstorm-stubs, before it
/// is cached in `ast_map` and `fqn_index`.  Only classes with known
/// deficiencies are patched; all others pass through unchanged.
///
/// This is the class-level counterpart of [`apply_function_stub_patches`].
pub fn apply_class_stub_patches(class: &mut ClassInfo) {
    match class.name.as_str() {
        "ArrayIterator" => patch_array_iterator(class),
        "ArrayObject" => patch_array_object(class),
        "SplDoublyLinkedList" => patch_spl_doubly_linked_list(class),
        "SplQueue" => patch_spl_queue(class),
        "SplStack" => patch_spl_stack(class),
        "SplPriorityQueue" => patch_spl_priority_queue(class),
        "SplFixedArray" => patch_spl_fixed_array(class),
        "SplObjectStorage" => patch_spl_object_storage(class),
        "WeakMap" => patch_weak_map(class),
        _ => {}
    }
}

/// Add `@template TKey of array-key`, `@template TValue`,
/// `@implements SeekableIterator<TKey, TValue>`,
/// `@implements ArrayAccess<TKey, TValue>`.
fn patch_array_iterator(class: &mut ClassInfo) {
    if !class.template_params.is_empty() {
        return;
    }
    add_templates(class, &[("TKey", Some("array-key")), ("TValue", None)]);
    add_implements_generics(class, "SeekableIterator", &["TKey", "TValue"]);
    add_implements_generics(class, "ArrayAccess", &["TKey", "TValue"]);
}

/// Add `@template TKey of array-key`, `@template TValue`,
/// `@implements IteratorAggregate<TKey, TValue>`,
/// `@implements ArrayAccess<TKey, TValue>`.
fn patch_array_object(class: &mut ClassInfo) {
    if !class.template_params.is_empty() {
        return;
    }
    add_templates(class, &[("TKey", Some("array-key")), ("TValue", None)]);
    add_implements_generics(class, "IteratorAggregate", &["TKey", "TValue"]);
    add_implements_generics(class, "ArrayAccess", &["TKey", "TValue"]);
}

/// Add `@template TKey of int`, `@template TValue`,
/// `@implements Iterator<TKey, TValue>`,
/// `@implements ArrayAccess<TKey, TValue>`.
fn patch_spl_doubly_linked_list(class: &mut ClassInfo) {
    if !class.template_params.is_empty() {
        return;
    }
    add_templates(class, &[("TKey", Some("int")), ("TValue", None)]);
    add_implements_generics(class, "Iterator", &["TKey", "TValue"]);
    add_implements_generics(class, "ArrayAccess", &["TKey", "TValue"]);
}

/// Add `@template TKey of int`, `@template TValue`,
/// `@extends SplDoublyLinkedList<TKey, TValue>`.
fn patch_spl_queue(class: &mut ClassInfo) {
    if !class.template_params.is_empty() {
        return;
    }
    add_templates(class, &[("TKey", Some("int")), ("TValue", None)]);
    add_extends_generics(class, "SplDoublyLinkedList", &["TKey", "TValue"]);
}

/// Add `@template TKey of int`, `@template TValue`,
/// `@extends SplDoublyLinkedList<TKey, TValue>`.
fn patch_spl_stack(class: &mut ClassInfo) {
    if !class.template_params.is_empty() {
        return;
    }
    add_templates(class, &[("TKey", Some("int")), ("TValue", None)]);
    add_extends_generics(class, "SplDoublyLinkedList", &["TKey", "TValue"]);
}

/// Add `@template TPriority`, `@template TValue`,
/// `@implements Iterator<int, TValue>`.
fn patch_spl_priority_queue(class: &mut ClassInfo) {
    if !class.template_params.is_empty() {
        return;
    }
    add_templates(class, &[("TPriority", None), ("TValue", None)]);
    add_implements_generics_typed(
        class,
        "Iterator",
        &[PhpType::int(), PhpType::Named("TValue".to_string())],
    );
}

/// Add `@template TValue`,
/// `@implements Iterator<int, TValue>`,
/// `@implements ArrayAccess<int, TValue>`.
fn patch_spl_fixed_array(class: &mut ClassInfo) {
    if !class.template_params.is_empty() {
        return;
    }
    add_templates(class, &[("TValue", None)]);
    add_implements_generics_typed(
        class,
        "Iterator",
        &[PhpType::int(), PhpType::Named("TValue".to_string())],
    );
    add_implements_generics_typed(
        class,
        "ArrayAccess",
        &[PhpType::int(), PhpType::Named("TValue".to_string())],
    );
}

/// Add `@template TObject of object`, `@template TData`,
/// `@implements Iterator<int, TObject>`,
/// `@implements ArrayAccess<TObject, TData>`.
fn patch_spl_object_storage(class: &mut ClassInfo) {
    if !class.template_params.is_empty() {
        return;
    }
    add_templates(class, &[("TObject", Some("object")), ("TData", None)]);
    add_implements_generics_typed(
        class,
        "Iterator",
        &[PhpType::int(), PhpType::Named("TObject".to_string())],
    );
    add_implements_generics_typed(
        class,
        "ArrayAccess",
        &[
            PhpType::Named("TObject".to_string()),
            PhpType::Named("TData".to_string()),
        ],
    );
}

/// Add `@template TKey of object`, `@template TValue`,
/// `@implements Iterator<TKey, TValue>`,
/// `@implements ArrayAccess<TKey, TValue>`.
fn patch_weak_map(class: &mut ClassInfo) {
    if !class.template_params.is_empty() {
        return;
    }
    add_templates(class, &[("TKey", Some("object")), ("TValue", None)]);
    add_implements_generics(class, "Iterator", &["TKey", "TValue"]);
    add_implements_generics(class, "ArrayAccess", &["TKey", "TValue"]);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Add template parameters with optional upper bounds.
///
/// Each entry is `(param_name, optional_bound)`.  The bound, if present,
/// is parsed into a `PhpType` and stored in `template_param_bounds`.
fn add_templates(class: &mut ClassInfo, templates: &[(&str, Option<&str>)]) {
    for &(name, bound) in templates {
        let param = atom(name);
        if !class.template_params.contains(&param) {
            class.template_params.push(param);
        }
        if let Some(bound_str) = bound {
            class
                .template_param_bounds
                .entry(atom(name))
                .or_insert_with(|| PhpType::parse(bound_str));
        }
    }
}

/// Add an `@implements InterfaceName<Param1, Param2, ...>` entry where
/// all type arguments are template parameter names (the common case).
fn add_implements_generics(class: &mut ClassInfo, iface_name: &str, params: &[&str]) {
    let args: Vec<PhpType> = params
        .iter()
        .map(|p| PhpType::Named((*p).to_string()))
        .collect();
    add_implements_generics_typed(class, iface_name, &args);
}

/// Add an `@implements InterfaceName<Type1, Type2, ...>` entry with
/// pre-built `PhpType` arguments.
fn add_implements_generics_typed(class: &mut ClassInfo, iface_name: &str, args: &[PhpType]) {
    // Don't add duplicate entries.
    if class
        .implements_generics
        .iter()
        .any(|(n, _)| n.as_str() == iface_name)
    {
        return;
    }
    class
        .implements_generics
        .push((atom(iface_name), args.to_vec()));
}

/// Add an `@extends ClassName<Param1, Param2, ...>` entry where all type
/// arguments are template parameter names.
fn add_extends_generics(class: &mut ClassInfo, class_name: &str, params: &[&str]) {
    // Don't add duplicate entries.
    if class
        .extends_generics
        .iter()
        .any(|(n, _)| n.as_str() == class_name)
    {
        return;
    }
    let args: Vec<PhpType> = params
        .iter()
        .map(|p| PhpType::Named((*p).to_string()))
        .collect();
    class.extends_generics.push((atom(class_name), args));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atom::atom;
    use crate::php_type::PhpType;
    use crate::test_fixtures::make_param;

    fn make_stub_array_reduce() -> FunctionInfo {
        FunctionInfo {
            name: atom("array_reduce"),
            parameters: vec![
                make_param("$array", Some("array"), true),
                make_param("$callback", Some("callable"), true),
                make_param("$initial", Some("mixed"), false),
            ],
            return_type: Some(PhpType::mixed()),
            ..empty_func_info()
        }
    }

    /// Minimal `FunctionInfo` with all fields zeroed/empty.
    fn empty_func_info() -> FunctionInfo {
        FunctionInfo {
            name: atom(""),
            name_offset: 0,
            parameters: Vec::new(),
            return_type: None,
            native_return_type: None,
            description: None,
            return_description: None,
            links: Vec::new(),
            see_refs: Vec::new(),
            namespace: None,
            conditional_return: None,
            type_assertions: Vec::new(),
            deprecation_message: None,
            deprecated_replacement: None,
            template_params: Vec::new(),
            template_bindings: Vec::new(),
            template_param_bounds: Default::default(),
            throws: Vec::new(),
            is_polyfill: false,
        }
    }

    #[test]
    fn array_reduce_gets_template_return() {
        let mut func = make_stub_array_reduce();
        apply_function_stub_patches(&mut func);

        assert_eq!(
            func.template_params,
            vec![atom("TReturn")],
            "Should add TReturn template param"
        );
        assert_eq!(
            func.template_bindings,
            vec![(atom("TReturn"), atom("$initial"))],
            "Should bind TReturn to $initial"
        );
        assert_eq!(
            func.return_type,
            Some(PhpType::Named("TReturn".to_string())),
            "Return type should be TReturn"
        );
    }

    #[test]
    fn array_reduce_not_patched_when_return_type_already_correct() {
        let mut func = make_stub_array_reduce();
        // Simulate upstream fix: return type is already templated.
        func.return_type = Some(PhpType::Named("TReturn".to_string()));
        func.template_params = vec![atom("TReturn")];

        apply_function_stub_patches(&mut func);

        // Should not double-add template params.
        assert_eq!(func.template_params.len(), 1);
        assert!(
            func.template_bindings.is_empty(),
            "Should not add bindings when return type is not mixed"
        );
    }

    #[test]
    fn unrelated_function_not_patched() {
        let mut func = FunctionInfo {
            name: atom("strlen"),
            return_type: Some(PhpType::int()),
            ..empty_func_info()
        };
        let original_return = func.return_type.clone();

        apply_function_stub_patches(&mut func);

        assert_eq!(func.return_type, original_return);
        assert!(func.template_params.is_empty());
    }

    // ── Class patch tests ───────────────────────────────────────────────

    fn empty_class(name: &str) -> ClassInfo {
        ClassInfo {
            name: atom(name),
            ..ClassInfo::default()
        }
    }

    #[test]
    fn array_iterator_gets_templates_and_implements() {
        let mut class = empty_class("ArrayIterator");
        apply_class_stub_patches(&mut class);

        assert_eq!(
            class.template_params,
            vec![atom("TKey"), atom("TValue")],
            "Should add TKey and TValue template params"
        );
        assert_eq!(
            class.template_param_bounds.get(&atom("TKey")),
            Some(&PhpType::parse("array-key")),
            "TKey should be bounded by array-key"
        );
        assert!(
            !class.template_param_bounds.contains_key(&atom("TValue")),
            "TValue should have no bound"
        );
        assert!(
            class
                .implements_generics
                .iter()
                .any(|(n, args)| n.as_str() == "SeekableIterator" && args.len() == 2),
            "Should have @implements SeekableIterator<TKey, TValue>"
        );
        assert!(
            class
                .implements_generics
                .iter()
                .any(|(n, args)| n.as_str() == "ArrayAccess" && args.len() == 2),
            "Should have @implements ArrayAccess<TKey, TValue>"
        );
    }

    #[test]
    fn array_iterator_not_patched_when_templates_exist() {
        let mut class = empty_class("ArrayIterator");
        class.template_params = vec![atom("T")];

        apply_class_stub_patches(&mut class);

        // Should not overwrite existing template params.
        assert_eq!(class.template_params, vec![atom("T")]);
        assert!(class.implements_generics.is_empty());
    }

    #[test]
    fn array_object_gets_templates_and_implements() {
        let mut class = empty_class("ArrayObject");
        apply_class_stub_patches(&mut class);

        assert_eq!(class.template_params, vec![atom("TKey"), atom("TValue")],);
        assert!(
            class
                .implements_generics
                .iter()
                .any(|(n, _)| n.as_str() == "IteratorAggregate"),
            "Should have @implements IteratorAggregate"
        );
        assert!(
            class
                .implements_generics
                .iter()
                .any(|(n, _)| n.as_str() == "ArrayAccess"),
            "Should have @implements ArrayAccess"
        );
    }

    #[test]
    fn spl_doubly_linked_list_gets_templates() {
        let mut class = empty_class("SplDoublyLinkedList");
        apply_class_stub_patches(&mut class);

        assert_eq!(class.template_params, vec![atom("TKey"), atom("TValue")],);
        assert_eq!(
            class.template_param_bounds.get(&atom("TKey")),
            Some(&PhpType::int()),
            "TKey should be bounded by int"
        );
        assert!(
            class
                .implements_generics
                .iter()
                .any(|(n, _)| n.as_str() == "Iterator"),
        );
    }

    #[test]
    fn spl_queue_gets_extends_generics() {
        let mut class = empty_class("SplQueue");
        apply_class_stub_patches(&mut class);

        assert_eq!(class.template_params, vec![atom("TKey"), atom("TValue")],);
        assert!(
            class
                .extends_generics
                .iter()
                .any(|(n, _)| n.as_str() == "SplDoublyLinkedList"),
            "Should have @extends SplDoublyLinkedList<TKey, TValue>"
        );
    }

    #[test]
    fn spl_fixed_array_gets_templates() {
        let mut class = empty_class("SplFixedArray");
        apply_class_stub_patches(&mut class);

        assert_eq!(class.template_params, vec![atom("TValue")]);
        assert!(
            class
                .implements_generics
                .iter()
                .any(|(n, args)| n.as_str() == "Iterator"
                    && args.len() == 2
                    && args[0] == PhpType::int()),
        );
    }

    #[test]
    fn spl_object_storage_gets_templates() {
        let mut class = empty_class("SplObjectStorage");
        apply_class_stub_patches(&mut class);

        assert_eq!(class.template_params, vec![atom("TObject"), atom("TData")],);
        assert_eq!(
            class.template_param_bounds.get(&atom("TObject")),
            Some(&PhpType::parse("object")),
        );
    }

    #[test]
    fn weak_map_gets_templates() {
        let mut class = empty_class("WeakMap");
        apply_class_stub_patches(&mut class);

        assert_eq!(class.template_params, vec![atom("TKey"), atom("TValue")],);
        assert_eq!(
            class.template_param_bounds.get(&atom("TKey")),
            Some(&PhpType::parse("object")),
        );
        assert!(
            class
                .implements_generics
                .iter()
                .any(|(n, _)| n.as_str() == "Iterator"),
        );
    }

    #[test]
    fn unrelated_class_not_patched() {
        let mut class = empty_class("MyApp\\Foo");
        let original_params = class.template_params.clone();

        apply_class_stub_patches(&mut class);

        assert_eq!(class.template_params, original_params);
        assert!(class.implements_generics.is_empty());
    }
}
