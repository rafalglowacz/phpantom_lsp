# PHPantom — Bug Fixes

Known bugs and incorrect behaviour. These are distinct from feature
requests — they represent cases where existing functionality produces
wrong results. Bugs should generally be fixed before new features at
the same impact tier.

Items are ordered by **impact** (descending), then **effort** (ascending).

| Label | Scale |
|---|---|
| **Impact** | **Critical**, **High**, **Medium-High**, **Medium**, **Low-Medium**, **Low** |
| **Effort** | **Low** (≤ 1 day), **Medium** (2-5 days), **Medium-High** (1-2 weeks), **High** (2-4 weeks), **Very High** (> 1 month) |

---

## 0. Native type hints not considered in virtual property specificity ranking

**Impact: Low-Medium — Effort: Medium**

The `type_specificity` function used during virtual member merging only
scores the `type_hint` field (the effective/docblock type). It does not
consider `native_type_hint` (the PHP-declared type on the property).

For example, a real property declared as `public string $name;` has
`native_type_hint = Some("string")` and `type_hint = Some("string")`.
If a docblock or virtual provider contributes `@property array<int> $name`,
the specificity comparison works correctly today because both values flow
through `type_hint`.

However, the broader issue is in `resolve_effective_type`: when a native
hint says `string` and a docblock says `array<int>`, the effective type
should be the docblock's version (it is more specific and deliberately
overrides the native hint). This is not specific to virtual member merging
but to the general type resolution pipeline. Fixing it here would not help
because the native vs docblock decision happens upstream in the parser.

This is out of scope for the virtual member specificity work but worth
tracking as a separate improvement to `resolve_effective_type`.

---

## 1. Diagnostics fire on type alias array shape object values

**Impact: High — Effort: Low**

**Release blocker for 0.5.0.** Our own `example.php` shows the bug.

When a method returns a `@phpstan-type` alias that expands to an array
shape containing object values (e.g. `array{name: string, pen: Pen}`),
accessing a method on the object value triggers a false
`unresolved_member_access` diagnostic:

```php
/** @phpstan-type UserData array{name: string, email: string, pen: Pen} */
class TypeAliasDemo {
    /** @return UserData */
    public function getUserData(): array { /* … */ }

    public function demo(): void {
        $data = $this->getUserData();
        $data['pen']->write();    // ← diagnostic: "Cannot resolve type of '$data['pen']'"
    }
}
```

Completion works correctly here (offering `Pen` methods after
`$data['pen']->`), so the type resolution pipeline knows the type.
The diagnostic provider is not reaching the same conclusion. The
likely cause is that the diagnostic's subject resolution does not
expand `@phpstan-type` aliases before checking array shape value
types.

**Reproduces in:** `example.php` lines 637–641 (`TypeAliasDemo`),
both `$data['pen']->write()` and `$status['owner']->getEmail()`.

### Where to start

- **Diagnostic entry point:**
  `src/diagnostics/unresolved_member_access.rs` →
  `collect_unresolved_member_access_diagnostics`. This is the function
  that emits the `unresolved_member_access` diagnostic. Trace how it
  resolves the subject type for a `$var['key']->method()` expression.
- **Working completion path:**
  `src/completion/array_shape.rs` → `build_array_key_completions`
  calls `resolve_type_alias` before extracting shape value types.
  The diagnostic path likely skips this alias expansion step.
- **Type alias resolution:**
  `src/completion/types/resolution.rs` → `resolve_type_alias`.
  This is the function that expands `@phpstan-type` names into their
  definitions. The fix is probably a matter of calling it in the
  diagnostic's subject resolution path before checking array shape
  value types.
- **Type alias storage:** `ClassInfo.type_aliases` in `src/types.rs`.
  Populated by `src/docblock/tags.rs` during parsing.

---

## 2. Inline array-element function calls resolve to native return type in diagnostics

**Impact: High — Effort: Low**

**Release blocker for 0.5.0.** Our own `example.php` shows the bug.

When an array-element function (`end()`, `current()`, `reset()`, etc.)
is called inline as the subject of a member access, the diagnostic
resolver falls back to the function's native PHP return type
(`mixed|false`) instead of extracting the element type from the
array argument's generic annotation:

```php
$src = new ScaffoldingArrayFunc(); // has members: array<int, Pen>

$cur = current($src->members);
$cur->write();                    // ✓ no diagnostic (variable assignment path works)

end($src->members)->write();      // ✗ diagnostic: "subject type 'mixed|false'"
```

The completion pipeline handles this correctly via
`resolve_call_return_types_expr` → `ARRAY_ELEMENT_FUNCS` →
`resolve_inline_arg_raw_type`. The diagnostic sees the same subject
text but apparently resolves the call's return type from the stub
signature rather than the generic element type.

**Reproduces in:** `example.php` line 1060
(`end($src->members)->write()`).

### Where to start

- **Diagnostic entry point:**
  `src/diagnostics/unknown_members.rs` →
  `collect_unknown_member_diagnostics`. This emits the
  `unknown_member` diagnostic. It calls `resolve_target_classes` to
  resolve the subject. Add a log or breakpoint here to see what
  subject text the diagnostic sees for the `end(…)->write()` span.
- **Subject extraction:**
  `src/subject_extraction.rs` extracts the text left of `->`. For
  `end($src->members)->write()`, the subject should be
  `end($src->members)`. Verify this is what reaches the resolver.
- **Working completion path:**
  `src/completion/call_resolution.rs` →
  `resolve_call_return_types_expr`, specifically the
  `SubjectExpr::FunctionCall` arm around line 407. It checks
  `ARRAY_ELEMENT_FUNCS`, calls `resolve_inline_arg_raw_type` to get
  the array's generic type, then extracts the element type via
  `extract_generic_value_type`. The diagnostic must be taking a
  different path that skips this logic and falls through to the
  stub's native `mixed|false` return type.
- **Array element function list:**
  `src/completion/variable/mod.rs` → `ARRAY_ELEMENT_FUNCS`. This is
  the list of functions (`end`, `current`, `reset`, etc.) that should
  resolve to the array's element type rather than their native return.

---

## 3. Flaky `unknown_member` diagnostic on Eloquent Builder scope chains

**Impact: High — Effort: Medium**

**Release blocker for 0.5.0.** Our own `example.php` shows the bug.

Eloquent scope methods on Builder chains produce false
`unknown_member` diagnostics that flip on and off based on cache
state:

```php
Bakery::where('open', true)->fresh()->get();
// ← flags: Method 'fresh' not found on 'Illuminate\Database\Eloquent\Builder'
// ← retyping `>` as `>` (a no-op edit) flips the diagnostic away
// ← retyping again brings it back
```

A no-op edit triggering `didChange` is enough to flip the result.
This confirms the issue is cache invalidation, not code proximity.
The resolved class cache serves a differently-merged `ClassInfo` for
`Builder` depending on whether the cache was populated before or
after the edit cycle.

The non-determinism is the critical part. A diagnostic that is
consistently wrong can be investigated and worked around. A diagnostic
that flickers on every keystroke erodes trust in every diagnostic
PHPantom produces. Users will assume all warnings are unreliable.

Possible causes:
- The `resolved_class_cache` returns a `ClassInfo` for `Builder`
  that was merged before the Model's scope methods were fully loaded.
  After `didChange` invalidates the cache, the next resolution
  happens to load in a different order and produces a complete merge.
- Virtual member merging (scope methods are forwarded from the Model
  to the Builder via `__call`) is order-dependent: the result differs
  based on whether the Model was fully resolved at the time the
  Builder entry was cached.
- The cache is keyed by class name but not by the set of local
  classes available at resolution time. Two resolution passes with
  different `ast_map` contents can produce different results under
  the same cache key.

**Reproduces in:** `example.php` line 1440
(`Bakery::where('open', true)->fresh()->get()`).

### Where to start

- **Diagnostic entry point:**
  `src/diagnostics/unknown_members.rs` →
  `collect_unknown_member_diagnostics`. Same entry point as bug §2.
  The diagnostic resolves the subject via `resolve_target_classes`,
  gets back a `ClassInfo` for `Builder`, then checks whether `fresh`
  exists on it via `member_exists`. Log what methods the resolved
  `Builder` class has to see whether the scope methods are present.
- **Resolved class cache:**
  `src/virtual_members.rs` → `ResolvedClassCache` (type alias for a
  `DashMap` or similar). This is what `resolved_class_cache` points
  to. Check how entries are inserted and whether `didChange` in
  `src/server.rs` clears or partially clears this cache.
- **Cache invalidation:**
  `src/server.rs` → `did_change` handler. After re-parsing the file,
  trace what caches are cleared. If `resolved_class_cache` is not
  fully cleared (or is cleared but re-populated from stale
  `ast_map` data), that would explain the flip-flop.
- **Virtual member merging:**
  `src/virtual_members.rs` or `src/inheritance.rs` →
  `resolve_full_class` or similar. This is where scope methods from
  a Model are merged onto its Builder. The merge depends on loading
  the Model class first. If the Model is not yet in `ast_map` when
  the Builder is resolved and cached, the cached Builder will be
  missing the scope methods.
- **Debugging approach:** Add logging to the cache lookup: on hit,
  log the class name and the number of methods on the cached
  `ClassInfo`. Compare runs with and without the no-op edit. The
  method count difference will tell you exactly which methods are
  missing from the incomplete merge.