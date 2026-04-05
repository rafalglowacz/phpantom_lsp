# PHPantom — CLI Fix Command

The `fix` subcommand applies automated code fixes across a PHP project,
modeled after php-cs-fixer. Each "rule" corresponds to a diagnostic code
and its associated code action.

## Implemented Rules

### `unused_import` — Remove unused imports

Detects unused `use` statements and removes them. Handles simple imports,
group imports (removing individual members), and blank-line collapsing to
keep formatting clean.

---

## Planned Native Rules

These rules use PHPantom's own diagnostics and do not require external
tools.

### FX1. `deprecated` — Replace deprecated symbol usage

**Prerequisite:** The existing `replace_deprecated` code action.

When a symbol is marked `@deprecated` and the deprecation message
contains a replacement hint (e.g. "use Foo::newMethod() instead"),
automatically apply the replacement. Only apply when the replacement
can be mechanically determined from the deprecation message.

### FX2. `unused_variable` — Remove unused variables

**Prerequisite:** D4 (Unused variable diagnostic, Sprint 5).

Remove assignments to variables that are never read. Skip variables
with side effects in the RHS (method calls, function calls). When the
RHS is pure (literal, property access, simple expression), remove the
entire statement.

### FX7. `add_return_type` — Generate `@return` docblocks from function bodies

Wire up the existing "Generate PHPDoc" code action's return-type
inference to the fix CLI. When a function or method has a native
`array` return type (or no return type at all) and the body contains
enough information to infer a specific element type, add a `@return`
tag with the inferred type (e.g. `@return list<Butterfly>`).

This lets teams that want to reach PHPStan level 6 (require return
type declarations) run a single command and get specific, useful
return types across the entire codebase for free, instead of adding
them by hand file by file.

---

## Planned PHPStan Rules

These rules require running PHPStan first to collect diagnostics.
They are gated behind `--with-phpstan`.

### FX3. `phpstan.return.unusedType` — Remove unused type from return union

**Backlog ID:** H10

Parse the unused type from PHPStan's message, find the return type
(native or `@return`), remove the unused member from the union or
intersection, and rewrite. If removing the type leaves a single-member
union, simplify.

### FX4. `phpstan.missingType.iterableValue` — Add `@return` with iterable type

**Backlog ID:** H17

When PHPStan reports that a return type has no value type specified in
an iterable type (e.g. `array`), add a `@return array<mixed>` docblock
tag. The simple approach silences the error while being explicit. A
future enhancement could infer element types from `return` statements.

### FX5. `phpstan.property.unused` / `phpstan.method.unused` — Remove unused member

**Backlog ID:** H19

When PHPStan reports an unused property, method, or class constant,
remove the entire declaration including its docblock.

### FX6. `phpstan.generics.callSiteVarianceRedundant` — Remove redundant variance

**Backlog ID:** H20

Strip `covariant` or `contravariant` keywords from generic type
arguments in docblocks when PHPStan reports them as redundant.

---

## Infrastructure

### Rule selection

Rules are identified by their diagnostic code string:
- Native rules: bare identifiers (e.g. `unused_import`)
- PHPStan rules: prefixed with `phpstan.` (e.g. `phpstan.return.unusedType`)

When no `--rule` flags are provided, all "preferred" native rules run.
A rule is "preferred" if its corresponding code action has
`is_preferred: true` in the LSP protocol.

PHPStan rules only run when `--with-phpstan` is passed. This is an
explicit opt-in because PHPStan adds significant runtime (it must
analyze the entire project first).

### PHPStan integration

When `--with-phpstan` is enabled:

1. Run PHPStan on all target files (or the entire project if no path
   filter is given) in a single batch invocation.
2. Parse the JSON output to collect diagnostics per file.
3. Match diagnostics to registered PHPStan rules.
4. For each matched diagnostic, compute and apply the code action edit.

To maximize efficiency, PHPStan runs once for all files rather than
per-file. The diagnostic-to-rule matching uses the PHPStan identifier
(e.g. `return.unusedType`) with the `phpstan.` prefix.

### Dry-run mode

`--dry-run` reports what would change without writing files. Exit code
`2` indicates fixable issues were found. This is useful for CI
pipelines that want to enforce code style without modifying files.

### Idempotency

Running `fix` twice should produce the same result as running it once.
Each rule must be idempotent: if the fix has already been applied, the
rule should detect no issues and make no changes.

### Exit codes

| Code | Meaning |
|------|---------|
| 0    | Success (fixes applied, or nothing to fix) |
| 1    | Error (bad arguments, write failure, etc.) |
| 2    | Dry-run found fixable issues |