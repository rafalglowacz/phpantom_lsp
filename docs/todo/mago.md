# PHPantom — Mago Crate Migration

All migrations are complete.

| Crate              | Replaces                                               | Status  |
| ------------------ | ------------------------------------------------------ | ------- |
| `mago-docblock`    | Manual docblock parsing scattered across the codebase  | ✅ Done |
| `mago-names`       | `src/parser/use_statements.rs` + `use_map` resolution  | ✅ Done |
| `mago-type-syntax` | String-based type pipeline (~4,700 lines)              | ✅ Done |

## Crates explicitly ruled out

| Crate              | Reason                                                                   |
| ------------------ | ------------------------------------------------------------------------ |
| `mago-codex`       | Replaces `ClassInfo` model with one that cannot carry `LaravelMetadata`. |
| `mago-semantics`   | 12K false positives on Laravel; no way to inject our type context.       |
| `mago-linter`      | Same problem; `Integration::Laravel` is surface-level only.              |
| `mago-fingerprint` | Requires `mago-names` for limited value; `signature_eq` already works.   |

## Version alignment

All Mago crates should be pinned to the same release. When upgrading,
update all Mago crates in a single commit and run the test suite.

## Remaining docblock string helpers

The following functions in `type_strings.rs` are kept by design. They
operate on raw docblock text (tokenizing tag descriptions, separating
type tokens from parameter names) rather than on structured type
fields, and have no `PhpType` equivalent.

- `split_type_token` — extracts a type token from a tag description
- `clean_type` — strips `?`, leading `\`, trailing punctuation
- `split_union_depth0` — splits raw union text at depth 0
- `split_generic_args` — splits raw generic arguments at depth 0
- `PHPDOC_TYPE_KEYWORDS` — completion candidate list for PHPDoc types