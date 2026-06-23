//! Converter (Library A): JsonLogic boolean expression -> DNF [`Expression`].
//!
//! [`to_dnf`] is a recursive descent over a JsonLogic tree that reuses the boolean algebra on [`Expression`]: `|` concatenates
//! product terms, `&` distributes (product of sums -> sum of products), and [`Expression::inverse`] applies De Morgan. Because
//! every operator collapses its operands into DNF as the tree is walked, the result is always in DNF.
//!
//! Accepted operators: the standard JsonLogic `var`, `!`, `and`, `or`, plus the boolean literals `true` / `false`, plus the
//! non-standard Beast extensions `xor`, `nand`, `nor` (input-only; desugared here and never emitted on output). Every other
//! operator is rejected.

use crate::json::Json;
use crate::variable_table::VariableTable;
use crate::{Expression, ProductTerm};

/// Parses a JsonLogic boolean expression into a DNF [`Expression`], registering variables in `table`.
///
/// # Errors
///
/// Returns `Err` with a human-readable message when the tree is not a valid boolean expression:
/// - a node object that does not have exactly one operator key,
/// - an unsupported operator (any non-boolean JsonLogic operator),
/// - an operator applied with the wrong arity (`!` with other than one operand, or empty `and` / `or` / `xor`),
/// - a `var` whose value is not a string, or a bare array / number / string / null where a boolean expression was expected,
/// - more variables than [`quine_mccluskey::MAX_VARIABLES`] (surfaced from `table`).
///
/// # Examples
///
/// ```
/// use beast::converter::to_dnf;
/// use beast::json::Json;
/// use beast::variable_table::VariableTable;
///
/// // a & (b | c) distributes into DNF: (a & b) | (a & c).
/// let json = Json::parse(r#"{"and":[{"var":"a"},{"or":[{"var":"b"},{"var":"c"}]}]}"#).unwrap();
/// let mut table = VariableTable::new();
/// let dnf = to_dnf(&json, &mut table).unwrap();
/// assert_eq!(dnf.product_terms().len(), 2);
///
/// // Unsupported operators are rejected.
/// let bad = Json::parse(r#"{"+":[{"var":"a"},{"var":"b"}]}"#).unwrap();
/// assert!(to_dnf(&bad, &mut VariableTable::new()).is_err());
/// ```
pub fn to_dnf(json: &Json, table: &mut VariableTable) -> Result<Expression, String> {
    match json {
        Json::Bool(true) => Ok(Expression::r#true()),
        Json::Bool(false) => Ok(Expression::r#false()),
        Json::Object(pairs) => {
            if pairs.len() != 1 {
                return Err(format!("each node must have exactly one operator key, found {}", pairs.len()));
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
        Json::Array(_) | Json::Null | Json::Number(_) | Json::String(_) => {
            Err("expected a boolean expression: an operator object or a boolean literal".to_string())
        }
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
    Ok(Expression::new(vec![ProductTerm { terms, mask }]))
}

/// Returns the operands of an n-ary operator. A JSON array gives its elements; a bare value is treated as a single operand.
fn args(value: &Json) -> Vec<&Json> {
    match value {
        Json::Array(items) => items.iter().collect(),
        other => vec![other],
    }
}

/// Returns the single operand of a unary operator (`!`), accepting both the array form `{"!": [x]}` and the bare form `{"!": x}`.
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

/// Folds operands with XOR (odd-parity), desugared pairwise into `and`/`or`/`!`: `xor(p, q) = (p & !q) | (!p & q)`.
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
        expr.product_terms().iter().filter(|c| !c.is_false()).any(|c| {
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
        // Every product term is a flat AND of literals; nothing nested. Structurally this is always true for our representation, so
        // we assert the model invariant: parallel vectors of equal length.
        expr.product_terms().iter().all(|c| c.terms.len() == c.mask.len())
    }

    #[test]
    fn var_is_single_literal() {
        let e = dnf(r#"{"var":"a"}"#).unwrap();
        assert_eq!(e.to_algebraic(), "$x0"); // table-less render of synthesized `x0`
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

    #[test]
    fn rejects_empty_object_node() {
        // Zero operator keys is not a valid node.
        assert!(dnf(r#"{}"#).is_err());
    }

    #[test]
    fn rejects_bare_non_boolean_values() {
        assert!(dnf("1").is_err());
        assert!(dnf(r#""a""#).is_err());
        assert!(dnf("null").is_err());
        assert!(dnf("[1,2]").is_err());
    }

    #[test]
    fn double_negation_cancels() {
        // !!a == a
        let e = dnf(r#"{"!":[{"!":[{"var":"a"}]}]}"#).unwrap();
        assert_equiv(&e, 1, |x| x & 1 != 0);
    }

    #[test]
    fn single_operand_or_and_are_identity() {
        let or = dnf(r#"{"or":[{"var":"a"}]}"#).unwrap();
        assert_equiv(&or, 1, |x| x & 1 != 0);
        let and = dnf(r#"{"and":[{"var":"a"}]}"#).unwrap();
        assert_equiv(&and, 1, |x| x & 1 != 0);
    }

    #[test]
    fn and_accepts_bare_single_operand() {
        // `args` treats a non-array value as a single operand.
        let e = dnf(r#"{"and":{"var":"a"}}"#).unwrap();
        assert_equiv(&e, 1, |x| x & 1 != 0);
    }

    #[test]
    fn nor_of_three_is_all_false() {
        // nor(a,b,c) is true only when a, b, c are all false.
        let e = dnf(r#"{"nor":[{"var":"a"},{"var":"b"},{"var":"c"}]}"#).unwrap();
        assert_equiv(&e, 3, |x| x == 0);
    }

    #[test]
    fn xor_single_operand_is_identity() {
        let e = dnf(r#"{"xor":[{"var":"a"}]}"#).unwrap();
        assert_equiv(&e, 1, |x| x & 1 != 0);
    }
}
