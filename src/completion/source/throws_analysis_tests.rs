use super::*;

// ── Low-level scanning tests ────────────────────────────────────────

#[test]
fn test_find_throw_statements_basic() {
    let body = r#"
        throw new InvalidArgumentException("bad");
        throw new \RuntimeException("oops");
    "#;
    let throws = find_throw_statements(body);
    assert_eq!(throws.len(), 2);
    assert_eq!(throws[0].type_name, "InvalidArgumentException");
    assert_eq!(throws[1].type_name, "\\RuntimeException");
}

#[test]
fn test_find_throw_statements_skips_strings() {
    let body = r#"
        $msg = "throw new FakeException()";
        throw new RealException("msg");
    "#;
    let throws = find_throw_statements(body);
    assert_eq!(throws.len(), 1);
    assert_eq!(throws[0].type_name, "RealException");
}

#[test]
fn test_find_throw_statements_skips_comments() {
    let body = r#"
        // throw new CommentException();
        /* throw new BlockException(); */
        throw new RealException("msg");
    "#;
    let throws = find_throw_statements(body);
    assert_eq!(throws.len(), 1);
    assert_eq!(throws[0].type_name, "RealException");
}

#[test]
fn test_find_method_throws_tags_basic() {
    let content = r#"
/**
 * @throws InvalidArgumentException
 * @throws \RuntimeException
 */
public function doSomething(): void {
}
    "#;
    let tags = find_method_throws_tags(content, "doSomething");
    assert_eq!(tags, vec!["InvalidArgumentException", "RuntimeException"]);
}

#[test]
fn test_find_method_throws_tags_with_modifiers() {
    let content = r#"
/**
 * @throws InvalidArgumentException
 */
private static function doSomething(): void {
}
    "#;
    let tags = find_method_throws_tags(content, "doSomething");
    assert_eq!(tags, vec!["InvalidArgumentException"]);
}

#[test]
fn test_find_method_return_type_native() {
    let content = r#"
public function createException(): RuntimeException {
}
    "#;
    let ret = find_method_return_type(content, "createException");
    assert_eq!(ret, Some("RuntimeException".to_string()));
}

#[test]
fn test_find_method_return_type_docblock() {
    let content = r#"
/**
 * @return RuntimeException
 */
public function createException() {
}
    "#;
    let ret = find_method_return_type(content, "createException");
    assert_eq!(ret, Some("RuntimeException".to_string()));
}

#[test]
fn test_find_method_return_type_skips_void() {
    let content = r#"
/**
 * @return void
 */
public function doNothing() {
}
    "#;
    let ret = find_method_return_type(content, "doNothing");
    assert_eq!(ret, None);
}

#[test]
fn test_find_inline_throws_annotations() {
    let body = r#"
        /** @throws InvalidArgumentException */
        $client->request();
        /** @throws RuntimeException when things go wrong */
        $db->query();
    "#;
    let annotations = find_inline_throws_annotations(body);
    let names: Vec<&str> = annotations.iter().map(|t| t.type_name.as_str()).collect();
    assert_eq!(names, vec!["InvalidArgumentException", "RuntimeException"]);
}

#[test]
fn test_find_propagated_throws() {
    let file_content = r#"
/**
 * @throws IOException
 * @throws NetworkException
 */
public function riskyMethod(): void {
    // ...
}

public function caller(): void {
    $this->riskyMethod();
}
    "#;
    // Scan the body of `caller`
    let body = "$this->riskyMethod();";
    let propagated = find_propagated_throws(body, file_content);
    let names: Vec<&str> = propagated.iter().map(|t| t.type_name.as_str()).collect();
    assert_eq!(names, vec!["IOException", "NetworkException"]);
}

#[test]
fn test_find_throw_expression_types() {
    let file_content = r#"
public function createException(): RuntimeException {
    return new RuntimeException("oops");
}

public function caller(): void {
    throw $this->createException();
}
    "#;
    let body = "throw $this->createException();";
    let types = find_throw_expression_types(body, file_content);
    assert_eq!(types.len(), 1);
    assert_eq!(types[0].type_name, "RuntimeException");
}

#[test]
fn test_skip_modifiers_backward() {
    assert_eq!(skip_modifiers_backward("    public static "), "");
    assert_eq!(
        skip_modifiers_backward("/** @return void */ private "),
        "/** @return void */"
    );
    assert_eq!(
        skip_modifiers_backward("no modifiers here"),
        "no modifiers here"
    );
}

#[test]
fn test_find_method_return_type_with_nested_parens() {
    let content = r#"
public function createException(array $opts = array()): RuntimeException {
}
    "#;
    let ret = find_method_return_type(content, "createException");
    assert_eq!(ret, Some("RuntimeException".to_string()));
}

// ── High-level analysis tests ───────────────────────────────────────

#[test]
fn test_extract_function_body_basic() {
    let content = "<?php\n/** @return void */\nfunction foo(): void {\n    echo \"hello\";\n}\n";
    let pos = Position {
        line: 1,
        character: 5,
    };
    let body = extract_function_body(content, pos);
    assert!(body.is_some());
    assert!(body.unwrap().contains("echo"));
}

#[test]
fn test_extract_function_body_abstract() {
    let content = "<?php\n/** @return void */\nabstract function foo(): void;\n";
    let pos = Position {
        line: 1,
        character: 5,
    };
    let body = extract_function_body(content, pos);
    assert!(body.is_none());
}

#[test]
fn test_extract_function_body_with_nested_braces() {
    let content = concat!(
        "<?php\n",
        "/** @return void */\n",
        "function foo(): void {\n",
        "    if (true) {\n",
        "        echo 'inner';\n",
        "    }\n",
        "}\n",
    );
    let pos = Position {
        line: 1,
        character: 5,
    };
    let body = extract_function_body(content, pos).unwrap();
    assert!(body.contains("if (true)"));
    assert!(body.contains("echo 'inner'"));
}

#[test]
fn test_find_catch_blocks_basic() {
    let body = r#"
        try {
            throw new InvalidArgumentException("bad");
        } catch (InvalidArgumentException $e) {
            // handled
        }
        throw new RuntimeException("oops");
    "#;
    let catches = find_catch_blocks(body);
    assert_eq!(catches.len(), 1);
    assert_eq!(catches[0].type_names, vec!["InvalidArgumentException"]);
}

#[test]
fn test_find_catch_blocks_multi_catch() {
    let body = r#"
        try {
            doSomething();
        } catch (InvalidArgumentException | RuntimeException $e) {
            // handled
        }
    "#;
    let catches = find_catch_blocks(body);
    assert_eq!(catches.len(), 1);
    assert_eq!(
        catches[0].type_names,
        vec!["InvalidArgumentException", "RuntimeException"]
    );
}

#[test]
fn test_parse_catch_types_basic() {
    let (types, var) = parse_catch_types("InvalidArgumentException $e");
    assert_eq!(types, vec!["InvalidArgumentException"]);
    assert_eq!(var.as_deref(), Some("$e"));
}

#[test]
fn test_parse_catch_types_multi() {
    let (types, var) = parse_catch_types("\\InvalidArgumentException | \\RuntimeException $e");
    assert_eq!(types, vec!["InvalidArgumentException", "RuntimeException"]);
    assert_eq!(var.as_deref(), Some("$e"));
}

#[test]
fn test_find_uncaught_throw_types_all_caught() {
    let content = concat!(
        "<?php\nclass Foo {\n",
        "    /**\n",
        "     * @\n",
        "     */\n",
        "    public function bar(): void {\n",
        "        try {\n",
        "            throw new InvalidArgumentException(\"bad\");\n",
        "        } catch (InvalidArgumentException $e) {\n",
        "            // handled\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let pos = Position {
        line: 3,
        character: 5,
    };
    let uncaught = find_uncaught_throw_types(content, pos);
    assert!(uncaught.is_empty());
}

#[test]
fn test_find_uncaught_throw_types_uncaught() {
    let content = concat!(
        "<?php\nclass Foo {\n",
        "    /**\n",
        "     * @\n",
        "     */\n",
        "    public function bar(): void {\n",
        "        throw new RuntimeException(\"oops\");\n",
        "    }\n",
        "}\n",
    );
    let pos = Position {
        line: 3,
        character: 5,
    };
    let uncaught = find_uncaught_throw_types(content, pos);
    assert_eq!(uncaught, vec!["RuntimeException"]);
}

#[test]
fn test_find_uncaught_throw_types_mixed() {
    let content = concat!(
        "<?php\nclass Foo {\n",
        "    /**\n",
        "     * @\n",
        "     */\n",
        "    public function bar(): void {\n",
        "        try {\n",
        "            throw new InvalidArgumentException(\"bad\");\n",
        "        } catch (InvalidArgumentException $e) {\n",
        "            // handled\n",
        "        }\n",
        "        throw new RuntimeException(\"oops\");\n",
        "    }\n",
        "}\n",
    );
    let pos = Position {
        line: 3,
        character: 5,
    };
    let uncaught = find_uncaught_throw_types(content, pos);
    assert_eq!(uncaught, vec!["RuntimeException"]);
}

#[test]
fn test_find_uncaught_throw_types_inline_annotation_caught() {
    let content = concat!(
        "<?php\nclass Foo {\n",
        "    /**\n",
        "     * @\n",
        "     */\n",
        "    public function bar(): void {\n",
        "        try {\n",
        "            /** @throws NotFoundException */\n",
        "            findOrFail();\n",
        "        } catch (NotFoundException $e) {\n",
        "            // handled\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let pos = Position {
        line: 3,
        character: 5,
    };
    let uncaught = find_uncaught_throw_types(content, pos);
    assert!(
        uncaught.is_empty(),
        "inline @throws inside try/catch should be excluded, got: {:?}",
        uncaught
    );
}

#[test]
fn test_find_uncaught_throw_types_inline_annotation_partially_caught() {
    let content = concat!(
        "<?php\nclass Foo {\n",
        "    /**\n",
        "     * @\n",
        "     */\n",
        "    public function bar(): void {\n",
        "        try {\n",
        "            /** @throws NotFoundException */\n",
        "            findOrFail();\n",
        "        } catch (NotFoundException $e) {\n",
        "            // handled\n",
        "        }\n",
        "        /** @throws RuntimeException */\n",
        "        riskyCall();\n",
        "    }\n",
        "}\n",
    );
    let pos = Position {
        line: 3,
        character: 5,
    };
    let uncaught = find_uncaught_throw_types(content, pos);
    assert_eq!(
        uncaught,
        vec!["RuntimeException"],
        "only the uncaught inline @throws should remain"
    );
}

// ── throw $variable tests ───────────────────────────────────────────

#[test]
fn test_find_uncaught_throw_variable_from_catch() {
    // `throw $e` inside a catch block re-throws the caught type.
    let content = concat!(
        "<?php\nclass Foo {\n",
        "    /**\n",
        "     * @\n",
        "     */\n",
        "    public function bar(): void {\n",
        "        try {\n",
        "            throw new ValidationException('bad');\n",
        "        } catch (ValidationException $e) {\n",
        "            throw $e;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let pos = Position {
        line: 3,
        character: 5,
    };
    let uncaught = find_uncaught_throw_types(content, pos);
    assert_eq!(
        uncaught,
        vec!["ValidationException"],
        "re-thrown catch variable should appear in uncaught list"
    );
}

#[test]
fn test_find_uncaught_throw_variable_not_rethrown() {
    // The caught exception is NOT re-thrown — should be empty.
    let content = concat!(
        "<?php\nclass Foo {\n",
        "    /**\n",
        "     * @\n",
        "     */\n",
        "    public function bar(): void {\n",
        "        try {\n",
        "            throw new ValidationException('bad');\n",
        "        } catch (ValidationException $e) {\n",
        "            // swallowed\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let pos = Position {
        line: 3,
        character: 5,
    };
    let uncaught = find_uncaught_throw_types(content, pos);
    assert!(
        uncaught.is_empty(),
        "caught and not re-thrown should be empty, got: {:?}",
        uncaught
    );
}

#[test]
fn test_find_uncaught_throw_variable_multiple_catches() {
    // Two catch blocks, each re-throwing — both types should appear.
    let content = concat!(
        "<?php\nclass Foo {\n",
        "    /**\n",
        "     * @\n",
        "     */\n",
        "    public function bar(): void {\n",
        "        try {\n",
        "            doSomething();\n",
        "        } catch (ValidationException $e) {\n",
        "            throw $e;\n",
        "        } catch (NotFoundException $e) {\n",
        "            throw $e;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let pos = Position {
        line: 3,
        character: 5,
    };
    let uncaught = find_uncaught_throw_types(content, pos);
    assert!(
        uncaught.contains(&"ValidationException".to_string()),
        "should contain ValidationException, got: {:?}",
        uncaught
    );
    assert!(
        uncaught.contains(&"NotFoundException".to_string()),
        "should contain NotFoundException, got: {:?}",
        uncaught
    );
}

// ── throw functionCall() tests ──────────────────────────────────────

#[test]
fn test_find_throw_expression_bare_function_call() {
    let file_content = r#"
function makeException(): RuntimeException {
    return new RuntimeException("oops");
}

public function caller(): void {
    throw makeException();
}
    "#;
    let body = "throw makeException();";
    let types = find_throw_expression_types(body, file_content);
    assert_eq!(types.len(), 1, "should resolve bare function call");
    assert_eq!(types[0].type_name, "RuntimeException");
}

#[test]
fn test_find_uncaught_throw_bare_function_call() {
    let content = concat!(
        "<?php\n",
        "function makeException(): RuntimeException {\n",
        "    return new RuntimeException('oops');\n",
        "}\n",
        "class Foo {\n",
        "    /**\n",
        "     * @\n",
        "     */\n",
        "    public function bar(): void {\n",
        "        throw makeException();\n",
        "    }\n",
        "}\n",
    );
    let pos = Position {
        line: 6,
        character: 5,
    };
    let uncaught = find_uncaught_throw_types(content, pos);
    assert_eq!(
        uncaught,
        vec!["RuntimeException"],
        "bare function call return type should appear in uncaught"
    );
}

#[test]
fn test_find_uncaught_throw_bare_function_caught() {
    // throw functionCall() inside a try/catch that catches it.
    let content = concat!(
        "<?php\n",
        "function makeException(): RuntimeException {\n",
        "    return new RuntimeException('oops');\n",
        "}\n",
        "class Foo {\n",
        "    /**\n",
        "     * @\n",
        "     */\n",
        "    public function bar(): void {\n",
        "        try {\n",
        "            throw makeException();\n",
        "        } catch (RuntimeException $e) {\n",
        "            // handled\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let pos = Position {
        line: 6,
        character: 5,
    };
    let uncaught = find_uncaught_throw_types(content, pos);
    assert!(
        uncaught.is_empty(),
        "caught bare function throw should be empty, got: {:?}",
        uncaught
    );
}

// ── Import helper tests ─────────────────────────────────────────────

#[test]
fn test_resolve_exception_fqn_from_use_map() {
    let mut use_map = HashMap::new();
    use_map.insert(
        "RuntimeException".to_string(),
        "App\\Exceptions\\RuntimeException".to_string(),
    );
    let result = resolve_exception_fqn("RuntimeException", &use_map, &None);
    assert_eq!(
        result,
        Some("App\\Exceptions\\RuntimeException".to_string())
    );
}

#[test]
fn test_resolve_exception_fqn_from_namespace() {
    let use_map = HashMap::new();
    let ns = Some("App\\Services".to_string());
    let result = resolve_exception_fqn("CustomException", &use_map, &ns);
    assert_eq!(result, Some("App\\Services\\CustomException".to_string()));
}

#[test]
fn test_resolve_exception_fqn_global() {
    let use_map = HashMap::new();
    let result = resolve_exception_fqn("RuntimeException", &use_map, &None);
    assert_eq!(result, None);
}

#[test]
fn test_has_use_import_direct() {
    let content = "<?php\nuse App\\Exceptions\\RuntimeException;\n";
    assert!(has_use_import(content, "App\\Exceptions\\RuntimeException"));
    assert!(!has_use_import(content, "App\\Exceptions\\LogicException"));
}

#[test]
fn test_has_use_import_group() {
    let content = "<?php\nuse App\\Exceptions\\{RuntimeException, LogicException};\n";
    assert!(has_use_import(content, "App\\Exceptions\\RuntimeException"));
    assert!(has_use_import(content, "App\\Exceptions\\LogicException"));
    assert!(!has_use_import(content, "App\\Exceptions\\CustomException"));
}

#[test]
fn test_has_use_import_alias() {
    let content = "<?php\nuse App\\Exceptions\\RuntimeException as RE;\n";
    assert!(has_use_import(content, "App\\Exceptions\\RuntimeException"));
}
