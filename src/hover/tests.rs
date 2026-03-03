use super::*;

#[test]
fn extract_description_simple() {
    let doc = "/** This is a simple description. */";
    assert_eq!(
        extract_docblock_description(Some(doc)),
        Some("This is a simple description.".to_string())
    );
}

#[test]
fn extract_description_multiline() {
    let doc = "/**\n * First line.\n * Second line.\n * @param string $x\n */";
    assert_eq!(
        extract_docblock_description(Some(doc)),
        Some("First line.\nSecond line.".to_string())
    );
}

#[test]
fn extract_description_none_when_only_tags() {
    let doc = "/**\n * @return string\n */";
    assert_eq!(extract_docblock_description(Some(doc)), None);
}

#[test]
fn extract_description_none_when_empty() {
    assert_eq!(extract_docblock_description(None), None);
}

#[test]
fn namespace_line_with_namespace() {
    assert_eq!(
        namespace_line(&Some("App\\Models".to_string())),
        "namespace App\\Models;\n"
    );
}

#[test]
fn namespace_line_without_namespace() {
    assert_eq!(namespace_line(&None), "");
}

#[test]
fn format_params_empty() {
    assert_eq!(format_params(&[]), "");
}

#[test]
fn format_params_with_types() {
    let params = vec![
        ParameterInfo {
            name: "$name".to_string(),
            type_hint: Some("string".to_string()),
            native_type_hint: Some("string".to_string()),
            description: None,
            default_value: None,
            is_required: true,
            is_variadic: false,
            is_reference: false,
        },
        ParameterInfo {
            name: "$age".to_string(),
            type_hint: Some("int".to_string()),
            native_type_hint: Some("int".to_string()),
            description: None,
            default_value: None,
            is_required: false,
            is_variadic: false,
            is_reference: false,
        },
    ];
    assert_eq!(format_params(&params), "string $name, int $age = ...");
}

#[test]
fn format_params_variadic() {
    let params = vec![ParameterInfo {
        name: "$items".to_string(),
        type_hint: Some("string".to_string()),
        native_type_hint: Some("string".to_string()),
        description: None,
        default_value: None,
        is_required: false,
        is_variadic: true,
        is_reference: false,
    }];
    assert_eq!(format_params(&params), "string ...$items");
}

#[test]
fn format_params_reference() {
    let params = vec![ParameterInfo {
        name: "$arr".to_string(),
        type_hint: Some("array".to_string()),
        native_type_hint: Some("array".to_string()),
        description: None,
        default_value: None,
        is_required: true,
        is_variadic: false,
        is_reference: true,
    }];
    assert_eq!(format_params(&params), "array &$arr");
}

#[test]
fn format_visibility_all() {
    assert_eq!(format_visibility(Visibility::Public), "public ");
    assert_eq!(format_visibility(Visibility::Protected), "protected ");
    assert_eq!(format_visibility(Visibility::Private), "private ");
}

// ─── short_name tests ───────────────────────────────────────────────────────

#[test]
fn short_name_plain() {
    assert_eq!(short_name("User"), "User");
}

#[test]
fn short_name_namespaced() {
    assert_eq!(short_name("App\\Models\\User"), "User");
}

#[test]
fn short_name_leading_backslash() {
    assert_eq!(short_name("\\App\\Models\\User"), "User");
}

#[test]
fn short_name_scalar() {
    assert_eq!(short_name("string"), "string");
}

#[test]
fn short_name_single_namespace() {
    assert_eq!(short_name("Demo\\Brush"), "Brush");
}

// ─── types_equivalent tests ─────────────────────────────────────────────────

#[test]
fn types_equivalent_identical_strings() {
    assert!(types_equivalent("Brush", "Brush"));
}

#[test]
fn types_equivalent_fqn_vs_short() {
    assert!(types_equivalent("Brush", "Demo\\Brush"));
    assert!(types_equivalent("Demo\\Brush", "Brush"));
}

#[test]
fn types_equivalent_leading_backslash_fqn() {
    assert!(types_equivalent("Brush", "\\Demo\\Brush"));
    assert!(types_equivalent("\\Demo\\Brush", "Brush"));
}

#[test]
fn types_equivalent_nullable() {
    assert!(types_equivalent("?Brush", "?Demo\\Brush"));
    assert!(types_equivalent("?Demo\\Brush", "?Brush"));
}

#[test]
fn types_equivalent_union_with_null() {
    assert!(types_equivalent("Brush|null", "Demo\\Brush|null"));
    assert!(types_equivalent("null|Brush", "Demo\\Brush|null"));
}

#[test]
fn types_equivalent_different_types() {
    assert!(!types_equivalent("array", "list<User>"));
}

#[test]
fn types_equivalent_different_component_count() {
    assert!(!types_equivalent("Brush", "Brush|null"));
}

#[test]
fn types_equivalent_scalars() {
    assert!(types_equivalent("string", "string"));
    assert!(!types_equivalent("string", "int"));
}

#[test]
fn types_equivalent_intersection() {
    assert!(types_equivalent(
        "Countable&Traversable",
        "Countable&Traversable"
    ));
    assert!(types_equivalent(
        "Countable&Traversable",
        "App\\Countable&App\\Traversable"
    ));
}

#[test]
fn types_equivalent_different_short_names() {
    assert!(!types_equivalent("Brush", "Demo\\Canvas"));
}

// ─── shorten_type_string tests ──────────────────────────────────────────────

#[test]
fn shorten_type_string_plain_class() {
    assert_eq!(shorten_type_string("App\\Models\\User"), "User");
}

#[test]
fn shorten_type_string_already_short() {
    assert_eq!(shorten_type_string("User"), "User");
}

#[test]
fn shorten_type_string_scalar() {
    assert_eq!(shorten_type_string("string"), "string");
}

#[test]
fn shorten_type_string_nullable() {
    assert_eq!(shorten_type_string("?App\\Models\\User"), "?User");
}

#[test]
fn shorten_type_string_union() {
    assert_eq!(shorten_type_string("App\\Models\\User|null"), "User|null");
}

#[test]
fn shorten_type_string_generic() {
    assert_eq!(shorten_type_string("list<App\\Models\\User>"), "list<User>");
}

#[test]
fn shorten_type_string_nested_generic() {
    assert_eq!(
        shorten_type_string("array<int, App\\Collection<string, App\\Models\\User>>"),
        "array<int, Collection<string, User>>"
    );
}

#[test]
fn shorten_type_string_intersection() {
    assert_eq!(
        shorten_type_string("App\\Countable&App\\Traversable"),
        "Countable&Traversable"
    );
}

#[test]
fn shorten_type_string_leading_backslash() {
    assert_eq!(shorten_type_string("\\App\\Models\\User"), "User");
}

#[test]
fn shorten_type_string_object_shape() {
    assert_eq!(
        shorten_type_string("object{name: string, user: App\\Models\\User}"),
        "object{name: string, user: User}"
    );
}

#[test]
fn shorten_type_string_mixed_union_with_generics() {
    assert_eq!(
        shorten_type_string("App\\Collection<int, App\\Models\\User>|null"),
        "Collection<int, User>|null"
    );
}

#[test]
fn shorten_type_string_parenthesized_callable_union() {
    assert_eq!(
        shorten_type_string(
            "(\\Closure(static): mixed)|string|array|\\Illuminate\\Contracts\\Database\\Query\\Expression"
        ),
        "(Closure(static): mixed)|string|array|Expression"
    );
}
