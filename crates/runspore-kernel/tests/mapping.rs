//! `spec/kernel.md` section 4 through the crate's public mapping API, which hosts use
//! to evaluate their own mappings over roots of their own.

use runspore_kernel::mapping::{check, evaluate, Error};
use runspore_types::model::code;
use serde_json::{json, Map, Value};

const ROOTS: &[&str] = &["input", "params", "run", "step"];

fn scope() -> Map<String, Value> {
    let Value::Object(scope) = json!({
        "input": {"task": "fix it", "n": 2},
        "params": {"tools": ["Read", "Edit"], "schema": {"type": "object"}},
        "run": {"id": "run_1", "tenant": "default"},
        "step": {"node": "implement", "visit": 1, "attempt": 2, "resumed": true},
    }) else {
        unreachable!()
    };
    scope
}

fn eval(mapping: Value) -> Result<Value, Error> {
    check(&mapping, ROOTS).unwrap_or_else(|reason| panic!("{reason}: {mapping}"));
    evaluate(&mapping, &scope(), &mut 0)
}

#[test]
fn a_host_evaluates_its_own_roots_with_every_operator() {
    let argv = json!([
        "claude",
        {"$if": [{"$get": ["step", "resumed"]}, "--resume", "--session-id"]},
        {"$uuid": [{"$get": ["run", "id"]}, {"$get": ["step", "node"]}]},
        "--tools",
        {"$join": [{"$get": ["params", "tools"]}, ","]},
        {"$json": {"$get": ["params", "schema"]}},
        "uuid",
    ]);
    assert_eq!(
        eval(argv).unwrap(),
        json!([
            "claude",
            "--resume",
            "5c03f59b-23f0-5d9e-90a9-5ba675f87c39",
            "--tools",
            "Read,Edit",
            r#"{"type":"object"}"#,
            "uuid",
        ])
    );
}

#[test]
fn check_refuses_a_root_the_host_does_not_provide() {
    let reason = check(&json!({"$get": ["nodes", "a"]}), ROOTS).unwrap_err();
    assert!(
        reason.contains(r#""input", "params", "run" or "step""#),
        "{reason}"
    );
    assert!(check(&json!({"$if": [true, 1]}), ROOTS).is_err());
}

/// The same parts always give the same UUID: three values pinned against Python's
/// `uuid.uuid5` in the namespace `spec/kernel.md` §4 names.
#[test]
fn uuid_values_are_pinned() {
    for (parts, expected) in [
        (
            json!(["run_1", "implement"]),
            "5c03f59b-23f0-5d9e-90a9-5ba675f87c39",
        ),
        (
            json!(["run_1", "implement", 2]),
            "ce5f425f-bcf4-5168-844f-089797da8642",
        ),
        (json!([""]), "1b507cab-3397-5c50-b79c-aa6a6192220e"),
    ] {
        assert_eq!(eval(json!({"$uuid": parts})).unwrap(), json!(expected));
    }
    assert_ne!(
        eval(json!({"$uuid": ["a", "b"]})).unwrap(),
        eval(json!({"$uuid": ["ab"]})).unwrap()
    );
    assert_ne!(
        eval(json!({"$uuid": ["2"]})).unwrap(),
        eval(json!({"$uuid": [2]})).unwrap()
    );
}

#[test]
fn if_evaluates_only_the_chosen_branch() {
    assert_eq!(
        eval(json!({"$if": [false, {"$get": ["input", "absent"]}, "else"]})).unwrap(),
        json!("else")
    );
    assert_eq!(
        eval(json!({"$if": [true, "then", {"$get": ["input", "absent"]}]})).unwrap(),
        json!("then")
    );
    assert_eq!(
        eval(json!({"$if": [true, {"$get": ["input", "absent"]}, "else"]})),
        Err(Error::MissingPath(json!(["input", "absent"])))
    );
}

#[test]
fn json_is_canonical_and_join_takes_any_separator() {
    assert_eq!(
        eval(json!({"$json": {"b": [1, "two"], "a": null}})).unwrap(),
        json!(r#"{"a":null,"b":[1,"two"]}"#)
    );
    assert_eq!(eval(json!({"$json": "x"})).unwrap(), json!(r#""x""#));
    assert_eq!(eval(json!({"$join": [[], ","]})).unwrap(), json!(""));
    assert_eq!(
        eval(json!({"$join": [["a", "b", "c"], ""]})).unwrap(),
        json!("abc")
    );
}

/// Each wrong operand names the operator, where it sits and the value it had.
#[test]
fn a_wrong_operand_is_a_mapping_error() {
    let cases = [
        (
            json!({"mode": {"$if": [{"$get": ["input", "n"]}, 1, 2]}}),
            json!({"operator": "$if", "path": ["argv", "mode", "$if", 0], "value": 2}),
        ),
        (
            json!([{"$uuid": ["a", {"$get": ["params", "tools"]}]}]),
            json!({"operator": "$uuid", "path": ["argv", 0, "$uuid", 1], "value": ["Read", "Edit"]}),
        ),
        (
            json!({"$uuid": [null]}),
            json!({"operator": "$uuid", "path": ["argv", "$uuid", 0], "value": null}),
        ),
        (
            json!({"$join": [["a", true], ","]}),
            json!({"operator": "$join", "path": ["argv", "$join", 0, 1], "value": true}),
        ),
        (
            json!({"$join": [{"$get": ["input", "task"]}, ","]}),
            json!({"operator": "$join", "path": ["argv", "$join", 0], "value": "fix it"}),
        ),
        (
            json!({"$join": [["a"], 1]}),
            json!({"operator": "$join", "path": ["argv", "$join", 1], "value": 1}),
        ),
    ];
    for (mapping, details) in cases {
        let error = eval(mapping.clone()).expect_err("an operand error");
        assert_eq!(error.code(), code::MAPPING_INVALID_OPERAND, "{mapping}");
        assert_eq!(error.details(&[json!("argv")]), details, "{mapping}");
    }
    let missing = eval(json!({"$get": ["step", "absent"]})).unwrap_err();
    assert_eq!(missing.code(), code::MAPPING_MISSING_PATH);
    assert_eq!(missing.details(&[]), json!({"path": ["step", "absent"]}));
}

/// An operator costs one plus its operands; `$uuid` and `$join` one more per part or
/// item, `$json` one more per value it writes.
#[test]
fn operators_count_their_work() {
    let cost = |mapping: Value| {
        let mut operations = 0;
        evaluate(&mapping, &scope(), &mut operations).unwrap();
        operations
    };
    // The $if object, the cond, the chosen branch.
    assert_eq!(cost(json!({"$if": [true, "a", ["b", "c"]]})), 3);
    // The object, two parts, two more for the parts.
    assert_eq!(cost(json!({"$uuid": ["a", "b"]})), 5);
    // The object, the array and its two items, then three values written.
    assert_eq!(cost(json!({"$json": ["a", "b"]})), 7);
    // The object, the list and its two items, the separator, two items joined.
    assert_eq!(cost(json!({"$join": [["a", "b"], ","]})), 7);
}
