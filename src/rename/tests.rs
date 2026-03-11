#![cfg(test)]

use crate::Backend;
use tower_lsp::LanguageServer;
use tower_lsp::lsp_types::*;

/// Helper: open a file in the backend.
async fn open_file(backend: &Backend, uri: &Url, text: &str) {
    let open_params = DidOpenTextDocumentParams {
        text_document: TextDocumentItem {
            uri: uri.clone(),
            language_id: "php".to_string(),
            version: 1,
            text: text.to_string(),
        },
    };
    backend.did_open(open_params).await;
}

/// Helper: send a prepare-rename request and return the response.
async fn prepare_rename(
    backend: &Backend,
    uri: &Url,
    line: u32,
    character: u32,
) -> Option<PrepareRenameResponse> {
    let params = TextDocumentPositionParams {
        text_document: TextDocumentIdentifier { uri: uri.clone() },
        position: Position { line, character },
    };

    backend.prepare_rename(params).await.unwrap()
}

/// Helper: send a rename request and return the workspace edit.
async fn rename(
    backend: &Backend,
    uri: &Url,
    line: u32,
    character: u32,
    new_name: &str,
) -> Option<WorkspaceEdit> {
    let params = RenameParams {
        text_document_position: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
            position: Position { line, character },
        },
        new_name: new_name.to_string(),
        work_done_progress_params: WorkDoneProgressParams::default(),
    };

    backend.rename(params).await.unwrap()
}

/// Collect all text edits for a given URI from a WorkspaceEdit.
fn edits_for_uri(edit: &WorkspaceEdit, uri: &Url) -> Vec<TextEdit> {
    edit.changes
        .as_ref()
        .and_then(|changes| changes.get(uri))
        .cloned()
        .unwrap_or_default()
}

/// Apply a set of text edits to source text and return the result.
/// Edits must not overlap; they are applied from last to first.
fn apply_edits(source: &str, edits: &[TextEdit]) -> String {
    let mut sorted: Vec<_> = edits.to_vec();
    // Sort by start position descending so we can apply from the end.
    sorted.sort_by(|a, b| {
        b.range
            .start
            .line
            .cmp(&a.range.start.line)
            .then(b.range.start.character.cmp(&a.range.start.character))
    });

    let lines: Vec<&str> = source.lines().collect();
    let mut result = source.to_string();

    for edit in &sorted {
        let start_offset = line_col_to_offset(&lines, edit.range.start);
        let end_offset = line_col_to_offset(&lines, edit.range.end);
        result.replace_range(start_offset..end_offset, &edit.new_text);
    }

    result
}

fn line_col_to_offset(lines: &[&str], pos: Position) -> usize {
    let mut offset = 0;
    for (i, line) in lines.iter().enumerate() {
        if i == pos.line as usize {
            return offset + pos.character as usize;
        }
        offset += line.len() + 1; // +1 for newline
    }
    offset
}

// ─── Variable Rename ────────────────────────────────────────────────────────

#[tokio::test]
async fn rename_variable_in_function() {
    let backend = Backend::new_test();
    let uri = Url::parse("file:///test.php").unwrap();
    let text = concat!(
        "<?php\n",
        "function demo(): void {\n",
        "    $user = new User();\n",
        "    $user->name = 'Alice';\n",
        "    echo $user->name;\n",
        "}\n",
    );

    open_file(&backend, &uri, text).await;

    // Rename $user on line 2 (the assignment)
    let edit = rename(&backend, &uri, 2, 5, "$person").await;
    assert!(
        edit.is_some(),
        "Expected a workspace edit for variable rename"
    );

    let edit = edit.unwrap();
    let file_edits = edits_for_uri(&edit, &uri);
    assert!(
        file_edits.len() >= 3,
        "Expected at least 3 edits for $user (decl + 2 usages), got {}",
        file_edits.len()
    );

    // All edits should use the new name with `$`.
    for te in &file_edits {
        assert_eq!(te.new_text, "$person");
    }
}

#[tokio::test]
async fn rename_variable_without_dollar_prefix() {
    let backend = Backend::new_test();
    let uri = Url::parse("file:///test.php").unwrap();
    let text = concat!(
        "<?php\n",
        "function demo(): void {\n",
        "    $x = 1;\n",
        "    echo $x;\n",
        "}\n",
    );

    open_file(&backend, &uri, text).await;

    // User provides new name without `$` — the handler should add it.
    let edit = rename(&backend, &uri, 2, 5, "y").await;
    assert!(edit.is_some());

    let file_edits = edits_for_uri(&edit.unwrap(), &uri);
    for te in &file_edits {
        assert_eq!(te.new_text, "$y");
    }
}

#[tokio::test]
async fn prepare_rename_variable() {
    let backend = Backend::new_test();
    let uri = Url::parse("file:///test.php").unwrap();
    let text = concat!(
        "<?php\n",
        "function demo(): void {\n",
        "    $count = 0;\n",
        "    $count++;\n",
        "}\n",
    );

    open_file(&backend, &uri, text).await;

    let response = prepare_rename(&backend, &uri, 2, 6).await;
    assert!(
        response.is_some(),
        "Expected prepare rename to succeed for $count"
    );

    if let Some(PrepareRenameResponse::RangeWithPlaceholder { placeholder, .. }) = response {
        assert_eq!(placeholder, "$count");
    } else {
        panic!("Expected RangeWithPlaceholder response");
    }
}

// ─── Non-Renameable Symbols ─────────────────────────────────────────────────

#[tokio::test]
async fn prepare_rename_rejects_this() {
    let backend = Backend::new_test();
    let uri = Url::parse("file:///test.php").unwrap();
    let text = concat!(
        "<?php\n",
        "class Foo {\n",
        "    public function bar(): void {\n",
        "        $this->baz();\n",
        "    }\n",
        "    public function baz(): void {}\n",
        "}\n",
    );

    open_file(&backend, &uri, text).await;

    // `$this` should not be renameable.
    let response = prepare_rename(&backend, &uri, 3, 9).await;
    assert!(response.is_none(), "$this should not be renameable");
}

#[tokio::test]
async fn prepare_rename_rejects_self() {
    let backend = Backend::new_test();
    let uri = Url::parse("file:///test.php").unwrap();
    let text = concat!(
        "<?php\n",
        "class Foo {\n",
        "    public static function create(): self {\n",
        "        return new self();\n",
        "    }\n",
        "}\n",
    );

    open_file(&backend, &uri, text).await;

    // `self` keyword on line 3 should not be renameable.
    let response = prepare_rename(&backend, &uri, 3, 20).await;
    assert!(response.is_none(), "self keyword should not be renameable");
}

#[tokio::test]
async fn prepare_rename_rejects_static() {
    let backend = Backend::new_test();
    let uri = Url::parse("file:///test.php").unwrap();
    let text = concat!(
        "<?php\n",
        "class Foo {\n",
        "    public static function create(): static {\n",
        "        return new static();\n",
        "    }\n",
        "}\n",
    );

    open_file(&backend, &uri, text).await;

    let response = prepare_rename(&backend, &uri, 3, 22).await;
    assert!(
        response.is_none(),
        "static keyword should not be renameable"
    );
}

#[tokio::test]
async fn prepare_rename_rejects_parent() {
    let backend = Backend::new_test();
    let uri = Url::parse("file:///test.php").unwrap();
    let text = concat!(
        "<?php\n",
        "class Base {\n",
        "    public function hello(): void {}\n",
        "}\n",
        "class Child extends Base {\n",
        "    public function hello(): void {\n",
        "        parent::hello();\n",
        "    }\n",
        "}\n",
    );

    open_file(&backend, &uri, text).await;

    let response = prepare_rename(&backend, &uri, 6, 10).await;
    assert!(
        response.is_none(),
        "parent keyword should not be renameable"
    );
}

// ─── Class Rename ───────────────────────────────────────────────────────────

#[tokio::test]
async fn rename_class_same_file() {
    let backend = Backend::new_test();
    let uri = Url::parse("file:///test.php").unwrap();
    let text = concat!(
        "<?php\n",
        "class Logger {\n",
        "    public function log(string $msg): void {}\n",
        "}\n",
        "function demo(Logger $logger): void {\n",
        "    $obj = new Logger();\n",
        "}\n",
    );

    open_file(&backend, &uri, text).await;

    // Rename from a reference site (type hint on line 4).
    let edit = rename(&backend, &uri, 4, 16, "AppLogger").await;
    assert!(edit.is_some(), "Expected a workspace edit for class rename");

    let file_edits = edits_for_uri(&edit.unwrap(), &uri);
    // Should find: declaration (L1), type hint (L4), new (L5) = at least 3.
    assert!(
        file_edits.len() >= 3,
        "Expected at least 3 edits for Logger, got {}",
        file_edits.len()
    );

    for te in &file_edits {
        assert_eq!(te.new_text, "AppLogger");
    }
}

#[tokio::test]
async fn rename_class_from_declaration() {
    let backend = Backend::new_test();
    let uri = Url::parse("file:///test.php").unwrap();
    let text = concat!(
        "<?php\n",
        "class Widget {\n",
        "    public function render(): string { return ''; }\n",
        "}\n",
        "function demo(Widget $w): void {\n",
        "    $obj = new Widget();\n",
        "}\n",
    );

    open_file(&backend, &uri, text).await;

    // Rename from the declaration site (line 1).
    let edit = rename(&backend, &uri, 1, 7, "Component").await;
    assert!(edit.is_some());

    let file_edits = edits_for_uri(&edit.unwrap(), &uri);
    assert!(
        file_edits.len() >= 3,
        "Expected at least 3 edits for Widget, got {}",
        file_edits.len()
    );

    for te in &file_edits {
        assert_eq!(te.new_text, "Component");
    }
}

#[tokio::test]
async fn prepare_rename_class() {
    let backend = Backend::new_test();
    let uri = Url::parse("file:///test.php").unwrap();
    let text = concat!(
        "<?php\n",
        "class Foo {}\n",
        "function demo(Foo $f): void {}\n",
    );

    open_file(&backend, &uri, text).await;

    let response = prepare_rename(&backend, &uri, 1, 7).await;
    assert!(response.is_some());

    if let Some(PrepareRenameResponse::RangeWithPlaceholder { placeholder, .. }) = response {
        assert_eq!(placeholder, "Foo");
    } else {
        panic!("Expected RangeWithPlaceholder response");
    }
}

// ─── Method Rename ──────────────────────────────────────────────────────────

#[tokio::test]
async fn rename_method() {
    let backend = Backend::new_test();
    let uri = Url::parse("file:///test.php").unwrap();
    let text = concat!(
        "<?php\n",
        "class Service {\n",
        "    public function process(): void {}\n",
        "}\n",
        "function demo(): void {\n",
        "    $s = new Service();\n",
        "    $s->process();\n",
        "    $s->process();\n",
        "}\n",
    );

    open_file(&backend, &uri, text).await;

    // Rename from call site (line 6).
    let edit = rename(&backend, &uri, 6, 9, "execute").await;
    assert!(
        edit.is_some(),
        "Expected a workspace edit for method rename"
    );

    let file_edits = edits_for_uri(&edit.unwrap(), &uri);
    // Should find: declaration (L2) + 2 call sites (L6, L7) = at least 3.
    assert!(
        file_edits.len() >= 3,
        "Expected at least 3 edits for process, got {}",
        file_edits.len()
    );

    for te in &file_edits {
        assert_eq!(te.new_text, "execute");
    }
}

#[tokio::test]
async fn rename_static_method() {
    let backend = Backend::new_test();
    let uri = Url::parse("file:///test.php").unwrap();
    let text = concat!(
        "<?php\n",
        "class Factory {\n",
        "    public static function create(): self { return new self(); }\n",
        "}\n",
        "function demo(): void {\n",
        "    Factory::create();\n",
        "    Factory::create();\n",
        "}\n",
    );

    open_file(&backend, &uri, text).await;

    let edit = rename(&backend, &uri, 5, 14, "build").await;
    assert!(edit.is_some());

    let file_edits = edits_for_uri(&edit.unwrap(), &uri);
    assert!(
        file_edits.len() >= 3,
        "Expected at least 3 edits for create, got {}",
        file_edits.len()
    );

    for te in &file_edits {
        assert_eq!(te.new_text, "build");
    }
}

// ─── Property Rename ────────────────────────────────────────────────────────

#[tokio::test]
async fn rename_property_from_access() {
    let backend = Backend::new_test();
    let uri = Url::parse("file:///test.php").unwrap();
    let text = concat!(
        "<?php\n",
        "class User {\n",
        "    public string $name = '';\n",
        "    public function greet(): string {\n",
        "        return $this->name;\n",
        "    }\n",
        "}\n",
        "function demo(): void {\n",
        "    $u = new User();\n",
        "    $u->name = 'Alice';\n",
        "}\n",
    );

    open_file(&backend, &uri, text).await;

    // Rename from access site (line 9, `$u->name`).
    let edit = rename(&backend, &uri, 9, 9, "displayName").await;
    assert!(
        edit.is_some(),
        "Expected a workspace edit for property rename"
    );

    let file_edits = edits_for_uri(&edit.unwrap(), &uri);
    // Should have edits for: declaration ($name), $this->name, $u->name.
    assert!(
        file_edits.len() >= 3,
        "Expected at least 3 edits for name property, got {}",
        file_edits.len()
    );

    // The declaration site includes `$`, access sites don't.
    for te in &file_edits {
        assert!(
            te.new_text == "displayName" || te.new_text == "$displayName",
            "Unexpected edit text: {}",
            te.new_text
        );
    }
}

// ─── Function Rename ────────────────────────────────────────────────────────

#[tokio::test]
async fn rename_function() {
    let backend = Backend::new_test();
    let uri = Url::parse("file:///test.php").unwrap();
    let text = concat!(
        "<?php\n",
        "function helper(): void {}\n",
        "function demo(): void {\n",
        "    helper();\n",
        "    helper();\n",
        "}\n",
    );

    open_file(&backend, &uri, text).await;

    let edit = rename(&backend, &uri, 3, 6, "utility").await;
    assert!(
        edit.is_some(),
        "Expected a workspace edit for function rename"
    );

    let file_edits = edits_for_uri(&edit.unwrap(), &uri);
    // declaration (L1) + 2 call sites (L3, L4) = at least 3.
    assert!(
        file_edits.len() >= 3,
        "Expected at least 3 edits for helper, got {}",
        file_edits.len()
    );

    for te in &file_edits {
        assert_eq!(te.new_text, "utility");
    }
}

// ─── Constant Rename ────────────────────────────────────────────────────────

#[tokio::test]
async fn rename_class_constant() {
    let backend = Backend::new_test();
    let uri = Url::parse("file:///test.php").unwrap();
    let text = concat!(
        "<?php\n",
        "class Status {\n",
        "    const ACTIVE = 1;\n",
        "}\n",
        "function demo(): void {\n",
        "    echo Status::ACTIVE;\n",
        "    $x = Status::ACTIVE;\n",
        "}\n",
    );

    open_file(&backend, &uri, text).await;

    let edit = rename(&backend, &uri, 5, 19, "ENABLED").await;
    assert!(
        edit.is_some(),
        "Expected a workspace edit for constant rename"
    );

    let file_edits = edits_for_uri(&edit.unwrap(), &uri);
    assert!(
        file_edits.len() >= 3,
        "Expected at least 3 edits for ACTIVE, got {}",
        file_edits.len()
    );

    for te in &file_edits {
        assert_eq!(te.new_text, "ENABLED");
    }
}

// ─── Cross-file Rename ─────────────────────────────────────────────────────

#[tokio::test]
async fn rename_class_cross_file() {
    let backend = Backend::new_test();
    let uri_a = Url::parse("file:///a.php").unwrap();
    let uri_b = Url::parse("file:///b.php").unwrap();

    let text_a = concat!(
        "<?php\n",
        "class Animal {\n",
        "    public function speak(): string { return ''; }\n",
        "}\n",
    );

    let text_b = concat!(
        "<?php\n",
        "function demo(Animal $a): void {\n",
        "    $obj = new Animal();\n",
        "}\n",
    );

    open_file(&backend, &uri_a, text_a).await;
    open_file(&backend, &uri_b, text_b).await;

    // Rename from file a (declaration).
    let edit = rename(&backend, &uri_a, 1, 7, "Creature").await;
    assert!(
        edit.is_some(),
        "Expected a workspace edit for cross-file class rename"
    );

    let edit = edit.unwrap();
    let edits_a = edits_for_uri(&edit, &uri_a);
    let edits_b = edits_for_uri(&edit, &uri_b);

    assert!(
        !edits_a.is_empty(),
        "Expected edits in file a (declaration)"
    );
    assert!(!edits_b.is_empty(), "Expected edits in file b (references)");

    for te in edits_a.iter().chain(edits_b.iter()) {
        assert_eq!(te.new_text, "Creature");
    }
}

#[tokio::test]
async fn rename_method_cross_file() {
    let backend = Backend::new_test();
    let uri_a = Url::parse("file:///a.php").unwrap();
    let uri_b = Url::parse("file:///b.php").unwrap();

    let text_a = concat!(
        "<?php\n",
        "class Printer {\n",
        "    public function print(): void {}\n",
        "}\n",
    );

    let text_b = concat!(
        "<?php\n",
        "function demo(): void {\n",
        "    $p = new Printer();\n",
        "    $p->print();\n",
        "}\n",
    );

    open_file(&backend, &uri_a, text_a).await;
    open_file(&backend, &uri_b, text_b).await;

    // Rename from the call site in file b.
    let edit = rename(&backend, &uri_b, 3, 9, "output").await;
    assert!(edit.is_some());

    let edit = edit.unwrap();
    let edits_a = edits_for_uri(&edit, &uri_a);
    let edits_b = edits_for_uri(&edit, &uri_b);

    assert!(
        !edits_a.is_empty(),
        "Expected edits in file a (declaration)"
    );
    assert!(!edits_b.is_empty(), "Expected edits in file b (call site)");

    for te in edits_a.iter().chain(edits_b.iter()) {
        assert_eq!(te.new_text, "output");
    }
}

// ─── Whitespace / No Symbol ─────────────────────────────────────────────────

#[tokio::test]
async fn prepare_rename_on_whitespace_returns_none() {
    let backend = Backend::new_test();
    let uri = Url::parse("file:///test.php").unwrap();
    let text = concat!("<?php\n", "\n", "function demo(): void {}\n",);

    open_file(&backend, &uri, text).await;

    // Line 1 is blank.
    let response = prepare_rename(&backend, &uri, 1, 0).await;
    assert!(response.is_none(), "Expected no rename on whitespace");
}

#[tokio::test]
async fn rename_on_whitespace_returns_none() {
    let backend = Backend::new_test();
    let uri = Url::parse("file:///test.php").unwrap();
    let text = concat!("<?php\n", "\n", "function demo(): void {}\n",);

    open_file(&backend, &uri, text).await;

    let edit = rename(&backend, &uri, 1, 0, "anything").await;
    assert!(edit.is_none(), "Expected no edit on whitespace");
}

// ─── Result Correctness ─────────────────────────────────────────────────────

#[tokio::test]
async fn rename_variable_produces_valid_php() {
    let backend = Backend::new_test();
    let uri = Url::parse("file:///test.php").unwrap();
    let text = concat!(
        "<?php\n",
        "function demo(): void {\n",
        "    $a = 1;\n",
        "    $b = $a + 2;\n",
        "    echo $a;\n",
        "}\n",
    );

    open_file(&backend, &uri, text).await;

    let edit = rename(&backend, &uri, 2, 5, "$z").await;
    assert!(edit.is_some());

    let file_edits = edits_for_uri(&edit.unwrap(), &uri);
    let result = apply_edits(text, &file_edits);

    // The renamed variable should appear as `$z` everywhere.
    assert!(result.contains("$z = 1;"), "Declaration not renamed");
    assert!(result.contains("$b = $z + 2;"), "RHS usage not renamed");
    assert!(result.contains("echo $z;"), "Echo usage not renamed");
    // And the old name should be gone.
    assert!(!result.contains("$a"), "Old variable name still present");
}

// ─── Variable Scoping ───────────────────────────────────────────────────────

#[tokio::test]
async fn rename_variable_does_not_leak_across_functions() {
    let backend = Backend::new_test();
    let uri = Url::parse("file:///test.php").unwrap();
    let text = concat!(
        "<?php\n",
        "function alpha(): void {\n",
        "    $x = 1;\n",
        "    echo $x;\n",
        "}\n",
        "function beta(): void {\n",
        "    $x = 2;\n",
        "    echo $x;\n",
        "}\n",
    );

    open_file(&backend, &uri, text).await;

    // Rename $x in alpha (line 2).
    let edit = rename(&backend, &uri, 2, 5, "$y").await;
    assert!(edit.is_some());

    let file_edits = edits_for_uri(&edit.unwrap(), &uri);
    let result = apply_edits(text, &file_edits);

    // alpha should have $y, beta should still have $x.
    assert!(result.contains("function alpha(): void {\n    $y = 1;\n    echo $y;\n}"));
    assert!(result.contains("function beta(): void {\n    $x = 2;\n    echo $x;\n}"));
}

// ─── Class-Aware Member Rename ──────────────────────────────────────────────

#[tokio::test]
async fn rename_method_does_not_leak_to_unrelated_class() {
    // Two unrelated classes with the same method name.  Renaming the
    // method on one class must not touch the other.
    let backend = Backend::new_test();
    let uri = Url::parse("file:///test.php").unwrap();
    let text = concat!(
        "<?php\n",                                 // L0
        "class Dog {\n",                           // L1
        "    public function speak(): void {}\n",  // L2
        "}\n",                                     // L3
        "class Cat {\n",                           // L4
        "    public function speak(): void {}\n",  // L5
        "}\n",                                     // L6
        "function demo(Dog $d, Cat $c): void {\n", // L7
        "    $d->speak();\n",                      // L8
        "    $c->speak();\n",                      // L9
        "}\n",                                     // L10
    );

    open_file(&backend, &uri, text).await;

    // Rename speak() from the Dog::speak declaration (line 2, col 21).
    // "    public function speak(): void {}"
    //                     ^ col 20
    let edit = rename(&backend, &uri, 2, 21, "bark").await;
    assert!(edit.is_some(), "Rename should produce edits");

    let file_edits = edits_for_uri(&edit.unwrap(), &uri);
    let result = apply_edits(text, &file_edits);

    // Dog::speak and $d->speak should be renamed to bark.
    assert!(
        result.contains("function bark()"),
        "Dog's method should be renamed to bark; got:\n{}",
        result
    );
    assert!(
        result.contains("$d->bark()"),
        "$d->speak() should become $d->bark(); got:\n{}",
        result
    );

    // Cat::speak and $c->speak must NOT be renamed.
    assert!(
        result.contains("class Cat {\n    public function speak(): void {}"),
        "Cat's method should remain speak; got:\n{}",
        result
    );
    assert!(
        result.contains("$c->speak()"),
        "$c->speak() should remain unchanged; got:\n{}",
        result
    );
}

#[tokio::test]
async fn rename_method_includes_inherited_class() {
    // Renaming a method on a parent class should also rename it on
    // accesses through a child class.
    let backend = Backend::new_test();
    let uri = Url::parse("file:///test.php").unwrap();
    let text = concat!(
        "<?php\n",                                    // L0
        "class Base {\n",                             // L1
        "    public function run(): void {}\n",       // L2
        "}\n",                                        // L3
        "class Child extends Base {}\n",              // L4
        "function demo(Base $b, Child $c): void {\n", // L5
        "    $b->run();\n",                           // L6
        "    $c->run();\n",                           // L7
        "}\n",                                        // L8
    );

    open_file(&backend, &uri, text).await;

    // Rename run() from $b->run() (line 6, col 10).
    let edit = rename(&backend, &uri, 6, 10, "execute").await;
    assert!(edit.is_some(), "Rename should produce edits");

    let file_edits = edits_for_uri(&edit.unwrap(), &uri);
    let result = apply_edits(text, &file_edits);

    // Both $b->run() and $c->run() should be renamed (Child extends Base).
    assert!(
        result.contains("$b->execute()"),
        "$b->run() should become $b->execute(); got:\n{}",
        result
    );
    assert!(
        result.contains("$c->execute()"),
        "$c->run() should become $c->execute() (inherited); got:\n{}",
        result
    );
}

#[tokio::test]
async fn rename_static_method_does_not_leak_to_unrelated_class() {
    let backend = Backend::new_test();
    let uri = Url::parse("file:///test.php").unwrap();
    let text = concat!(
        "<?php\n",                                        // L0
        "class Alpha {\n",                                // L1
        "    public static function create(): void {}\n", // L2
        "}\n",                                            // L3
        "class Beta {\n",                                 // L4
        "    public static function create(): void {}\n", // L5
        "}\n",                                            // L6
        "function demo(): void {\n",                      // L7
        "    Alpha::create();\n",                         // L8
        "    Beta::create();\n",                          // L9
        "}\n",                                            // L10
    );

    open_file(&backend, &uri, text).await;

    // Rename create() from Alpha::create() call (line 8, col 12).
    // "    Alpha::create();"
    //             ^ col 11
    let edit = rename(&backend, &uri, 8, 12, "make").await;
    assert!(edit.is_some(), "Rename should produce edits");

    let file_edits = edits_for_uri(&edit.unwrap(), &uri);
    let result = apply_edits(text, &file_edits);

    // Alpha::create should be renamed.
    assert!(
        result.contains("Alpha::make()"),
        "Alpha::create() should become Alpha::make(); got:\n{}",
        result
    );

    // Beta::create must NOT be renamed.
    assert!(
        result.contains("Beta::create()"),
        "Beta::create() should remain unchanged; got:\n{}",
        result
    );
}
