# PHPantom — Mago Crate Migration

This document describes the migration from hand-rolled PHP parsing
subsystems to upstream Mago crates. The goal is to replace fragile,
maintenance-heavy internal code with well-tested, upstream-maintained
libraries — improving correctness and robustness while reducing the
long-term maintenance burden.

> **Guiding principle:** Correctness and robustness win over raw
> performance. We accept modest overhead from structured
> representations in exchange for eliminating entire classes of
> edge-case bugs in string-based type manipulation.

## Crates to adopt

| Crate              | Replaces                                               | Effort      | Status |
| ------------------ | ------------------------------------------------------ | ----------- | ------ |
| `mago-docblock`    | Manual docblock parsing scattered across the codebase   | Medium-High | ✅ Done |
| `mago-type-syntax` | `src/docblock/{type_strings,generics,shapes,callable_types,conditional}.rs` + string-based type pipeline | Very High | M4 |
| `mago-names`       | `src/parser/use_statements.rs` + `use_map` resolution   | Medium-High | M3 |

`mago-docblock` is fully integrated — all modules that benefit from
structured parsing use `DocblockInfo` / `TagInfo`. The remaining
raw-text docblock code in the codebase operates on individual lines
for surgical text editing and is better served by direct string
manipulation.

A fifth crate, `mago-reporting`, comes in as a transitive dependency
of `mago-semantics` and `mago-names`. It does not replace any
PHPantom code but will appear in `Cargo.toml`.

### Crates explicitly ruled out

| Crate              | Reason                                                                   |
| ------------------ | ------------------------------------------------------------------------ |
| `mago-codex`       | Replaces `ClassInfo` model with one that cannot carry `LaravelMetadata`. |
| `mago-semantics`   | 12K false positives on Laravel; no way to inject our type context.       |
| `mago-linter`      | Same problem; `Integration::Laravel` is surface-level only.              |
| `mago-fingerprint` | Requires `mago-names` for limited value; `signature_eq` already works.   |

---

## M3. Migrate to `mago-names`

**What it replaces:** `src/parser/use_statements.rs` (130 lines),
the `use_map: DashMap<String, HashMap<String, String>>` on `Backend`,
and the lazy name-resolution helpers in `src/resolution.rs` that
manually look up the use map.

**Why:** `mago-names` resolves every identifier in a PHP file to its
fully-qualified name in a single pass. This is more correct than our
lazy approach for edge cases: names that resolve differently depending
on whether they appear in a type hint vs. a `new` expression vs. a
function call (PHP's different name resolution rules for classes,
functions, and constants). It also provides `is_imported()` which
tells us whether a name came from a `use` statement — useful for
auto-import code actions and unused-import diagnostics.

**What it does NOT replace:** Cross-file resolution
(`find_or_load_class`, PSR-4 resolution, classmap lookup, stub
loading). Those stay in `src/resolution.rs`. `mago-names` handles
only the within-file syntactic resolution (use statements + namespace
context → FQN).

**Risk:** Medium-high. The `use_map` is read from many places. The
arena lifetime for `ResolvedNames` must outlive the consumers.
Requires restructuring how we store per-file name resolution data.

### Steps

1. ✅ **Add `mago-names` to `Cargo.toml`.**
   This also brings in `foldhash` as a transitive dependency.
   *Done — resolved to v1.15.2.*

2. ✅ **Run the name resolver in `update_ast_inner`.**
   After parsing the `Program`, call
   `mago_names::resolver::NameResolver::new(&arena).resolve(program)`
   to produce a `ResolvedNames`. This happens in the same arena as
   the parse.
   *Done — resolver runs right after `parse_file_content`, while
   the arena is still alive.*

3. ✅ **Store resolved names per file.**
   Used Option A (copy to owned storage).  Added
   `resolved_names: Arc<RwLock<HashMap<String, Arc<OwnedResolvedNames>>>>`
   to `Backend`.  Populated in `update_ast_inner` for files open in the
   editor.  Not populated for vendor/stub files loaded via
   `parse_and_cache_content_versioned` (those files are never queried
   by byte offset).  Shared via `Arc::clone` in
   `clone_for_diagnostic_worker`, cleaned up in `clear_file_maps`.

4. ✅ **Build an `OwnedResolvedNames` wrapper.**
   Created `src/names.rs` with `OwnedResolvedNames` providing:
   - `get(offset) -> Option<&str>` — FQN lookup by byte offset
   - `is_imported(offset) -> bool` — was the name from a `use` stmt
   - `iter()` — iterate all `(offset, fqn, imported)` triples
   - `to_use_map()` — transitional helper building `HashMap<String, String>`

   Also added `FileContext::resolve_name_at(name, offset)` as a
   convenience method that tries `resolved_names` first and falls
   back to the legacy `resolve_to_fqn` logic.

5. 🔶 **Replace `use_map` reads incrementally.** *(in progress)*

   **Migrated to byte-offset lookups (`resolved_names`):**
   - `src/diagnostics/unknown_classes.rs` — `ClassReference` FQN
     resolution and `is_imported` check
   - `src/diagnostics/deprecated.rs` — `ClassReference` FQN resolution
   - `src/definition/resolve.rs` — `FunctionCall`, `ConstantReference`,
     and `ClassReference` resolution via `ctx.resolve_name_at()`
   - `src/definition/implementation.rs` — `resolve_class_implementation`
   - `src/highlight/mod.rs` — `ClassReference` FQN resolution
   - `src/references/mod.rs` — `ClassReference` and `FunctionCall`
     resolution
   - `src/rename/mod.rs` — `ClassReference` FQN resolution
   - `src/type_hierarchy.rs` — `ClassReference`, `ClassDeclaration`,
     and `SelfStaticParent` resolution

   **Switched to `self.file_use_map(uri)` helper** (still reads the
   legacy `use_map`, but funnelled through a single method for future
   replacement):
   - `src/diagnostics/deprecated.rs`
   - `src/diagnostics/unknown_classes.rs`
   - `src/diagnostics/unknown_functions.rs`
   - `src/diagnostics/unknown_members.rs`
   - `src/diagnostics/unused_imports.rs`
   - `src/code_actions/import_class.rs`
   - `src/code_actions/phpstan/add_override.rs`
   - `src/code_actions/phpstan/add_throws.rs`
   - `src/code_actions/replace_deprecated.rs`

   **Not yet migrated** (still reading `use_map` through `FileContext`
   or `file_use_map`):
   - `src/resolution.rs` — `resolve_class_name`, `resolve_function_name`
     (used by loader closures that resolve names without byte offsets)
   - `src/completion/` (various modules, via `FileContext`)
   - Cross-file reference scanning in `src/references/mod.rs`
     (`find_class_references`, `find_function_references`)

   **Key finding:** The legacy `use_map` cannot be fully replaced by
   `to_use_map()` because `resolved_names` only contains names that
   are actually *referenced* in the code, not all *declared* imports.
   The unused-imports diagnostic and import-class code action need
   the full set of declared imports.  The `use_map` must remain until
   a proper declared-imports structure (possibly from `mago-names`
   scope data) is introduced.

6. **Deprecate and remove `use_map`.** *(blocked on step 5)*
   Once all consumers use `OwnedResolvedNames`, remove the
   `use_map: DashMap<String, HashMap<String, String>>` from
   `Backend`. Also remove `extract_use_items` and
   `extract_use_statements_from_statements` from
   `src/parser/use_statements.rs`.

7. ✅ **Keep `namespace_map` for now.**
   The per-file namespace is still needed for PSR-4 resolution and
   class index construction. `mago-names` doesn't expose the file's
   namespace as a standalone value, so keep `namespace_map` or extract
   the namespace from the AST directly (it's trivial — first
   `Statement::Namespace` node).

8. **Update unused-import diagnostics.** *(deferred)*
   `mago-names` provides `is_imported()` for each resolved name. An
   unused import is a `use` statement whose imported names never
   appear in `ResolvedNames` with `imported = true`. This may
   simplify the current `unused_imports.rs` logic.  However, as noted
   in step 5, `resolved_names` only tracks *referenced* names, so
   detecting *unreferenced* imports requires cross-referencing with
   the declared-import list (still sourced from `use_map`).

9. ✅ **Run the full test suite.**
   All 2735 unit tests + 254 fixture tests pass after each step.

### Interaction with M4

M4 (mago-type-syntax) does NOT depend on mago-names. Type expression
parsing is purely syntactic — it takes a string and returns a type
AST. However, once both M3 and M4 are complete, the combination
enables a powerful pattern: resolve an identifier's FQN via
mago-names, then parse its docblock type via mago-type-syntax, and
work with fully-resolved structured types throughout. This is
especially valuable for the Laravel provider, where a relationship
return type like `HasMany<Post, $this>` needs both FQN resolution
(what is `Post`?) and type structure (what are the generic args?).

---

## M4. Migrate to `mago-type-syntax`

**What it replaces:** The string-based type pipeline — approximately
4,700 lines across:

- `src/docblock/type_strings.rs` (~630 lines — `split_type_token`,
  `split_union_depth0`, `clean_type`, `base_class_name`,
  `replace_self_in_type`, etc.)
- `src/docblock/generics.rs` (~230 lines — `parse_generic_args`,
  `extract_generic_value_type`, etc.)
- `src/docblock/shapes.rs` (~340 lines — `parse_array_shape`,
  `parse_object_shape`, etc.)
- `src/docblock/callable_types.rs` (~290 lines —
  `extract_callable_return_type`, `extract_callable_param_types`, etc.)
- `src/docblock/conditional.rs` (~215 lines —
  `extract_conditional_return_type`, `parse_conditional_expr`)
- Scattered `split_type_token` / `split_union_depth0` calls
  throughout `src/hover/`, `src/completion/`, `src/resolution.rs`,
  and `src/symbol_map/docblock.rs`.

**Why:** Every type in the system is `Option<String>`. Consumers
decompose these strings with hand-written depth-tracking parsers
(counting `<>`, `{}`, `()` nesting) at every use site. This is
fragile, repetitive, and makes it impossible to add features like
conditional-type evaluation, generic type substitution, or type
compatibility checks without yet more string surgery.

`mago-type-syntax` provides `PhpType` — a structured enum that
represents unions, intersections, generics, callables, shapes,
conditionals, etc. as a tree. One parse at extraction time; pattern
matching everywhere else.

**Risk:** Very high blast radius. Every struct that carries a type
field (`ParameterInfo::type_hint`, `MethodInfo::return_type`,
`PropertyInfo::type_hint`, `ConditionalReturnType::Concrete`, etc.)
is affected. The phased approach below is designed to make this
manageable.

### Phase 1: Introduce the type representation

**Goal:** Define `PhpType`, implement `PhpType::parse()` and
`PhpType::to_string()`, and prove they round-trip correctly against
the existing string pipeline. No existing code changes.

1. Add `mago-type-syntax` to `Cargo.toml`.

2. Create `src/types/php_type.rs`:
   ```
   pub enum PhpType {
       Named(String),                  // e.g. "int", "App\\User"
       Nullable(Box<PhpType>),         // ?T
       Union(Vec<PhpType>),            // T|U
       Intersection(Vec<PhpType>),     // T&U
       Generic(String, Vec<PhpType>),  // Collection<int, User>
       Array(Box<PhpType>),            // T[]
       ArrayShape(Vec<ShapeEntry>),    // array{name: string, age: int}
       ObjectShape(Vec<ShapeEntry>),   // object{name: string}
       Callable {                      // callable(T): U
           params: Vec<PhpType>,
           return_type: Option<Box<PhpType>>,
       },
       Conditional {                   // ($x is T ? U : V)
           param: String,
           condition: Box<PhpType>,
           then_type: Box<PhpType>,
           else_type: Box<PhpType>,
       },
       ClassString(Option<Box<PhpType>>),  // class-string<T>
       KeyOf(Box<PhpType>),            // key-of<T>
       ValueOf(Box<PhpType>),          // value-of<T>
       Raw(String),                    // fallback for unparseable strings
   }
   ```

3. Implement `PhpType::parse(s: &str) -> PhpType` using
   `mago_type_syntax::parse_str()` to parse into the crate's AST,
   then convert to our `PhpType`. The `Raw(String)` variant is the
   fallback for anything the parser rejects — this guarantees the
   function never fails.

4. Implement `PhpType::to_string() -> String` so we can convert back
   to the string representation for display (hover, completion
   detail, etc.).

5. Write round-trip tests: for every type string in the existing test
   suite, assert `PhpType::parse(s).to_string() == s` (or a
   canonically equivalent form).

### Phase 2: Dual representation on core types

**Goal:** Add `_parsed: Option<PhpType>` fields alongside existing
string fields on the core types. Populate them at extraction time.
No consumers change yet.

1. Add `return_type_parsed: Option<PhpType>` to `MethodInfo`.
2. Add `type_hint_parsed: Option<PhpType>` to `ParameterInfo`.
3. Add `type_hint_parsed: Option<PhpType>` to `PropertyInfo`.
4. Add `type_hint_parsed: Option<PhpType>` to `ConstantInfo`.
5. Populate these fields in `src/parser/classes.rs` and
   `src/parser/functions.rs` by calling `PhpType::parse()` on the
   existing string value.

At this point, every extracted type has both representations. Old
code keeps reading the string field; new code can read the parsed
field.

### Phase 3: Migrate consumers to structured types

**Goal:** Replace string-based type manipulation with `PhpType`
pattern matching, one module at a time. After each module is
migrated, its string-field reads are removed.

Modules in recommended migration order (least dependencies first):

1. **`src/hover/`** — Type display. Replace `split_type_token` /
   `clean_type` chains with `PhpType::to_string()` formatting.

2. **`src/completion/`** — Type matching for member access. Replace
   `base_class_name` / `extract_generic_value_type` chains with
   `PhpType::Generic` / `PhpType::Named` pattern matching.

3. **`src/resolution.rs`** — `resolve_type_string`. Replace the
   string-surgery approach (split on `|`, recurse, rejoin) with
   tree traversal on `PhpType`.

4. **`src/docblock/types.rs` and sub-modules** — The old string
   parsers. Once all consumers use `PhpType`, these become dead code
   and can be deleted.

5. **`src/symbol_map/docblock.rs`** — `emit_type_spans`. Replace the
   423-line recursive string decomposer with `PhpType` tree
   traversal + span emission. (The tag-level migration is already
   done; this is the type-level migration.)

6. **`src/diagnostics/`** — Type compatibility checks. Pattern match
   on `PhpType` variants instead of string prefix checks.

7. **`src/code_actions/`** — Type-aware refactorings. Use `PhpType`
   for type comparison, docblock generation, etc. Currently,
   `ResolvedType::type_strings_joined` joins all resolved types
   with `|`, which flattens intersection types (`A&B`) into unions
   (`A|B`). With `PhpType::Intersection` this is preserved.

### Phase 4: Migrate the Laravel provider

The Laravel provider (`src/laravel/`) has its own type manipulation
for Eloquent models, relationships, collections, and facades.

1. **Eloquent attribute types** — Replace string-based cast-type
   mapping with `PhpType` construction.

2. **Relationship return types** — Replace the string template
   `"HasMany<{model}>"` with `PhpType::Generic("HasMany",
   [PhpType::Named(model)])`.

3. **Collection generics** — Replace `format!("Collection<{},
   {}>", key, value)` with `PhpType::Generic` construction.

4. **Facade accessor resolution** — The `getFacadeAccessor` →
   class lookup produces a class name string. This stays as a string
   (it's a class name, not a type expression), but the *return type*
   it produces can be `PhpType::Named`.

### Phase 5: Remove string type fields

Once all consumers read `_parsed` fields:

1. Remove `return_type: Option<String>` from `MethodInfo` (rename
   `return_type_parsed` → `return_type`).
2. Remove `type_hint: Option<String>` from `ParameterInfo`,
   `PropertyInfo`, `ConstantInfo` (rename similarly).
3. Remove `native_return_type: Option<String>` /
   `native_type_hint: Option<String>` — these become
   `PhpType::Named` values populated from the AST hint.
4. Delete `src/docblock/type_strings.rs`, `generics.rs`, `shapes.rs`,
   `callable_types.rs` — the old string parsers.
5. Delete `ConditionalReturnType` enum (replaced by
   `PhpType::Conditional`).
6. Run the full test suite.

---

## Testing strategy

Each migration step (M3, M4) must pass the **full existing test
suite** before merging. This is the primary safety net.

Additional testing per migration:

| Migration | Extra tests |
| --------- | ----------- |
| M3 | Snapshot tests comparing `use_map`-based resolution with `OwnedResolvedNames`-based resolution across the fixture corpus. |
| M4 | Round-trip tests (`PhpType::parse(s).to_string() == s`) for every type string in the test suite. Per-module migration tests comparing old string-based output with new `PhpType`-based output. |

For M4 specifically, the dual-representation phase (Phase 2) enables
**shadow testing**: compute the result both ways and assert they
match, before removing the old path. This catches regressions without
blocking progress.

---

## Version alignment

All Mago crates should be pinned to the same release. At the time of
writing, the latest version is **1.15.x**. When upgrading, update all
Mago crates in a single commit and run the test suite.

The `mago-docblock` crate is already present in `Cargo.toml`. When
adding `mago-type-syntax` and `mago-names`, align them to the same
version.

---

## What this enables

Once M3 + M4 are complete:

- **Structured types everywhere.** No more string surgery for type
  manipulation. Generic substitution, conditional evaluation, and
  type compatibility checks become tree operations.

- **Correct name resolution.** Every identifier resolves to its FQN
  in a single pass. Auto-import and unused-import diagnostics become
  straightforward.

- **Foundation for advanced features.** Laravel Eloquent attribute
  completion, Blade template support, and PHPStan-level type
  inference all require structured types and correct name resolution.
  Building them on the new foundation avoids double work.

- **Reduced maintenance burden.** ~6,500+ lines of hand-written
  parsers replaced by well-tested upstream crates.