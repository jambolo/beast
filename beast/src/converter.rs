//! Converter (Library A): JsonLogic boolean expression -> DNF [`Expression`].
//!
//! [`to_dnf`] is a recursive descent over a JsonLogic tree that reuses the
//! boolean algebra on [`Expression`]: `|` concatenates clauses, `&` distributes
//! (product of sums -> sum of products), and [`Expression::inverse`] applies De
//! Morgan. Because every operator collapses its operands into DNF as the tree is
//! walked, the result is always in DNF.
//!
//! Accepted operators: the standard JsonLogic `var`, `!`, `and`, `or`, plus the
//! boolean literals `true` / `false`, plus the non-standard Beast extensions
//! `xor`, `nand`, `nor` (input-only; desugared here and never emitted on
//! output). Every other operator is rejected.

use crate::json::Json;
use crate::variable_table::VariableTable;
use crate::{Clause, Expression};

/// Parses a JsonLogic boolean expression into a DNF [`Expression`], registering
/// variables in `table`.
///
/// Returns `Err` with a human-readable message for malformed structure, unknown
/// operators, bad arity, or too many distinct variables.
pub fn to_dnf(json: &Json, table: &mut VariableTable) -> Result<Expression, String> {
    match json {
        Json::Bool(true) => Ok(Expression::r#true()),
        Json::Bool(false) => Ok(Expression::r#false()),
        Json::Object(pairs) => {
            if pairs.len() != 1 {
                return Err(format!(
                    "each node must have exactly one operator key, found {}",
                    pairs.len()
                ));
            }
            let (op, value) = &pairs[0];
            match op.as_str() {
                "var" => convert_var(value, table),
                "!" => {
                    let operand = unary_operand(value)?;
                    Ok(to_dnf(operand, table)?.inverse())
                }
                "and" => and_fold(args(value), table),
                "or" => or_fold(args(value), table),
                "nand" => Ok(and_fold(args(value), table)?.inverse()),
                "nor" => Ok(or_fold(args(value), table)?.inverse()),
                "xor" => xor_fold(args(value), table),
                other => Err(format!("unsupported operator {:?}", other)),
            }
        }
        Json::Array(_) | Json::Null | Json::Number(_) | Json::String(_) => Err(
            "expected a boolean expression: an operator object or a boolean literal".to_string(),
        ),
    }
}

/// `{"var": name}` -> a single positive literal.
fn convert_var(value: &Json, table: &mut VariableTable) -> Result<Expression, String> {
    let name = value
        .as_str()
        .ok_or_else(|| "\"var\" requires a string variable name".to_string())?;
    let index = table.index_of(name)?;
    let width = index + 1;
    let mut terms = vec![false; width];
    let mut mask = vec![false; width];
    terms[index] = true;
    mask[index] = true;
    Ok(Expression::new(vec![Clause { terms, mask }]))
}

/// Returns the operands of an n-ary operator. A JSON array gives its elements; a
/// bare value is treated as a single operand.
fn args(value: &Json) -> Vec<&Json> {
    match value {
        Json::Array(items) => items.iter().collect(),
        other => vec![other],
    }
}

/// Returns the single operand of a unary operator (`!`), accepting both the
/// array form `{"!": [x]}` and the bare form `{"!": x}`.
fn unary_operand(value: &Json) -> Result<&Json, String> {
    match value {
        Json::Array(items) => {
            if items.len() == 1 {
                Ok(&items[0])
            } else {
                Err(format!("\"!\" takes exactly one operand, found {}", items.len()))
            }
        }
        other => Ok(other),
    }
}

/// Folds operands with AND, starting from the TRUE identity.
fn and_fold(operands: Vec<&Json>, table: &mut VariableTable) -> Result<Expression, String> {
    if operands.is_empty() {
        return Err("\"and\" requires at least one operand".to_string());
    }
    let mut acc = Expression::r#true();
    for operand in operands {
        let next = to_dnf(operand, table)?;
        acc &= &next;
    }
    Ok(acc)
}

/// Folds operands with OR, starting from the FALSE identity.
fn or_fold(operands: Vec<&Json>, table: &mut VariableTable) -> Result<Expression, String> {
    if operands.is_empty() {
        return Err("\"or\" requires at least one operand".to_string());
    }
    let mut acc = Expression::r#false();
    for operand in operands {
        let next = to_dnf(operand, table)?;
        acc |= &next;
    }
    Ok(acc)
}

/// Folds operands with XOR (odd-parity), desugared pairwise into `and`/`or`/`!`:
/// `xor(p, q) = (p & !q) | (!p & q)`.
fn xor_fold(operands: Vec<&Json>, table: &mut VariableTable) -> Result<Expression, String> {
    let mut iter = operands.into_iter();
    let first = iter
        .next()
        .ok_or_else(|| "\"xor\" requires at least one operand".to_string())?;
    let mut acc = to_dnf(first, table)?;
    for operand in iter {
        let next = to_dnf(operand, table)?;
        let next_inv = next.inverse();
        let acc_inv = acc.inverse();
        let left = acc & &next_inv;
        let right = acc_inv & &next;
        acc = left | &right;
    }
    Ok(acc)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dnf(text: &str) -> Result<Expression, String> {
        let json = Json::parse(text).unwrap();
        let mut table = VariableTable::new();
        to_dnf(&json, &mut table)
    }

    // Evaluates a DNF expression under an assignment (bit i = variable i).
    fn eval(expr: &Expression, assignment: u32) -> bool {
        if expr.is_true() {
            return true;
        }
        expr.clauses().iter().filter(|c| !c.is_false()).any(|c| {
            (0..c.terms.len()).all(|i| {
                if c.mask[i] {
                    let bit = assignment & (1 << i) != 0;
                    bit == c.terms[i]
                } else {
                    true
                }
            })
        })
    }

    // Asserts two single-variable... generic truth-table equivalence over `vars`.
    fn assert_equiv(expr: &Expression, vars: usize, f: impl Fn(u32) -> bool) {
        for a in 0..(1u32 << vars) {
            assert_eq!(eval(expr, a), f(a), "mismatch at assignment {:0b}", a);
        }
    }

    fn dnf_shape_ok(expr: &Expression) -> bool {
        // Every clause is a flat AND of literals; nothing nested. Structurally
        // this is always true for our representation, so we assert the model
        // invariant: parallel vectors of equal length.
        expr.clauses().iter().all(|c| c.terms.len() == c.mask.len())
    }

    #[test]
    fn var_is_single_literal() {
        let e = dnf(r#"{"var":"a"}"#).unwrap();
        assert_eq!(e.to_algebraic(), "x0"); // table-less render
        assert!(dnf_shape_ok(&e));
    }

    #[test]
    fn and_or_distribute_to_dnf() {
        // a & (b | c) -> (a&b) | (a&c)
        let e = dnf(r#"{"and":[{"var":"a"},{"or":[{"var":"b"},{"var":"c"}]}]}"#).unwrap();
        assert!(dnf_shape_ok(&e));
        assert_equiv(&e, 3, |x| {
            let a = x & 1 != 0;
            let b = x & 2 != 0;
            let c = x & 4 != 0;
            a && (b || c)
        });
    }

    #[test]
    fn not_uses_de_morgan() {
        // !(a & b) -> !a | !b
        let e = dnf(r#"{"!":[{"and":[{"var":"a"},{"var":"b"}]}]}"#).unwrap();
        assert_equiv(&e, 2, |x| !((x & 1 != 0) && (x & 2 != 0)));
    }

    #[test]
    fn not_accepts_bare_form() {
        let e = dnf(r#"{"!":{"var":"a"}}"#).unwrap();
        assert_equiv(&e, 1, |x| x & 1 == 0);
    }

    #[test]
    fn xor_is_odd_parity() {
        let e = dnf(r#"{"xor":[{"var":"a"},{"var":"b"}]}"#).unwrap();
        assert_equiv(&e, 2, |x| (x & 1 != 0) ^ (x & 2 != 0));
    }

    #[test]
    fn nary_xor_is_odd_parity() {
        let e = dnf(r#"{"xor":[{"var":"a"},{"var":"b"},{"var":"c"}]}"#).unwrap();
        assert_equiv(&e, 3, |x| {
            ((x & 1 != 0) as u32 + (x & 2 != 0) as u32 + (x & 4 != 0) as u32) % 2 == 1
        });
    }

    #[test]
    fn nand_and_nor_desugar() {
        let nand = dnf(r#"{"nand":[{"var":"a"},{"var":"b"}]}"#).unwrap();
        assert_equiv(&nand, 2, |x| !((x & 1 != 0) && (x & 2 != 0)));
        let nor = dnf(r#"{"nor":[{"var":"a"},{"var":"b"}]}"#).unwrap();
        assert_equiv(&nor, 2, |x| !((x & 1 != 0) || (x & 2 != 0)));
    }

    #[test]
    fn literals_become_constants() {
        assert!(dnf("true").unwrap().is_true());
        assert!(dnf("false").unwrap().is_false());
    }

    #[test]
    fn rejects_unknown_operator() {
        assert!(dnf(r#"{">":[1,2]}"#).is_err());
    }

    #[test]
    fn rejects_multi_key_node() {
        assert!(dnf(r#"{"var":"a","or":[]}"#).is_err());
    }

    #[test]
    fn rejects_bad_arity() {
        assert!(dnf(r#"{"!":[{"var":"a"},{"var":"b"}]}"#).is_err());
        assert!(dnf(r#"{"and":[]}"#).is_err());
    }

    #[test]
    fn rejects_non_string_var() {
        assert!(dnf(r#"{"var":1}"#).is_err());
    }
}
