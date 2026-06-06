//! Beast: a boolean expression simplifier.
//!
//! This uses the Quine-McCluskey algorithm to simplify a boolean expression in
//! disjunctive normal form (DNF). The architecture is two libraries:
//!
//! - a **converter** (to be written) that turns an arbitrary boolean expression
//!   into an `Expression` in DNF, and
//! - a **simplifier** (the [`quine_mccluskey`] crate) that minimizes a DNF
//!   expression.
//!
//! The crate is wrapped by a thin CLI (`src/main.rs`). The headline feature is
//! not finished yet: [`simplify`] / [`simplify_json`] are stubs and the
//! Quine-McCluskey `minimize` lacks prime-implicant selection. See `plan.md`
//! for the completion roadmap.

pub mod json;

// Re-export the simplifier crate so dependents can reach it as `beast::quine_mccluskey`.
pub use quine_mccluskey;

use json::Json;
use std::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign};

/// A conjunctive clause: an AND of literals.
///
/// Literals are stored in two parallel vectors indexed by variable bit index:
/// `terms[i]` is the literal's sign (true = unnegated, false = negated) and
/// `mask[i]` is whether variable `i` is present in the clause.
///
/// Convention: **a clause with no terms represents the value `false`.**
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Clause {
    /// Unnegated (true) or negated (false).
    pub terms: Vec<bool>,
    /// Present (true) or not (false).
    pub mask: Vec<bool>,
}

impl Clause {
    /// Ands this clause with another clause, in place.
    pub fn and_assign(&mut self, rhs: &Clause) {
        // Work against the shorter of the two so we only iterate its terms.
        let mut shorter = rhs.clone();
        if shorter.terms.len() > self.terms.len() {
            std::mem::swap(&mut self.terms, &mut shorter.terms);
            std::mem::swap(&mut self.mask, &mut shorter.mask);
        }

        for i in 0..shorter.terms.len() {
            // If the rhs clause doesn't have this term, then ignore it.
            if !shorter.mask[i] {
                continue;
            }

            // If this clause does not have this term, then add it. Otherwise, if
            // both clauses have it, the signs must agree.
            if !self.mask[i] {
                self.terms[i] = shorter.terms[i];
                self.mask[i] = true;
            } else if shorter.terms[i] != self.terms[i] {
                // Conflicting signs make the clause false.
                self.terms.clear();
                self.mask.clear();
                break;
            }
        }
    }

    /// Returns a JSON object representing this clause.
    pub fn to_json(&self) -> Json {
        let mut a: Vec<Json> = Vec::new();
        for i in 0..self.terms.len() {
            if self.mask[i] {
                let term = format!("x{}", i);
                if self.terms[i] {
                    a.push(Json::String(term));
                } else {
                    a.push(Json::Array(vec![
                        Json::String("not".to_string()),
                        Json::String(term),
                    ]));
                }
            }
        }
        Json::Array(vec![Json::String("and".to_string()), Json::Array(a)])
    }

    /// Returns a string representing this clause in an algebraic format.
    pub fn to_algebraic(&self) -> String {
        let mut a = String::new();
        for i in 0..self.terms.len() {
            if self.mask[i] {
                let term = format!("x{}", i);
                if !a.is_empty() {
                    a.push_str(" & ");
                }
                if !self.terms[i] {
                    a.push('!');
                }
                a.push_str(&term);
            }
        }
        a
    }
}

/// Returns one clause anded with another clause.
///
/// Note: a clause with no terms represents the value false.
impl BitAnd for Clause {
    type Output = Clause;
    fn bitand(mut self, rhs: Clause) -> Clause {
        self.and_assign(&rhs);
        self
    }
}

/// Returns an expression oring one clause with another clause.
///
/// Note: a clause with no terms represents the value false.
impl BitOr for Clause {
    type Output = Expression;
    fn bitor(self, rhs: Clause) -> Expression {
        Expression {
            clauses: vec![self, rhs],
        }
    }
}

/// A boolean expression in disjunctive normal form: an OR of [`Clause`]s.
///
/// Convention: an expression with no clauses represents the value `false`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Expression {
    clauses: Vec<Clause>,
}

impl Expression {
    /// Constructs an expression from its clauses.
    pub fn new(clauses: Vec<Clause>) -> Self {
        Expression { clauses }
    }

    /// Returns the clauses that make up this expression.
    pub fn clauses(&self) -> &[Clause] {
        &self.clauses
    }

    /// Returns a JSON object representing the expression.
    pub fn to_json(&self) -> Json {
        let a: Vec<Json> = self.clauses.iter().map(|c| c.to_json()).collect();
        Json::Array(vec![Json::String("or".to_string()), Json::Array(a)])
    }

    /// Returns a string representing the expression in an algebraic format.
    pub fn to_algebraic(&self) -> String {
        let mut a = String::new();
        for c in &self.clauses {
            if !c.terms.is_empty() {
                if !a.is_empty() {
                    a.push_str(" + ");
                }
                a.push_str(&c.to_algebraic());
            }
        }
        a
    }

    /// Returns the inverse of this expression (De Morgan's laws), distributed
    /// back into DNF.
    ///
    /// KNOWN BUG (plan.md §4, BUG-10): the fold starts from an empty (FALSE)
    /// accumulator instead of the TRUE identity, so this currently returns
    /// FALSE for every input. The fix depends on settling the constant
    /// representation (plan task B3); the converter (Phase C) requires a
    /// correct `inverse` before it can rely on it.
    pub fn inverse(&self) -> Expression {
        // De Morgan's Laws:
        //      !(x | y | ...) = !x & !y & ...
        //      !(x & y & ...) = !x | !y | ...
        let mut inverse = Expression::default();
        for x in &self.clauses {
            // The inverse of a clause is itself an expression.
            let mut not_x = Expression::default();
            for i in 0..x.terms.len() {
                not_x.clauses.push(Clause {
                    terms: vec![!x.terms[i] && x.mask[i]],
                    mask: vec![x.mask[i]],
                });
            }
            inverse &= &not_x;
        }
        inverse
    }
}

/// Ors this expression with another (concatenates clauses).
impl BitOrAssign<&Expression> for Expression {
    fn bitor_assign(&mut self, rhs: &Expression) {
        self.clauses.extend(rhs.clauses.iter().cloned());
    }
}

impl BitOr<&Expression> for Expression {
    type Output = Expression;
    fn bitor(mut self, rhs: &Expression) -> Expression {
        self |= rhs;
        self
    }
}

/// Ands this expression with another (distributes: product of sums -> sum of
/// products), dropping any clause that reduces to false.
impl BitAndAssign<&Expression> for Expression {
    fn bitand_assign(&mut self, rhs: &Expression) {
        let mut product: Vec<Clause> = Vec::new();
        for x in &self.clauses {
            for y in &rhs.clauses {
                let c = x.clone() & y.clone();
                if !c.terms.is_empty() {
                    product.push(c);
                }
            }
        }
        self.clauses = product;
    }
}

impl BitAnd<&Expression> for Expression {
    type Output = Expression;
    fn bitand(mut self, rhs: &Expression) -> Expression {
        self &= rhs;
        self
    }
}

/// Returns a simplified expression from a JSON expression in DNF.
///
/// STUB: this currently returns an empty expression. The converter
/// (JsonLogic -> DNF) is Phase C in `plan.md`.
pub fn simplify_json(_json: &Json) -> Expression {
    Expression::default()
}

/// Returns a simplified expression.
///
/// STUB: this currently returns an empty expression. Quine-McCluskey
/// selection is Phase D in `plan.md`.
pub fn simplify(_x: &Expression) -> Expression {
    Expression::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: a single-literal clause over `width` variables.
    fn literal(index: usize, width: usize, positive: bool) -> Clause {
        let mut terms = vec![false; width];
        let mut mask = vec![false; width];
        terms[index] = positive;
        mask[index] = true;
        Clause { terms, mask }
    }

    #[test]
    fn clause_and_combines_literals() {
        let mut a = literal(0, 2, true);
        a.and_assign(&literal(1, 2, true));
        assert_eq!(a.to_algebraic(), "x0 & x1");
    }

    #[test]
    fn clause_and_conflict_is_false() {
        let mut a = literal(0, 2, true);
        a.and_assign(&literal(0, 2, false)); // x0 & !x0
        assert!(a.terms.is_empty());
    }

    #[test]
    fn expression_or_concatenates() {
        let e = Expression::new(vec![literal(0, 1, true)]);
        let f = Expression::new(vec![literal(0, 1, false)]);
        let g = e | &f;
        assert_eq!(g.clauses().len(), 2);
    }

    #[test]
    fn expression_and_distributes() {
        // (x0) & (x1) -> x0 & x1
        let e = Expression::new(vec![literal(0, 2, true)]);
        let f = Expression::new(vec![literal(1, 2, true)]);
        let g = e & &f;
        assert_eq!(g.to_algebraic(), "x0 & x1");
    }

    #[test]
    fn serializers_describe_same_clause() {
        let mut a = literal(0, 2, true);
        a.and_assign(&literal(1, 2, false));
        assert_eq!(a.to_algebraic(), "x0 & !x1");
        assert_eq!(a.to_json().to_string(), r#"["and",["x0",["not","x1"]]]"#);
    }

    #[test]
    fn inverse_returns_false_known_bug_10() {
        // BUG-10 (plan.md §4): `inverse` folds from a FALSE accumulator, so it
        // returns an empty (FALSE) expression for every input. This pins the
        // current behavior; update it when BUG-10 is fixed.
        let e = Expression::new(vec![literal(0, 1, true)]); // x0
        assert!(e.inverse().clauses().is_empty());
    }

    #[test]
    fn empty_expression_serializes_to_empty_or() {
        // The stub produces this output for any input.
        let e = Expression::default();
        assert_eq!(e.to_json().to_string(), r#"["or",[]]"#);
    }
}
