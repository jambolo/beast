//! Beast: a boolean expression simplifier.
//!
//! This uses the Quine-McCluskey algorithm to simplify a boolean expression in
//! disjunctive normal form (DNF). The architecture is two libraries:
//!
//! - a **converter** ([`converter`]) that turns an arbitrary JsonLogic boolean
//!   expression into an [`Expression`] in DNF, and
//! - a **simplifier** (the [`quine_mccluskey`] crate) that minimizes a DNF
//!   expression.
//!
//! The crate is wrapped by a thin CLI (`src/main.rs`). Data flows
//! JsonLogic -> DNF -> minimized DNF -> JsonLogic.
//!
//! # Constant representation
//!
//! Constants are encoded in the DNF data model as follows (see the [`Clause`]
//! and [`Expression`] docs):
//!
//! - **FALSE** is an [`Expression`] with no clauses (the empty disjunction), or
//!   equivalently a [`Clause`] with empty `terms` (the conflict sentinel).
//! - **TRUE** is an [`Expression`] containing an *empty conjunction*: a
//!   [`Clause`] with non-empty `terms` but no literal present (all `mask` bits
//!   false). An AND of zero literals is `true`, and `true | anything == true`.

pub mod converter;
pub mod json;
pub mod variable_table;

// Re-export the simplifier crate so dependents can reach it as `beast::quine_mccluskey`.
pub use quine_mccluskey;

use std::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign};
use std::rc::Rc;

use json::Json;
use quine_mccluskey::{minimize, Implicant, Term};
use variable_table::VariableTable;

/// A conjunctive clause: an AND of literals.
///
/// Literals are stored in two parallel vectors indexed by variable bit index:
/// `terms[i]` is the literal's sign (true = unnegated, false = negated) and
/// `mask[i]` is whether variable `i` is present in the clause.
///
/// Conventions:
/// - **A clause with empty `terms` represents the value `false`** (the sentinel
///   produced when a clause contains contradictory literals such as `x & !x`).
/// - **A clause with non-empty `terms` but no `mask` bit set represents `true`**
///   (an empty conjunction).
///
/// # Examples
///
/// ```
/// use beast::Clause;
///
/// // The literal `x0` (present, unnegated) over one variable.
/// let x0 = Clause { terms: vec![true], mask: vec![true] };
/// assert!(!x0.is_false());
/// assert!(!x0.is_true());
///
/// // The `false` sentinel has empty `terms`.
/// assert!(Clause { terms: vec![], mask: vec![] }.is_false());
///
/// // An empty conjunction (non-empty `terms`, no `mask` bit set) is `true`.
/// assert!(Clause { terms: vec![false], mask: vec![false] }.is_true());
/// ```
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Clause {
    /// Unnegated (true) or negated (false).
    pub terms: Vec<bool>,
    /// Present (true) or not (false).
    pub mask: Vec<bool>,
}

impl Clause {
    /// Returns `true` if this clause is the constant `false` sentinel.
    pub fn is_false(&self) -> bool {
        self.terms.is_empty()
    }

    /// Returns `true` if this clause is the empty conjunction (constant `true`).
    pub fn is_true(&self) -> bool {
        !self.terms.is_empty() && self.mask.iter().all(|&m| !m)
    }

    /// Ands this clause with another clause, in place.
    ///
    /// Combining literals with conflicting signs (`x & !x`) collapses the clause
    /// to the `false` sentinel (empty `terms`/`mask`).
    ///
    /// # Examples
    ///
    /// ```
    /// use beast::Clause;
    ///
    /// // x0 & x1 keeps both literals.
    /// let mut c = Clause { terms: vec![true, false], mask: vec![true, false] };
    /// c.and_assign(&Clause { terms: vec![false, true], mask: vec![false, true] });
    /// assert_eq!(c.mask, vec![true, true]);
    ///
    /// // x0 & !x0 is a contradiction: the clause becomes `false`.
    /// let mut c = Clause { terms: vec![true], mask: vec![true] };
    /// c.and_assign(&Clause { terms: vec![false], mask: vec![true] });
    /// assert!(c.is_false());
    /// ```
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

    /// Returns the JsonLogic representation of this clause's literals.
    ///
    /// A single-literal clause is emitted as the bare literal; a multi-literal
    /// clause is wrapped in `{"and": [...]}`. Callers must handle the empty
    /// conjunction (constant `true`) themselves — this method assumes at least
    /// one present literal.
    fn to_json(&self, table: &VariableTable) -> Json {
        let mut literals: Vec<Json> = Vec::new();
        for i in 0..self.terms.len() {
            if self.mask[i] {
                literals.push(literal_to_json(table, i, self.terms[i]));
            }
        }
        if literals.len() == 1 {
            literals.pop().unwrap()
        } else {
            Json::Object(vec![("and".to_string(), Json::Array(literals))])
        }
    }

    /// Returns a string representing this clause in an algebraic format.
    fn to_algebraic(&self, table: &VariableTable) -> String {
        let mut a = String::new();
        for i in 0..self.terms.len() {
            if self.mask[i] {
                if !a.is_empty() {
                    a.push_str(" & ");
                }
                if !self.terms[i] {
                    a.push('!');
                }
                a.push_str(&variable_name(table, i));
            }
        }
        a
    }
}

/// Returns the JsonLogic for a single literal: `{"var": name}` if positive, or
/// `{"!": [{"var": name}]}` if negated.
fn literal_to_json(table: &VariableTable, index: usize, positive: bool) -> Json {
    let var = Json::Object(vec![(
        "var".to_string(),
        Json::String(variable_name(table, index)),
    )]);
    if positive {
        var
    } else {
        Json::Object(vec![("!".to_string(), Json::Array(vec![var]))])
    }
}

/// Resolves a bit index to its variable name, falling back to a synthesized
/// `x{index}` when the table has no entry (e.g. for table-less expressions built
/// directly in tests).
fn variable_name(table: &VariableTable, index: usize) -> String {
    if index < table.len() {
        table.name_of(index).to_string()
    } else {
        format!("x{}", index)
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
            table: Rc::default(),
        }
    }
}

/// A boolean expression in disjunctive normal form: an OR of [`Clause`]s.
///
/// An expression with no clauses represents the value `false`. An expression
/// containing an empty-conjunction clause represents `true` (see the
/// crate-level docs).
///
/// The expression carries a shared [`VariableTable`] so the serializers
/// ([`to_json`](Expression::to_json) / [`to_algebraic`](Expression::to_algebraic))
/// can restore the original variable names without taking extra arguments.
///
/// # Examples
///
/// ```
/// use beast::{Clause, Expression};
///
/// // x0 | x1 over two variables: an OR of two single-literal clauses.
/// let x0 = Clause { terms: vec![true, false], mask: vec![true, false] };
/// let x1 = Clause { terms: vec![false, true], mask: vec![false, true] };
/// let e = Expression::new(vec![x0, x1]);
/// assert_eq!(e.clauses().len(), 2);
/// assert!(!e.is_true() && !e.is_false());
/// ```
#[derive(Clone, Debug, Default)]
pub struct Expression {
    clauses: Vec<Clause>,
    table: Rc<VariableTable>,
}

/// Two expressions are equal when their clauses are equal; the variable table is
/// metadata for serialization and does not affect logical identity.
impl PartialEq for Expression {
    fn eq(&self, other: &Self) -> bool {
        self.clauses == other.clauses
    }
}

impl Expression {
    /// Constructs an expression from its clauses (with an empty variable table).
    pub fn new(clauses: Vec<Clause>) -> Self {
        Expression {
            clauses,
            table: Rc::default(),
        }
    }

    /// The constant `false` (empty disjunction).
    ///
    /// # Examples
    ///
    /// ```
    /// use beast::Expression;
    ///
    /// assert!(Expression::r#false().is_false());
    /// assert_eq!(Expression::r#false().to_json().to_string(), "false");
    /// ```
    pub fn r#false() -> Self {
        Expression::default()
    }

    /// The constant `true` (a single empty-conjunction clause).
    ///
    /// # Examples
    ///
    /// ```
    /// use beast::Expression;
    ///
    /// assert!(Expression::r#true().is_true());
    /// assert_eq!(Expression::r#true().to_json().to_string(), "true");
    /// ```
    pub fn r#true() -> Self {
        Expression {
            clauses: vec![Clause {
                terms: vec![false],
                mask: vec![false],
            }],
            table: Rc::default(),
        }
    }

    /// Returns this expression with `table` attached for serialization.
    pub fn with_table(mut self, table: Rc<VariableTable>) -> Self {
        self.table = table;
        self
    }

    /// Returns the shared variable table.
    pub fn table(&self) -> &Rc<VariableTable> {
        &self.table
    }

    /// Returns the clauses that make up this expression.
    pub fn clauses(&self) -> &[Clause] {
        &self.clauses
    }

    /// Returns `true` if this expression is the constant `true` (it contains an
    /// empty-conjunction clause).
    pub fn is_true(&self) -> bool {
        self.clauses.iter().any(|c| c.is_true())
    }

    /// Returns `true` if this expression is the constant `false` (every clause,
    /// if any, is the `false` sentinel).
    pub fn is_false(&self) -> bool {
        self.clauses.iter().all(|c| c.is_false())
    }

    /// Returns the JsonLogic representation of the expression.
    ///
    /// Constants serialize to the JSON booleans `true` / `false`. Otherwise the
    /// `false` sentinel clauses are dropped; a single remaining clause is
    /// emitted directly and multiple clauses are wrapped in `{"or": [...]}`.
    ///
    /// # Examples
    ///
    /// ```
    /// use beast::{Clause, Expression};
    ///
    /// // A single-literal expression emits the bare literal (no `or` wrapper).
    /// let x0 = Clause { terms: vec![true], mask: vec![true] };
    /// let e = Expression::new(vec![x0]);
    /// assert_eq!(e.to_json().to_string(), r#"{"var":"x0"}"#);
    /// ```
    pub fn to_json(&self) -> Json {
        if self.is_true() {
            return Json::Bool(true);
        }
        if self.is_false() {
            return Json::Bool(false);
        }
        let mut parts: Vec<Json> = self
            .clauses
            .iter()
            .filter(|c| !c.is_false())
            .map(|c| c.to_json(&self.table))
            .collect();
        if parts.len() == 1 {
            parts.pop().unwrap()
        } else {
            Json::Object(vec![("or".to_string(), Json::Array(parts))])
        }
    }

    /// Returns a string representing the expression in an algebraic format.
    ///
    /// Kept consistent with [`to_json`](Expression::to_json): constants render as
    /// `true` / `false` and `false` sentinel clauses are dropped.
    pub fn to_algebraic(&self) -> String {
        if self.is_true() {
            return "true".to_string();
        }
        if self.is_false() {
            return "false".to_string();
        }
        let mut a = String::new();
        for c in &self.clauses {
            if c.is_false() {
                continue;
            }
            if !a.is_empty() {
                a.push_str(" + ");
            }
            a.push_str(&c.to_algebraic(&self.table));
        }
        a
    }

    /// Returns the inverse of this expression (De Morgan's laws), distributed
    /// back into DNF.
    ///
    /// `!(c1 | c2 | ...) = !c1 & !c2 & ...`, where the inverse of a clause is the
    /// OR of its negated literals. The fold starts from the TRUE identity, so
    /// `!false == true` and `!true == false`.
    ///
    /// # Examples
    ///
    /// ```
    /// use beast::{Clause, Expression};
    ///
    /// // !(x0) == !x0
    /// let x0 = Clause { terms: vec![true], mask: vec![true] };
    /// let e = Expression::new(vec![x0]);
    /// assert_eq!(e.inverse().to_json().to_string(), r#"{"!":[{"var":"x0"}]}"#);
    ///
    /// // The constants invert into each other.
    /// assert!(Expression::r#true().inverse().is_false());
    /// assert!(Expression::r#false().inverse().is_true());
    /// ```
    pub fn inverse(&self) -> Expression {
        let mut acc = Expression::r#true();
        for clause in &self.clauses {
            // A `false` sentinel clause contributes nothing to the disjunction,
            // so it contributes nothing to the inverse either.
            if clause.is_false() {
                continue;
            }
            // The inverse of a clause is an OR (expression) of its negated
            // literals. An empty conjunction (`true`) inverts to `false`.
            let mut not_clause = Expression::r#false();
            for i in 0..clause.terms.len() {
                if clause.mask[i] {
                    let mut terms = vec![false; clause.terms.len()];
                    let mut mask = vec![false; clause.terms.len()];
                    terms[i] = !clause.terms[i];
                    mask[i] = true;
                    not_clause.clauses.push(Clause { terms, mask });
                }
            }
            acc &= &not_clause;
        }
        acc.with_table(self.table.clone())
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
                if !c.is_false() {
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

/// Returns the ON-set minterms of a DNF `Expression` over `num_vars` variables.
///
/// Each clause is expanded over the variables it leaves free (every position in
/// `0..num_vars` it does not constrain), and the resulting minterms are unioned.
fn expression_to_minterms(expr: &Expression, num_vars: usize) -> Vec<Term> {
    let mut seen = vec![false; 1usize << num_vars];
    for clause in &expr.clauses {
        if clause.is_false() {
            continue;
        }
        // Fixed bits from the clause's present literals; everything else is free.
        let mut base: Term = 0;
        let mut free: Vec<usize> = Vec::new();
        for i in 0..num_vars {
            if i < clause.mask.len() && clause.mask[i] {
                if clause.terms[i] {
                    base |= 1 << i;
                }
            } else {
                free.push(i);
            }
        }
        // Enumerate every assignment of the free variables.
        for combo in 0..(1usize << free.len()) {
            let mut m = base;
            for (bit, &pos) in free.iter().enumerate() {
                if combo & (1 << bit) != 0 {
                    m |= 1 << pos;
                }
            }
            seen[m as usize] = true;
        }
    }
    (0..(1u32 << num_vars))
        .filter(|&m| seen[m as usize])
        .collect()
}

/// Converts a selected `Implicant` to a [`Clause`] over `num_vars` variables.
///
/// Bit `i` becomes a literal iff it is a care bit (`d` bit clear); its sign comes
/// from `v`. Don't-care bits are left absent from the clause.
fn implicant_to_clause(imp: &Implicant, num_vars: usize) -> Clause {
    let mut terms = vec![false; num_vars];
    let mut mask = vec![false; num_vars];
    for i in 0..num_vars {
        if imp.d & (1 << i) == 0 {
            mask[i] = true;
            terms[i] = imp.v & (1 << i) != 0;
        }
    }
    Clause { terms, mask }
}

/// Returns a simplified expression from a JsonLogic expression.
///
/// Converts the input to DNF (building a [`VariableTable`]), then minimizes it.
///
/// # Errors
///
/// Returns `Err` with a human-readable message when the input is not a valid
/// boolean expression: a node with anything other than exactly one operator key,
/// an unsupported operator, an operator with the wrong arity, a non-string
/// `var` name, or more than [`quine_mccluskey::MAX_VARIABLES`] distinct
/// variables.
///
/// # Examples
///
/// ```
/// use beast::{json::Json, simplify_json};
///
/// // (a & b) | (a & !b) simplifies to just `a`.
/// let json = Json::parse(
///     r#"{"or":[{"and":[{"var":"a"},{"var":"b"}]},{"and":[{"var":"a"},{"!":[{"var":"b"}]}]}]}"#,
/// )
/// .unwrap();
/// assert_eq!(simplify_json(&json).unwrap().to_json().to_string(), r#"{"var":"a"}"#);
///
/// // Comparison operators are not boolean connectives and are rejected.
/// let bad = Json::parse(r#"{">":[{"var":"a"},{"var":"b"}]}"#).unwrap();
/// assert!(simplify_json(&bad).is_err());
/// ```
pub fn simplify_json(json: &Json) -> Result<Expression, String> {
    let mut table = VariableTable::new();
    let dnf = converter::to_dnf(json, &mut table)?;
    let dnf = dnf.with_table(Rc::new(table));
    Ok(simplify(&dnf))
}

/// Returns the minimal-DNF form of `x` via Quine-McCluskey.
///
/// The number of variables is taken from the attached [`VariableTable`].
/// Constants are handled directly: a tautology returns the constant `true` and
/// an unsatisfiable expression returns the constant `false`.
///
/// # Examples
///
/// ```
/// use beast::{json::Json, simplify, simplify_json};
///
/// // `simplify` operates on a DNF `Expression`; build one via the converter.
/// let dnf = simplify_json(
///     &Json::parse(r#"{"or":[{"var":"a"},{"var":"a"}]}"#).unwrap(),
/// )
/// .unwrap();
/// // Redundant disjuncts collapse to a single literal.
/// assert_eq!(simplify(&dnf).to_json().to_string(), r#"{"var":"a"}"#);
/// ```
pub fn simplify(x: &Expression) -> Expression {
    let table = x.table.clone();
    if x.is_true() {
        return Expression::r#true().with_table(table);
    }
    if x.is_false() {
        return Expression::r#false().with_table(table);
    }

    let num_vars = table.len();
    // A non-constant expression must reference at least one variable.
    debug_assert!(num_vars > 0);

    let minterms = expression_to_minterms(x, num_vars);
    if minterms.is_empty() {
        return Expression::r#false().with_table(table);
    }
    if minterms.len() == (1usize << num_vars) {
        return Expression::r#true().with_table(table);
    }

    let implicants = minimize(&minterms, &[], num_vars);
    let clauses = implicants
        .iter()
        .map(|imp| implicant_to_clause(imp, num_vars))
        .collect();
    Expression { clauses, table }
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
        assert_eq!(a.to_algebraic(&VariableTable::new()), "x0 & x1");
    }

    #[test]
    fn clause_and_conflict_is_false() {
        let mut a = literal(0, 2, true);
        a.and_assign(&literal(0, 2, false)); // x0 & !x0
        assert!(a.is_false());
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
        let e = Expression::new(vec![{
            let mut a = literal(0, 2, true);
            a.and_assign(&literal(1, 2, false));
            a
        }]);
        assert_eq!(e.to_algebraic(), "x0 & !x1");
        assert_eq!(
            e.to_json().to_string(),
            r#"{"and":[{"var":"x0"},{"!":[{"var":"x1"}]}]}"#
        );
    }

    #[test]
    fn single_literal_clause_emits_bare_literal() {
        let e = Expression::new(vec![literal(0, 1, true)]);
        assert_eq!(e.to_json().to_string(), r#"{"var":"x0"}"#);
    }

    #[test]
    fn constants_serialize_to_json_booleans() {
        assert_eq!(Expression::r#true().to_json().to_string(), "true");
        assert_eq!(Expression::r#false().to_json().to_string(), "false");
        assert_eq!(Expression::r#true().to_algebraic(), "true");
        assert_eq!(Expression::r#false().to_algebraic(), "false");
    }

    #[test]
    fn inverse_obeys_de_morgan() {
        // !(x0) == !x0
        let e = Expression::new(vec![literal(0, 1, true)]);
        assert_eq!(e.inverse().to_algebraic(), "!x0");
        // !true == false, !false == true
        assert!(Expression::r#true().inverse().is_false());
        assert!(Expression::r#false().inverse().is_true());
    }

    #[test]
    fn minterm_round_trip_preserves_truth_table() {
        // (x0 & x1) | (x0 & !x1) over 2 vars -> x0 true -> minterms {1, 3}.
        let mut c0 = literal(0, 2, true);
        c0.and_assign(&literal(1, 2, true));
        let mut c1 = literal(0, 2, true);
        c1.and_assign(&literal(1, 2, false));
        let e = Expression::new(vec![c0, c1]);
        let minterms = expression_to_minterms(&e, 2);
        assert_eq!(minterms, vec![1, 3]);
    }

    #[test]
    fn and_assign_swaps_to_widen_shorter_clause() {
        // The lhs is narrower than the rhs: the wider operand must survive.
        let mut narrow = literal(0, 1, true); // width 1
        narrow.and_assign(&literal(2, 3, true)); // width 3
        assert_eq!(narrow.to_algebraic(&VariableTable::new()), "x0 & x2");
    }

    #[test]
    fn default_expression_is_false() {
        let e = Expression::default();
        assert!(e.is_false());
        assert!(!e.is_true());
        assert_eq!(e.clauses().len(), 0);
    }

    #[test]
    fn empty_conjunction_clause_is_true_at_expression_level() {
        // A TRUE clause anywhere makes the whole disjunction TRUE.
        let e = Expression::new(vec![
            literal(0, 1, true),
            Clause {
                terms: vec![false],
                mask: vec![false],
            },
        ]);
        assert!(e.is_true());
        assert_eq!(e.to_json().to_string(), "true");
    }

    #[test]
    fn with_table_round_trips_names() {
        let mut table = VariableTable::new();
        table.index_of("rain").unwrap();
        let e = Expression::new(vec![literal(0, 1, true)]).with_table(Rc::new(table));
        assert_eq!(e.to_algebraic(), "rain");
        assert_eq!(e.to_json().to_string(), r#"{"var":"rain"}"#);
    }

    #[test]
    fn to_json_drops_false_sentinel_clauses() {
        // A real literal ORed with a FALSE sentinel keeps only the literal.
        let e = Expression::new(vec![
            literal(0, 1, true),
            Clause {
                terms: vec![],
                mask: vec![],
            },
        ]);
        assert_eq!(e.to_json().to_string(), r#"{"var":"x0"}"#);
    }

    #[test]
    fn multi_clause_expression_wraps_in_or() {
        let e = Expression::new(vec![literal(0, 2, true), literal(1, 2, false)]);
        assert_eq!(
            e.to_json().to_string(),
            r#"{"or":[{"var":"x0"},{"!":[{"var":"x1"}]}]}"#
        );
    }

    #[test]
    fn implicant_to_clause_maps_care_bits_only() {
        // Implicant "x1" over 2 vars: bit 1 fixed to 1, bit 0 don't-care.
        let imp = Implicant { v: 0b10, d: 0b01 };
        let c = implicant_to_clause(&imp, 2);
        assert_eq!(c.mask, vec![false, true]);
        assert!(c.terms[1]);
    }

    #[test]
    fn expression_equality_ignores_table() {
        // Logical identity is clause equality; the table is serialization
        // metadata only.
        let a = Expression::new(vec![literal(0, 1, true)]);
        let mut table = VariableTable::new();
        table.index_of("a").unwrap();
        let b = Expression::new(vec![literal(0, 1, true)]).with_table(Rc::new(table));
        assert_eq!(a, b);
    }

    #[test]
    fn simplify_collapses_redundant_clauses() {
        // x0 | x0 -> x0 (table with one variable so num_vars == 1).
        let mut table = VariableTable::new();
        table.index_of("a").unwrap();
        let e = Expression::new(vec![literal(0, 1, true), literal(0, 1, true)])
            .with_table(Rc::new(table));
        let s = simplify(&e);
        assert_eq!(s.to_algebraic(), "a");
    }
}
