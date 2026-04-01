# PHPantom — Diagnostics

Items are ordered by **impact** (descending), then **effort** (ascending)
within the same impact tier.

| Label      | Scale                                                                                                                  |
| ---------- | ---------------------------------------------------------------------------------------------------------------------- |
| **Impact** | **Critical**, **High**, **Medium-High**, **Medium**, **Low-Medium**, **Low**                                           |
| **Effort** | **Low** (≤ 1 day), **Medium** (2-5 days), **Medium-High** (1-2 weeks), **High** (2-4 weeks), **Very High** (> 1 month) |

---

## Severity philosophy

PHPantom assigns diagnostic severity based on runtime consequences:

| Severity        | Criteria                                                                                                                                                                                                                                                                                                                                                                                     | Examples                                                                                                                                                                                                                                                                      |
| --------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Error**       | Would crash at runtime. The code is definitively wrong.                                                                                                                                                                                                                                                                                                                                      | Member access on a scalar type (`$int->foo()`). Calling a function that doesn't exist (`doesntExist()`).                                                                                                                                                                      |
| **Warning**     | Likely wrong but could work for reasons we can't verify statically. The types are poor but the code might be correct at runtime.                                                                                                                                                                                                                                                             | Accessing a member that doesn't exist on a non-final class (`$user->grantAccess()` where `User` has no such method but a subclass might). Unknown class in a type position (`Class 'Foo' not found`). Subject type resolved to an unknown class so members can't be verified. |
| **Hint**        | The codebase lacks type information. Off by default or very subtle. Poorly typed PHP is so common that showing these by default would be noise for most users. Anyone who does care about type safety is likely running PHPStan already. Unless our engine becomes very strong, these diagnostics either expose our own inference gaps or bother users who never opted into static analysis. | `mixed` subject member access (opt-in via `unresolved-member-access`). Deprecated symbol usage (rendered as strikethrough).                                                                                                                                                   |
| **Information** | Advisory. Something the developer might want to know.                                                                                                                                                                                                                                                                                                                                        | Unused `use` import (rendered as dimmed). Unresolved type in a PHPDoc tag.                                                                                                                                                                                                    |

---

## D3. Deprecated rendering — chain subject resolution

**Impact: Low-Medium · Effort: Medium**

Chain subjects like `getHelper()->deprecatedMethod()` do not produce
a deprecated diagnostic because `resolve_subject_to_class_name` in
`diagnostics/deprecated.rs` returns `None` for non-variable,
non-keyword subjects (the `_ => None` arm). The function call return
type is never resolved, so the member deprecation check is skipped.

**Fix:** Route chain subjects through the completion/type-inference
pipeline to resolve the return type of the call before checking the
member for deprecation. The variable-resolution path already works
for `$var->deprecatedMethod()` via `resolve_variable_subject`; the
gap is function-call and method-call return types in subject position.

The following have been verified and are covered by tests:

- Deprecated class references in `new`, type hints, `extends`, and
  `implements` positions all render with strikethrough.
- Deprecated method calls, property accesses, and constants render
  with strikethrough (via both `$var->` and `ClassName::` subjects).
- Offset-based class resolution for `$this`/`self`/`static` resolves
  to the correct class in files with multiple class declarations.

---

## D5. Diagnostic suppression intelligence

**Impact: Medium · Effort: Medium**

When PHPantom proxies diagnostics from external tools, users need a way
to suppress specific warnings. Rather than forcing them to install a
separate extension or memorise each tool's suppression syntax, PHPantom
can offer **code actions to insert the correct suppression comment** for
the tool that produced the diagnostic.

PHPStan suppression is implemented: "Ignore PHPStan error" adds
`// @phpstan-ignore <identifier>` (appending to existing ignores when
present), and "Remove unnecessary @phpstan-ignore" cleans up unmatched
ignores reported by PHPStan. What remains:

### Remaining tools

- PHPCS: `// phpcs:ignore [Sniff.Name]` or `// phpcs:disable` /
  `// phpcs:enable` blocks.
- PHPMD (3.0): `#[SuppressWarnings(RuleName::class)]` as a PHP attribute.
- For PHPantom's own diagnostics: support `@suppress phpantom.*`
  in docblocks (matching PHP Tools' convention) and a config flag
  `phpantom.diagnostics.enabled: bool` (default `true`).

**Prerequisites:** Each tool needs a diagnostic proxy before its
suppression actions can be wired up.

---

## D6. Unreachable code diagnostic

**Impact: Low-Medium · Effort: Low**

Dim code that appears after unconditional control flow exits:
`return`, `throw`, `exit`, `die`, `continue`, `break`. This is a
Phase 1 (fast) diagnostic since it requires only AST structure, not
type resolution.

### Behaviour

| Scenario                                           | Rendering                           |
| -------------------------------------------------- | ----------------------------------- |
| Code after `return $x;` in same block              | Dimmed (DiagnosticTag::UNNECESSARY) |
| Code after `throw new \Exception()`                | Dimmed                              |
| Code after `exit(1)` or `die()`                    | Dimmed                              |
| Code after `continue` or `break` in a loop         | Dimmed                              |
| Code after `if (...) { return; } else { return; }` | Dimmed (both branches exit)         |

Severity: **Hint** with `DiagnosticTag::UNNECESSARY` so editors dim
the text rather than underlining it. This matches how unused imports
are rendered.

### Implementation

Walk the AST statement list. After encountering a statement that
unconditionally exits the current scope (return, throw, expression
statement containing `exit`/`die`), mark all subsequent statements in
the same block as unreachable. The span covers from the start of the
first unreachable statement to the end of the last statement in the
block.

Phase 1 only handles the simple single-block case. Whole-branch
analysis (both if/else branches exit) is a future refinement.

### Debugging value

When our type engine silently resolves a method to a `never` return
type (e.g. an incorrectly resolved overload), unreachable code after
the call becomes visible, signalling the bug.

---

## D8. Undefined variable diagnostic

**Impact: High · Effort: Medium**

Flag reads of variables that have no prior assignment or definition in
the reachable scope. This is the single most impactful diagnostic
PHPantom is missing compared to Intelephense (P1008). Every PHP
developer has been bitten by a variable name typo.

### Behaviour

| Scenario                                      | Severity | Message                                                          |
| --------------------------------------------- | -------- | ---------------------------------------------------------------- |
| `echo $nmae;` where only `$name` was assigned | Error    | Undefined variable '$nmae'                                       |
| `$x = $y + 1;` where `$y` was never assigned  | Error    | Undefined variable '$y'                                          |
| Variable used only inside one branch of an if | Error    | Undefined variable '$result' (if not assigned on the path taken) |

Severity is **Error** because accessing an undefined variable is a
runtime notice/warning (and `ErrorException` in strict setups).

### What counts as a definition

A variable is considered defined if any of the following occur before
the read, in the same or an enclosing scope:

- Direct assignment: `$x = ...;`, `$x += ...;`, `$x[] = ...;`
- Parameter: `function foo($x)`
- `foreach` binding: `foreach ($items as $key => $value)`
- `for` initialiser: `for ($i = 0; ...)`
- `catch` variable: `catch (Exception $e)`
- `list()` / `[...]` destructuring on the left-hand side of `=`
- `global $x;` statement
- `static $x;` statement
- Closure `use ($x)` clause
- Match arm binding (if PHP adds it in the future)
- `$this` inside a non-static method or closure bound to `$this`

### Superglobals (always defined)

`$_GET`, `$_POST`, `$_SERVER`, `$_REQUEST`, `$_SESSION`, `$_COOKIE`,
`$_FILES`, `$_ENV`, `$GLOBALS`, `$argc`, `$argv`, `$http_response_header`,
`$php_errormsg`.

### Suppression / false-positive avoidance

The following patterns should suppress the diagnostic for a variable:

- **`isset($var)` or `empty($var)`** — the variable is being guarded,
  not used.
- **`compact('var')`** — `$var` is referenced by string name.
- **`extract($array)`** — any variable could be defined; suppress all
  undefined-variable diagnostics in the function after an `extract()`
  call.
- **`$$dynamic`** — variable variables make static analysis unsound;
  suppress diagnostics in functions that use variable variables.
- **`@$var`** — the error suppression operator signals intentional use
  of a potentially undefined variable.
- **`unset($var)`** — marks the variable as undefined from that point
  forward, but `unset()` itself should not be flagged.
- **PHPDoc `@var Type $var`** on the preceding line — the developer is
  asserting the variable exists.

### Scope rules

- Function/method bodies are independent scopes. A variable assigned
  in one function is not visible in another.
- Closures capture variables only via `use ()` or `$this`.
- Arrow functions (`fn() =>`) capture all variables from the enclosing
  scope implicitly.
- Variables assigned inside an `if`/`else`/`while`/`for`/`switch`
  branch are potentially defined (Phase 1 can treat any assignment
  anywhere in the function as a definition; Phase 2 can do proper
  branch analysis).
- Global scope (outside any function) is a single scope.

### Implementation

Phase 1 (conservative, low false-positive rate):

1. Walk each function/method/closure body.
2. Collect all variable definitions (assignments, parameters, foreach,
   catch, list, global, static, use clause).
3. For each variable read (`$var` in an expression), check whether the
   name appears in the definitions set.
4. If not, and the name is not a superglobal, emit the diagnostic.
5. If the function contains `extract()` or `$$dynamic`, skip the
   entire function.

This is deliberately simple: it treats any assignment anywhere in the
function as sufficient, regardless of control flow. This avoids false
positives from branch-dependent definitions at the cost of missing
some genuinely undefined variables that are only assigned in one
branch. This is the same approach Intelephense takes.

Phase 2 (future, optional):

- Track definitions per control-flow branch.
- Flag variables that are defined in some branches but not all before
  the read point.
- This is significantly more complex and can be deferred.

### Existing infrastructure

The variable resolution code in `completion/variable/resolution.rs`
already walks AST scopes and tracks variable assignments for type
inference. The undefined-variable diagnostic needs the same scope
walking but with a simpler question: "was this name ever assigned?"
rather than "what type is it?". Much of the traversal logic can be
shared or adapted.

---

## D10. PHPMD diagnostic proxy

**Impact: Low · Effort: Medium**

Proxy PHPMD (PHP Mess Detector) diagnostics into the editor, following
the same pattern as the existing PHPStan proxy. PHPMD 3.0 (once
released) is the target version. It will get a `[phpmd]` TOML section
with `command`, `timeout`, and tool-specific options mirroring the
`[phpstan]` schema.

### Prerequisites

- PHPMD 3.0 must be released. Current 2.x output formats and rule
  naming may change.
- The diagnostic suppression code action (D5) should support PHPMD's
  `@SuppressWarnings(PHPMD.[RuleName])` syntax once the proxy exists.

### Implementation

1. Add a `[phpmd]` section to the config schema in `src/config.rs`
   with `command` (default `"vendor/bin/phpmd"`), `timeout`, and
   an `enabled` flag.
2. Run PHPMD with XML or JSON output on the current file (or changed
   files) and parse the results into LSP diagnostics.
3. Map PHPMD rule names to diagnostic codes so that suppression
   actions (D5) can insert the correct `@SuppressWarnings` annotation.
4. Respect the same debounce and queueing logic used by the PHPStan
   proxy to avoid overwhelming the tool on rapid edits.

---

## D11. Invalid class-like kind in context

**Impact: Medium · Effort: Low**

PHP accepts certain class-like names syntactically in positions where
they are guaranteed to fail at runtime or be silently useless. These
are not parse errors, so `php -l` does not catch them. PHPStan catches
some of these (e.g. `new` on an abstract class) but not all. A
dedicated diagnostic rule can flag them all consistently using the
same context knowledge that the completion system already applies
(the `ClassNameContext` enum and `TypeHint` variant).

The rule table:

| Position                   | What to flag                                  | Severity | Runtime behavior                          |
| -------------------------- | --------------------------------------------- | -------- | ----------------------------------------- |
| `new X`                    | Abstract class, interface, trait, enum        | Error    | Fatal error: Cannot instantiate           |
| `throw new X`              | Non-Throwable class                           | Error    | Fatal error: Cannot throw                 |
| `throw new X`              | Abstract class, interface, trait, enum        | Error    | Fatal error: Cannot instantiate           |
| `$x instanceof X`          | Trait                                         | Warning  | Always evaluates to `false`               |
| `catch (X $e)`             | Trait                                         | Warning  | Never catches anything                    |
| `catch (X $e)`             | Non-Throwable class or interface              | Error    | Never catches, uncaught exception crashes |
| `class A extends X`        | Final class                                   | Error    | Fatal error: Cannot extend final class    |
| `class A implements X`     | Class, trait, enum                            | Error    | Fatal error: Not an interface             |
| `interface A extends X`    | Class, trait, enum                            | Error    | Fatal error: Not an interface             |
| `class A { use X; }`       | Class, interface, enum                        | Error    | Fatal error: Not a trait                  |
| `function f(X $p)`, `): X` | Trait                                         | Warning  | Type check always fails                   |
| `public X $prop`           | Trait                                         | Warning  | Type check always fails                   |
| `@param X`, `@return X`    | Trait                                         | Hint     | Documents unsatisfiable constraint        |
| `@throws X`                | Non-Throwable class or interface, trait, enum | Hint     | Documents impossible throw                |

**Why Warning for traits in type positions (not Error).** PHP does not
reject the code at parse time or class loading time. The fatal
`TypeError` only occurs at the specific call site when a value actually
reaches the type check. Code paths that are never executed with a
mismatched value will run without error. This is different from `class
extends final` which crashes unconditionally when the class is loaded.

**Why Hint for PHPDoc.** PHPDoc has no runtime enforcement at all. A
trait in `@param` is useless documentation but does not crash anything.
This aligns with the severity philosophy: hints are for code quality
issues that static analysis enthusiasts care about.

**Implementation:**

1. During AST extraction (or as a post-parse diagnostic pass), walk
   class declarations and check `extends`, `implements`, and `use`
   references against loaded `ClassInfo` entries. If the referenced
   class is loaded and its kind does not match the position, emit
   a diagnostic.

2. For `new X`, `throw new X`, `instanceof X`, and `catch (X)`, scan
   expression nodes in method bodies. Resolve `X` to a `ClassInfo`
   (if loaded) and check kind/modifier compatibility.

3. For native type hints, scan parameter types, return types, and
   property types. Resolve each class-like reference and check for
   trait kind.

4. For PHPDoc types, scan `@param`, `@return`, `@var`, and `@throws`
   tags. Resolve each class-like reference. Flag traits in type
   positions and non-Throwable types in `@throws`.

5. Only flag references where the target class is loaded (in
   `ast_map` or stubs). Unknown classes should not be flagged here
   (that is D4's job). This avoids false positives from unloaded
   classmap entries where the kind is unknown.

**Relationship to completion filtering.** The completion context
detector (`ClassNameContext` enum in `class_completion.rs`) and this
diagnostic rule use the same underlying knowledge (which kinds are
valid in which positions). The completion system already prevents the
user from inserting a wrong kind; this diagnostic catches wrong kinds
that are already in the code. Both should share the same rule table
to stay in sync.

---

## D12. Mago linter integration (optional diagnostics)

**Impact: Medium · Effort: Medium**

PHPantom already depends on several mago crates (`mago-syntax`,
`mago-docblock`, `mago-names`, `mago-formatter`, `mago-span`). The
`mago-linter` crate provides ~159 lint rules covering redundancy,
best practices, clarity, consistency, correctness, and deprecation.
Integrating it as an optional diagnostics provider would give users
"PHPStan-lite" diagnostics without requiring PHPStan to be installed.

**Integration approach:** call `Linter::lint()` on the parsed AST
(already available), convert `IssueCollection` to LSP `Diagnostic`s,
convert `TextEdit` fixes to LSP `CodeAction`s. The linter is AST-only
(no type inference), so it is fast.

Offer as opt-in via `.phpantom.toml` configuration. Default to
disabled so it does not conflict with users who already run PHPStan
or Psalm. Mark with `source: "mago"` to distinguish from PHPantom's
own diagnostics.

**Notable rules:** `no-redundant-method-override`,
`str-contains`/`str-starts-with` modernization,
`prefer-arrow-function`, `constant-condition`, `no-self-assignment`,
`explicit-nullable-param`, `valid-docblock`.
