# Extracting Test Value from Phpactor

Phpactor ships with **261 `.test` fixture files** in `phpactor/lib/WorseReflection/Tests/Inference/` plus completion-level integration tests in `phpactor/lib/Completion/Tests/`. These files encode years of real-world PHP edge cases that we can mine for coverage gaps and regression scenarios.

This document is the plan for doing that systematically.

---

## How Phpactor's Tests Work

Each `.test` file is a standalone PHP snippet with inline type assertions via a magic `wrAssertType()` call:

```php
<?php

/** @template T */
class Foo {
    /** @return T */
    public function bar() {}
}

/** @extends Foo<Baz> */
class Child extends Foo {}

$c = new Child();
wrAssertType('Baz', $c->bar());
```

A single PHPUnit runner (`SelfTest.php`) globs every `.test` file, parses it through Phpactor's reflector, and the `wrAssertType` calls fire assertions internally. The tests are organised by category:

| Directory | Count | What it covers |
|---|---|---|
| `if-statement/` | 35 | Type narrowing: `instanceof`, `is_*`, `!`, `&&`, `\|\|`, early return, `die`, `break`, `continue` |
| `generics/` | 43 | `@template`, `@extends`, `class-string<T>`, constructor inference, iterators, generators |
| `function/` | 20 | Built-in function stubs: `array_map`, `is_int`, `assert`, `in_array`, `iterator_to_array` |
| `foreach/` | 13 | Key/value types, list destructuring, `IteratorAggregate`, docblock overrides |
| `type/` | 26 | Array shapes, conditional return types, `class-string`, closures, callables, `static`, `self`, literals, `never`, variadic |
| `reflection/` | 12 | Mixins (class, generic, recursive, static, multiple), promoted properties, circular deps |
| `assignment/` | 10 | Array mutation, list assignment, nested destructuring, ternary assignment |
| `enum/` | 6 | Backed/unit enum cases, traits on enums, custom members |
| `virtual_member/` | 7 | `@method`, `@property`, `@method static`, trait virtual methods, `$this`/`static` return |
| `binary-expression/` | 7 | Arithmetic, concat, bitwise, comparison, logical, array union |
| `call-expression/` | 5 | First-class callables, `__invoke`, closure invocation |
| `narrowing/` | 4 | `@phpstan-assert`, negated assertions, generic narrowing |
| `combination/` | 8 | Multi-type params, union narrowing with ancestors, inline assertion, intersection interfaces |
| Other | 65 | catch, cast, arrow functions, anonymous functions, ternary, subscript, null-coalesce, constants, generators, property hooks (8.4), pipe operator, qualified names, return statements, global, require/include, resolver, invalid AST |

Their completion tests (`WorseClassMemberCompletorTest.php`, `WorseLocalVariableCompletorTest.php`, etc.) use a `<>` cursor marker in PHP heredocs and assert on the returned suggestion names, types, short descriptions, and snippets.

---

## What We Can't Port Directly

- **The test runner.** `SelfTest.php` feeds PHP through Phpactor's `Reflector->reflectOffset()` API. We don't have that API — PHPantom doesn't expose a "resolve type at offset" function. It resolves types in service of specific LSP features (completion, definition, hover, signature help).
- **The completion harness.** Their `CompletorTestCase` creates PHP-level `Completor` objects. Our tests create a Rust `Backend` and drive it through `tower-lsp` types.
- **The assertion mechanism.** `wrAssertType()` is a magic function resolved inside Phpactor's inference engine. We assert on completion item labels, definition locations, and hover content.
- **Multi-assertion fixtures.** Many `.test` files call `wrAssertType` at multiple offsets in the same file (e.g. before and after an early return). Our fixture format supports a single cursor position per file. Multi-assertion fixtures must be split into separate fixture files — one per cursor position.

So we're not porting infrastructure — we're **mining scenarios**.

---

## What to Skip or Adjust

### Skip: tests that duplicate our existing 2,111 tests

Before converting any Phpactor fixture, search `tests/` for an existing test that covers the same scenario. We already have extensive coverage for:
- Basic member completion (methods, properties, constants)
- Visibility filtering (public/protected/private)
- Static vs instance access
- Parent:: completion
- `@method` / `@property` / `@mixin` virtual members
- `@extends` generic resolution
- Array shapes and object shapes
- Conditional return types
- Foreach collection iteration
- Guard clause narrowing (`instanceof`, early return, `assert`)
- Laravel model/factory/scope resolution
- Named arguments, signature help, hover

If a Phpactor fixture tests something we already have covered, skip it.

### Skip: tests that assert Phpactor-specific architecture

Some fixtures test Phpactor's internal reflection API, not PHP language semantics. Skip:
- `phpactor_reflection_collection` and `phpactor_reflection_of_type` in `generics/`
- Any fixture that asserts on Phpactor-specific type representations (e.g. literal int types like `12`, string literals like `"hello"`) that we don't surface

### Adjust: union completion semantics

PHPantom deliberately shows the **union** of all members across all possible types, not the intersection (see `ARCHITECTURE.md` § Union Type Completion). Phpactor sometimes asserts intersection semantics. When converting `combination/` and `if-statement/union_*` fixtures, adjust the expected results to match our design:
- After `instanceof A && instanceof B`, we show members from both A and B (union), not just shared members (intersection)
- Members that only exist on one branch of a union still appear in completion

### Adjust: `class-string<T>` constructor inference

Phpactor infers template types from constructor call-site arguments (e.g. `new Foo('hello')` resolves `T` to `string`). PHPantom resolves generics from **declared** `@extends`/`@implements` annotations and explicit `@var` tags, not from runtime argument analysis. The 4 `constructor-*` fixtures in `generics/` will not pass today and should be marked `#[ignore]` with a note linking to todo.md §2 (function-level `@template` generic resolution), which covers the infrastructure needed to make them work.

---

## Phase 1: Build a Fixture Runner (Infrastructure)

Before converting fixtures by hand, build a test runner that reads `.fixture` files from disk so adding new cases is a 30-second task.

### Fixture format

```
// test: generic extends resolves template parameter
// feature: completion
// expect: bar(
---
<?php

/** @template T */
class Foo {
    /** @return T */
    public function bar() {}
}

/** @extends Foo<Baz> */
class Child extends Foo {}

$c = new Child();
$c-><>
```

**Header** (above `---`):
- `// test:` — human-readable test name (becomes the `#[test]` name)
- `// feature:` — one of `completion`, `hover`, `definition`, `signature_help`
- `// expect:` — for completion: a label prefix that must appear in results (repeatable)
- `// expect_absent:` — a label that must NOT appear (repeatable)
- `// expect_hover:` — `symbol => ExpectedSubstring` to fire a hover request on `symbol` and check the response contains the substring. This is the only way to assert on resolved types, since we don't have a "resolve type at offset" API.
- `// expect_definition:` — `file:line` or `self:line` for go-to-definition
- `// ignore:` — mark the fixture as `#[ignore]` with a reason (e.g. `// ignore: needs todo.md §2 (function-level @template)`)
- `// files:` — optional, marks the start of multi-file fixtures (see below)

**Body** (below `---`):
- PHP source with a single `<>` cursor marker indicating where the LSP request fires.
- The runner strips `<>`, records its line/character, opens the file on a test `Backend`, and fires the appropriate LSP request.

> **Note on multi-assertion Phpactor fixtures:** Many `.test` files make multiple `wrAssertType` calls at different offsets. Since our format supports one cursor per file, split these into separate `.fixture` files — e.g. `type_after_return_before.fixture` and `type_after_return_after.fixture`. Name them clearly so the connection is obvious.

### Multi-file fixtures

For cross-file scenarios, the body can declare multiple files:

```
// test: cross-file PSR-4 completion
// feature: completion
// expect: doWork(
// files: src/Service.php, src/Helper.php
---
=== src/Helper.php ===
<?php
namespace App;
class Helper {
    public function doWork(): void {}
}
=== src/Service.php ===
<?php
namespace App;
class Service {
    public function run(Helper $h): void {
        $h-><>
    }
}
```

### Runner implementation

Create `tests/fixtures/` for the `.fixture` files and a runner module:

```
tests/
  fixtures/
    generics/
      class_extend_template.fixture
      constructor_params.fixture          # ignored: needs todo.md §2
      ...
    narrowing/
      instanceof.fixture
      type_after_return_narrowed.fixture
      ...
    ...
  fixture_runner.rs          # the generic test runner
```

`fixture_runner.rs` does:
1. Glob `tests/fixtures/**/*.fixture`
2. For each file: parse header + body, strip `<>` to get cursor position
3. Create a `Backend`, open file(s), fire the LSP request for the declared `feature`
4. Assert `expect` / `expect_absent` / `expect_hover` / `expect_definition`
5. Respect `// ignore:` by emitting `#[ignore]`

Use the `test_case` pattern or `datatest-stable` crate to generate one `#[test]` per fixture file, so each shows up individually in `cargo test` output.

### Tasks

- [x] Define the fixture header format (documented above)
- [x] Write `parse_fixture()` → `(TestMeta, Vec<(String, String)>, CursorPosition)`
- [x] Write runner functions for each feature: `run_completion_fixture`, `run_hover_fixture`, `run_definition_fixture`, `run_signature_help_fixture`
- [x] Integrate with `cargo test` via `datatest-stable` (`tests/fixture_runner.rs` with `harness = false`)
- [x] Add a `tests/fixtures/README.md` explaining the format
- [x] Add 3–5 trivial fixtures to prove the runner works end-to-end

---

## Phase 2: Audit Phpactor's Fixtures Against Our Coverage

Go through each Phpactor category and mark which scenarios we already cover, which we partially cover, and which are net-new.

### How to audit

For each `.test` file in `phpactor/lib/WorseReflection/Tests/Inference/<category>/`:
1. Read the PHP snippet and the `wrAssertType` assertion
2. Translate the assertion into "what would PHPantom need to return?" (completion item, hover content, definition location)
3. Search our `tests/` directory for an existing test that exercises the same scenario
4. Mark it in the checklist below as: ✅ covered, 🔶 partial, ❌ gap, ⏭️ skip (architecture mismatch or Phpactor-internal)

### Audit checklist

#### generics/ (43 files)

- [x] `class_extend1` — ✅ `generics/class_extend_template.fixture` — `@extends Parent<Concrete>` resolves template on inherited method
- [x] `class_extend2` — ✅ `generics/class_extend2_first.fixture` + `class_extend2_second.fixture` — chained extends with two template params (split into 2 fixtures for the 2 assertions)
- [x] `class_implements_single1` — ❌ `generics/class_implements_single.fixture` (ignored: @implements generic resolution not yet supported)
- [x] `class_implements_multiple1` — ❌ `generics/class_implements_multiple.fixture` (ignored: @implements generic resolution not yet supported)
- [x] `class_template_extends1` — ❌ `generics/class_template_extends.fixture` (ignored: @template-extends syntax not recognized, only @extends)
- [x] `class_template_implements1` — ❌ `generics/class_template_implements.fixture` (ignored: @implements generic resolution not yet supported, @template-implements syntax not recognized)
- [x] `constructor-params` — ❌ `generics/constructor_params.fixture` (ignored: needs todo.md §2)
- [x] `constructor-array_arg` — ❌ `generics/constructor_array_arg.fixture` (ignored: needs todo.md §2)
- [x] `constructor-generic-arg` — ❌ `generics/constructor_generic_arg.fixture` (ignored: needs todo.md §2)
- [x] `constructor-param-and-extend` — ❌ `generics/constructor_param_and_extend.fixture` (ignored: needs todo.md §2)
- [x] `class-string-generic` — ✅ `generics/class_string_generic.fixture` — `class-string<T>` resolves T from `Foo::class`
- [x] `class-string-generic-union` — ❌ `generics/class_string_generic_union.fixture` (ignored: needs function-level @template argument inference with variadic params, todo.md §2)
- [x] `class-string-generic-nested-return` — ❌ `generics/class_string_nested_return.fixture` (ignored: needs function-level @template argument inference, todo.md §2)
- [x] `class-string-generic-decared-interface` — ❌ `generics/class_string_generic_interface.fixture` (ignored: class-string<T> on interface method not inherited by implementing class)
- [x] `method_generic` — ❌ `generics/method_generic.fixture` (ignored: needs todo.md §2 function-level @template argument inference)
- [x] `method_generic_class-string-2nd-arg` — ✅ `generics/class_string_2nd_arg.fixture` — class-string as 2nd parameter
- [ ] `method_generic_class-string-union_return` — class-string method with union return (similar to class-string-generic-union but on method)
- [ ] `method_generic_covariant` — `@template-covariant` (hover-only assertion, low priority)
- [x] `method_returns_collection` — ❌ `generics/method_returns_collection.fixture` (ignored: needs @implements generic resolution and IteratorAggregate foreach support, todo.md §4)
- [ ] `method_returns_collection2` — variant of collection return (interface chain, similar to method_returns_collection)
- [ ] `method_returns_templated_generic` — method returns `Generic<T>` (needs constructor argument type inference, todo.md §2)
- [x] `nullable_template_param` — ✅ `generics/nullable_template_param.fixture` — `?T` template usage
- [ ] `parameter` — template parameter type resolution (needs @implements generic resolution on method params)
- [ ] `type_from_template_in_class` — template used as property type (hover-only assertion, low priority)
- [ ] `generic_with_this` — template resolving to `$this` (needs constructor argument type inference, todo.md §2)
- [x] `generator_1` — ❌ `generics/generator_foreach.fixture` (ignored: needs Generator generic foreach resolution, todo.md §4)
- [ ] `generator_2` — Generator with single type param (similar to generator_1)
- [ ] `generator_yield_from_1` — yield from with generics (uses wrReturnType, not applicable to completion)
- [x] `interface` — ❌ `generics/interface_extends_traversable.fixture` (ignored: needs @implements generic resolution and Traversable foreach support, todo.md §4)
- [ ] `iterable` — `iterable<K, V>` resolution (needs generic iterable foreach support)
- [x] `iterator1` — covered by `iterator2` fixture below (iterator1 has single type param, iterator2 has key+value)
- [x] `iterator2` — ❌ `generics/iterator_foreach.fixture` (ignored: needs @implements generic resolution and Iterator foreach support, todo.md §4)
- [x] `iterator_aggregate1` — ❌ `generics/iterator_aggregate_foreach.fixture` (ignored: needs @implements generic resolution and IteratorAggregate foreach support, todo.md §4)
- [ ] `iterator_aggregate2` — aggregate with complex nested type (similar to iterator_aggregate1)
- [ ] `array_access1` — `ArrayAccess<K, V>` offset get (relevant to todo.md §23)
- [ ] `array_access_resolve_method_type1` — ArrayAccess method resolution (relevant to todo.md §23)
- [x] `phpactor_reflection_collection` — ⏭️ **skip:** Phpactor-internal
- [x] `phpactor_reflection_of_type` — ⏭️ **skip:** Phpactor-internal
- [x] `gh-1530-example` — ✅ `generics/collection_chain_gh1530.fixture` — Collection first() through generic interface chain
- [x] `gh-1771` — ⏭️ **skip:** uses wrAssertOffset, not applicable to completion/hover
- [x] `gh-1800` — ❌ `generics/reflection_collection_chain.fixture` (ignored: needs @implements generic resolution and complex generic chain)
- [ ] `gh-1875` — GitHub issue regression (nested generic iterator chain, relevant to todo.md §16)
- [x] `gh-2295-test` — ✅ `generics/nested_factory_extends.fixture` — nested factory extends resolves through inheritance chain

#### if-statement/ (35 files)

> **Note:** Our narrowing module (`completion/types/narrowing.rs`) already handles `instanceof` (positive and negative), early return/die/break/continue guard clauses, `assert($x instanceof Foo)`, `@phpstan-assert`, `@phpstan-assert-if-true/false`, match-arm narrowing, ternary narrowing, and compound `&&`/`||` conditions. Most of these fixtures should **pass today** and belong in Tier 1 as regression tests, not Tier 2.
>
> Exceptions that are genuine gaps: `property` / `property_negated` (narrowing on `$this->prop`, not bare variables), `is_*()` function narrowing (depends on todo.md §3), and `variable_introduced_in_branch`.

- [x] `instanceof` — ✅ `narrowing/instanceof_narrows_type.fixture` — basic `instanceof` narrows type
- [x] `instanceof_removes_null` — ✅ `narrowing/instanceof_removes_null.fixture` — `instanceof` strips null from union
- [x] `instanceof_removes_scalar` — ✅ `narrowing/instanceof_removes_scalar.fixture` — `instanceof` strips scalar from union
- [x] `type_after_return` — ✅ `narrowing/type_after_early_return.fixture` — type narrows after early return (single assertion; original had 2)
- [x] `type_after_break` — ✅ `narrowing/type_after_break.fixture` — type narrows after break
- [x] `type_after_continue` — ✅ `narrowing/type_after_continue.fixture` — type narrows after continue
- [x] `type_after_exception` — ✅ `narrowing/type_after_throw.fixture` — type narrows after throw
- [x] `die` — ✅ `narrowing/type_after_die.fixture` — type narrows after `die()`/`exit()`
- [ ] `else` — else branch gets the negated type (uses `is_string`, depends on todo.md §3)
- [ ] `else_assign` — variable assigned in else (literal string types, low priority)
- [ ] `elseif` — elseif chain (uses `is_string`/`is_int`, depends on todo.md §3)
- [ ] `elseifdie` — elseif with die (uses `is_string`/`is_int`, depends on todo.md §3)
- [x] `and` — ✅ `narrowing/and_compound.fixture` — `&&` compound narrowing
- [x] `bang` — ✅ `narrowing/bang_negated_instanceof_die.fixture` — `!` negation with die
- [ ] `bangbang` — `!!` double negation (low priority edge case)
- [x] `false` — ✅ `narrowing/false_comparison_narrowing.fixture` — `=== false` check with die
- [ ] `if_or` — `||` in condition (uses untyped `$foo`, low priority)
- [ ] `is_not_string_and_not_instanceof` — compound negated checks (depends on todo.md §3 for `is_string` part)
- [ ] `multile_nested` — deeply nested if/else (low priority, no completion impact)
- [x] `multiple_statements` — ✅ `narrowing/sequential_narrowing.fixture` — sequential if blocks with returns
- [x] `multiple_statements_open_branches` — ✅ `narrowing/open_branches_no_leak.fixture` — multiple non-terminating branches
- [x] `multiple_statements_with_class` — ✅ `narrowing/narrowing_in_class_method.fixture` — narrowing inside class method
- [ ] `namespace` — narrowing with namespaced types (low priority, namespace resolution already tested elsewhere)
- [ ] `no_vars` — if without variables (no completion impact)
- [ ] `non-terminating-branch` — branch that doesn't terminate (uses `is_int`, depends on todo.md §3)
- [x] `nullable` — ✅ `narrowing/nullable_guard.fixture` — null check narrowing via negated instanceof + throw
- [x] `property` — ❌ `narrowing/property_narrowing.fixture` (ignored: narrowing on `$this->prop` not supported)
- [x] `property_negated` — ❌ `narrowing/property_narrowing_negated.fixture` (ignored: negated property narrowing not supported)
- [x] `remove_null_type1` — ✅ `narrowing/remove_null_not_null_check.fixture` — `!== null` strips null
- [x] `remove_null_type2` — ✅ `narrowing/remove_null_equal_return.fixture` — `null ===` with return strips null
- [x] `union_and` — ❌ `narrowing/union_and_instanceof.fixture` (ignored: double instanceof in && condition does not narrow untyped variable)
- [x] `union_and_else` — ✅ `narrowing/union_and_else.fixture` — after && instanceof with early return, remaining branches show all members
- [x] `union_or` — ✅ `narrowing/or_instanceof.fixture` — `instanceof A || instanceof B` → union
- [x] `union_or_else` — ✅ `narrowing/or_instanceof_else_narrows.fixture` — else after `||` strips both types
- [x] `variable_introduced_in_branch` — ❌ `narrowing/variable_introduced_in_branch.fixture` (ignored: variable assigned inside if branch not tracked after branch merges)

#### function/ (20 files)

> **Note:** These test `is_*()` function narrowing and built-in function return types. The `is_*()` narrowing depends on todo.md §3 (conditional return type parsing from stubs). Array function return types depend on todo.md §19 (array functions needing new code paths). Not yet converted; mark as `#[ignore]` with cross-references when converted.

- [ ] `array_map` — `array_map` return type
- [ ] `array_merge` — `array_merge` return type (relevant to todo.md §19)
- [ ] `array_pop` — `array_pop` return type
- [ ] `array_reduce` — `array_reduce` return type (relevant to todo.md §19)
- [ ] `array_shift` — `array_shift` return type
- [ ] `array_sum` — `array_sum` return type (relevant to todo.md §19)
- [ ] `assert` / `assert.properties` — `assert($x instanceof Foo)` narrows (likely already covered)
- [ ] `assert_not_object` / `assert_not_string` / `assert_object` / `assert_string` — `assert(is_string($x))` etc. (**ignore:** depends on todo.md §3)
- [ ] `assert_variable_and_not_is_string` — compound assert (**ignore:** depends on todo.md §3)
- [ ] `in_array` — `in_array` with strict narrows
- [ ] `is_callable` / `is_float` / `is_int` / `is_null` / `is_string` — `is_*()` narrowing (**ignore:** depends on todo.md §3)
- [ ] `iterator_to_array` / `iterator_to_array_from_generic` — `iterator_to_array` return type (relevant to todo.md §19)
- [ ] `namespaced` — function in namespace
- [ ] `reset` — `reset()` return type

#### type/ (26 files)

- [ ] `arrayshape` / `arrayshape_multiline` / `arrayshape_multiline_optional` — array shape parsing (likely already covered)
- [ ] `callable` — callable type (hover-only assertion, no completion impact)
- [x] `class-string` — ⏭️ **skip:** hover-only (asserts class-string<Foo> type string, no completion impact)
- [x] `class-string-new` — ❌ `type/class_string_new.fixture` (ignored: new $classStringVar does not resolve to the class type)
- [ ] `class-string-new-no-type` — new from untyped class-string (low priority)
- [x] `class-string-static-call` — ❌ `type/class_string_static_call.fixture` (ignored: $classStringVar::staticMethod() does not resolve return type)
- [ ] `closure` — Closure type (hover-only assertion, no completion impact)
- [x] `conditional-type` — ✅ `type/conditional_return_type.fixture` — conditional return type with class-string resolves
- [x] `conditional-type2` — ❌ `type/conditional_return_type_string.fixture` (ignored: literal string conditional not supported)
- [ ] `conditional-type3` — literal string conditional (non-matching branch, similar to conditional-type2)
- [x] `conditional-type-container` — ✅ `type/conditional_return_container.fixture` — conditional return type on container interface resolves from class-string
- [ ] `conditional-type-nested` — nested conditional (literal string matching, low priority)
- [x] `conditional-type-nullable` — ✅ `type/conditional_return_null.fixture` — conditional with null parameter resolves
- [ ] `conditional-type-on-function` — conditional return on standalone function (relevant to todo.md §3)
- [ ] `false` — `false` pseudo-type (hover-only assertion, no completion impact)
- [ ] `int-range` — `int<0, max>` range type (low priority — no completion impact)
- [ ] `list` — `list<T>` type (hover-only assertion, no completion impact)
- [ ] `never` — `never` type (hover-only assertion, no completion impact)
- [ ] `parenthesized` / `parenthesized_closure` — `(A|B)` grouping (hover-only assertions)
- [x] `self_context_trait` — ✅ `type/self_in_trait.fixture` — `self` in trait resolves to using class
- [x] `static` — ✅ `type/static_return_type.fixture` — `static` return type resolves to declaring class
- [x] `static_context` — ✅ `type/static_return_child.fixture` — `static` on parent resolves to child class
- [ ] `string-literal` — string literal type (low priority — no completion impact)
- [ ] `union_from_relative_docblock` — union from relative docblock reference (hover-only assertion)
- [ ] `variadic` — variadic parameter type (hover-only assertion)
- [x] `phpstan-type-alias` — ❌ `type/phpstan_type_alias.fixture` (ignored: @phpstan-type alias not resolved when used as return type in foreach)
- [x] `psalm-type-alias` — ⏭️ **skip:** structurally identical to phpstan-type-alias

#### foreach/ (13 files)

- [x] `assigns_type_to_item` — ✅ `foreach/item_type_from_docblock.fixture` — basic foreach item typing from `@var Type[] $arr`
- [ ] `assigns_type_to_key` — basic foreach key typing (hover-only, no completion fixture)
- [ ] `generic_iterator_aggregate` / `generic_iterator_aggregate_then_foreach` — IteratorAggregate in foreach (relevant to todo.md §16)
- [ ] `list_deconstruct` / `list_deconstruct_1` — `foreach ($arr as [$a, $b])` (literal types, low priority)
- [ ] `literal_keys` / `literal_values` / `literal_values_removes_dupes` — literal type preservation (low priority)
- [x] `namespaced` — ✅ `foreach/namespaced.fixture` — foreach with namespaced types resolves
- [ ] `preserve_types_after_break` — type after `break` in foreach (hover-only assertion)
- [x] `with_docblock` — ❌ `foreach/docblock_override.fixture` (ignored: @var on foreach loop variable with untyped collection not resolved)
- [ ] `gh-1708` — regression test (hover-only assertion)

#### reflection/ (12 files)

- [x] `mixin_class` — ✅ `reflection/mixin_class.fixture` — @mixin provides members from another class
- [x] `mixin_generic` — ✅ `reflection/mixin_generic.fixture` — @mixin with generic parameter resolves template
- [x] `mixin_properties` — ✅ `reflection/mixin_properties.fixture` — @mixin provides access to mixed-in class properties
- [ ] `mixin_recursive` — recursive mixin resolution
- [x] `mixin_static` — ✅ `reflection/mixin_static.fixture` — @mixin with static return type resolves to consuming class
- [x] `multiple_mixins` — ✅ `reflection/multiple_mixins.fixture` — multiple @mixin tags contribute members from all mixed classes
- [x] `mixin_recursive` — ✅ `reflection/mixin_recursive.fixture` — recursive mixin resolves without infinite loop
- [ ] `promoted_property_with_params` — constructor promotion (hover-only assertion)
- [ ] `self-referencing-constant` — constant referencing self (hover-only assertion)
- [x] `virtial_static_method` — ❌ `reflection/virtual_static_method.fixture` (ignored: @method static return type with generics not resolved on static call)
- [ ] `circular-dependency-trait` / `circular-dependency_interface` / `circular-dependency_parent` — circular dep protection (we have `MAX_INHERITANCE_DEPTH` / `MAX_TRAIT_DEPTH` / `MAX_MIXIN_DEPTH` guards, already tested indirectly)
- [ ] `gh-2207` — regression (hover-only assertion)

#### virtual_member/ (7 files)

- [x] `method` — ✅ `virtual_member/method_tag.fixture` — `@method` virtual methods appear in completion
- [ ] `method2` — complex `@method` with overridden parent (multi-assertion, would need splitting)
- [x] `property` — ✅ `virtual_member/property_tag.fixture` — `@property` virtual properties appear in completion
- [x] `method_and_property_with_same_name` — ✅ `virtual_member/method_and_property_same_name.fixture` — both appear in completion
- [x] `trait_method1` — ❌ `virtual_member/trait_method.fixture` (ignored: @method on trait does not propagate to class using it)
- [x] `virtual-method-returns-static` — ❌ `virtual_member/method_returns_static.fixture` (ignored: @method static return chaining not resolved to child class)
- [x] `virtual-method-returns-this` — ❌ `virtual_member/method_returns_this.fixture` (ignored: @method $this return chaining not resolved)

#### Remaining categories

- [ ] `assignment/` (10) — array mutation, list, ternary, nested destructuring
- [ ] `binary-expression/` (7) — arithmetic, concat, bitwise, comparison (low priority — no completion impact)
- [x] `call-expression/` (5) — 1 converted: `call_expression/invoke_return_type.fixture` ❌ (ignored: __invoke() return type not resolved when calling $obj()). Remaining: `1st-class-callable` (covered by `completion_first_class_callable.rs`), `invoke2` (Generator __invoke, needs generator support), `invoke-gh-1686` (regression), `type-from-invoked-callable` (callable invocation).
- [x] `combination/` (8) — 6 converted: `combination/narrow_abstract_assert.fixture` ✅, `combination/param_with_multiple_types.fixture` ✅, `combination/union_narrow_with_ancestors.fixture` ✅, `combination/union_narrow_negated.fixture` ✅, `combination/intersect_interface_assert.fixture` ❌ (ignored: sequential assert narrowing), `combination/property_instanceof.fixture` ❌ (ignored: property-level narrowing). Remaining 2: `union` (hover-only), `inline_assertion` (wrAssertType inside assert, not completion-testable).
- [x] `narrowing/` (4) — 3 converted: `narrowing/phpstan_assert_function.fixture` ✅, `narrowing/phpstan_assert_static.fixture` ❌ (ignored: static method @phpstan-assert), `narrowing/phpstan_assert_negated.fixture` ❌ (ignored: negated assert). 1 remaining: `narrow-generic` (generic narrowing with @phpstan-assert and class-string<T>).
- [x] `enum/` (5) — 5 converted: `enum/custom_member.fixture` ✅, `enum/enum_trait.fixture` ✅, `enum/enum_implements_interface.fixture` ✅, `enum/enum_case_members.fixture` ❌ (ignored: enum case instance properties not shown), `enum/backed_enum_case_members.fixture` ❌ (ignored: enum case instance properties not shown).
- [x] `catch-clause/` (2) — 2 converted: `catch_clause/basic_exception.fixture` ✅, `catch_clause/union_catch.fixture` ✅.
- [ ] `cast/` (1) — cast expression types (low priority)
- [ ] `anonymous_function/` (2) — closure as Closure type (hover-only assertion)
- [ ] `arrow_function/` (5) — arrow function parameter/return
- [ ] `constant/` (3) — namespaced constants, imported constants
- [ ] `generator/` (1) — yield expression type (likely already covered via `completion_generators.rs`)
- [ ] `ternary_expression/` (2) — ternary type inference
- [ ] `subscript-expression/` (1) — array shape access (relevant to todo.md §23: GTD for array shape keys)
- [ ] `null-coalesce/` (2) — `??` strips null (hover-only assertions)
- [x] `type-alias/` (2) — 1 converted: `type/phpstan_type_alias.fixture` ❌ (ignored: @phpstan-type alias not resolved when used as return type in foreach). `psalm-type-alias` is structurally identical; skip.
- [ ] `property-hooks/` (4) — PHP 8.4 property hooks (**ignore:** depends on todo.md §14)
- [ ] `pipe-operator/` (1) — pipe operator (**ignore:** depends on todo.md §1)
- [ ] `return-statement/` (4) — return type inference (low priority — no completion impact)
- [ ] `qualified-name/` (4) — function/class name resolution
- [ ] `global/` (1) — `global` keyword
- [ ] `invalid-ast/` (2) — missing paren, missing token recovery
- [ ] `variable/` (2) — braced expressions, pass-by-ref (relevant to todo.md §15: narrow types of `&$var` parameters)
- [ ] `resolver/` (2) — closure call expression

---

## Phase 3: Convert High-Value Fixtures

After auditing, convert the most valuable gaps into `.fixture` files. Priority order:

### Tier 1 — Regression tests for existing features (do first)

These cover scenarios where PHPantom already has the feature working. The value is catching regressions and confirming edge cases. Skip any that duplicate an existing `tests/completion_*.rs` test.

1. **if-statement/** — Most of the 35 files should pass today since we already handle `instanceof`, guard clauses, `assert`, `@phpstan-assert`, ternary narrowing, and compound `&&`/`||`. Convert as regression tests. Exclude: `property`/`property_negated` (genuine gap), `is_not_string_and_not_instanceof` (depends on §3), `union_and`/`union_and_else` (need assertion adjustment for union semantics). Remember to split multi-assertion fixtures.

2. **virtual_member/** — All 7 files. Direct regression tests for our `virtual_members` module. Likely high overlap with `completion_mixins.rs` — check before converting.

3. **type/** — Array shapes (3 files), conditional return types (7 files), `static`/`self` (3 files). These directly exercise our `docblock::conditional` and `docblock::shapes` modules. Skip `int-range` and `string-literal` (no completion impact).

4. **reflection/** — All mixin fixtures (6 files). Direct tests for `PHPDocProvider` mixin resolution. Check overlap with `completion_mixins.rs`.

5. **narrowing/** — All 4 `@phpstan-assert` files. We already support this in `narrowing.rs` — these are regression coverage.

6. **generics/** — Focus on: `class-string<T>` resolution (6 files), method-level templates (5 files), `@extends`/`@implements` chains (6 files). Skip the 4 `constructor-*` files (architecture mismatch) and 2 Phpactor-internal files. The `gh-*` regression files are worth converting if they cover non-trivial scenarios.

7. **foreach/** — IteratorAggregate (2 files), destructuring (2 files). Check overlap with `completion_foreach_collections.rs`.

8. **combination/** — All 8 files, with assertion adjustment for our union-completion design.

### Tier 2 — Ignored tests for planned features

These test features we don't have yet. Convert them as `#[ignore]` fixtures with a comment linking to the relevant todo.md item. They become ready-made acceptance tests when we start that work.

> **When converting an ignored fixture, also add a brief note to the relevant todo.md item** under a "Pre-existing test fixtures" heading, so anyone picking up that task knows they have tests waiting.

| Phpactor category | Blocked on | todo.md reference | Fixture count |
|---|---|---|---|
| `generics/constructor-*` | Constructor argument type inference for generics | §2 (function-level `@template`) | 4 |
| `function/is_*`, `function/assert_*_string` | `($param is T ? A : B)` return types from stubs | §3 (conditional return types) | ~10 |
| `property-hooks/` | PHP 8.4 property hook support | §14 (property hooks) | 4 |
| `pipe-operator/` | PHP 8.5 pipe operator | §1 (pipe operator) | 1 |
| `function/iterator_to_array*` | Array function return type resolvers | §19 (array functions) | 2 |
| `variable/pass-by-ref` | Reference parameter type narrowing | §15 (`&$var` parameters) | 1 |
| `if-statement/property*` | Property-level narrowing | No todo item yet — file one if fixtures fail | 2 |

### Tier 3 — Low priority (park for later)

These test scenarios with little completion impact or that require significant new infrastructure. Don't convert unless you're actively working in that area.

- **assignment/** (10) — expression-level type inference for array mutation, list destructuring
- **binary-expression/** (7) — arithmetic/concat/bitwise result types (only useful for diagnostics)
- **cast/** (1) — cast expression types (only useful for diagnostics)
- **return-statement/** (4) — return type inference (internal to Phpactor's frame system)
- **global/** (1) — `global` keyword (rare in modern PHP)
- **invalid-ast/** (2) — error recovery robustness
- **int-range, string-literal** from `type/` — no completion impact

---

## Phase 4: Also Mine the Completion Tests

Phpactor's completion tests in `Completion/Tests/Integration/Bridge/TolerantParser/WorseReflection/` are a separate gold mine. These test the end-to-end completion result (name, type, snippet, documentation) rather than just type inference. They map more directly to our test format since we already assert on completion items.

Key files:

| Test file | Cases | Relevance | Status |
|---|---|---|---|
| `WorseClassMemberCompletorTest.php` | ~60 yields | Member completion: visibility, static, virtual, parent::, nullable, union narrowing with completion | 🔶 12 fixtures converted |
| `WorseLocalVariableCompletorTest.php` | ~12 yields | Variable completion: partial matching, array shape keys as variables, closure `use` vars | Not started |
| `WorseSignatureHelperTest.php` | ~10 yields | Signature help edge cases | 🔶 3 fixtures converted |
| `WorseNamedParameterCompletorTest.php` | ~10 yields | Named argument completion | Not started |
| `WorseConstructorCompletorTest.php` | ? | Constructor completion | Not started |
| `WorseFunctionCompletorTest.php` | ? | Standalone function completion | Not started |
| `WorseSubscriptCompletorTest.php` | ~4 yields | Array subscript completion | Not started |
| `DocblockCompletorTest.php` | ? | PHPDoc tag completion | Not started |

The conversion is straightforward:

**Phpactor:**
```php
yield 'Public property access' => [
    '<?php
    class Barar { public $bar; }
    class Foobar { /** @var Barar */ public $foo; }
    $foobar = new Foobar();
    $foobar->foo-><>',
    [['type' => 'property', 'name' => 'bar']]
];
```

**PHPantom fixture:**
```
// test: chained property access resolves docblock type
// feature: completion
// expect: bar
---
<?php
class Barar { public $bar; }
class Foobar { /** @var Barar */ public $foo; }
$foobar = new Foobar();
$foobar->foo-><>
```

### Tasks

- [x] Read through `WorseClassMemberCompletorTest.php` and note unique scenarios not in our `tests/completion_*.rs`
- [x] Convert first batch of gaps into `.fixture` files in `completion/` directory (12 fixtures)
- [x] Read through `WorseSignatureHelperTest.php` and convert 3 signature help fixtures
- [ ] Read through `WorseLocalVariableCompletorTest.php` — their array-shape-key-as-variable-completion pattern is interesting
- [ ] Read through `WorseNamedParameterCompletorTest.php` and convert gaps
- [ ] Read through `WorseSubscriptCompletorTest.php` and convert gaps
- [ ] The `parent::` and `parent::__construct` completion tests are worth comparing against `completion_parent.rs` (✅ already converted as fixtures)

---

## Phase 5: Smoke Tests and Benchmarks

Phpactor has two more test layers we lack:

### Smoke tests

Their `tests/Smoke/RpcHandlerTest.php` verifies that every registered RPC handler is reachable. Our equivalent: start the actual `phpantom_lsp` binary, send `initialize` → `initialized` → a completion request → `shutdown`, and verify we get valid JSON-RPC responses.

- [ ] Create `tests/smoke.rs` (or a `tests/smoke/` directory)
- [ ] Test: binary starts, responds to `initialize`, returns capabilities
- [ ] Test: `textDocument/completion` returns valid items for a trivial PHP file
- [ ] Test: `textDocument/hover` returns content
- [ ] Test: `textDocument/definition` returns a location
- [ ] Test: `textDocument/signatureHelp` returns signatures
- [ ] Test: `shutdown` succeeds cleanly

### Benchmarks

Their `tests/Benchmark/CompleteBench.php` uses phpbench to track completion latency. We should do the same with `criterion` or `divan`:

- [ ] Create `benches/completion.rs`
- [ ] Benchmark: completion on a 500-line file with deep inheritance chain
- [ ] Benchmark: completion with 1000 classmap entries loaded
- [ ] Benchmark: cross-file completion via PSR-4 resolution
- [ ] Benchmark: `update_ast` parse time for a 2000-line file
- [ ] Track results in CI to catch regressions

---

## Cross-Reference: todo.md Items With Pre-Existing Phpactor Fixtures

When working on these todo.md items, check the corresponding Phpactor fixtures first — they may already be converted as `#[ignore]` fixtures, or the raw `.test` files provide ready-made test scenarios.

| todo.md item | Phpactor fixtures | Notes |
|---|---|---|
| §1 Pipe operator (PHP 8.5) | `pipe-operator/pipe-operator.test` | Single fixture, convert as `#[ignore]` |
| §2 Function-level `@template` | `generics/constructor-params.test`, `constructor-array_arg.test`, `constructor-generic-arg.test`, `constructor-param-and-extend.test` | 4 fixtures testing constructor inference; also `generics/method_generic.test` and related for method-level templates |
| §3 `($param is T ? A : B)` return types | `function/is_string.test`, `is_int.test`, `is_null.test`, `is_float.test`, `is_callable.test`, `assert_string.test`, `assert_not_string.test`, `assert_object.test`, `assert_not_object.test`, `assert_variable_and_not_is_string.test`; `type/conditional-type-on-function.test` | ~11 fixtures — the biggest payoff, these become acceptance tests for the `is_*()` narrowing feature |
| §5 Find References | No direct fixtures (Phpactor tests references at a different level) | Build our own |
| §7 Document Highlighting | No direct fixtures | Build our own using the smoke test pattern |
| §10 BackedEnum::from/tryFrom | `enum/backed_enum_case.test`, `enum/custom_member.test` | 2 fixtures, may partially cover |
| §14 Property hooks (PHP 8.4) | `property-hooks/*.test` | 4 fixtures, convert as `#[ignore]` |
| §15 `&$var` parameters | `variable/pass-by-ref.test` | 1 fixture |
| §16 SPL iterator generic stubs | `generics/iterator1.test`, `iterator2.test`, `iterator_aggregate1.test`, `iterator_aggregate2.test`; `foreach/generic_iterator_aggregate*.test` | 6 fixtures testing Iterator/IteratorAggregate generic resolution |
| §19 Array functions | `function/array_map.test`, `array_merge.test`, `array_pop.test`, `array_reduce.test`, `array_shift.test`, `array_sum.test`, `iterator_to_array*.test` | 8 fixtures for array function return types |
| §23 Array shape key GTD | `subscript-expression/array_shape_access.test` | 1 fixture |
| §30 `@deprecated` diagnostics | No direct fixtures (Phpactor tests this at the extension level) | Build our own; we already have `completion_deprecated.rs` |
| §31 Resolution-failure diagnostics | No direct fixtures | Build our own |

---

## Summary of Deliverables

| Phase | Deliverable | Status |
|---|---|---|
| 1 | Fixture runner infrastructure (`tests/fixture_runner.rs`, format spec, 5 proof-of-concept fixtures) | ✅ Done |
| 2 | Audit: 261 Phpactor fixtures mapped to our existing coverage (use the checklists above) | 🔶 Most categories audited; function/ and some remaining categories still pending |
| 3 Tier 1 | Regression tests for existing features | 🔶 72 passing fixtures converted across 12 categories |
| 3 Tier 2 | Ignored tests for planned features, with cross-references | 🔶 41 ignored fixtures converted |
| 4 | Completion test mining from Phpactor | 🔶 Started: 12 class-member completion fixtures + 3 signature help fixtures |
| 5 | Smoke test suite + benchmark suite | Not started |

**Current fixture counts (113 total):**

| Category | Passing | Ignored | Total |
|---|---|---|---|
| narrowing (if-statement + narrowing/) | 21 | 6 | 27 |
| generics | 8 | 18 | 26 |
| completion (from Phase 4 mining) | 15 | 2 | 17 |
| type | 6 | 4 | 10 |
| reflection | 6 | 1 | 7 |
| combination | 4 | 2 | 6 |
| virtual_member | 3 | 3 | 6 |
| enum | 3 | 2 | 5 |
| foreach | 2 | 1 | 3 |
| signature_help | 3 | 0 | 3 |
| catch_clause | 2 | 0 | 2 |
| call_expression | 0 | 1 | 1 |

**Gaps discovered during conversion (not previously tracked):**
- `@implements` generic resolution (class_implements_single, class_implements_multiple)
- `@template-extends` syntax (only `@extends` is recognized)
- `@template-implements` syntax (only `@implements` is recognized)
- `class-string<T>` on interface method not inherited by implementing class
- `@method` on trait does not propagate to class using it
- `@method` with `static` or `$this` return type does not chain through to child class
- `@phpstan-assert` on static method calls (only standalone function calls work)
- Negated `@phpstan-assert !Type` does not remove the asserted type from unions
- `@var` on foreach loop variable with untyped collection not resolved
- Literal string conditional return type resolution (`$param is "foo"`)
- Property-level narrowing (`$this->prop instanceof Foo`)
- `new $classStringVar` does not resolve to the class type
- `$classStringVar::staticMethod()` does not resolve return type
- `__invoke()` return type not resolved when calling `$obj()`
- Accessor on new line with extra whitespace before cursor not resolved
- Enum case instance properties (`name`, `value`) not shown in `->` completion
- Sequential `assert($x instanceof Foo)` calls do not accumulate narrowed types
- Double negated `instanceof` with early return does not narrow to remaining types
- Variable assigned inside if branch not tracked after branch merges
- `@phpstan-type` alias not resolved when used as method return type in foreach
- `@method static` with generic return type not resolved on static `::` call

**Recommended next steps: Phase 4 remaining (WorseLocalVariableCompletorTest, WorseNamedParameterCompletorTest, WorseSubscriptCompletorTest) → Phase 3 remaining categories (function/, assignment/, arrow_function/) → Phase 5 (smoke tests)**