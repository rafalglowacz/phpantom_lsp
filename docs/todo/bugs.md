# PHPantom — Bug Fixes

## B1 — Pathological `unknown_member` performance on large service files

`collect_unknown_member_diagnostics` takes 194+ seconds on
`src/core/Purchase/Services/PurchaseFileService.php`, causing the
analyze command to time out (debug build) or hang indefinitely
(release build). The next-slowest collectors on the same file never
even get to run.

Other files show the same pattern at smaller scale: `unknown_member`
dominates the breakdown on every slow file (often 50–70% of the total
time). The worst offenders are large service/repository classes that
chain many method calls on Eloquent models, payment gateways, and
similar deeply-inherited types.

Likely causes to investigate:

- Repeated full class merges (inheritance + virtual members) for the
  same type within a single file — the per-file resolved-class cache
  may not be covering all code paths in the unknown-member walker.
- Expensive subject resolution (long `$this->foo()->bar()->baz()`
  chains) re-resolved for every member access instead of being
  cached across diagnostic collectors that share the same file.
- Virtual member synthesis (Laravel model provider) running
  repeatedly for the same model class.

Reproducer: run `phpantom_lsp analyze --project-root <project>`
on any Laravel project with large service classes and observe the
`unknown_member` timing in the slow-file breakdown.

## B2 — Variable resolution pipeline produces short names instead of FQN

The variable resolution pipeline (`resolve_rhs_expression`,
`try_inline_var_override`, `try_standalone_var_docblock`, etc.)
returns `ResolvedType` values whose `type_string` field contains
short class names from raw docblock text or AST identifiers.
Parameter types on `ClassInfo` members are already FQN (resolved
during `resolve_parent_class_names`), so comparisons between the
two fail on name form alone.

Sources of short names:

- `try_inline_var_override` in `completion/variable/resolution.rs`
  gets a `PhpType` from `find_inline_var_docblock` and passes it
  to `from_type_string` or `from_classes_with_hint` without
  resolving names through the use-map.
- `resolve_rhs_instantiation` in `completion/variable/rhs_resolution.rs`
  constructs `PhpType::Named(name.to_string())` from the raw AST
  identifier (short name) and passes it through
  `from_classes_with_hint`. The `ClassInfo` has the FQN, but the
  `type_string` field retains the short name.
- `try_standalone_var_docblock` in `closure_resolution.rs` has the
  same pattern as `try_inline_var_override`.
- `find_iterable_raw_type_in_source` and `find_var_raw_type_in_source`
  in `docblock/tags.rs` return raw docblock types; every caller
  that stores them in a `ResolvedType` preserves short names.

Current mitigation: `collect_type_error_diagnostics` applies
`resolve_names` with the class loader on every resolved argument
type before comparison, so `type_error.argument` diagnostics are
not affected. But other consumers (hover type display, definition
matching, etc.) still see short names.

Fixing at the source is complicated because the same `ResolvedType`
values feed the PHPDoc generation code actions, which need short
names for user-facing output. The proper fix is to always store
FQN in `type_string` and shorten at display time (the way
`implement_methods.rs` already does with `shorten_type`).

## B3 — Array access on bare `array` returns empty instead of `mixed`

When a parameter is typed as bare `array` (no generic annotation),
accessing an element with `$params['key']` resolves to an empty
type instead of `mixed`. This causes downstream issues:

- `$x = $params['key'] ?? null` resolves `$x` to `null` (only
  the RHS of `??`) instead of `mixed|null`, because the LHS
  array access produced nothing.
- `type_error.argument` then flags `null` passed to `string`
  even though the value could be any type at runtime.

The fix should make array access on bare `array` (and `mixed`)
return `mixed` so that downstream resolution and diagnostics
see the correct "we don't know" type.

Reproducer:

```php
function foo(array $params = []): void {
    $authToken = $params['authToken'] ?? null;
    if (!$authToken || !is_string($authToken)) {
        throw new \Exception('missing');
    }
    // $authToken is string here, but diagnostic sees null
    bar($authToken);
}
function bar(string $s): void {}
```

## B4 — Foreach loop prescan leaks reassigned type into RHS of same assignment

The loop-body prescan in `walk_foreach_statement` (around line 2512
in `src/completion/variable/resolution.rs`) walks the entire foreach
body with `cursor_offset = body_end` to discover loop-carried
assignments. When a foreach key variable is reassigned inside the
body (e.g. `$type = DeviationType::from($type)`), the prescan
resolves the RHS and adds the result (`DeviationType`) to the
variable's type set. This leaks the reassigned type into positions
where it should not be visible — specifically, the `$type` argument
on the RHS of the same assignment should still be `string` (the
foreach key type), not `DeviationType`.

The diagnostic false positive is now suppressed by the backed enum
check, but hover on `$type` inside `from($type)` still incorrectly
shows `DeviationType` instead of `string`.

The prescan should exclude assignments whose RHS contains the
variable being resolved, or the prescan results should not be
merged until after the current statement's RHS has been resolved.

Reproducer:

```php
/** @var array<string, string> */
$regexes = [];
foreach ($regexes as $type => $regex) {
    if (preg_match($regex, $message)) {
        $type = DeviationType::from($type);
        // hover on $type inside from() shows DeviationType
        // instead of string
    }
}
```

## B5 — Unresolved template parameters leak raw names into argument diagnostics

When a template parameter is not bound to a concrete type (either
because no argument carries the binding, or because the class was
instantiated without a generic annotation), the raw template name
(e.g. `TValue`, `TKey`, `TReduceReturnType`, `TClosure`) leaks
through to the type error checker. The diagnostic then reports
"expects TValue, got string" instead of recognising the parameter
as unresolved and suppressing the check.

This affects both function-level and class-level templates:

- **Function-level:** `@template TReduceReturnType` with
  `@return TReduceReturnType` where no argument binds the param.
- **Class-level:** `Collection<TKey, TValue>` where the Collection
  was created without a generic annotation (e.g. `collect([])`,
  `new Collection()`). Methods like `push($item)` still have
  `@param TValue $item` with the raw template name, so passing
  a `string` fires "expects TValue, got string".

Template substitution should either resolve the parameter from the
call-site arguments / class-level generic annotation, or fall back
to the template's bound (defaulting to `mixed`) so the raw name
never leaks through to downstream diagnostics.

Reproducer (function-level):

```php
/**
 * @template TReduceReturnType
 * @return TReduceReturnType
 */
function reduce_result() { return null; }

function takes_int(int $x): void {}

function test(): void {
    $result = reduce_result();
    takes_int($result); // false positive: "expects int, got TReduceReturnType"
}
```

Reproducer (class-level):

```php
/**
 * @template TValue
 */
class Collection {
    /** @param TValue $item */
    public function push($item): void {}
}

function test(): void {
    $items = new Collection();   // no generic annotation
    $items->push('hello');       // false positive: "expects TValue, got string"
}
```

## B7 — `createMock()` returns `MockObject` instead of `MockObject&T` intersection

PHPUnit's `createMock(Foo::class)` should return the intersection
type `MockObject&Foo`, but the resolution pipeline only produces
`MockObject`. This causes false-positive type errors whenever a
mock is passed to a function expecting the mocked type.

The fix belongs in the call resolution pipeline: when the callee
is `TestCase::createMock` (or `getMockBuilder(...)->getMock()`,
`createPartialMock`, `createStub`, etc.) and the argument is a
`class-string<T>` literal, the return type should be
`MockObject&T`. This is the same pattern as `@template T` with
`@return MockObject&T` — the stubs may already declare this, in
which case the issue is that template substitution doesn't fire
for the `class-string` argument.

Reproducer:

```php
use PHPUnit\Framework\TestCase;

class FooService {
    public function doWork(): string { return 'ok'; }
}

class FooTest extends TestCase {
    public function testFoo(): void {
        $mock = $this->createMock(FooService::class);
        // $mock is MockObject, should be MockObject&FooService
        $this->useFoo($mock); // false positive: expects FooService, got MockObject
    }
    private function useFoo(FooService $svc): void {}
}
```

## B8 — Class-level template parameters lost through chained method calls

When a method returns a generic class (e.g. `Collection<Product>`)
and the next method in the chain returns `self<TItem>` or another
type referencing a class-level template parameter, the template
substitution is lost because `resolve_call_return_types_expr`
converts intermediate `ResolvedType`s (which carry generic args) to
`Vec<Arc<ClassInfo>>` via `into_arced_classes`, discarding the
`type_string` field that holds the generic parameters.

The first call in the chain now correctly propagates generics (B6
fix), but the second call resolves the base through
`resolve_call_return_types_expr` → `MethodCall` →
`into_arced_classes`, which strips the generic info before the
method's return type can be template-substituted.

Fixing this requires threading `ResolvedType` (with its
`type_string`) through the `MethodCall` arm of
`resolve_call_return_types_expr` so that class-level template
arguments survive into the method return-type resolution step.

Reproducer:

```php
/**
 * @template TItem
 */
class Collection {
    /** @param TItem $item */
    public function add($item): void {}

    /** @return self<TItem> */
    public function filter(): self { return $this; }
}

class Product {}

class Store {
    /** @return Collection<Product> */
    public function products(): Collection { return new Collection(); }
}

function test(): void {
    $store = new Store();
    $product = new Product();
    // First level works (B6 fix): $store->products()->add($product)
    // Second level fails: $store->products()->filter()->add($product)
    // false positive: "expects TItem, got Product"
    $store->products()->filter()->add($product);
}
```


