# PHPantom — Diagnostics

Items are ordered by **impact** (descending), then **effort** (ascending)
within the same impact tier.

| Label | Scale |
|---|---|
| **Impact** | **Critical**, **High**, **Medium-High**, **Medium**, **Low-Medium**, **Low** |
| **Effort** | **Low** (≤ 1 day), **Medium** (2-5 days), **Medium-High** (1-2 weeks), **High** (2-4 weeks), **Very High** (> 1 month) |

---

## Severity philosophy

PHPantom assigns diagnostic severity based on runtime consequences:

| Severity | Criteria | Examples |
|---|---|---|
| **Error** | Would crash at runtime. The code is definitively wrong. | Member access on a scalar type (`$int->foo()`). Calling a function that doesn't exist (`doesntExist()`). |
| **Warning** | Likely wrong but could work for reasons we can't verify statically. The types are poor but the code might be correct at runtime. | Accessing a member that doesn't exist on a non-final class (`$user->grantAccess()` where `User` has no such method but a subclass might). Unknown class in a type position (`Class 'Foo' not found`). Subject type resolved to an unknown class so members can't be verified. |
| **Hint** | The codebase lacks type information. Off by default or very subtle. Poorly typed PHP is so common that showing these by default would be noise for most users. Anyone who does care about type safety is likely running PHPStan already. Unless our engine becomes very strong, these diagnostics either expose our own inference gaps or bother users who never opted into static analysis. | `mixed` subject member access (opt-in via `unresolved-member-access`). Deprecated symbol usage (rendered as strikethrough). |
| **Information** | Advisory. Something the developer might want to know. | Unused `use` import (rendered as dimmed). Unresolved type in a PHPDoc tag. |

---

## 1. Scalar member access diagnostic — remaining gaps
**Impact: High · Effort: Low**

Member access on a scalar type is always a runtime error. The
`scalar_member_access` diagnostic (Error severity) is implemented for
bare variables, property chains, and call expression returns. Duplicate
suppression (dropping the `unresolved_member_access` hint when a more
specific diagnostic overlaps) is also done.

### Remaining gaps

| Scenario | Current | Expected |
|---|---|---|
| `$user->getName()->trim()` | Silent | **Error**: Cannot access method 'trim' on type 'string' |
| `$user->getAge()->value` | Silent | **Error**: Cannot access property 'value' on type 'int' |

These are method-return-chain subjects where the return type is scalar.
The `resolve_scalar_subject_type` function handles `CallExpr` callee
types for standalone functions and static methods, but method call
chains where the intermediate callee is itself a call expression are
not yet covered.

---

## 2. Chain and function-return member diagnostics
**Impact: High · Effort: Medium**

When a method return chain or function call return resolves to a known
class, unknown member diagnostics should fire on the final member. This
is the same "known type, missing member" logic that already works for
direct variables and parameters, but the symbol map's `subject_text`
for chain and call expressions isn't being resolved through to the end.

### Current state

- Direct variable and parameter member access: working.
- Static access on known class: working.
- Property chains (`$user->getProfile()->nonexistent`): NOT flagged.
- Function return member access (`getUser()->nonexistent`): NOT flagged.

### Gaps to fix

| Scenario | Current | Expected |
|---|---|---|
| `$user->getProfile()->nonexistent` | Silent | **Warning**: Property 'nonexistent' not found on class 'Profile' |
| `$user->getProfile()->fakeFn()` | Silent | **Warning**: Method 'fakeFn' not found on class 'Profile' |
| `getUser()->nonexistent` | Silent | **Warning**: Property 'nonexistent' not found on class 'User' |
| `getUser()->fakeMethod()` | Silent | **Warning**: Method 'fakeMethod' not found on class 'User' |

### Implementation notes

The completion resolver pipeline (`resolve_target_classes`) handles all
these subject forms correctly for completion. The issue is that the
symbol map's `subject_text` for these expressions may not carry enough
context, or the diagnostic walker doesn't pass it through the full
resolver pipeline. Verify that the `subject_text` captured for
`$user->getProfile()->nonexistent` is `$user->getProfile()` and that
`resolve_target_classes` returns `Profile` for it.

---

## 3. Chain error propagation (flag only the first broken link)
**Impact: Medium · Effort: Medium**

In a fluent chain like `$m->callHome()->callMom()->callDad()`, only the
first broken link should be flagged. Subsequent links in the chain
depend on the return type of the broken call, so flagging them adds
noise without actionable information.

### Current state

- Fluent chains on `mixed` subjects: only the first link is flagged
  (the chain members have no `MemberAccess` spans because the parser
  can't resolve the subject). This works by accident.
- Fluent chains on known types where the first method is unknown:
  only the first is flagged (same reason, parser stops). Also works
  by accident.
- Scalar chains (`$user->getAge()->value->deep`): the scalar member
  access at `->value` should be flagged but `->deep` should be silent.
  Currently `->value` is not flagged at all (see item 1).

### Desired behavior

- `$m->callHome()->callMom()->callDad()` — flag only `callHome`.
- `$m->callHome(); $m->callMom();` — flag both (separate statements).
- `$user->fakeMethod()->next()->deep()` — flag only `fakeMethod`.
- `$user->getAge()->value->deep` — flag only `->value` (scalar error).

### Cross-assignment propagation (nice to have)

```
$home = $m->callHome();  // flagged
$home->callMom();        // ideally silent
```

This is harder because it requires tracking that `$home` was assigned
from an already-flagged expression. Acceptable if not implemented in
the first pass.

---

## 4. Deprecated rendering
**Impact: Low-Medium · Effort: Low**

Deprecated class references (e.g. `new OldHelper()`) currently show a
hint message but the `DiagnosticTag::DEPRECATED` tag (which renders as
strikethrough in most editors) may not be applied correctly for all
deprecated symbol types. Verify that:

- Deprecated class references in `new`, type hints, `extends`, and
  `implements` positions all render with strikethrough.
- Deprecated method calls render with strikethrough.
- Deprecated property accesses render with strikethrough.
- The deprecated diagnostic resolver uses offset-based class resolution
  (not "first class in file") for `$this`/`self`/`static` subjects, so
  that files with multiple classes resolve correctly.
- Chain subjects (`getHelper()->deprecatedMethod()`) resolve through
  the full completion pipeline, not the hand-rolled
  `resolve_subject_to_class_name` helper that can't handle chains.

---

## 5. Unresolved type in PHPDoc
**Impact: Medium · Effort: Medium**

A `@return`, `@param`, `@var`, `@throws`, `@mixin`, or `@extends` tag
references a class that cannot be resolved. This is advisory (the code
may still work if the type is only used for static analysis), so it
should be **Information** severity.

| Scenario | Expected |
|---|---|
| `@return SomeAlias` where SomeAlias is not a class, type alias, or template | **Info**: Type 'SomeAlias' in @return could not be resolved |
| `@param NonExistent $x` | **Info**: Type 'NonExistent' in @param could not be resolved |
| `@throws FakeException` | **Info**: Type 'FakeException' in @throws could not be resolved |

### Implementation notes

This partially overlaps with `unknown_classes.rs` which already flags
`ClassReference` spans in docblock type positions. The remaining gap is
PHPDoc tags that reference types which are not emitted as
`ClassReference` spans by the symbol map. Audit which docblock type
positions produce `ClassReference` spans and which don't.

---

## 6. Diagnostic suppression intelligence
**Impact: Medium · Effort: Medium**

When PHPantom proxies diagnostics from external tools (PHPStan, Psalm,
PHPMD, PHP_CodeSniffer), users need a way to suppress specific warnings.
Rather than forcing them to install a separate extension or memorise each
tool's suppression syntax, PHPantom can offer **code actions to insert
the correct suppression comment** for the tool that produced the
diagnostic.

### Behaviour

- When the cursor is on a diagnostic that originated from a proxied
  tool, offer a code action: `Suppress [TOOL] [RULE] for this line` /
  `...for this function` / `...for this file`.
- Insert the correct comment syntax for the originating tool:
  - PHPStan: `// @phpstan-ignore [identifier]` (line-level), or
    `@phpstan-ignore-next-line` above the line.
  - Psalm: `/** @psalm-suppress [IssueType] */` on the line or above
    the function/class.
  - PHPCS: `// phpcs:ignore [Sniff.Name]` or `// phpcs:disable` /
    `// phpcs:enable` blocks.
  - PHPMD: `// @SuppressWarnings(PHPMD.[RuleName])` in a docblock.
- For PHPantom's own diagnostics: support `@suppress PHPxxxx`
  in docblocks (matching PHP Tools' convention) and a config flag
  `phpantom.diagnostics.enabled: bool` (default `true`).

**Prerequisites:** External tool integration is a later phase. Start
with suppression for our own diagnostics.

---

## 7. Warn when composer.json is missing or classmap is not optimized
**Impact: High · Effort: Medium**

PHPantom relies on Composer artifacts (`vendor/composer/autoload_classmap.php`,
`autoload_psr4.php`, `autoload_files.php`) for class discovery. When these
are missing or incomplete, completions silently degrade. The user should be
told what's wrong and offered help fixing it.

### Detection (during `initialized`)

| Condition | Severity | Message |
|---|---|---|
| No `composer.json` in workspace root | Warning | "No composer.json found. Class completions will be limited to open files and stubs." |
| `composer.json` exists but `vendor/` directory is missing | Warning | "No vendor directory found. Run `composer install` to enable full completions." |
| PSR-4 prefixes exist but no user classes in classmap | Info | "Composer classmap does not contain your project classes. Run `composer dump-autoload -o` for full class completions." |

For the no-composer.json case, offer to generate a minimal one via
`window/showMessageRequest`:

1. **"Generate composer.json"** -- create a `composer.json` that maps
   the entire project root as a classmap (`"autoload": {"classmap": ["./"]}`).
   Then run `composer dump-autoload -o` to build the classmap. This
   covers legacy projects and single-directory setups that don't follow
   PSR-4 conventions.
2. **"Dismiss"** -- do nothing.

The third condition needs care. The classmap is rarely empty because
vendor packages like PHPUnit use `classmap` autoloading (not PSR-4), so
there will be vendor entries even without `-o`. The real signal is:
the project's `composer.json` declares PSR-4 prefixes (e.g. `App\`,
`Tests\`), but none of the classmap FQNs start with any of those
project prefixes. This means the user's own classes were not dumped
into the classmap, which is exactly what `-o` fixes.

Detection logic:
1. Collect non-vendor PSR-4 prefixes from `psr4_mappings` (already
   tagged with `is_vendor`).
2. After loading the classmap, check whether any classmap FQN starts
   with one of those prefixes.
3. If there are project PSR-4 prefixes but zero matching classmap
   entries, the autoloader is not optimized.

### Actions (via `window/showMessageRequest`)

For the non-optimized classmap case, offer action buttons:

1. **"Run composer dump-autoload -o"** -- spawn the command in the
   workspace root, reload the classmap on success, show a progress
   notification.
2. **"Add to composer.json & run"** -- add
   `"config": {"optimize-autoloader": true}` to `composer.json` so
   future `composer install` / `composer update` always produce an
   optimized classmap, then run `composer dump-autoload`.
3. **"Dismiss"** -- do nothing.

### UX guidelines

- The no-composer.json and no-vendor warnings are safe to show via
  `window/showMessage` (informational, no action taken).
- The classmap warning should use `window/showMessageRequest` with
  action buttons so the user explicitly opts in before we touch files
  or run commands.
- Only show once per session. Do not re-trigger on every `didOpen`.
- Never modify `composer.json` or run commands without explicit user
  confirmation via an action button.
- If the spawned `composer` command fails (e.g. PHP not installed
  locally, Docker-only setup), catch the error gracefully and show
  "Composer command failed. You may need to run it manually."
- Log the detection result to the output panel regardless (already done
  for the "Loaded N classmap entries" message, just add context when
  zero user classes are found).