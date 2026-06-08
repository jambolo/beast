//! End-to-end tests for the full JsonLogic -> DNF -> minimize -> JsonLogic
//! pipeline (`simplify_json`).

use std::collections::HashMap;

use beast::json::Json;
use beast::{simplify_json, Expression};

/// Simplifies a JsonLogic string, returning the compact JsonLogic output.
fn simplify_str(text: &str) -> String {
    let json = Json::parse(text).unwrap();
    simplify_json(&json).unwrap().to_json().to_string()
}

/// Evaluates a simplified [`Expression`] under a name->value assignment.
fn eval(expr: &Expression, assign: &HashMap<&str, bool>) -> bool {
    if expr.is_true() {
        return true;
    }
    if expr.is_false() {
        return false;
    }
    expr.clauses().iter().filter(|c| !c.is_false()).any(|c| {
        (0..c.terms.len()).all(|i| {
            if c.mask[i] {
                let name = expr.table().name_of(i);
                assign[name] == c.terms[i]
            } else {
                true
            }
        })
    })
}

#[test]
fn readme_example_reduces_to_a() {
    let input =
        r#"{"or":[{"and":[{"var":"a"},{"var":"b"}]},{"and":[{"var":"a"},{"!":{"var":"b"}}]}]}"#;
    assert_eq!(simplify_str(input), r#"{"var":"a"}"#);
}

#[test]
fn tautology_becomes_true() {
    assert_eq!(
        simplify_str(r#"{"or":[{"var":"a"},{"!":[{"var":"a"}]}]}"#),
        "true"
    );
}

#[test]
fn contradiction_becomes_false() {
    assert_eq!(
        simplify_str(r#"{"and":[{"var":"a"},{"!":[{"var":"a"}]}]}"#),
        "false"
    );
}

#[test]
fn variable_names_round_trip() {
    // Arbitrary names are preserved through simplification.
    let input = r#"{"or":[{"and":[{"var":"raining"},{"var":"cold"}]},{"and":[{"var":"raining"},{"!":[{"var":"cold"}]}]}]}"#;
    assert_eq!(simplify_str(input), r#"{"var":"raining"}"#);
}

#[test]
fn too_many_variables_is_rejected() {
    // 33 distinct variables exceeds MAX_VARIABLES.
    let vars: Vec<String> = (0..33).map(|i| format!(r#"{{"var":"v{}"}}"#, i)).collect();
    let input = format!(r#"{{"and":[{}]}}"#, vars.join(","));
    let json = Json::parse(&input).unwrap();
    assert!(simplify_json(&json).is_err());
}

#[test]
fn malformed_operator_is_rejected() {
    let json = Json::parse(r#"{">":[{"var":"a"},{"var":"b"}]}"#).unwrap();
    assert!(simplify_json(&json).is_err());
}

/// Builds JsonLogic input as the sum (OR) of the given minterms over 3 named
/// variables a (bit 0), b (bit 1), c (bit 2).
fn input_from_minterms(minterms: &[u32]) -> Json {
    if minterms.is_empty() {
        return Json::Bool(false);
    }
    let names = ["a", "b", "c"];
    let clauses: Vec<Json> = minterms
        .iter()
        .map(|&m| {
            let literals: Vec<Json> = (0..3)
                .map(|i| {
                    let var = Json::Object(vec![(
                        "var".to_string(),
                        Json::String(names[i].to_string()),
                    )]);
                    if m & (1 << i) != 0 {
                        var
                    } else {
                        Json::Object(vec![("!".to_string(), Json::Array(vec![var]))])
                    }
                })
                .collect();
            Json::Object(vec![("and".to_string(), Json::Array(literals))])
        })
        .collect();
    Json::Object(vec![("or".to_string(), Json::Array(clauses))])
}

#[test]
fn simplify_preserves_truth_table_over_three_vars() {
    // DOD-4: for every boolean function of 3 variables, simplifying preserves
    // the truth table and produces valid DNF output.
    for table in 0u32..256 {
        let minterms: Vec<u32> = (0..8).filter(|&m| table & (1 << m) != 0).collect();
        let input = input_from_minterms(&minterms);
        let simplified = simplify_json(&input).unwrap();

        for assignment in 0u32..8 {
            let map: HashMap<&str, bool> = [
                ("a", assignment & 1 != 0),
                ("b", assignment & 2 != 0),
                ("c", assignment & 4 != 0),
            ]
            .into_iter()
            .collect();
            let expected = minterms.contains(&assignment);
            assert_eq!(
                eval(&simplified, &map),
                expected,
                "table={:08b} assignment={:03b}",
                table,
                assignment
            );
        }
    }
}
