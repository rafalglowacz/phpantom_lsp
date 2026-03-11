//! Remove unused import code action.
//!
//! When the cursor overlaps with an unused `use` statement (identified by
//! matching diagnostics with `DiagnosticTag::Unnecessary`), offer:
//!
//! 1. A per-import quick-fix: `Remove unused import 'Foo\Bar'`
//! 2. A bulk action: `Remove all unused imports` (when ≥ 2 unused imports exist)
//!
//! The detection reuses the same logic as `diagnostics::unused_imports` —
//! we collect unused-import diagnostics and then generate `TextEdit`s that
//! delete the corresponding lines.

use std::collections::HashMap;

use tower_lsp::lsp_types::*;

use crate::Backend;

impl Backend {
    /// Collect "Remove unused import" code actions.
    ///
    /// For each unused-import diagnostic that overlaps with the request
    /// range, offer a quick-fix to remove it.  When there are two or more
    /// unused imports in the file, also offer a bulk "Remove all unused
    /// imports" action.
    pub(crate) fn collect_remove_unused_import_actions(
        &self,
        uri: &str,
        content: &str,
        params: &CodeActionParams,
        out: &mut Vec<CodeActionOrCommand>,
    ) {
        // ── Collect all unused-import diagnostics for this file ─────────
        let mut all_unused_diags: Vec<Diagnostic> = Vec::new();
        self.collect_unused_import_diagnostics(uri, content, &mut all_unused_diags);

        if all_unused_diags.is_empty() {
            return;
        }

        let doc_uri: Url = match uri.parse() {
            Ok(u) => u,
            Err(_) => return,
        };

        // ── Find diagnostics that overlap with the request range ────────
        let overlapping: Vec<&Diagnostic> = all_unused_diags
            .iter()
            .filter(|d| ranges_overlap(&d.range, &params.range))
            .collect();

        for diag in &overlapping {
            let removal_edit = build_line_deletion_edit(content, &diag.range);

            let title = format!(
                "Remove {}",
                diag.message
                    .strip_prefix("Unused import ")
                    .map(|rest| format!("unused import {rest}"))
                    .unwrap_or_else(|| "unused import".to_string())
            );

            let mut changes = HashMap::new();
            changes.insert(doc_uri.clone(), vec![removal_edit]);

            out.push(CodeActionOrCommand::CodeAction(CodeAction {
                title,
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![(*diag).clone()]),
                edit: Some(WorkspaceEdit {
                    changes: Some(changes),
                    document_changes: None,
                    change_annotations: None,
                }),
                command: None,
                is_preferred: Some(true),
                disabled: None,
                data: None,
            }));
        }

        // ── Bulk action: remove unused imports ──────────────────────────
        // Only offer when the cursor is on any namespace-level `use`
        // import line (used or unused), so it doesn't pop up on
        // unrelated lines elsewhere in the file.
        if !all_unused_diags.is_empty()
            && cursor_on_use_import_line(content, params.range.start.line)
        {
            let mut bulk_edits: Vec<TextEdit> = all_unused_diags
                .iter()
                .map(|d| build_line_deletion_edit(content, &d.range))
                .collect();

            // Sort edits in reverse order so that byte offsets remain
            // valid as we apply deletions from bottom to top.
            bulk_edits.sort_by(|a, b| b.range.start.cmp(&a.range.start));

            let mut changes = HashMap::new();
            changes.insert(doc_uri.clone(), bulk_edits);

            out.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: "Remove all unused imports".to_string(),
                kind: Some(CodeActionKind::new("source.organizeImports")),
                diagnostics: Some(all_unused_diags),
                edit: Some(WorkspaceEdit {
                    changes: Some(changes),
                    document_changes: None,
                    change_annotations: None,
                }),
                command: None,
                is_preferred: None,
                disabled: None,
                data: None,
            }));
        }
    }
}

/// Check whether the given 0-based line number falls on a namespace-level
/// `use` import statement (i.e. not a trait `use` inside a class body).
///
/// Uses the same brace-depth heuristic as the unused-import diagnostic
/// collector to distinguish top-level imports from trait uses.
fn cursor_on_use_import_line(content: &str, line: u32) -> bool {
    let target = line as usize;
    let mut brace_depth: usize = 0;
    let mut namespace_brace_depth: Option<usize> = None;

    for (line_idx, raw_line) in content.split('\n').enumerate() {
        let code = raw_line.split("//").next().unwrap_or(raw_line);
        let code = code.split('#').next().unwrap_or(code);
        let trimmed = raw_line.trim_start();

        if trimmed.starts_with("namespace ") && code.contains('{') {
            namespace_brace_depth = Some(brace_depth);
        }

        for ch in code.chars() {
            match ch {
                '{' => brace_depth += 1,
                '}' => {
                    brace_depth = brace_depth.saturating_sub(1);
                    if namespace_brace_depth == Some(brace_depth) {
                        namespace_brace_depth = None;
                    }
                }
                _ => {}
            }
        }

        if line_idx == target {
            let top_level_depth = namespace_brace_depth.map_or(0, |d| d + 1);
            return trimmed.starts_with("use ")
                && trimmed.contains(';')
                && brace_depth == top_level_depth;
        }
    }

    false
}

/// Check whether two LSP ranges overlap (share at least one position).
fn ranges_overlap(a: &Range, b: &Range) -> bool {
    a.start <= b.end && b.start <= a.end
}

/// Build a `TextEdit` that deletes the line(s) covered by `range`,
/// including the trailing newline so no blank lines accumulate.
///
/// For group import members (where the diagnostic range covers just the
/// member name within `{...}`), this deletes only the member text plus
/// its trailing comma/space.
fn build_line_deletion_edit(content: &str, range: &Range) -> TextEdit {
    let lines: Vec<&str> = content.split('\n').collect();

    let start_line = range.start.line as usize;
    let end_line = range.end.line as usize;

    // Check if this diagnostic covers a full `use` statement line.
    // If the range spans from the `use` keyword to the semicolon (or end
    // of line), we delete the entire line including its newline.
    let is_full_line = if start_line == end_line && start_line < lines.len() {
        let line = lines[start_line];
        let trimmed = line.trim();
        let leading_ws = line.len() - trimmed.len();
        // Check if the diagnostic range covers the whole trimmed content
        // of a `use` statement line (not just a member inside a group).
        let range_covers_full_line = range.start.character as usize <= leading_ws
            && range.end.character as usize >= leading_ws + trimmed.len();
        range_covers_full_line && trimmed.starts_with("use ") && trimmed.ends_with(';')
    } else {
        false
    };

    if is_full_line {
        // Delete the entire line including the trailing newline.
        let delete_end_line = end_line + 1;
        TextEdit {
            range: Range {
                start: Position::new(start_line as u32, 0),
                end: Position::new(delete_end_line as u32, 0),
            },
            new_text: String::new(),
        }
    } else {
        // Partial deletion (e.g. a member inside a group import).
        // Delete the exact range the diagnostic covers.
        //
        // For group members we also try to clean up a trailing comma
        // and whitespace to keep the group tidy.
        let extended_range = extend_range_for_group_member(content, range);
        TextEdit {
            range: extended_range,
            new_text: String::new(),
        }
    }
}

/// When removing a member from a group import (`use Foo\{Bar, Baz};`),
/// extend the deletion range to include the trailing comma and
/// whitespace (or leading comma and whitespace if it's the last member).
fn extend_range_for_group_member(content: &str, range: &Range) -> Range {
    let lines: Vec<&str> = content.split('\n').collect();
    let line_idx = range.end.line as usize;
    if line_idx >= lines.len() {
        return *range;
    }
    let line = lines[line_idx];
    let end_char = range.end.character as usize;

    // Check for trailing comma + optional whitespace after the member name.
    let after = &line[end_char..];
    if let Some(rest) = after.strip_prefix(',') {
        // Consume optional whitespace after the comma.
        let extra_ws = rest.len() - rest.trim_start().len();
        let new_end_char = end_char + 1 + extra_ws; // 1 for comma + whitespace
        return Range {
            start: range.start,
            end: Position::new(range.end.line, new_end_char as u32),
        };
    }

    // If there's no trailing comma, this might be the last member.
    // Check for a leading comma + whitespace before the member name.
    let start_char = range.start.character as usize;
    let line_for_start = lines[range.start.line as usize];
    let before = &line_for_start[..start_char];
    if before.ends_with(", ") {
        return Range {
            start: Position::new(range.start.line, (start_char - 2) as u32),
            end: range.end,
        };
    }
    if before.ends_with(',') {
        return Range {
            start: Position::new(range.start.line, (start_char - 1) as u32),
            end: range.end,
        };
    }

    *range
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ranges_overlap tests ────────────────────────────────────────────

    #[test]
    fn overlapping_ranges() {
        let a = Range::new(Position::new(1, 0), Position::new(1, 10));
        let b = Range::new(Position::new(1, 5), Position::new(1, 15));
        assert!(ranges_overlap(&a, &b));
    }

    #[test]
    fn non_overlapping_ranges() {
        let a = Range::new(Position::new(1, 0), Position::new(1, 5));
        let b = Range::new(Position::new(2, 0), Position::new(2, 5));
        assert!(!ranges_overlap(&a, &b));
    }

    #[test]
    fn touching_ranges_overlap() {
        let a = Range::new(Position::new(1, 0), Position::new(1, 5));
        let b = Range::new(Position::new(1, 5), Position::new(1, 10));
        assert!(ranges_overlap(&a, &b));
    }

    #[test]
    fn cursor_inside_range() {
        // Cursor at a single point inside the range.
        let a = Range::new(Position::new(3, 0), Position::new(3, 20));
        let b = Range::new(Position::new(3, 5), Position::new(3, 5));
        assert!(ranges_overlap(&a, &b));
    }

    // ── build_line_deletion_edit tests ───────────────────────────────────

    #[test]
    fn deletes_full_use_line() {
        let content = "<?php\nuse Foo\\Bar;\nuse Baz\\Qux;\n";
        let range = Range::new(Position::new(1, 0), Position::new(1, 12));
        let edit = build_line_deletion_edit(content, &range);
        assert_eq!(edit.new_text, "");
        assert_eq!(edit.range.start, Position::new(1, 0));
        assert_eq!(edit.range.end, Position::new(2, 0));
    }

    #[test]
    fn deletes_partial_group_member_trailing_comma() {
        // `use Foo\{Bar, Baz};` — removing "Bar" which has a trailing ", "
        let content = "<?php\nuse Foo\\{Bar, Baz};\n";
        // Range covering just "Bar" inside the braces.
        let range = Range::new(Position::new(1, 9), Position::new(1, 12));
        let edit = build_line_deletion_edit(content, &range);
        // Should extend to include the trailing ", "
        assert_eq!(edit.range.start, Position::new(1, 9));
        assert_eq!(edit.range.end, Position::new(1, 14)); // "Bar, " = 5 chars from 9
    }

    // ── Integration test: remove unused import action ───────────────────

    #[test]
    fn remove_action_offered_for_unused_import() {
        let backend = crate::Backend::new_test();
        let uri = "file:///test.php";
        let content = "<?php\nnamespace App;\n\nuse Foo\\Bar;\n\nclass Baz {}\n";

        backend.update_ast(uri, content);

        let params = CodeActionParams {
            text_document: TextDocumentIdentifier {
                uri: uri.parse().unwrap(),
            },
            range: Range {
                start: Position::new(3, 0),
                end: Position::new(3, 0),
            },
            context: CodeActionContext {
                diagnostics: vec![],
                only: None,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let actions = backend.handle_code_action(uri, content, &params);
        let remove_actions: Vec<_> = actions
            .iter()
            .filter(|a| match a {
                CodeActionOrCommand::CodeAction(ca) => ca.title.starts_with("Remove"),
                _ => false,
            })
            .collect();
        assert!(
            !remove_actions.is_empty(),
            "expected at least one remove action for unused import"
        );
    }

    #[test]
    fn no_remove_action_for_used_import() {
        let backend = crate::Backend::new_test();
        let uri = "file:///test.php";
        let content = "<?php\nnamespace App;\n\nuse Foo\\Bar;\n\nclass Baz extends Bar {}\n";

        backend.update_ast(uri, content);

        let params = CodeActionParams {
            text_document: TextDocumentIdentifier {
                uri: uri.parse().unwrap(),
            },
            range: Range {
                start: Position::new(3, 0),
                end: Position::new(3, 0),
            },
            context: CodeActionContext {
                diagnostics: vec![],
                only: None,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let actions = backend.handle_code_action(uri, content, &params);
        let remove_actions: Vec<_> = actions
            .iter()
            .filter(|a| match a {
                CodeActionOrCommand::CodeAction(ca) => ca.title.starts_with("Remove"),
                _ => false,
            })
            .collect();
        assert!(
            remove_actions.is_empty(),
            "should not offer remove for used import, got: {:?}",
            remove_actions
                .iter()
                .map(|a| match a {
                    CodeActionOrCommand::CodeAction(ca) => ca.title.clone(),
                    CodeActionOrCommand::Command(c) => c.title.clone(),
                })
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn bulk_remove_offered_when_multiple_unused() {
        let backend = crate::Backend::new_test();
        let uri = "file:///test.php";
        let content = "<?php\nnamespace App;\n\nuse Foo\\Bar;\nuse Baz\\Qux;\n\nclass X {}\n";

        backend.update_ast(uri, content);

        let params = CodeActionParams {
            text_document: TextDocumentIdentifier {
                uri: uri.parse().unwrap(),
            },
            range: Range {
                start: Position::new(3, 0),
                end: Position::new(3, 0),
            },
            context: CodeActionContext {
                diagnostics: vec![],
                only: None,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let actions = backend.handle_code_action(uri, content, &params);
        let bulk_action = actions.iter().find(|a| match a {
            CodeActionOrCommand::CodeAction(ca) => ca.title == "Remove all unused imports",
            _ => false,
        });
        assert!(
            bulk_action.is_some(),
            "expected a bulk 'Remove all unused imports' action"
        );
    }

    #[test]
    fn bulk_remove_offered_for_single_unused_import() {
        let backend = crate::Backend::new_test();
        let uri = "file:///test.php";
        let content = "<?php\nnamespace App;\n\nuse Foo\\Bar;\n\nclass Baz {}\n";

        backend.update_ast(uri, content);

        let params = CodeActionParams {
            text_document: TextDocumentIdentifier {
                uri: uri.parse().unwrap(),
            },
            range: Range {
                start: Position::new(3, 0),
                end: Position::new(3, 0),
            },
            context: CodeActionContext {
                diagnostics: vec![],
                only: None,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let actions = backend.handle_code_action(uri, content, &params);
        let bulk_action = actions.iter().find(|a| match a {
            CodeActionOrCommand::CodeAction(ca) => ca.title == "Remove all unused imports",
            _ => false,
        });
        assert!(
            bulk_action.is_some(),
            "expected 'Remove all unused imports' action for a single unused import"
        );
    }

    #[test]
    fn bulk_remove_not_offered_when_cursor_outside_import_block() {
        let backend = crate::Backend::new_test();
        let uri = "file:///test.php";
        let content = "<?php\nnamespace App;\n\nuse Foo\\Bar;\nuse Baz\\Qux;\n\nclass X {}\n";

        backend.update_ast(uri, content);

        // Cursor on `class X {}` (line 6), well outside the import block (lines 3–4).
        let params = CodeActionParams {
            text_document: TextDocumentIdentifier {
                uri: uri.parse().unwrap(),
            },
            range: Range {
                start: Position::new(6, 0),
                end: Position::new(6, 0),
            },
            context: CodeActionContext {
                diagnostics: vec![],
                only: None,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let actions = backend.handle_code_action(uri, content, &params);
        let bulk_action = actions.iter().find(|a| match a {
            CodeActionOrCommand::CodeAction(ca) => ca.title == "Remove all unused imports",
            _ => false,
        });
        assert!(
            bulk_action.is_none(),
            "bulk 'Remove all unused imports' should NOT be offered when cursor is outside the import block"
        );

        // Individual remove actions should also not appear (cursor doesn't overlap any diagnostic).
        let remove_actions: Vec<_> = actions
            .iter()
            .filter(|a| match a {
                CodeActionOrCommand::CodeAction(ca) => ca.title.starts_with("Remove"),
                _ => false,
            })
            .collect();
        assert!(
            remove_actions.is_empty(),
            "no remove actions should be offered outside the import block, got: {:?}",
            remove_actions
                .iter()
                .map(|a| match a {
                    CodeActionOrCommand::CodeAction(ca) => ca.title.clone(),
                    CodeActionOrCommand::Command(c) => c.title.clone(),
                })
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn bulk_remove_offered_when_cursor_on_used_import() {
        let backend = crate::Backend::new_test();
        let uri = "file:///test.php";
        // Foo\Bar is used (in extends), Baz\Qux and Baz\Quux are unused.
        let content = "<?php\nnamespace App;\n\nuse Foo\\Bar;\nuse Baz\\Qux;\nuse Baz\\Quux;\n\nclass X extends Bar {}\n";

        backend.update_ast(uri, content);

        // Cursor on `use Foo\Bar;` (line 3) which is a *used* import.
        let params = CodeActionParams {
            text_document: TextDocumentIdentifier {
                uri: uri.parse().unwrap(),
            },
            range: Range {
                start: Position::new(3, 0),
                end: Position::new(3, 0),
            },
            context: CodeActionContext {
                diagnostics: vec![],
                only: None,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let actions = backend.handle_code_action(uri, content, &params);
        let bulk_action = actions.iter().find(|a| match a {
            CodeActionOrCommand::CodeAction(ca) => ca.title == "Remove all unused imports",
            _ => false,
        });
        assert!(
            bulk_action.is_some(),
            "bulk 'Remove all unused imports' should be offered when cursor is on any use import line"
        );
    }

    // ── cursor_on_use_import_line unit tests ────────────────────────────

    #[test]
    fn cursor_on_use_line_returns_true() {
        let content = "<?php\nnamespace App;\n\nuse Foo\\Bar;\nuse Baz\\Qux;\n\nclass X {}\n";
        assert!(cursor_on_use_import_line(content, 3));
        assert!(cursor_on_use_import_line(content, 4));
    }

    #[test]
    fn cursor_on_non_use_line_returns_false() {
        let content = "<?php\nnamespace App;\n\nuse Foo\\Bar;\nuse Baz\\Qux;\n\nclass X {}\n";
        assert!(!cursor_on_use_import_line(content, 0)); // <?php
        assert!(!cursor_on_use_import_line(content, 1)); // namespace
        assert!(!cursor_on_use_import_line(content, 2)); // blank
        assert!(!cursor_on_use_import_line(content, 5)); // blank
        assert!(!cursor_on_use_import_line(content, 6)); // class X
    }

    #[test]
    fn cursor_on_trait_use_returns_false() {
        let content = "<?php\nuse Foo\\Bar;\n\nclass X {\n    use SomeTrait;\n}\n";
        assert!(cursor_on_use_import_line(content, 1)); // namespace import
        assert!(!cursor_on_use_import_line(content, 4)); // trait use inside class
    }

    #[test]
    fn cursor_on_use_in_braced_namespace_returns_true() {
        let content = "<?php\nnamespace App {\n    use Foo\\Bar;\n    class X {}\n}\n";
        assert!(cursor_on_use_import_line(content, 2));
    }

    /// Reproduces the "Remove all 2 unused imports" bug where two unused
    /// imports are separated by many lines (e.g. lines 20 and 2075 in
    /// example.php).  The bulk action title says "2" but may only
    /// generate a valid edit for one of them.
    #[test]
    fn bulk_remove_deletes_both_widely_separated_unused_imports() {
        let backend = crate::Backend::new_test();
        let uri = "file:///test.php";
        // Simulate example.php layout: namespace block with an unused
        // import near the top, many lines of code, then another unused
        // import near the bottom.
        let mut lines = vec![
            "<?php",
            "namespace Demo {",
            "",
            "use Stringable;", // line 3 — unused
            "use Exception;",  // line 4 — used
            "",
        ];
        // Pad with ~50 lines of class body so the two use statements
        // are far apart.
        lines.extend(std::iter::repeat_n("// filler", 50));
        lines.push("class Foo extends Exception {}"); // uses Exception
        lines.push("");
        lines.push("use ReflectionClass;"); // unused, far from the first
        lines.push("");
        lines.push("class Bar {}");
        lines.push("} // end namespace");

        let content = lines.join("\n");
        backend.update_ast(uri, &content);

        // Put cursor on the first use line (Stringable, line 3).
        let params = CodeActionParams {
            text_document: TextDocumentIdentifier {
                uri: uri.parse().unwrap(),
            },
            range: Range {
                start: Position::new(3, 0),
                end: Position::new(3, 0),
            },
            context: CodeActionContext {
                diagnostics: vec![],
                only: None,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let actions = backend.handle_code_action(uri, &content, &params);

        // Find the bulk action.
        let bulk = actions.iter().find_map(|a| match a {
            CodeActionOrCommand::CodeAction(ca) if ca.title == "Remove all unused imports" => {
                Some(ca)
            }
            _ => None,
        });
        let bulk = bulk.expect("expected a bulk 'Remove all unused imports' action");

        // Extract the edits and apply them (bottom-to-top) to verify
        // both unused lines are removed.
        let edits = bulk
            .edit
            .as_ref()
            .unwrap()
            .changes
            .as_ref()
            .unwrap()
            .values()
            .next()
            .unwrap();

        // There should be exactly 2 text edits.
        assert_eq!(
            edits.len(),
            2,
            "expected 2 text edits in the bulk action, got {}",
            edits.len()
        );

        // Apply edits (already sorted bottom-to-top by the code action).
        let mut result = content.clone();
        for edit in edits {
            let start = lsp_position_to_byte_offset(&result, &edit.range.start);
            let end = lsp_position_to_byte_offset(&result, &edit.range.end);
            result = format!("{}{}{}", &result[..start], &edit.new_text, &result[end..]);
        }

        // After removal, neither unused import line should remain.
        assert!(
            !result.contains("use Stringable;"),
            "Stringable import should have been removed:\n{}",
            result
        );
        assert!(
            !result.contains("use ReflectionClass;"),
            "ReflectionClass import should have been removed:\n{}",
            result
        );
        // The used import should still be there.
        assert!(
            result.contains("use Exception;"),
            "Exception import should still be present:\n{}",
            result
        );
    }

    /// Faithful reproduction of example.php's braced namespace layout.
    /// The file uses `namespace Demo { ... }` with class bodies between
    /// the two unused import groups, so the brace-depth tracker must
    /// handle many open/close braces correctly.
    #[test]
    fn bulk_remove_in_braced_namespace_with_class_bodies_between() {
        let backend = crate::Backend::new_test();
        let uri = "file:///test.php";
        let content = [
            "<?php",
            "namespace Demo {",
            "",
            "use Stringable;", // line 3 — unused
            "use Exception;",  // line 4 — used
            "",
            "class Alpha extends Exception {",   // line 6
            "    public function foo(): void {", // line 7
            "        if (true) {",               // line 8
            "            $x = 1;",               // line 9
            "        }",                         // line 10
            "    }",                             // line 11
            "}",                                 // line 12
            "",
            "class Beta {",                       // line 14
            "    public function bar(): void {}", // line 15
            "}",                                  // line 16
            "",
            "use ReflectionClass;", // line 18 — unused
            "",
            "class Gamma {}",          // line 20
            "} // end namespace Demo", // line 21
        ]
        .join("\n");

        backend.update_ast(uri, &content);

        // Cursor on `use Stringable;` (line 3).
        let params = CodeActionParams {
            text_document: TextDocumentIdentifier {
                uri: uri.parse().unwrap(),
            },
            range: Range {
                start: Position::new(3, 0),
                end: Position::new(3, 0),
            },
            context: CodeActionContext {
                diagnostics: vec![],
                only: None,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let actions = backend.handle_code_action(uri, &content, &params);

        // Find the bulk action.
        let bulk = actions.iter().find_map(|a| match a {
            CodeActionOrCommand::CodeAction(ca) if ca.title == "Remove all unused imports" => {
                Some(ca)
            }
            _ => None,
        });
        let bulk = bulk.expect("expected a bulk 'Remove all unused imports' action");

        let edits = bulk
            .edit
            .as_ref()
            .unwrap()
            .changes
            .as_ref()
            .unwrap()
            .values()
            .next()
            .unwrap();

        assert_eq!(edits.len(), 2, "expected 2 edits, got {}", edits.len());

        // Apply and verify.
        let mut result = content.clone();
        for edit in edits {
            let start = lsp_position_to_byte_offset(&result, &edit.range.start);
            let end = lsp_position_to_byte_offset(&result, &edit.range.end);
            result = format!("{}{}{}", &result[..start], &edit.new_text, &result[end..]);
        }

        assert!(
            !result.contains("use Stringable;"),
            "Stringable should be removed:\n{}",
            result
        );
        assert!(
            !result.contains("use ReflectionClass;"),
            "ReflectionClass should be removed:\n{}",
            result
        );
        assert!(
            result.contains("use Exception;"),
            "Exception should remain:\n{}",
            result
        );
    }

    /// Convert an LSP Position to a byte offset in content.
    fn lsp_position_to_byte_offset(content: &str, pos: &Position) -> usize {
        let mut offset = 0;
        for (i, line) in content.split('\n').enumerate() {
            if i == pos.line as usize {
                return offset + pos.character as usize;
            }
            offset += line.len() + 1; // +1 for '\n'
        }
        offset
    }
}
