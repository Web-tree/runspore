//! `spec/schema.md`: the subset, the authoring form, the minimum size, and the check of
//! data with its error format.

use runspore_schema::{check, fit, inline, shown, smallest, validate, ROOT_LEVEL};
use serde_json::{json, Value};

fn refused(schema: Value) -> (String, String) {
    match check(&schema, ROOT_LEVEL) {
        Ok(()) => panic!("accepted: {schema}"),
        Err(e) => (e.path, e.message),
    }
}

fn changes() -> Value {
    json!({
        "type": "object",
        "properties": {
            "requests": {
                "type": "array", "minItems": 1,
                "items": {
                    "type": "object",
                    "properties": {"file": {"type": "string"}, "change": {"type": "string"}},
                    "required": ["file", "change"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["requests"],
        "additionalProperties": false
    })
}

#[test]
fn the_subset_accepts_every_keyword_it_lists() {
    check(&changes(), ROOT_LEVEL).unwrap();
    let all = json!({
        "title": "t", "description": "d",
        "anyOf": [
            {"type": "string", "minLength": 1, "maxLength": 5, "enum": ["a", "bb"]},
            {"type": "integer", "minimum": -3, "maximum": 3},
            {"type": "boolean"}, {"type": "null"}, {"const": 7},
            {"type": "array", "items": {"enum": [1, "x", true, null]}, "maxItems": 2}
        ]
    });
    check(&all, ROOT_LEVEL).unwrap();
}

#[test]
fn keywords_outside_the_subset_are_refused_with_their_path() {
    for keyword in ["pattern", "format"] {
        let (path, message) = refused(json!({"type": "object", "properties":
            {"name": {"type": "string", keyword: "x"}}}));
        assert_eq!(path, format!("/properties/name/{keyword}"));
        assert!(message.contains("not in the schema subset"), "{message}");
    }
    for keyword in ["oneOf", "allOf"] {
        let (path, _) = refused(json!({keyword: [{"type": "string"}]}));
        assert_eq!(path, format!("/{keyword}"));
    }
    let (path, _) = refused(json!({"if": {"type": "string"}, "then": {}}));
    assert_eq!(path, "/if");
    let (path, message) = refused(json!({"type": "number"}));
    assert_eq!(path, "/type");
    assert!(message.contains("integer"), "{message}");
    let (path, _) = refused(json!({"type": ["string", "null"]}));
    assert_eq!(path, "/type");
    let (path, message) = refused(json!({"$ref": "#/$defs/x"}));
    assert_eq!(path, "/$ref");
    assert!(message.contains("build"), "{message}");
}

#[test]
fn keywords_need_their_type_and_well_formed_values() {
    let cases = [
        (json!({"minLength": 1}), "/minLength"),
        (json!({"type": "array", "properties": {}}), "/properties"),
        (
            json!({"type": "object", "additionalProperties": true}),
            "/additionalProperties",
        ),
        (
            json!({"type": "object", "additionalProperties": {"type": "string"}}),
            "/additionalProperties",
        ),
        (
            json!({"type": "object", "properties": {"a": {}}, "required": ["b"]}),
            "/required/0",
        ),
        (
            json!({"type": "object", "properties": {"a": {}}, "required": ["a", "a"]}),
            "/required/1",
        ),
        (json!({"type": "string", "maxLength": 65537}), "/maxLength"),
        (json!({"type": "string", "minLength": -1}), "/minLength"),
        (
            json!({"type": "string", "minLength": 3, "maxLength": 2}),
            "/minLength",
        ),
        (json!({"enum": []}), "/enum"),
        (json!({"enum": ["a", "a"]}), "/enum/1"),
        (json!({"enum": [{"a": 1}]}), "/enum/0"),
        (json!({"anyOf": []}), "/anyOf"),
        (
            json!({"anyOf": [{"type": "string"}], "type": "string"}),
            "/type",
        ),
        (
            json!({"anyOf": [{"type": "string", "pattern": "x"}]}),
            "/anyOf/0/pattern",
        ),
        (
            json!({"type": "object", "properties": {"a/b": {"type": 1}}}),
            "/properties/a~1b/type",
        ),
        (json!("string"), ""),
    ];
    for (schema, expected) in cases {
        let (path, _) = refused(schema.clone());
        assert_eq!(path, expected, "{schema}");
    }
}

/// Wraps a schema in `levels` nested object properties.
fn nested(levels: usize) -> Value {
    (0..levels).fold(
        json!({"type": "string"}),
        |inner, _| json!({"type": "object", "properties": {"a": inner}}),
    )
}

#[test]
fn depth_counts_from_the_package_root() {
    // The root is level 5; each nested property adds two levels (`properties`, then
    // the property's schema), so 13 of them reach level 31 and 14 reach level 33.
    check(&nested(13), ROOT_LEVEL).unwrap();
    let (path, message) = refused(nested(14));
    assert_eq!(path, "");
    assert!(message.contains("32 levels"), "{message}");
    // The same schema deeper in the package has less room.
    assert!(check(&nested(13), ROOT_LEVEL + 2).is_err());
}

#[test]
fn a_schema_must_admit_a_value_within_the_result_limit() {
    let sizes = [
        (json!({"type": "null"}), 4),
        (json!({"type": "boolean"}), 4),
        (json!({"type": "integer"}), 1),
        (json!({"type": "integer", "minimum": 100}), 3),
        (json!({"type": "integer", "maximum": -20}), 3),
        (json!({"type": "string", "minLength": 3}), 5),
        (json!({"type": "array"}), 2),
        (
            json!({"type": "array", "minItems": 3, "items": {"type": "string"}}),
            10,
        ),
        (
            json!({"type": "object", "properties": {"ab": {"type": "integer"}, "c": {}},
                "required": ["ab", "c"]}),
            14,
        ),
        (json!({"enum": ["long", 12]}), 2),
        (
            json!({"anyOf": [{"type": "string", "minLength": 9}, {"type": "null"}]}),
            4,
        ),
        (json!({}), 1),
    ];
    for (schema, size) in sizes {
        assert_eq!(smallest(&schema), size, "{schema}");
    }
    check(&json!({"type": "string", "minLength": 65534}), ROOT_LEVEL).unwrap();
    let (path, message) = refused(json!({"type": "string", "minLength": 65535}));
    assert_eq!(path, "");
    assert!(message.contains("65537 bytes"), "{message}");
    let (_, message) = refused(json!({"type": "array", "minItems": 40000,
                                       "items": {"type": "null"}}));
    assert!(message.contains("result limit"), "{message}");
}

#[test]
fn the_authoring_form_inlines_local_definitions() {
    let authored = json!({
        "$defs": {"file": {"type": "string", "minLength": 1}},
        "type": "object",
        "properties": {
            "a": {"$ref": "#/$defs/file"},
            "b": {"$ref": "#/$defs/file", "description": "the other"}
        }
    });
    let inlined = inline(&authored).unwrap();
    assert_eq!(
        inlined,
        json!({"type": "object", "properties": {
            "a": {"type": "string", "minLength": 1},
            "b": {"type": "string", "minLength": 1, "description": "the other"}}})
    );
    check(&inlined, ROOT_LEVEL).unwrap();

    let bad = [
        (
            json!({"$ref": "https://example.com/s.json"}),
            "/$ref",
            "remote",
        ),
        (json!({"$ref": "other.json#/x"}), "/$ref", "remote"),
        (json!({"$ref": "#/definitions/x"}), "/$ref", "#/$defs/"),
        (
            json!({"$ref": "#/$defs/missing", "$defs": {}}),
            "/$ref",
            "no",
        ),
        (
            json!({"$defs": {"a": {"$ref": "#/$defs/b"}, "b": {"$ref": "#/$defs/a"}}}),
            "/$defs/b/$ref",
            "recurse",
        ),
        (
            json!({"$defs": {"a": {"type": "string"}}, "$ref": "#/$defs/a", "type": "string"}),
            "/type",
            "stands alone",
        ),
        (
            json!({"type": "object", "properties": {"x": {"$defs": {}}}}),
            "/properties/x/$defs",
            "root",
        ),
    ];
    for (schema, path, words) in bad {
        let e = inline(&schema).expect_err(&schema.to_string());
        assert_eq!(e.path, path, "{schema}");
        assert!(e.message.contains(words), "{schema}: {}", e.message);
    }
}

#[test]
fn data_errors_name_their_json_pointer_and_what_arrived() {
    let errors = validate(
        &changes(),
        &json!({"requests": [
        {"file": "a.rs", "change": "x"},
        {"file": 42, "extra": true},
    ], "other": 1}),
    );
    let lines: Vec<String> = errors.iter().map(ToString::to_string).collect();
    assert_eq!(
        lines,
        [
            "/other: not allowed; allowed properties: requests",
            "/requests/1/change: required, but missing",
            "/requests/1/extra: not allowed; allowed properties: change, file",
            "/requests/1/file: expected type \"string\", got integer (42)",
        ]
    );
    let root = validate(&changes(), &Value::Null);
    assert_eq!(root[0].path, "");
    assert_eq!(
        root[0].to_string(),
        "(root): expected type \"object\", got null"
    );
    assert_eq!(shown(""), "(root)");
}

#[test]
fn every_keyword_is_enforced() {
    let fits = |schema: Value, data: Value| validate(&schema, &data).is_empty();
    assert!(fits(json!({"type": "string", "minLength": 2}), json!("éé")));
    assert!(!fits(
        json!({"type": "string", "minLength": 3}),
        json!("éé")
    ));
    assert!(!fits(
        json!({"type": "string", "maxLength": 1}),
        json!("ab")
    ));
    assert!(!fits(json!({"type": "integer", "minimum": 1}), json!(0)));
    assert!(!fits(json!({"type": "integer", "maximum": 1}), json!(2)));
    assert!(!fits(json!({"type": "array", "minItems": 1}), json!([])));
    assert!(!fits(
        json!({"type": "array", "maxItems": 1}),
        json!([1, 2])
    ));
    assert!(!fits(
        json!({"type": "array", "items": {"type": "string"}}),
        json!(["a", 1])
    ));
    assert!(!fits(json!({"enum": ["a", "b"]}), json!("c")));
    assert!(!fits(json!({"const": "a"}), json!("b")));
    assert!(fits(
        json!({"anyOf": [{"type": "string"}, {"type": "null"}]}),
        json!(null)
    ));
    assert!(!fits(
        json!({"anyOf": [{"type": "string"}, {"type": "null"}]}),
        json!(1)
    ));
    assert!(fits(
        json!({"type": "object", "properties": {"a": {}}}),
        json!({"b": 1})
    ));
    assert!(fits(json!({}), json!({"anything": [1]})));
    assert!(!fits(json!({"type": "boolean"}), json!("true")));
}

#[test]
fn at_most_sixteen_errors_are_reported() {
    let schema = json!({"type": "array", "items": {"type": "string"}});
    let data = Value::Array((0..40).map(|i| json!(i)).collect());
    let errors = validate(&schema, &data);
    assert_eq!(errors.len(), 16);
    assert_eq!(errors[15].path, "/15");
}

#[test]
fn a_misfit_carries_the_outcome_and_its_errors() {
    let misfit = fit("changes", &changes(), &json!({})).unwrap_err();
    assert_eq!(
        misfit.details(),
        json!({"outcome": "changes", "errors": [
            {"path": "/requests", "message": "required, but missing"}]})
    );
    assert_eq!(
        misfit.to_string(),
        "the data for outcome \"changes\" does not fit its schema\n/requests: required, but missing"
    );
    assert!(fit(
        "changes",
        &changes(),
        &json!({"requests": [{"file": "a", "change": "b"}]})
    )
    .is_ok());
}
