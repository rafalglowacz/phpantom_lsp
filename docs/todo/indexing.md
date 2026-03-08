# PHPantom — Indexing & File Discovery

This document covers how PHPantom discovers, parses, and caches class
definitions across the workspace. The goal is to remain fast and
lightweight by default while offering progressively richer modes for
users who want exhaustive workspace intelligence.

---

## Current state

PHPantom relies on Composer's generated `autoload_classmap.php` for
cross-file class resolution. This works well when the classmap is
present and up to date, but fails silently when:

- The user has not run `composer dump-autoload -o`.
- The classmap is stale (new classes added since last dump).
- The project does not use Composer at all.

Diagnostics run asynchronously but still trigger a cascade of lazy
stub and vendor file parses on first open. This is fast enough to not
block the editor, but contributes to memory growth and delays
diagnostic results.

Find References relies on `collect_php_files_gitignore` to walk the
entire workspace directory sequentially, then `update_ast` each file
one at a time. Go-to-Implementation walks Composer PSR-4 directories
via `collect_php_files`. Both process files sequentially and on large
codebases this is noticeably slow.

---

## Strategy modes

Four indexing strategies, selectable via `.phpantom.toml`:

```toml
[indexing]
# "composer" (default) - use composer classmap, self-scan on fallback
# "self"    - always self-scan, ignore composer classmap
# "full"    - background-parse all project files for rich intelligence
# "none"    - no proactive scanning
strategy = "composer"
```

### `"composer"` (default)

Use Composer's classmap when available and complete. Fall back to
self-scan when the classmap is missing or incomplete. This is the
zero-config experience.

### `"self"`

Always build the classmap ourselves. Ignores `autoload_classmap.php`
entirely. For users who refuse to run `composer dump-autoload -o` or
who are actively editing `composer.json` dependencies.

### `"full"`

Background-parse every PHP file in the project. Uses Composer data to
guide file discovery when available, falls back to scanning all PHP
files in the workspace when it is not. Populates the ast_map,
symbol_maps, and all derived indices. Enables workspace symbols, fast
find-references without on-demand scanning, and rich hover on
completion items. Memory usage grows proportionally to project size.

### `"none"`

No proactive file scanning. Still uses Composer's classmap if present,
still resolves classes on demand when the user triggers completion or
hover, still has embedded stubs. The only difference from `"composer"`
is that it never falls back to self-scan when the classmap is missing
or incomplete. This is essentially what PHPantom does today.

---

## Phase 1: Self-generated classmap

**Goal:** When the Composer classmap is missing or incomplete, build
one ourselves so the user gets cross-file resolution without manual
steps. This is the top priority for 0.5.0.

### Detection

1. Check whether `vendor/composer/autoload_classmap.php` exists.
2. If it exists, check whether it contains namespace prefixes from
   `composer.json`'s PSR-4 mappings. If the PSR-4 config lists
   namespaces that are absent from the classmap, the classmap is
   likely from a non-optimized dump and is incomplete.
3. If the classmap is missing or incomplete, fall back to self-scan.

### Self-scan implementation

Mirror what Composer does: walk directories listed in
`composer.json`'s `autoload` section (PSR-4, classmap entries) and
vendor packages, read each `.php` file, extract `namespace\ClassName`
pairs, and populate `self.classmap`.

The scanner should be a single-pass byte-level state machine (not a
full AST parse). Composer uses `php_strip_whitespace()` followed by a
regex; we can do better with a direct scan that skips
strings/comments/heredocs inline. Libretto's `FastScanner::find_classes`
is a good reference implementation: ~300 lines, handles all PHP edge
cases, uses `memchr` for SIMD-accelerated keyword detection.

The scan has two parts:

**User files** (from `composer.json`):
- `autoload.psr-4` directories (with PSR-4 compliance filtering).
- `autoload.classmap` directories.
- `autoload-dev.psr-4` and `autoload-dev.classmap` directories.

**Vendor files** (from `vendor/composer/installed.json`):
- Each installed package's autoload directories.

`composer.json` only describes the user's own code. Vendor package
locations come from `installed.json`, which Composer writes into the
vendor directory and users never touch. If `installed.json` does not
exist (the user hasn't run `composer install` yet), we can only scan
what `composer.json` describes. A file watcher will pick up vendor
files once they appear, but that is Phase 2 work. For 0.5.0 the user
restarts the LSP after installing dependencies.

PSR-0 support is deferred. It can be plugged in later with minimal
effort but covers very few modern packages.

### Non-Composer projects

When no `composer.json` exists at all, self-scan falls back to walking
all PHP files under the workspace root (excluding hidden directories).
This produces a classmap with no PSR-4 compliance filtering. It will
pick up some irrelevant files but provides basic cross-file resolution
for projects that don't use Composer.

### Output

The result is a `HashMap<String, PathBuf>` in the same format as the
existing `self.classmap`. Everything downstream (resolution,
diagnostics, go-to-definition) works unchanged.

### User feedback

When self-scan is triggered, log a message:
> PHPantom: Building class index. Run `composer dump-autoload -o` for
> faster startup.

If the user has `strategy = "self"` configured, skip the suggestion.

### Reference material

- `composer/class-map-generator` (`PhpFileParser.php`,
  `PhpFileCleaner.php`) is the source of truth for what Composer
  actually does. Our scanner must produce the same classmap for the
  same input.
- Libretto's `libretto-autoloader` crate (`fast_parser.rs`,
  `scanner.rs`) is a working Rust implementation of the same logic.
  MIT licensed, uses `mago-syntax` and `rayon`. The `FastScanner`
  (byte-level, no AST) is the right model for Phase 1. The full
  `Parser` (mago-syntax AST) is overkill for classmap generation.
- Libretto's `IncrementalCache` (`lib.rs`) uses mtime + semantic
  fingerprint tracking with `rkyv` serialization. Worth evaluating
  if/when we add disk caching, but not needed for Phase 1 where the
  in-memory scan should be fast enough.

---

## Phase 2: Staleness detection and auto-refresh

**Goal:** Keep the classmap fresh without user intervention.

### Trigger points

- On `workspace/didChangeWatchedFiles`: if `composer.json` or
  `composer.lock` changed, schedule a rescan of vendor directories
  (the user likely ran `composer install` or `composer update`).
- On `did_save` of a PHP file: if the file is in a PSR-4 directory,
  do a targeted single-file rescan (read the file, extract class
  names, update the classmap entry). This is cheap enough to do
  synchronously.

### Targeted refresh

For single-file changes, re-scan only that file and update/remove its
classmap entries. No need to rescan the entire workspace.

For dependency changes (vendor rescan), this is the expensive case but
happens rarely (a few times per day at most).

---

## Phase 2.5: Lazy autoload file indexing

**Goal:** Replace the eager full-AST parse of every Composer autoload
file at init with a lightweight byte-level scan that builds name-to-path
indices, deferring full parsing to the moment a symbol is actually used.

### Problem

During `initialized`, PHPantom calls `update_ast` on every file listed
in `vendor/composer/autoload_files.php` (and any files discovered by
following `require_once` chains). `update_ast` is a full mago AST
parse that extracts `ClassInfo`, `FunctionInfo`, `DefineInfo`,
`SymbolMap`, use maps, and namespace maps. It also populates
`ast_map`, `symbol_maps`, `use_map`, `namespace_map`, `class_index`,
`global_functions`, and `global_defines`.

A typical Laravel project has 50-100+ autoload files. Parsing all of
them eagerly at startup adds hundreds of milliseconds to init time and
fills memory with AST data for files the user may never interact with.

The justification was that functions and `define()` constants can only
be discovered through these files (classes have PSR-4 and the classmap
as alternative discovery paths). But "discovery" and "full parsing"
are separate concerns. The stubs already prove this: `stub_function_index`
maps function names to raw PHP source at build time, and
`find_or_load_function` parses the source lazily on first access,
caching the result in `global_functions` for subsequent lookups.

### Approach: lightweight scan + lazy parse

Apply the same pattern the stubs use. At init, run a fast byte-level
scan over autoload files to build three lightweight indices:

| Index | Key | Value | Purpose |
|---|---|---|---|
| `autoload_function_index` | FQN (e.g. `"Illuminate\\Support\\str"`) | file path | Lazy function resolution |
| `autoload_constant_index` | constant name (e.g. `"LARAVEL_START"`) | file path | Lazy constant resolution |
| `class_index` | FQN (e.g. `"Some\\NonPsr4\\Helper"`) | file URI | Cross-file class lookup (same as today) |

No `ClassInfo`, `FunctionInfo`, `SymbolMap`, or use maps are built at
init. The indices contain only names and paths.

### Scanner design

Extend the byte-level state machine in `classmap_scanner.rs` (or
build a sibling module) to also recognise:

- **`function` keyword** at a valid keyword boundary, followed by a
  name. Combine with the current namespace to produce the FQN.
  Skip `function` inside class bodies (track brace depth after
  `class`/`trait`/`interface`/`enum` to distinguish methods from
  standalone functions).
- **`define(` calls** where the first argument is a string literal.
  Extract the constant name from the string. These are always global
  (not affected by namespace).
- **`const` keyword** at the top level (brace depth 0 or inside a
  namespace block but outside a class body). Combine with namespace
  to produce the FQN.

The existing comment/string/heredoc skipping logic is reused
unchanged. The `memchr` quick-rejection pass can check for `function`,
`define`, and `const` in addition to the class keywords.

`require_once` following stays as-is (it already works at the text
level via `extract_require_once_paths`). The only change is that
discovered files are scanned with the lightweight scanner instead of
`update_ast`.

### Resolution changes

**Functions.** Add a new phase to `find_or_load_function` between the
existing Phase 1 (global_functions) and Phase 2 (stubs):

1. Check `global_functions` (already-parsed user code and cached results).
2. **New: check `autoload_function_index`.** If the function name maps
   to a file path, read the file, call `update_ast` (or a targeted
   function-only parse), cache the results in `global_functions`, and
   return the match. The index entry can be removed or left as-is
   (the global_functions cache takes priority on subsequent lookups).
3. Check `stub_function_index` (built-in PHP functions).

**Constants.** Same pattern for `resolve_constant_definition` and
constant completion: check `global_defines` first, then the new
`autoload_constant_index`, then `stub_constant_index`.

**Classes.** No change needed. Classes from autoload files are already
discoverable via `class_index` (populated by the lightweight scan)
and lazily parsed on demand by `find_or_load_class`.

**Completion.** Function name completion and constant name completion
currently iterate `global_functions` and `global_defines`. They need
to also iterate the keys of the new indices to show autoload-file
symbols that haven't been lazily parsed yet. For these not-yet-parsed
entries, the completion item can omit the detail/signature (just show
the name). The full detail appears once the user selects the item and
triggers resolution, or after the first use triggers a lazy parse.

### What this does NOT change

- Files the user opens (`did_open` / `did_change`) still get a full
  `update_ast`. This is about init-time processing of vendor autoload
  files only.
- The `require_once` following logic is unchanged.
- Stub resolution is unchanged.

### Effort and dependencies

**Effort:** Medium. The scanner extension is straightforward (the
hard parts of the state machine already exist). The resolution
changes are small (one new lookup phase each for functions and
constants, following the existing stub pattern). Completion changes
are minor (iterate one extra map's keys).

**Dependencies:** None. This is independent of the performance
prerequisites in Sprint 2.5 and the parallelism work in Phase 3.
It can be done at any time.

**Risk:** The byte-level scanner may miss edge cases that the full AST
parse handles (e.g. functions defined inside `if (! function_exists(...))`
guards, or `define()` calls with concatenated names). These are
acceptable misses: the symbol simply won't appear in completion until
the file is opened or lazily parsed through another path. The same
limitation applies to the classmap scanner today.

### Measurables

- **Init time:** Measure before/after on a Laravel project. Expect
  a reduction proportional to the number of autoload files (50-100
  fewer full AST parses at startup).
- **Memory at idle:** Measure RSS after init, before any files are
  opened. Expect a reduction from not holding `ClassInfo`, `SymbolMap`,
  and AST data for autoload files that are never accessed.
- **First-use latency:** The first completion or hover that triggers a
  lazy parse of an autoload file will be slightly slower than today
  (one file parse on demand). This is the same tradeoff stubs make
  and is not noticeable in practice.

---

## Phase 3: Parallel file processing

**Goal:** Speed up workspace-wide operations (find references,
go-to-implementation, self-scan, diagnostics) by processing files in
parallel with priority awareness.

**Prerequisites (from [performance.md](performance.md)):**

- **§3 `RwLock` for read-heavy maps.** Parallel `spawn_blocking` tasks
  need concurrent read access to `ast_map`, `symbol_maps`, `use_map`,
  and `namespace_map`. With `Mutex`, every reader serializes against
  every other reader, defeating the purpose of parallelism regardless
  of how clever the priority scheduling is. `RwLock` lets all readers
  proceed concurrently; only writes (which happen on `did_change` and
  on-demand file loading) take an exclusive lock.
- **§5 `Arc<String>` for file content.** Parallel tasks that need file
  content should share references rather than each cloning the full
  string out of `open_files`.
- **§6 `Arc<SymbolMap>` to avoid snapshot cloning.** Find References
  snapshots all user-file symbol maps before scanning. With owned
  `SymbolMap` values, this deep-clones hundreds of maps. With
  `Arc<SymbolMap>`, the snapshot is cheap and the lock is released
  immediately.

### Why not rayon?

`rayon` is the obvious choice for "process N files in parallel" and
Libretto uses it successfully. But it runs its own thread pool
separate from tokio's runtime. When rayon saturates all cores on a
batch scan, tokio's async tasks (completion, hover, signature help)
get starved for CPU time. There is no clean way to pause a rayon
batch when a high-priority LSP request arrives.

### Why the classmap is not a prerequisite

The classmap is a convenience for O(1) class lookup and class name
completion. But most resolution already works on demand via PSR-4
(derive path from namespace, check if file exists). Class name
completion is a minor subset of what users actually trigger. This
means classmap generation can run at normal priority without blocking
the user. They can start writing code immediately while the classmap
builds in the background.

### Architecture

A single `tokio::spawn_blocking` pool with priority-aware scheduling.
All file processing goes through one system:

- **High:** Files needed for an active LSP request (completion, hover,
  go-to-definition). The user is typing. Serve immediately.
- **Normal:** Classmap generation, find-references, go-to-
  implementation scans. Important but can wait.
- **Low:** Full background indexing (Phase 5) and diagnostics. Runs
  when nothing else needs attention.

When nothing is queued at High, Normal runs at full speed. When
nothing is at Normal, Low gets all resources. The moment a completion
request arrives, Low pauses, High is served, Low resumes. No idle
waste, no starvation.

Implementation: a priority channel or a simple check-before-spawn
pattern. Before spawning the next Low task, check whether any High or
Normal work is pending. This keeps us in one thread pool, one
scheduling model, no contention between separate runtimes.

### Quick wins before full parallelism

- Pre-filter files before reading them. For find-references looking
  for class `Foo`, use the classmap to find which files define `Foo`,
  then only scan files that contain the string "Foo". This is what
  we already do for GTI but could be applied more broadly.
- Use `memmap2` for reading files instead of `read_to_string` when
  scanning large directories. Avoids copying file contents into
  userspace when the OS page cache already has them.

---

## Phase 4: Completion item detail on demand

**Goal:** Show type signatures, docblock descriptions, and
deprecation info in completion item hover without parsing every
possible class up front.

### Current limitation

When completion shows `SomeClass::doThing()`, hovering over that item
in the completion menu shows nothing because we haven't parsed
`SomeClass`'s file yet. Parsing it on demand would be fine for one
item, but the editor may request resolve for dozens of items as the
user scrolls.

### Approach: "what's already discovered"

Use `completionItem/resolve` to populate `detail` and
`documentation` fields. If the class is already in the ast_map (parsed
during a prior resolution), return the full signature and docblock.
If not, return just the item label with no extra detail.

In `"full"` mode, everything is already parsed, so every completion
item gets rich hover for free. In `"composer"` / `"self"` mode, items
that happen to have been resolved earlier in the session get rich
detail; others don't. This is a graceful degradation that never blocks
the completion response.

### Future: speculative background parsing

When a completion list is generated, queue the unresolved classes for
background parsing at low priority. If the user lingers on the
completion menu, resolved items will progressively gain detail. This
is a nice-to-have, not a requirement.

---

## Phase 5: Full background indexing

**Goal:** Parse every PHP file in the project in the background,
enabling workspace symbols, fast find-references without on-demand
scanning, and complete completion item detail.

**Prerequisites (from [performance.md](performance.md)):**

- **§1 FQN secondary index.** The second pass calls `update_ast` on
  every file, populating `ast_map` with thousands of entries. Without
  a FQN index, every `find_class_in_ast_map` call (Phase 1 of
  `find_or_load_class`) becomes an O(thousands) linear scan. With the
  index, it is O(1).
- **§2 `Arc<ClassInfo>`.** Full indexing stores a `ClassInfo` for every
  class in the project. Without `Arc`, every resolution clones the
  entire struct out of the map. With `Arc`, retrieval is a
  reference-count increment. This is the difference between full
  indexing using ~200 MB vs. ~500 MB for a large project.
- **§3 `RwLock`.** The second pass writes to `ast_map` at Low priority
  while High-priority LSP requests read from it. `Mutex` would force
  every completion/hover request to wait for the current background
  parse to finish its map insertion. `RwLock` lets reads proceed
  concurrently with other reads; only the brief write window blocks.
- **§4 `HashSet` dedup.** Full indexing means every class resolution
  pulls from a fully populated inheritance tree. Eloquent models with
  150+ inherited methods across 8+ levels hit the O(N²) dedup path
  on every resolution. The `HashSet` fix brings this to O(N).

### Trigger

When `strategy = "full"` is set in `.phpantom.toml`.

### Design: self + second pass

Full mode is not a separate discovery system. It works exactly like
`"self"` mode (Phase 1) and then schedules a second pass:

1. **First pass (same as self):** Build the classmap via byte-level
   scanning. This completes in about a second and gives us class
   name completion and O(1) file lookup.
2. **Second pass:** Iterate every file path in the now-populated
   in-memory classmap and call `update_ast` on each one at Low
   priority. This populates ast_map, symbol_maps, class_index,
   global_functions, and global_defines.

No new file discovery logic is needed. The classmap from the first
pass already contains every relevant file path. The second pass just
enriches it.

When `composer.json` does not exist (e.g. the user opened a monorepo
root or a non-Composer project), the first pass falls back to walking
all PHP files under the workspace root, so the second pass still has
a complete file list to work from.

### Progressive enrichment

The user experiences three stages:

1. **Immediate:** LSP requests are up and running. Completion, hover,
   and go-to-definition work via on-demand resolution and stubs.
2. **Seconds:** Classmap is ready. Class name completion covers the
   full project. Cross-file resolution is O(1).
3. **Under a minute:** Full AST parse complete. Workspace symbols,
   fast find-references (no on-demand scanning), rich hover on
   completion items.

Each stage improves on the last without blocking the previous one.

### Behaviour

1. Respect the priority system from Phase 3: pause the second pass
   when higher-priority work arrives.
2. Process user code first, then vendor.
3. Report progress via `$/progress` tokens so the editor can show
   "Indexing: 1,234 / 5,678 files".

### Memory

Currently we store `ClassInfo`, `FunctionInfo`, and `SymbolMap`
structs that are not as lean as they could be. For a 21K-file
codebase, full indexing will use meaningful RAM. This is acceptable
because it's an opt-in mode, but we should profile and trim struct
sizes over time. The aim is to stay under 512 MB for a full project.

The performance prerequisites above (§2 `Arc<ClassInfo>`, §5
`Arc<String>`, §6 `Arc<SymbolMap>`) directly reduce memory usage by
sharing data across the ast_map, caches, and snapshot copies instead
of deep-cloning each. These should be measured before and after to
validate the 512 MB target.

### Workspace symbols

With the full index populated, `workspace/symbol` becomes a simple
filter over the ast_map and global_functions maps. No additional
infrastructure needed.

In other modes, workspace symbols still works but only returns results
from already-parsed files (opened files, on-demand resolutions, stubs).
When the user invokes workspace symbols outside of full mode, show a
one-time hint suggesting they enable `strategy = "full"` in
`.phpantom.toml` for complete coverage.

---

## Phase 6: Disk cache (evaluate later)

**Goal:** Persist the full index to disk so that restarts don't
require a full rescan.

### When to consider

Only if Phase 5 background indexing is slow enough on cold start that
users complain. Given that:
- Mago can lint 45K files in 2 seconds.
- A regex classmap scan over 21K files should be sub-second.
- Full AST parsing of a few thousand user files should take single
  digit seconds.

...disk caching may never justify its complexity. The primary use
case would be memory savings (load from disk on demand instead of
holding everything in RAM), not startup speed.

### Format options

- `bincode` / `postcard`: simple, small dependency footprint, tolerant
  of struct changes (deserialization fails gracefully instead of
  reading garbage memory). The right default choice.
- SQLite: robust, queryable, but heavier than needed for a flat
  key-value store.

Zero-copy formats like `rkyv` are ruled out. They map serialized bytes
directly into memory as if they were the original structs, which means
any struct layout change between versions reads corrupt data. PHPantom's
internal types change frequently and will continue to do so. A cache
format that silently produces garbage after an update is worse than no
cache at all.

### Invalidation

Store file mtime + content hash per entry. On startup, walk the
directory, compare mtimes, re-parse only changed files. This is
Libretto's `IncrementalCache` approach and it works well.

### Decision criteria

Implement disk caching only if:
1. Full-mode cold start exceeds 10 seconds on a representative large
   codebase, AND
2. The memory overhead of holding the full index exceeds the 512 MB
   target, or users on constrained systems report issues.

If neither condition is met, skip this phase entirely. Simpler is
better.