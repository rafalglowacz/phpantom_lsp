# PHPantom — Code Actions

Items are ordered by **impact** (descending), then **effort** (ascending)
within the same impact tier.

| Label | Scale |
|---|---|
| **Impact** | **Critical**, **High**, **Medium-High**, **Medium**, **Low-Medium**, **Low** |
| **Effort** | **Low** (≤ 1 day), **Medium** (2-5 days), **Medium-High** (1-2 weeks), **High** (2-4 weeks), **Very High** (> 1 month) |

No quick fixes or refactoring suggestions exist today. No `codeActionProvider` in
`ServerCapabilities`, no `textDocument/codeAction` handler, and no
`WorkspaceEdit` generation infrastructure beyond trivial `TextEdit`s for
use-statement insertion.

---

## 1. Implement missing abstract/interface methods
**Impact: Medium · Effort: Medium**

When a non-abstract class extends an abstract class or implements an
interface but is missing required method implementations, offer a code
action to generate stubs for all missing methods.

### Behaviour

- Detect the gap: resolve the full class hierarchy (already done by
  `resolve_class_with_inheritance`), collect all abstract methods from
  parent classes and all methods from implemented interfaces, subtract
  the methods the class already defines.
- Offer a code action: `Implement missing methods` (or list them
  individually: `Implement Foo::bar`, `Implement Baz::qux`).
- Generate method stubs at the end of the class body with:
  - Correct visibility and static modifiers matching the interface/abstract declaration.
  - Parameter names, type hints, and default values from the parent.
  - Return type from the parent.
  - PHPDoc block inherited from the parent (or `{@inheritDoc}`).
  - Body: `throw new \RuntimeException('Not implemented');` or
    `// TODO: Implement` — pick one convention.

**Why this is a good first code action:** It exercises the full
`codeActionProvider` → `WorkspaceEdit` → `TextEdit` pipeline without
needing scope analysis or cross-file edits. The class hierarchy data
is already fully resolved. This builds the infrastructure that Extract
Function and other code actions depend on.

---

## 2. Simplify with null coalescing / null-safe operator
**Impact: Medium · Effort: Medium**

Offer code actions to simplify common nullable patterns:

- `isset($x) ? $x : $default` → `$x ?? $default`
- `$x !== null ? $x : $default` → `$x ?? $default`
- `$x === null ? $default : $x` → `$x ?? $default`
- `$x !== null ? $x->foo() : null` → `$x?->foo()`
- `if ($x !== null) { return $x->foo(); } return null;` → `return $x?->foo();`

### Implementation

- Register as code actions with kind `quickfix` or `refactor.rewrite`.
- Pattern-match on ternary expressions and simple if-null-return blocks
  in the AST. The conditions are structural — no type resolution needed
  for the basic patterns (just checking for `=== null` / `!== null` /
  `isset()`).
- Generate replacement text preserving the original variable/expression
  names.
- Only offer `?->` suggestions when the project targets PHP 8.0+
  (check `self.php_version()`).

**Scope:** Start with ternary expressions (simplest AST match). The
if-statement patterns are a follow-up.

---

## 3. Extract Function refactoring
**Impact: Low-Medium · Effort: Very High**

Select a range of statements inside a method/function and extract them into a
new function. The LSP would need to:

1. **Scope analysis** — determine which variables are read in the selection but
   defined before it (→ parameters) and which are written in the selection but
   read after it (→ return values).
2. **Statement boundary validation** — reject selections that split an
   expression or cross control-flow boundaries in invalid ways.
3. **Type annotation** — use variable type resolution to generate parameter and
   return type hints on the new function.
4. **Code generation** — produce a `WorkspaceEdit` that replaces the selection
   with a call and inserts the new function definition nearby.

### Scope analysis detail

Step 1 is the hard part. Today our variable resolution
(`completion/variable/resolution.rs`) walks backward from the cursor to
find assignments, which is sufficient for completion but not for
extract-function. Extract-function needs a **forward** pass that tracks
_all_ variable definitions and usages across the enclosing function body,
not just the ones that lead to the cursor.

Phpactor solves this with a "frame" model — a stack of scopes where each
scope records its own local variable assignments with byte offsets. The
key ideas worth borrowing:

- **Frame = scope boundary.** Each function body, closure, arrow
  function, and `catch` block opens a new frame. Variables defined inside
  a frame are local to it (closures capture via `use()`, arrow functions
  capture by value). A `foreach`, `if`, or `for` block does _not_ open a
  new frame in PHP — variables leak into the enclosing scope.

- **Assignment list with offsets.** Each frame stores a flat list of
  `(variable_name, byte_offset, type)` entries. Walking the AST in
  source order and recording every `$var = …`, parameter declaration,
  `foreach ($x as $k => $v)`, and `catch (E $e)` populates this.

- **Read set / write set per range.** Given the user's selected range
  `[start, end)`:
  - **Parameters** = variables _read_ inside `[start, end)` whose most
    recent assignment is _before_ `start`.
  - **Return values** = variables _written_ inside `[start, end)` that
    are _read after_ `end` in the enclosing scope.
  - **Locals** = variables whose entire lifetime (first write to last
    read) is contained within `[start, end)` — these stay inside the
    extracted function and do not become parameters or return values.

- **`$this` handling.** If the selection reads `$this` (or `self::`/
  `static::`), the extracted code must be a method on the same class,
  not a standalone function.

- **Reference parameters (`&$var`).** If a variable is passed by
  reference into the selection _and_ modified, the extracted function
  needs a `&$param` — or it becomes part of the return tuple.

We do _not_ need Phpactor's full per-expression-node resolver system for
this. Our existing variable resolution + type narrowing infrastructure
can resolve the type of each variable at the extraction boundary. The new
piece is the forward walk that collects the read/write sets.

**Implementation approach:** build a lightweight `ScopeCollector` that
walks the enclosing function's AST once, recording every variable
read/write with its byte offset. The extract-function logic then
partitions those entries by `[start, end)` to derive params, returns,
and locals. This collector could also serve document highlights
(`textDocument/documentHighlight`) since it produces "all occurrences of
variable X in this scope" as a natural byproduct.

### Prerequisites (build these first)

| Feature | What it contributes |
|---|---|
| Hover | "Resolve type at arbitrary position" — needed to type params |
| Document Symbols (see `todo-lsp-features.md`) | AST range → symbol mapping — needed to find enclosing function and valid insertion points |
| Find References (see `todo-lsp-features.md`) | Variable usage tracking across a scope — the same "which variables are used where" analysis |
| Implement missing methods (§1) | Builds the code action + `WorkspaceEdit` plumbing |

---

## 4. Switch → match conversion
**Impact: Low · Effort: Medium**

Offer a code action to convert a `switch` statement to a `match`
expression when the conversion is safe (PHP 8.0+).

### When the conversion is safe

- Every `case` body is a single expression statement (assignment to the
  same variable, or a `return`).
- No `case` body falls through to the next (every case ends with
  `break`, `return`, or `throw`).
- The switch subject is a simple expression (variable, property access,
  method call) — not something with side effects that shouldn't be
  evaluated multiple times.

### Implementation

- Walk the AST for `Statement::Switch` nodes.
- Check each arm against the safety criteria above.
- If all arms pass, build the `match` expression text:
  - Each `case VALUE:` becomes `VALUE =>`.
  - `default:` becomes `default =>`.
  - The body expression (minus the trailing `break;`) becomes the arm's
    RHS.
  - If all arms assign to the same variable, hoist the assignment:
    `$result = match ($x) { ... };`.
  - If all arms return, convert to `return match ($x) { ... };`.
- Offer as `refactor.rewrite` code action kind.
- Only offer when `php_version >= 8.0`.

**Note:** This is a structural AST transformation with no type
resolution dependency, but the safety checks for fall-through and
side-effect-free subjects require careful AST inspection. Not trivial,
but bounded in scope.

---

## 5. Import class
**Impact: High · Effort: Medium**

When a class name is used without a corresponding `use` statement (or
full qualification), offer a code action to add the import. This is the
single most commonly used code action in any PHP language server.

### Behaviour

- **Trigger:** The cursor is on an unqualified or partially qualified
  class name that does not match any existing `use` import or any class
  defined in the current namespace.
- **Resolution:** Search the class index (class_map, PSR-4 loader,
  stubs) for classes whose short name matches. If multiple candidates
  exist, offer one code action per candidate so the user can pick.
- **Edit:** Insert `use Fully\Qualified\ClassName;` in the import block
  at the top of the file (after the `namespace` declaration, grouped
  with existing `use` statements). If no `use` block exists yet, create
  one with a blank line separating it from the namespace declaration and
  the first code line.
- **Sorting:** Insert the new `use` statement in alphabetical order
  among existing imports. Do not reorder or reformat existing imports.
- **Code action kind:** `quickfix` (so it appears in the lightbulb menu
  and can be triggered via the quick-fix keybinding).

### Edge cases

- **Group imports:** `use Foo\{Bar, Baz}` — insert a new standalone
  `use` rather than modifying the group. Merging into groups is a
  follow-up.
- **Aliased imports:** If the short name collides with an existing
  import (e.g. `use Other\Request;` already exists), offer to import
  with an alias: `use Fully\Qualified\Request as FqRequest;`. Choosing
  a good alias name automatically is hard — a simple heuristic is to
  prepend the parent namespace segment (e.g. `HttpRequest`,
  `FoundationRequest`).
- **Trait `use` inside class bodies:** These are not namespace imports
  and should not be touched.
- **Functions and constants:** `use function` and `use const` imports
  are a follow-up. Start with class imports only.

### Implementation

- Register as `codeActionProvider` in `ServerCapabilities` (first code
  action to do so — builds the infrastructure).
- Add a `textDocument/codeAction` handler that checks whether the
  symbol under the cursor is an unresolved class reference.
- Use `find_or_load_class` / class index to find candidates.
- Generate a `WorkspaceEdit` containing a single `TextEdit` that
  inserts the `use` line at the correct position.

### Prerequisites

None — the class resolution infrastructure already exists.

---

## 6. Remove unused imports
**Impact: Medium · Effort: Low**

Offer a code action to remove `use` statements that are not referenced
anywhere in the file. Pairs naturally with the unused `use` dimming
diagnostic (see `todo-diagnostics.md §4`).

### Behaviour

- **Trigger:** The cursor is on (or the diagnostic is attached to) a
  `use` statement that is not referenced in the file.
- **Single removal:** One code action per unused import:
  `Remove unused import 'Foo\Bar'`.
- **Bulk removal:** When multiple unused imports exist, also offer
  `Remove all unused imports` which removes all of them in a single
  edit.
- **Code action kind:** `quickfix` for the single removal (attached to
  the diagnostic), `source.organizeImports` for the bulk removal.

### What counts as "used"

A `use Foo\Bar;` import is considered used if `Bar` (or the alias name)
appears as:
- A type hint in a parameter, return type, property type, or catch clause.
- A class reference in `new Bar()`, `Bar::method()`, `Bar::CONST`,
  `Bar::$prop`, `instanceof Bar`, or `extends`/`implements` clauses.
- A reference in a PHPDoc type: `@param Bar $x`, `@return Bar`,
  `@var Bar`, `@throws Bar`, `@mixin Bar`, `@extends Bar<T>`,
  `@implements Bar<T>`.
- A reference in an attribute: `#[Bar]`, `#[Bar(args)]`.
- A string literal class reference is NOT counted (too unreliable).

### Implementation

- Reuse the unused-use detection logic from the dimming diagnostic
  (`todo-diagnostics.md §4`). The diagnostic marks which imports are
  unused; the code action just generates `TextEdit`s to delete them.
- When deleting a `use` line, also delete the trailing newline so no
  blank lines accumulate.
- For group imports (`use Foo\{Bar, Baz}`), if only some members are
  unused, remove just those members from the group. If all members are
  unused, remove the entire group statement.

### Prerequisites

- Unused `use` detection (shared with the dimming diagnostic).