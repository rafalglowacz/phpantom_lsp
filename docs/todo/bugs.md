# PHPantom — Bug Fixes

Every bug below must be fixed at its root cause. "Detect the
symptom and suppress the diagnostic" is not an acceptable fix.
If the type resolution pipeline produces wrong data, fix the
pipeline so it produces correct data. Downstream consumers
(diagnostics, hover, completion, definition) should never need
to second-guess upstream output.

## B2 — Variable resolution pipeline produces short names instead of FQN

**Root cause:** The variable resolution pipeline returns
`ResolvedType` values whose `type_string` field contains short
class names taken verbatim from docblock text or AST identifiers.
The pipeline never resolves these names through the use-map or
class loader before storing them.

**Where to fix:** Every code path that produces a `ResolvedType`
from raw source text must resolve names to FQN before returning.
The fix belongs in the resolution pipeline itself, not in each
downstream consumer. Specifically:

- `try_inline_var_override` in `completion/variable/resolution.rs`
  gets a `PhpType` from `find_inline_var_docblock` and passes it
  to `from_type_string` or `from_classes_with_hint` without
  resolving names through the use-map. It must resolve first.
- `resolve_rhs_instantiation` in `completion/variable/rhs_resolution.rs`
  constructs `PhpType::Named(name.to_string())` from the raw AST
  identifier (short name). It must resolve the name to FQN before
  wrapping it.
- `try_standalone_var_docblock` in `closure_resolution.rs` has the
  same pattern as `try_inline_var_override`.
- `find_iterable_raw_type_in_source` and `find_var_raw_type_in_source`
  in `docblock/tags.rs` return raw docblock types. Every caller
  that stores them in a `ResolvedType` must resolve names first.

Current mitigation: `collect_type_error_diagnostics` applies
`resolve_names` with the class loader on every resolved argument
type before comparison. This papers over the problem for one
consumer but leaves others broken (hover type display, definition
matching, etc.).

The proper fix is to always store FQN in `type_string` at the
point of creation and shorten at display time (the way
`implement_methods.rs` already does with `shorten_type`).
Consumers that need short names for user-facing output (e.g.
PHPDoc generation code actions) should shorten on the way out,
not expect short names from the pipeline.

## B3 — Array access on bare `array` returns empty instead of `mixed`

**Root cause:** The type resolution pipeline does not handle array
element access on the bare `array` type. When a parameter is typed
as `array` (no generic annotation), accessing an element with
`$params['key']` resolves to an empty/untyped result instead of
`mixed`.

**Where to fix:** The array access resolution code (wherever
`$var['key']` is resolved to a type) must recognise bare `array`
and `mixed` as "unknown element type" and return `mixed`. This is
a fix in the variable/expression type resolution pipeline, not in
any diagnostic.

**Downstream effect:** Once the pipeline returns `mixed` for array
access on bare `array`, the following resolve correctly without any
additional changes:

- `$x = $params['key'] ?? null` resolves `$x` to `mixed|null`
  instead of just `null`.
- `type_error.argument` no longer flags `null` passed to `string`
  because the resolved type is `mixed|null`, which is compatible
  with anything.

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

## B9 — `parent::__construct()` does not substitute `@extends` generics into inherited parameter types

**Root cause:** When a child class has `@extends Parent<Concrete>`
and calls `parent::__construct($arg)`, the diagnostic pipeline
resolves the callable target to the parent's constructor without
applying the child's `@extends` generic substitution. The parent
constructor's `@param ?T $item` retains the raw template name `T`
instead of being substituted with the concrete type from the
child's `@extends` annotation.

**Where to fix:** The callable target resolution for
`parent::__construct(...)` (in `resolve_constructor_callable` or
the `NewExpr` arm of `resolve_callable_target_with_args`) must
detect that the call originates from a child class, look up the
child's `extends_generics`, and apply template substitution to the
parent class before returning its constructor's parameter types.

Reproducer:

```php
/**
 * @template T of object
 */
class ItemResult {
    /** @param ?T $item */
    public function __construct(private readonly ?object $item) {}
}

/**
 * @extends ItemResult<BonusCashItem>
 */
final class BonusCashItemResult extends ItemResult {
    public function __construct(?BonusCashItem $credited) {
        parent::__construct($credited);
        // false positive: "expects ?T, got BonusCashItem"
    }
}

class BonusCashItem {}
```

## B10 — Foreach iteration on `@extends` subclass yields raw template param instead of concrete type

**Root cause:** When iterating over a variable whose type is a
subclass that extends a generic collection (e.g.
`IntCollection extends Collection<int, int>`), the foreach
element-type extraction does not look through the child's
`@extends` generics to substitute the parent's template params.
The iteration variable gets typed as raw `TValue` instead of `int`.

**Where to fix:** The foreach element-type resolution (in
`foreach_resolution.rs` or wherever the iterable element type is
extracted) must resolve `@extends` generics from the child class
before extracting the element type. When the variable's class is
`IntCollection` and it extends `Collection<int, int>`, the
iteration element type must be `int`, not `TValue`.

**Replicate on shared project:**

```
phpantom_lsp analyze --project-root shared --no-colour 2>/dev/null -- src/database/Model/Products/Filters/ProductFilterTermCollection.php
```

Reproducer:

```php
/**
 * @template TKey of array-key
 * @template TValue
 */
class Collection implements \ArrayAccess {
    /** @return TValue */
    public function offsetGet(mixed $offset): mixed {}
    public function offsetExists(mixed $offset): bool {}
    public function offsetSet(mixed $offset, mixed $value): void {}
    public function offsetUnset(mixed $offset): void {}
}

/** @extends Collection<int, int> */
final class IntCollection extends Collection {}

function test(): void {
    $ids = new IntCollection();
    foreach ($ids as $id) {
        // $id should be int, but resolves to TValue
        array_key_exists($id, [1 => 'a']);
        // false positive: "expects int|string, got TValue"
    }
}
```

## B11 — Static method-level `@template` not substituted when argument is a closure literal

**Root cause:** When a static method declares a method-level
`@template T of SomeType` and `@param T $param`, and the call-site
argument is a closure literal (e.g. `fn(array $q): bool => ...`),
`build_method_template_subs` either fails to resolve the argument
text to a type or the binding mode does not fire. The raw template
name (e.g. `TClosure`) leaks into the parameter type.

**Where to fix:** `build_method_template_subs` in
`call_resolution.rs` and/or `resolve_arg_text_to_type`. When the
argument text starts with `fn(` or `function(`, it should be
recognised as a `Closure` type (or more specifically
`Closure(params): ReturnType`) and used to bind the template param.

Reproducer:

```php
class Mockery {
    /**
     * @template TClosure of \Closure
     * @param TClosure $closure
     * @return ClosureMatcher
     */
    public static function on($closure) {
        return new ClosureMatcher($closure);
    }
}

class ClosureMatcher {}

function test(): void {
    Mockery::on(fn(array $query): bool => true);
    // false positive: "expects TClosure, got Closure"
}
```

## B8 — Class-level template parameters lost through chained method calls

**Root cause:** When a method returns a generic class (e.g.
`Collection<Product>`) and the next method in the chain accesses a
member of that class, the generic type arguments are discarded
during the chain resolution. Specifically,
`resolve_call_return_types_expr` converts intermediate
`ResolvedType` values (which carry generic args in their
`type_string` field) to `Vec<Arc<ClassInfo>>` via
`into_arced_classes`. This conversion discards the `type_string`,
so by the time the next method's return type needs to be
template-substituted, the generic arguments are gone.

**Where to fix:** The `MethodCall` arm of
`resolve_call_return_types_expr` must thread `ResolvedType` (with
its `type_string`) through to the method return-type resolution
step instead of flattening to bare `ClassInfo` first. The generic
arguments from the intermediate return type must survive into
`build_generic_subs` so that template substitution works at every
level of the chain, not just the first.

The first call in a chain already works (B6 fix). The fix here is
to apply the same pattern to subsequent calls in the chain.

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
    // First level works: $store->products()->add($product)
    // Second level fails: $store->products()->filter()->add($product)
    // false positive: "expects TItem, got Product"
    $store->products()->filter()->add($product);
}
```

## B12 — Hover cross-file property docblock cache invalidation fails after edits

**Root cause:** When a class is loaded from a cross-file source
(PSR-4 or classmap) and its docblock is later edited, hover
continues to show the stale docblock content instead of the updated
version. The parsed `ClassInfo` cached in `ast_map` and/or
`fqn_index` is not invalidated when the dependency file changes.

**Tests:** Six integration tests covering this bug were removed
because they were committed in a failing state. The fix must
include new passing tests for at least these scenarios:

- PSR-4 lazy-loaded class, then docblock edited (`did_change`)
- Dependent child class inheriting a changed `@property`
- `@var`-annotated variable accessing a cross-file property
- Method-chain access (`$this->getJob()->class_name`)
- Cache warm → edit → hover (eviction path)
- Child class with Model parent (Laravel `@property` interaction)

**Where to fix:** The cache layer that stores cross-file
`ClassInfo` results must be invalidated (or re-parsed) when
`didChange` or `didSave` fires for the dependency file. The
`resolved_class_cache` and/or `fqn_index` entries for the changed
URI must be evicted so that the next hover request re-parses the
file and picks up the new docblock content.
