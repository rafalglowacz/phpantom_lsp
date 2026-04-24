# PHPantom — Phpactor Test Parity

Track remaining gaps between phpactor's inference test suite
(`phpactor/lib/WorseReflection/Tests/Inference/`) and PHPantom's
fixture tests (`tests/fixtures/`). Each section groups related gaps
and references the specific phpactor `.test` files to port when the
underlying feature is implemented or verified.

When completing an item, port the phpactor test as a `.fixture` file,
verify it passes, and delete the item from this file. If a feature is
not planned, mark the item with *(won't fix)* and a reason.

---

## Already tracked elsewhere

These gaps have dedicated todo items with fixtures already created
(some ignored). No action needed here — they are listed for
completeness so we don't duplicate work.

| Gap | Todo ref | Fixture(s) |
|-----|----------|------------|
| Null coalesce type refinement | [T8](type-inference.md#t8-null-coalesce--type-refinement) | `null_coalesce/non_nullable_lhs.fixture`, `null_coalesce/nullable_lhs.fixture` |
| Dead-code after `never` return | [T9](type-inference.md#t9-dead-code-elimination-after-never-returning-calls) | `type/never_return_type.fixture` |
| Ternary RHS in list destructuring | [T10](type-inference.md#t10-ternary-expression-as-rhs-of-list-destructuring) | `assignment/list_destructuring_conditional.fixture` |
| Nested list destructuring | [T11](type-inference.md#t11-nested-list-destructuring) | `assignment/nested_list_destructuring.fixture` |

---

## 10. Variable-variable (`${$bar}`) resolution

phpactor tests `${$bar}` resolving to the type of the inner
variable's value.

**phpactor ref:** `variable/braced_expression.test`

**Effort: Low-Medium** — niche feature. Ignored fixture created at
`variable/variable_variable.fixture`.

---

## 19. Binary expression literal-value precision

Type-level binary expression inference is implemented and ported from
phpactor as fixtures in `binary_expression/`. PHPantom resolves the
*type* of each operator (e.g. `1 + 2` → `int|float`,
`'a' . 'b'` → `string`, `1 === 1` → `bool`). What remains is
phpactor's literal-value precision — resolving constant expressions
to their concrete values:

| Category | phpactor ref | PHPantom | phpactor |
|----------|-------------|----------|---------|
| Arithmetic | `binary-expression/arithmetic.test` | `int\|float` | `2`, `1`, `4`, … |
| Concatenation | `binary-expression/concat.test` | `string` | `"ab"` |
| Comparison | `binary-expression/compare.scalar.test` | `bool` | `true`/`false` |
| Logical | `binary-expression/logical.test` | `bool` | `true`/`false` |
| Bitwise | `binary-expression/bitwise.test` | `int` | `0`, `1`, `5`, … |
| Array union | `binary-expression/array-union.test` | `array` | combined shape |

**Effort: Medium** — requires a constant-expression evaluator.
Fixtures ported at type level:
`binary_expression/arithmetic.fixture`,
`binary_expression/array_union.fixture`,
`binary_expression/bitwise.fixture`,
`binary_expression/comparison.fixture`,
`binary_expression/concat.fixture`,
`binary_expression/logical.fixture`,
`binary_expression/instanceof.fixture`.

---

## 20. Postfix increment/decrement

`$i++` on a literal `0` → `1`, `$i--` on literal `2` → `1`.

**phpactor ref:** `postfix-update/increment.test`,
`postfix-update/decrement.test`

**Effort: Low** — niche. Only relevant for literal type tracking.

---

## Summary by effort

### Low-Medium effort (need minor code changes)

| # | Item | phpactor ref |
|---|------|-------------|
| 10 | Variable-variable `${$bar}` | `variable/braced_expression.test` |

### Low priority

| # | Item | phpactor ref |
|---|------|-------------|
| 19 | Binary expression literal-value precision | `binary-expression/*.test` |
| 20 | Postfix increment/decrement | `postfix-update/*.test` |