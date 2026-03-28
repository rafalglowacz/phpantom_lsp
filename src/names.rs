//! Owned name resolution data extracted from `mago_names::ResolvedNames`.
//!
//! [`OwnedResolvedNames`] mirrors the `ResolvedNames` API from `mago-names`
//! but owns its data, decoupling it from the arena lifetime.  Built once
//! per file in `update_ast_inner` for files open in the editor and stored
//! in `Backend::resolved_names` for the lifetime of the file.

use std::collections::HashMap;

/// Per-file name resolution data (owned, lifetime-free).
///
/// Each entry maps a byte offset in the source file to the fully-qualified
/// name that the identifier at that offset resolves to, together with a
/// flag indicating whether the resolution came from an explicit `use`
/// statement (as opposed to namespace-relative resolution or a definition).
///
/// Built from `mago_names::ResolvedNames` at the end of a parse pass.
#[derive(Debug, Clone, Default)]
pub struct OwnedResolvedNames {
    /// Byte offset → (fully-qualified name, was-imported flag).
    names: HashMap<u32, (String, bool)>,
}

impl OwnedResolvedNames {
    /// Build an `OwnedResolvedNames` by copying every entry out of the
    /// arena-backed `ResolvedNames`.
    pub fn from_resolved(resolved: &mago_names::ResolvedNames<'_>) -> Self {
        let entries = resolved.all();
        let mut names = HashMap::with_capacity(entries.len());
        for (&offset, &(fqn, imported)) in entries {
            names.insert(offset, (fqn.to_owned(), imported));
        }
        Self { names }
    }

    /// Look up the fully-qualified name for the identifier at `offset`.
    ///
    /// Returns `None` when no resolved name exists at that position
    /// (e.g. keywords, literals, or identifiers that `mago-names` does
    /// not track).
    pub fn get(&self, offset: u32) -> Option<&str> {
        self.names.get(&offset).map(|(name, _)| name.as_str())
    }

    /// Whether the name at `offset` was introduced by an explicit `use`
    /// statement.
    ///
    /// Returns `false` when the offset is not tracked or when the name
    /// was resolved via the current namespace / is a definition.
    pub fn is_imported(&self, offset: u32) -> bool {
        self.names
            .get(&offset)
            .is_some_and(|(_, imported)| *imported)
    }

    /// Return the number of resolved names stored.
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// Whether no names are stored.
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// Iterate over all resolved names as `(offset, fqn, imported)` triples.
    pub fn iter(&self) -> impl Iterator<Item = (u32, &str, bool)> {
        self.names
            .iter()
            .map(|(&offset, (name, imported))| (offset, name.as_str(), *imported))
    }

    /// Build a reverse lookup table: short-name → FQN, mimicking the
    /// shape of the legacy `use_map` (`HashMap<String, String>`).
    ///
    /// Only entries where `imported == true` are included, matching the
    /// semantics of the old `extract_use_items` output (which only
    /// recorded explicit `use` statements, not namespace-relative
    /// resolutions).
    ///
    /// The short name is the last `\`-delimited segment of the FQN
    /// (e.g. `"Foo\Bar\Baz"` → short name `"Baz"`).
    ///
    /// This is a **transitional helper** intended to keep existing call
    /// sites working during the incremental migration away from
    /// `Backend::use_map`.  New code should use [`get`](Self::get) or
    /// [`is_imported`](Self::is_imported) directly with byte offsets.
    pub fn to_use_map(&self) -> HashMap<String, String> {
        let mut map = HashMap::new();
        for (fqn, imported) in self.names.values() {
            if !imported {
                continue;
            }
            let short = crate::util::short_name(fqn);
            // First-wins: if two offsets map to the same short name
            // (e.g. `use Foo\Bar; use Baz\Bar;`), the first inserted
            // entry wins.  This matches the old `extract_use_items`
            // behavior where `HashMap::insert` overwrites silently —
            // the exact winner is unspecified in that situation anyway.
            map.entry(short.to_owned()).or_insert_with(|| fqn.clone());
        }
        map
    }
}
