/// `use` statement and namespace extraction.
///
/// This module handles parsing PHP `use` statements and namespace
/// declarations from the AST, building a mapping of short (imported)
/// names to their fully-qualified equivalents.
use std::collections::HashMap;

use mago_syntax::ast::*;

use crate::Backend;
use crate::util::short_name;

impl Backend {
    /// Walk statements and extract `use` statement mappings.
    pub(crate) fn extract_use_statements_from_statements<'a>(
        statements: impl Iterator<Item = &'a Statement<'a>>,
        use_map: &mut HashMap<String, String>,
    ) {
        for statement in statements {
            match statement {
                Statement::Use(use_stmt) => {
                    Self::extract_use_items(&use_stmt.items, use_map);
                }
                Statement::Namespace(namespace) => {
                    // Recurse into namespace bodies to find use statements
                    Self::extract_use_statements_from_statements(
                        namespace.statements().iter(),
                        use_map,
                    );
                }
                _ => {}
            }
        }
    }

    /// Extract individual use items from a `UseItems` node.
    pub(crate) fn extract_use_items(items: &UseItems, use_map: &mut HashMap<String, String>) {
        match items {
            UseItems::Sequence(seq) => {
                // `use Foo\Bar;` or `use Foo\Bar, Baz\Qux;`
                for item in seq.items.iter() {
                    Self::register_use_item(item, None, use_map);
                }
            }
            UseItems::TypedSequence(seq) => {
                // `use function Foo\bar;` or `use const Foo\BAR;`
                // We only care about class imports, skip function/const
                if seq.r#type.is_function() || seq.r#type.is_const() {
                    return;
                }
                for item in seq.items.iter() {
                    Self::register_use_item(item, None, use_map);
                }
            }
            UseItems::TypedList(list) => {
                // `use function Foo\{bar, baz};` — skip function/const
                if list.r#type.is_function() || list.r#type.is_const() {
                    return;
                }
                let prefix = list.namespace.value();
                for item in list.items.iter() {
                    Self::register_use_item(item, Some(prefix), use_map);
                }
            }
            UseItems::MixedList(list) => {
                // `use Foo\{Bar, function baz, const QUX};`
                let prefix = list.namespace.value();
                for maybe_typed in list.items.iter() {
                    // Skip function/const imports
                    if let Some(ref t) = maybe_typed.r#type
                        && (t.is_function() || t.is_const())
                    {
                        continue;
                    }
                    Self::register_use_item(&maybe_typed.item, Some(prefix), use_map);
                }
            }
        }
    }

    /// Register a single `UseItem` into the use_map.
    ///
    /// If `group_prefix` is `Some`, the item name is relative to that prefix
    /// (e.g. for `use Foo\{Bar}`, prefix is `"Foo"` and item name is `"Bar"`,
    /// giving FQN `"Foo\Bar"`).
    fn register_use_item(
        item: &UseItem,
        group_prefix: Option<&str>,
        use_map: &mut HashMap<String, String>,
    ) {
        let item_name = item.name.value();

        // Build the fully-qualified name
        let fqn = if let Some(prefix) = group_prefix {
            format!("{}\\{}", prefix, item_name)
        } else {
            item_name.to_string()
        };

        // The short (imported) name is either the alias or the last segment
        let alias_name = if let Some(ref alias) = item.alias {
            alias.identifier.value.to_string()
        } else {
            // Last segment of the FQN
            short_name(&fqn).to_string()
        };

        use_map.insert(alias_name, fqn);
    }

    /// Walk statements and extract the first namespace declaration found.
    pub(crate) fn extract_namespace_from_statements<'a>(
        statements: impl Iterator<Item = &'a Statement<'a>>,
    ) -> Option<String> {
        for statement in statements {
            if let Statement::Namespace(namespace) = statement {
                // The namespace name is an `Option<Identifier>`.
                // Both implicit (`namespace Foo;`) and brace-delimited
                // (`namespace Foo { ... }`) forms may have a name.
                if let Some(ident) = &namespace.name {
                    let name = ident.value();
                    if !name.is_empty() {
                        return Some(name.to_string());
                    }
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Grouped `use` statement: `use Foo\{Bar, Baz};`
    ///
    /// This is the syntax reported in issue #42 — verify that both the
    /// legacy `extract_use_items` path and the new `mago-names` resolver
    /// produce correct mappings.
    #[test]
    fn grouped_use_populates_use_map_and_resolved_names() {
        let backend = Backend::new_test();
        let uri = "file:///test.php";
        let content = r#"<?php
namespace Controllers\Registration;

use Models\Common\{Disciplines, TeamMembers, TournamentLeagueRosters, TournamentsLeagues};

class RegistrationController {
    public function foo(Disciplines $d): TeamMembers {
    }
}
"#;
        backend.update_ast(uri, content);

        // ── Legacy use_map ──────────────────────────────────────────
        let use_map = backend.use_map.read();
        let file_map = use_map
            .get(uri)
            .expect("use_map should have an entry for the file");

        assert_eq!(
            file_map.get("Disciplines"),
            Some(&"Models\\Common\\Disciplines".to_string()),
            "Disciplines should be in the use_map"
        );
        assert_eq!(
            file_map.get("TeamMembers"),
            Some(&"Models\\Common\\TeamMembers".to_string()),
            "TeamMembers should be in the use_map"
        );
        assert_eq!(
            file_map.get("TournamentLeagueRosters"),
            Some(&"Models\\Common\\TournamentLeagueRosters".to_string()),
            "TournamentLeagueRosters should be in the use_map"
        );
        assert_eq!(
            file_map.get("TournamentsLeagues"),
            Some(&"Models\\Common\\TournamentsLeagues".to_string()),
            "TournamentsLeagues should be in the use_map"
        );
        drop(use_map);

        // ── mago-names resolved_names ───────────────────────────────
        let resolved = backend.resolved_names.read();
        let rn = resolved
            .get(uri)
            .expect("resolved_names should have an entry for the file");

        // The `Disciplines` type hint in `foo(Disciplines $d)` should
        // resolve to its FQN via the grouped import.
        let hint_offset = content
            .find("Disciplines $d")
            .expect("should find Disciplines type hint") as u32;
        assert_eq!(
            rn.get(hint_offset),
            Some("Models\\Common\\Disciplines"),
            "mago-names should resolve Disciplines type hint to FQN"
        );

        // The `TeamMembers` return type should also resolve.
        let ret_offset = content
            .find("): TeamMembers")
            .map(|p| p + "): ".len())
            .expect("should find TeamMembers return type") as u32;
        assert_eq!(
            rn.get(ret_offset),
            Some("Models\\Common\\TeamMembers"),
            "mago-names should resolve TeamMembers return type to FQN"
        );
    }

    /// Aliased grouped `use`: `use Foo\{Bar as B, Baz};`
    #[test]
    fn grouped_use_with_alias() {
        let backend = Backend::new_test();
        let uri = "file:///test.php";
        let content = "<?php\nuse Models\\Common\\{Disciplines as Disc, TeamMembers};\n\nclass X extends Disc {}\n";

        backend.update_ast(uri, content);

        let use_map = backend.use_map.read();
        let file_map = use_map.get(uri).expect("use_map entry");

        assert_eq!(
            file_map.get("Disc"),
            Some(&"Models\\Common\\Disciplines".to_string()),
            "aliased short name should map to the full FQN"
        );
        assert_eq!(
            file_map.get("TeamMembers"),
            Some(&"Models\\Common\\TeamMembers".to_string()),
        );
        // The original name should NOT appear — only the alias.
        assert!(
            !file_map.contains_key("Disciplines"),
            "original name should not be in the use_map when aliased"
        );
    }
}
