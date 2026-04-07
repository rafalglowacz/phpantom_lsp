//! Integration tests for JetBrains `.phpstorm.meta.php` `override()` return-type inference.

use crate::common::create_psr4_workspace;
use phpantom_lsp::Backend;
use tower_lsp::LanguageServer;
use tower_lsp::lsp_types::*;

/// Synthetic document URI (matches other integration tests). Workspace PHP
/// classes and `.phpstorm.meta.php` still live on disk under `workspace_root`.
fn site_uri() -> Url {
    Url::parse("file:///site.php").unwrap()
}

const COMPOSER: &str = r#"{
    "autoload": {
        "psr-4": {
            "Demo\\": "src/"
        }
    }
}"#;

const ALPHA: &str = concat!(
    "<?php\n",
    "namespace Demo;\n",
    "class Alpha {\n",
    "    public function alphaMethod(): void {}\n",
    "}\n",
);

const BETA: &str = concat!(
    "<?php\n",
    "namespace Demo;\n",
    "class Beta {\n",
    "    public function betaMethod(): void {}\n",
    "}\n",
);

/// `override(..., type(0))` — return type follows the first argument (`::class` → class).
#[tokio::test]
async fn completion_phpstorm_meta_type_arg() {
    let factory = concat!(
        "<?php\n",
        "namespace Demo;\n",
        "class Factory {\n",
        "    public static function get(string $name) {\n",
        "        return null;\n",
        "    }\n",
        "}\n",
    );
    let meta = concat!(
        "<?php\n",
        "namespace PHPSTORM_META {\n",
        "    override(\\Demo\\Factory::get(0), type(0));\n",
        "}\n",
    );
    let site = concat!(
        "<?php\n",
        "namespace App;\n",
        "class X {\n",
        "    function t() {\n",
        "        \\Demo\\Factory::get(\\Demo\\Alpha::class)->\n",
        "    }\n",
        "}\n",
    );

    let (backend, dir) = create_psr4_workspace(
        COMPOSER,
        &[
            ("src/Alpha.php", ALPHA),
            ("src/Factory.php", factory),
            (".phpstorm.meta.php", meta),
            ("site.php", site),
        ],
    );

    backend.initialized(InitializedParams {}).await;

    let uri = site_uri();
    let text = std::fs::read_to_string(dir.path().join("site.php")).unwrap();
    backend
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: "php".to_string(),
                version: 1,
                text,
            },
        })
        .await;

    let line = "        \\Demo\\Factory::get(\\Demo\\Alpha::class)->";
    let col = line.chars().count() as u32;

    let result = backend
        .completion(CompletionParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: Position {
                    line: 4,
                    character: col,
                },
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
            context: None,
        })
        .await
        .unwrap();

    let items = match result {
        Some(CompletionResponse::Array(items)) => items,
        other => panic!("expected completion list, got {:?}", other),
    };
    let methods: Vec<&str> = items
        .iter()
        .filter(|i| i.kind == Some(CompletionItemKind::METHOD))
        .map(|i| i.filter_text.as_deref().unwrap_or(i.label.as_str()))
        .collect();
    assert!(
        methods.contains(&"alphaMethod"),
        "expected Demo\\Alpha::alphaMethod from type(0), got {:?}",
        methods
    );
}

/// `override(..., elementType(0))` — return type is the array value type of arg 0.
#[tokio::test]
async fn completion_phpstorm_meta_element_type_arg() {
    let factory = concat!(
        "<?php\n",
        "namespace Demo;\n",
        "class Factory {\n",
        "    public static function first(array $items) {\n",
        "        return null;\n",
        "    }\n",
        "}\n",
    );
    let meta = concat!(
        "<?php\n",
        "namespace PHPSTORM_META {\n",
        "    override(\\Demo\\Factory::first(0), elementType(0));\n",
        "}\n",
    );
    let site = concat!(
        "<?php\n",
        "namespace App;\n",
        "class X {\n",
        "    function t() {\n",
        "        \\Demo\\Factory::first([new \\Demo\\Alpha])->\n",
        "    }\n",
        "}\n",
    );

    let (backend, dir) = create_psr4_workspace(
        COMPOSER,
        &[
            ("src/Alpha.php", ALPHA),
            ("src/Factory.php", factory),
            (".phpstorm.meta.php", meta),
            ("site.php", site),
        ],
    );

    backend.initialized(InitializedParams {}).await;

    let uri = site_uri();
    let text = std::fs::read_to_string(dir.path().join("site.php")).unwrap();
    backend
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: "php".to_string(),
                version: 1,
                text,
            },
        })
        .await;

    let line = "        \\Demo\\Factory::first([new \\Demo\\Alpha])->";
    let col = line.chars().count() as u32;

    let result = backend
        .completion(CompletionParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: Position {
                    line: 4,
                    character: col,
                },
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
            context: None,
        })
        .await
        .unwrap();

    let items = match result {
        Some(CompletionResponse::Array(items)) => items,
        other => panic!("expected completion list, got {:?}", other),
    };
    let methods: Vec<&str> = items
        .iter()
        .filter(|i| i.kind == Some(CompletionItemKind::METHOD))
        .map(|i| i.filter_text.as_deref().unwrap_or(i.label.as_str()))
        .collect();
    assert!(
        methods.contains(&"alphaMethod"),
        "expected element type Demo\\Alpha from [new Alpha], got {:?}",
        methods
    );
}

/// `override(..., map([...]))` — string key match and `''` default.
#[tokio::test]
async fn completion_phpstorm_meta_map_keys() {
    let factory = concat!(
        "<?php\n",
        "namespace Demo;\n",
        "class Factory {\n",
        "    public static function make(string $key) {\n",
        "        return null;\n",
        "    }\n",
        "}\n",
    );
    let meta = concat!(
        "<?php\n",
        "namespace PHPSTORM_META {\n",
        "    override(\\Demo\\Factory::make(0), map([\n",
        "        'a' => \\Demo\\Alpha::class,\n",
        "        '' => \\Demo\\Beta::class,\n",
        "    ]));\n",
        "}\n",
    );

    let (backend, dir) = create_psr4_workspace(
        COMPOSER,
        &[
            ("src/Alpha.php", ALPHA),
            ("src/Beta.php", BETA),
            ("src/Factory.php", factory),
            (".phpstorm.meta.php", meta),
        ],
    );

    backend.initialized(InitializedParams {}).await;

    async fn complete_at_site(
        backend: &Backend,
        dir: &tempfile::TempDir,
        site_body: &str,
        line_idx: u32,
        col: u32,
    ) -> Vec<String> {
        std::fs::write(dir.path().join("site.php"), site_body).unwrap();
        let uri = site_uri();
        backend
            .did_open(DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri: uri.clone(),
                    language_id: "php".to_string(),
                    version: 1,
                    text: site_body.to_string(),
                },
            })
            .await;

        let result = backend
            .completion(CompletionParams {
                text_document_position: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri },
                    position: Position {
                        line: line_idx,
                        character: col,
                    },
                },
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: PartialResultParams::default(),
                context: None,
            })
            .await
            .unwrap();

        let items = match result {
            Some(CompletionResponse::Array(items)) => items,
            other => panic!("expected completion list, got {:?}", other),
        };
        items
            .iter()
            .filter(|i| i.kind == Some(CompletionItemKind::METHOD))
            .map(|i| {
                i.filter_text
                    .as_deref()
                    .unwrap_or(i.label.as_str())
                    .to_string()
            })
            .collect()
    }

    let prefix = concat!(
        "<?php\n",
        "namespace App;\n",
        "class X {\n",
        "    function t() {\n",
    );
    let suffix = concat!(
        "\n",
        "    }\n",
        "}\n",
    );

    // Explicit key 'a' → Alpha
    let line_a = "        \\Demo\\Factory::make('a')->";
    let body_a = format!("{prefix}{line_a}{suffix}");
    let col_a = line_a.chars().count() as u32;
    let methods_a = complete_at_site(&backend, &dir, &body_a, 4, col_a).await;
    assert!(
        methods_a.contains(&"alphaMethod".to_string()),
        "map key 'a' should resolve to Alpha, got {:?}",
        methods_a
    );

    // Unknown key → default Beta
    let line_d = "        \\Demo\\Factory::make('other')->";
    let body_d = format!("{prefix}{line_d}{suffix}");
    let col_d = line_d.chars().count() as u32;
    let methods_d = complete_at_site(&backend, &dir, &body_d, 4, col_d).await;
    assert!(
        methods_d.contains(&"betaMethod".to_string()),
        "default map key should resolve to Beta, got {:?}",
        methods_d
    );
}
