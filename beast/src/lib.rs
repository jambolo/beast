#![doc = include_str!("../../README.md")]
//!
//! ---
//!
//! # Implementation notes
//!
//! This uses the Quine-McCluskey algorithm to simplify a boolean expression in disjunctive normal form (DNF). The architecture is
//! two libraries:
//!
//! - a **converter** ([`converter`]) that turns an arbitrary JsonLogic boolean expression into an [`Expression`] in DNF, and
//! - a **simplifier** (the [`quine_mccluskey`] crate) that minimizes a DNF expression.
//!
//! The crate is wrapped by a thin CLI (`src/main.rs`). Data flows JsonLogic -> DNF -> minimized DNF -> JsonLogic.
//!
//! # Constant representation
//!
//! Constants are encoded in the DNF data model as follows (see the [`ProductTerm`] and [`Expression`] docs):
//!
//! - **FALSE** is an [`Expression`] with no product terms (the empty disjunction), or equivalently a [`ProductTerm`] with empty
//!   `terms` (the conflict sentinel).
//! - **TRUE** is an [`Expression`] containing an *empty conjunction*: a [`ProductTerm`] with non-empty `terms` but no literal
//!   present (all `mask` bits false). An AND of zero literals is `true`, and `true | anything == true`.

pub mod algebraic;
pub mod converter;
pub mod json;
pub mod variable_table;

// Re-export the simplifier crate so dependents can reach it as `beast::quine_mccluskey`.
pub use quine_mccluskey;

use std::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign};
use std::rc::Rc;

use json::Json;
use quine_mccluskey::{Implicant, Term, minimize};
use variable_table::VariableTable;

/// Combining overbar (U+0305): the postfix negation of a single-letter variable in the [`Common`](AlgebraicStyle::Common) style
/// (`a\u{0305}` renders `!a`).
const OVERBAR: char = '\u{0305}';

/// Selects the operators and whitespace used by [`Expression::to_algebraic_styled`].
///
/// The style affects *only* which operator glyphs are emitted and how they are spaced; the structure of the output (product term
/// order, literal order, constants `1`/`0`) is identical across styles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AlgebraicStyle {
    /// Textbook notation: `+` (spaced) for OR, juxtaposition for AND, and a combining overbar for NOT on a single-letter variable
    /// (`~` prefix for a multi-character one). Adjacent factors are separated by a space only when a multi-character name would
    /// otherwise merge into its neighbour. The default.
    #[default]
    Common,
    /// Programming notation: `|` for OR, `&` for AND (both spaced), `!` prefix for NOT. Re-parses as algebraic input.
    Code,
    /// Logic notation: `\u{2228}` for OR, `\u{2227}` for AND (both spaced), `\u{00AC}` prefix for NOT.
    Logic,
}

/// A product term: an AND of literals.
///
/// Literals are stored in two parallel vectors indexed by variable bit index: `terms[i]` is the literal's sign (true = unnegated,
/// false = negated) and `mask[i]` is whether variable `i` is present in the product term.
///
/// Conventions:
/// - **A product term with empty `terms` represents the value `false`** (the sentinel produced when a product term contains
///   contradictory literals such as `x & !x`).
/// - **A product term with non-empty `terms` but no `mask` bit set represents `true`** (an empty conjunction).
///
/// # Examples
///
/// ```
/// use beast::ProductTerm;
///
/// // The literal `x0` (present, unnegated) over one variable.
/// let x0 = ProductTerm { terms: vec![true], mask: vec![true] };
/// assert!(!x0.is_false());
/// assert!(!x0.is_true());
///
/// // The `false` sentinel has empty `terms`.
/// assert!(ProductTerm { terms: vec![], mask: vec![] }.is_false());
///
/// // An empty conjunction (non-empty `terms`, no `mask` bit set) is `true`.
/// assert!(ProductTerm { terms: vec![false], mask: vec![false] }.is_true());
/// ```
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProductTerm {
    /// Unnegated (true) or negated (false).
    pub terms: Vec<bool>,
    /// Present (true) or not (false).
    pub mask: Vec<bool>,
}

impl ProductTerm {
    /// Returns `true` if this product term is the constant `false` sentinel.
    pub fn is_false(&self) -> bool {
        self.terms.is_empty()
    }

    /// Returns `true` if this product term is the empty conjunction (constant `true`).
    pub fn is_true(&self) -> bool {
        !self.terms.is_empty() && self.mask.iter().all(|&m| !m)
    }

    /// Ands this product term with another product term, in place.
    ///
    /// Combining literals with conflicting signs (`x & !x`) collapses the product term to the `false` sentinel (empty
    /// `terms`/`mask`).
    ///
    /// # Examples
    ///
    /// ```
    /// use beast::ProductTerm;
    ///
    /// // x0 & x1 keeps both literals.
    /// let mut c = ProductTerm { terms: vec![true, false], mask: vec![true, false] };
    /// c.and_assign(&ProductTerm { terms: vec![false, true], mask: vec![false, true] });
    /// assert_eq!(c.mask, vec![true, true]);
    ///
    /// // x0 & !x0 is a contradiction: the product term becomes `false`.
    /// let mut c = ProductTerm { terms: vec![true], mask: vec![true] };
    /// c.and_assign(&ProductTerm { terms: vec![false], mask: vec![true] });
    /// assert!(c.is_false());
    /// ```
    pub fn and_assign(&mut self, rhs: &ProductTerm) {
        // Work against the shorter of the two so we only iterate its terms.
        let mut shorter = rhs.clone();
        if shorter.terms.len() > self.terms.len() {
            std::mem::swap(&mut self.terms, &mut shorter.terms);
            std::mem::swap(&mut self.mask, &mut shorter.mask);
        }

        for i in 0..shorter.terms.len() {
            // If the rhs product term doesn't have this term, then ignore it.
            if !shorter.mask[i] {
                continue;
            }

            // If this product term does not have this term, then add it. Otherwise, if both product terms have it, the signs must
            // agree.
            if !self.mask[i] {
                self.terms[i] = shorter.terms[i];
                self.mask[i] = true;
            } else if shorter.terms[i] != self.terms[i] {
                // Conflicting signs make the product term false.
                self.terms.clear();
                self.mask.clear();
                break;
            }
        }
    }

    /// Returns the JsonLogic representation of this product term's literals.
    ///
    /// A single-literal product term is emitted as the bare literal; a multi-literal product term is wrapped in `{"and": [...]}`.
    /// Callers must handle the empty conjunction (constant `true`) themselves — this method assumes at least one present literal.
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

    /// Returns a string representing this product term in the given algebraic `style`.
    ///
    /// Multi-character names are emitted as `$name` in every style so the output re-parses to the same variable; a single ASCII
    /// letter is left bare.
    fn to_algebraic(&self, table: &VariableTable, style: AlgebraicStyle) -> String {
        let mut a = String::new();
        let mut prev_multichar = false; // previous literal was a `$`-prefixed name
        for i in 0..self.terms.len() {
            if !self.mask[i] {
                continue;
            }
            let name = variable_name(table, i);
            let positive = self.terms[i];
            // A single ASCII letter is one variable on input, needing no `$` and juxtaposing unambiguously; every other name is
            // emitted as `$name`.
            let single = name.len() == 1 && name.starts_with(|c: char| c.is_ascii_alphabetic());
            let var = if single { name } else { format!("${}", name) };
            match style {
                AlgebraicStyle::Common => {
                    // AND is juxtaposition. A `$`-name greedily consumes the following identifier characters, so a multi-character
                    // name needs a separating space before an adjacent single letter; `$`, `~`, and the overbar are self-delimiting
                    // otherwise.
                    if prev_multichar && single {
                        a.push(' ');
                    }
                    if positive {
                        a.push_str(&var);
                    } else if single {
                        a.push_str(&var);
                        a.push(OVERBAR);
                    } else {
                        a.push('~');
                        a.push_str(&var);
                    }
                }
                AlgebraicStyle::Code | AlgebraicStyle::Logic => {
                    let (and_op, not_op) = match style {
                        AlgebraicStyle::Code => (" & ", "!"),
                        _ => (" \u{2227} ", "\u{00AC}"),
                    };
                    if !a.is_empty() {
                        a.push_str(and_op);
                    }
                    if !positive {
                        a.push_str(not_op);
                    }
                    a.push_str(&var);
                }
            }
            prev_multichar = !single;
        }
        a
    }
}

/// Returns the JsonLogic for a single literal: `{"var": name}` if positive, or `{"!": [{"var": name}]}` if negated.
fn literal_to_json(table: &VariableTable, index: usize, positive: bool) -> Json {
    let var = Json::Object(vec![("var".to_string(), Json::String(variable_name(table, index)))]);
    if positive {
        var
    } else {
        Json::Object(vec![("!".to_string(), Json::Array(vec![var]))])
    }
}

/// Resolves a bit index to its variable name, falling back to a synthesized `x{index}` when the table has no entry (e.g. for
/// table-less expressions built directly in tests).
fn variable_name(table: &VariableTable, index: usize) -> String {
    if index < table.len() {
        table.name_of(index).to_string()
    } else {
        format!("x{}", index)
    }
}

/// Returns one product term anded with another product term.
///
/// Note: a product term with no terms represents the value false.
impl BitAnd for ProductTerm {
    type Output = ProductTerm;
    fn bitand(mut self, rhs: ProductTerm) -> ProductTerm {
        self.and_assign(&rhs);
        self
    }
}

/// Returns an expression oring one product term with another product term.
///
/// Note: a product term with no terms represents the value false.
impl BitOr for ProductTerm {
    type Output = Expression;
    fn bitor(self, rhs: ProductTerm) -> Expression {
        Expression {
            product_terms: vec![self, rhs],
            table: Rc::default(),
        }
    }
}

/// A boolean expression in disjunctive normal form: an OR of [`ProductTerm`]s.
///
/// An expression with no product terms represents the value `false`. An expression containing an empty-conjunction product term
/// represents `true` (see the crate-level docs).
///
/// The expression carries a shared [`VariableTable`] so the serializers ([`to_json`](Expression::to_json) /
/// [`to_algebraic`](Expression::to_algebraic)) can restore the original variable names without taking extra arguments.
///
/// # Examples
///
/// ```
/// use beast::{ProductTerm, Expression};
///
/// // x0 | x1 over two variables: an OR of two single-literal product terms.
/// let x0 = ProductTerm { terms: vec![true, false], mask: vec![true, false] };
/// let x1 = ProductTerm { terms: vec![false, true], mask: vec![false, true] };
/// let e = Expression::new(vec![x0, x1]);
/// assert_eq!(e.product_terms().len(), 2);
/// assert!(!e.is_true() && !e.is_false());
/// ```
#[derive(Clone, Debug, Default)]
pub struct Expression {
    product_terms: Vec<ProductTerm>,
    table: Rc<VariableTable>,
}

/// Two expressions are equal when their product terms are equal; the variable table is metadata for serialization and does not
/// affect logical identity.
impl PartialEq for Expression {
    fn eq(&self, other: &Self) -> bool {
        self.product_terms == other.product_terms
    }
}

impl Expression {
    /// Constructs an expression from its product terms (with an empty variable table).
    pub fn new(product_terms: Vec<ProductTerm>) -> Self {
        Expression {
            product_terms,
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

    /// The constant `true` (a single empty-conjunction product term).
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
            product_terms: vec![ProductTerm {
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

    /// Returns the product terms that make up this expression.
    pub fn product_terms(&self) -> &[ProductTerm] {
        &self.product_terms
    }

    /// Returns `true` if this expression is the constant `true` (it contains an empty-conjunction product term).
    pub fn is_true(&self) -> bool {
        self.product_terms.iter().any(|c| c.is_true())
    }

    /// Returns `true` if this expression is the constant `false` (every product term, if any, is the `false` sentinel).
    pub fn is_false(&self) -> bool {
        self.product_terms.iter().all(|c| c.is_false())
    }

    /// Returns the JsonLogic representation of the expression.
    ///
    /// Constants serialize to the JSON booleans `true` / `false`. Otherwise the `false` sentinel product terms are dropped; a
    /// single remaining product term is emitted directly and multiple product terms are wrapped in `{"or": [...]}`.
    ///
    /// # Examples
    ///
    /// ```
    /// use beast::{ProductTerm, Expression};
    ///
    /// // A single-literal expression emits the bare literal (no `or` wrapper).
    /// let x0 = ProductTerm { terms: vec![true], mask: vec![true] };
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
            .product_terms
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

    /// Returns a string representing the expression in the default ([`Common`](AlgebraicStyle::Common)) algebraic style.
    ///
    /// The dual of [`algebraic::parse_algebraic`]. See [`to_algebraic_styled`](Expression::to_algebraic_styled) to pick the
    /// operator/whitespace style.
    pub fn to_algebraic(&self) -> String {
        self.to_algebraic_styled(AlgebraicStyle::default())
    }

    /// Returns a string representing the expression in the given algebraic `style` (see [`AlgebraicStyle`]).
    ///
    /// Product terms are joined by the style's OR operator, literals within a product term by its AND operator, negation by its NOT
    /// operator, and the constants render as `1` / `0` in every style. As in [`to_json`](Expression::to_json), the `false` sentinel
    /// product terms are dropped.
    pub fn to_algebraic_styled(&self, style: AlgebraicStyle) -> String {
        if self.is_true() {
            return "1".to_string();
        }
        if self.is_false() {
            return "0".to_string();
        }
        let or_op = match style {
            AlgebraicStyle::Common => " + ",
            AlgebraicStyle::Code => " | ",
            AlgebraicStyle::Logic => " \u{2228} ",
        };
        let mut a = String::new();
        for c in &self.product_terms {
            if c.is_false() {
                continue;
            }
            if !a.is_empty() {
                a.push_str(or_op);
            }
            a.push_str(&c.to_algebraic(&self.table, style));
        }
        a
    }

    /// Returns the inverse of this expression (De Morgan's laws), distributed back into DNF.
    ///
    /// `!(c1 | c2 | ...) = !c1 & !c2 & ...`, where the inverse of a product term is the OR of its negated literals. The fold starts
    /// from the TRUE identity, so `!false == true` and `!true == false`.
    ///
    /// # Examples
    ///
    /// ```
    /// use beast::{ProductTerm, Expression};
    ///
    /// // !(x0) == !x0
    /// let x0 = ProductTerm { terms: vec![true], mask: vec![true] };
    /// let e = Expression::new(vec![x0]);
    /// assert_eq!(e.inverse().to_json().to_string(), r#"{"!":[{"var":"x0"}]}"#);
    ///
    /// // The constants invert into each other.
    /// assert!(Expression::r#true().inverse().is_false());
    /// assert!(Expression::r#false().inverse().is_true());
    /// ```
    pub fn inverse(&self) -> Expression {
        let mut acc = Expression::r#true();
        for term in &self.product_terms {
            // A `false` sentinel product term contributes nothing to the disjunction, so it contributes nothing to the inverse
            // either.
            if term.is_false() {
                continue;
            }
            // The inverse of a product term is an OR (expression) of its negated literals. An empty conjunction (`true`) inverts to
            // `false`.
            let mut not_term = Expression::r#false();
            for i in 0..term.terms.len() {
                if term.mask[i] {
                    let mut terms = vec![false; term.terms.len()];
                    let mut mask = vec![false; term.terms.len()];
                    terms[i] = !term.terms[i];
                    mask[i] = true;
                    not_term.product_terms.push(ProductTerm { terms, mask });
                }
            }
            acc &= &not_term;
        }
        acc.with_table(self.table.clone())
    }
}

/// Ors this expression with another (concatenates product terms).
impl BitOrAssign<&Expression> for Expression {
    fn bitor_assign(&mut self, rhs: &Expression) {
        self.product_terms.extend(rhs.product_terms.iter().cloned());
    }
}

impl BitOr<&Expression> for Expression {
    type Output = Expression;
    fn bitor(mut self, rhs: &Expression) -> Expression {
        self |= rhs;
        self
    }
}

/// Ands this expression with another (distributes: product of sums -> sum of products), dropping any product term that reduces to
/// false.
impl BitAndAssign<&Expression> for Expression {
    fn bitand_assign(&mut self, rhs: &Expression) {
        let mut product: Vec<ProductTerm> = Vec::new();
        for x in &self.product_terms {
            for y in &rhs.product_terms {
                let c = x.clone() & y.clone();
                if !c.is_false() {
                    product.push(c);
                }
            }
        }
        self.product_terms = product;
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
/// Each product term is expanded over the variables it leaves free (every position in `0..num_vars` it does not constrain), and the
/// resulting minterms are unioned.
fn expression_to_minterms(expr: &Expression, num_vars: usize) -> Vec<Term> {
    let mut seen = vec![false; 1usize << num_vars];
    for term in &expr.product_terms {
        if term.is_false() {
            continue;
        }
        // Fixed bits from the product term's present literals; everything else is free.
        let mut base: Term = 0;
        let mut free: Vec<usize> = Vec::new();
        for i in 0..num_vars {
            if i < term.mask.len() && term.mask[i] {
                if term.terms[i] {
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
    (0..(1u32 << num_vars)).filter(|&m| seen[m as usize]).collect()
}

/// Converts a selected `Implicant` to a [`ProductTerm`] over `num_vars` variables.
///
/// Bit `i` becomes a literal iff it is a care bit (`d` bit clear); its sign comes from `v`. Don't-care bits are left absent from
/// the product term.
fn implicant_to_product_term(imp: &Implicant, num_vars: usize) -> ProductTerm {
    let mut terms = vec![false; num_vars];
    let mut mask = vec![false; num_vars];
    for i in 0..num_vars {
        if imp.d & (1 << i) == 0 {
            mask[i] = true;
            terms[i] = imp.v & (1 << i) != 0;
        }
    }
    ProductTerm { terms, mask }
}

/// Returns a simplified expression from a JsonLogic expression.
///
/// Converts the input to DNF (building a [`VariableTable`]), then minimizes it.
///
/// # Errors
///
/// Returns `Err` with a human-readable message when the input is not a valid boolean expression: a node with anything other than
/// exactly one operator key, an unsupported operator, an operator with the wrong arity, a non-string `var` name, or more than
/// [`quine_mccluskey::MAX_VARIABLES`] distinct variables.
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

/// Parses an algebraic boolean expression and returns its minimal DNF form.
///
/// The algebraic counterpart to [`simplify_json`]: it runs the [`algebraic`] parser instead of the JsonLogic [`converter`], then
/// the same Quine-McCluskey [`simplify`]. The result carries the variable table, so it can be serialized back with either
/// [`Expression::to_algebraic`] or [`Expression::to_json`].
///
/// # Errors
///
/// Returns `Err` with a human-readable message on a lexing or syntax error, an empty expression, or more than
/// [`quine_mccluskey::MAX_VARIABLES`] variables (see [`algebraic::parse_algebraic`]).
///
/// # Examples
///
/// ```
/// use beast::simplify_algebraic;
///
/// // ab + a!b simplifies to just `a`.
/// assert_eq!(simplify_algebraic("ab + a!b").unwrap().to_algebraic(), "a");
/// ```
pub fn simplify_algebraic(input: &str) -> Result<Expression, String> {
    let mut table = VariableTable::new();
    let dnf = algebraic::parse_algebraic(input, &mut table)?;
    let dnf = dnf.with_table(Rc::new(table));
    Ok(simplify(&dnf))
}

/// Returns the minimal-DNF form of `x` via Quine-McCluskey.
///
/// The number of variables is taken from the attached [`VariableTable`]. Constants are handled directly: a tautology returns the
/// constant `true` and an unsatisfiable expression returns the constant `false`.
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
    let product_terms = implicants
        .iter()
        .map(|imp| implicant_to_product_term(imp, num_vars))
        .collect();
    Expression { product_terms, table }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: a single-literal product term over `width` variables.
    fn literal(index: usize, width: usize, positive: bool) -> ProductTerm {
        let mut terms = vec![false; width];
        let mut mask = vec![false; width];
        terms[index] = positive;
        mask[index] = true;
        ProductTerm { terms, mask }
    }

    #[test]
    fn product_term_and_combines_literals() {
        let mut a = literal(0, 2, true);
        a.and_assign(&literal(1, 2, true));
        assert_eq!(a.to_algebraic(&VariableTable::new(), AlgebraicStyle::Code), "$x0 & $x1");
    }

    #[test]
    fn product_term_and_conflict_is_false() {
        let mut a = literal(0, 2, true);
        a.and_assign(&literal(0, 2, false)); // x0 & !x0
        assert!(a.is_false());
    }

    #[test]
    fn expression_or_concatenates() {
        let e = Expression::new(vec![literal(0, 1, true)]);
        let f = Expression::new(vec![literal(0, 1, false)]);
        let g = e | &f;
        assert_eq!(g.product_terms().len(), 2);
    }

    #[test]
    fn expression_and_distributes() {
        // (x0) & (x1) -> x0 & x1
        let e = Expression::new(vec![literal(0, 2, true)]);
        let f = Expression::new(vec![literal(1, 2, true)]);
        let g = e & &f;
        assert_eq!(g.to_algebraic_styled(AlgebraicStyle::Code), "$x0 & $x1");
    }

    #[test]
    fn serializers_describe_same_product_term() {
        let e = Expression::new(vec![{
            let mut a = literal(0, 2, true);
            a.and_assign(&literal(1, 2, false));
            a
        }]);
        assert_eq!(e.to_algebraic_styled(AlgebraicStyle::Code), "$x0 & !$x1");
        assert_eq!(e.to_json().to_string(), r#"{"and":[{"var":"x0"},{"!":[{"var":"x1"}]}]}"#);
    }

    #[test]
    fn single_literal_product_term_emits_bare_literal() {
        let e = Expression::new(vec![literal(0, 1, true)]);
        assert_eq!(e.to_json().to_string(), r#"{"var":"x0"}"#);
    }

    #[test]
    fn constants_serialize_to_format_specific_literals() {
        assert_eq!(Expression::r#true().to_json().to_string(), "true");
        assert_eq!(Expression::r#false().to_json().to_string(), "false");
        // Algebraic uses `1` / `0` (matching the algebraic input parser).
        assert_eq!(Expression::r#true().to_algebraic(), "1");
        assert_eq!(Expression::r#false().to_algebraic(), "0");
    }

    #[test]
    fn inverse_obeys_de_morgan() {
        // !(x0) == !x0
        let e = Expression::new(vec![literal(0, 1, true)]);
        assert_eq!(e.inverse().to_algebraic_styled(AlgebraicStyle::Code), "!$x0");
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
    fn and_assign_swaps_to_widen_shorter_product_term() {
        // The lhs is narrower than the rhs: the wider operand must survive.
        let mut narrow = literal(0, 1, true); // width 1
        narrow.and_assign(&literal(2, 3, true)); // width 3
        assert_eq!(narrow.to_algebraic(&VariableTable::new(), AlgebraicStyle::Code), "$x0 & $x2");
    }

    #[test]
    fn default_expression_is_false() {
        let e = Expression::default();
        assert!(e.is_false());
        assert!(!e.is_true());
        assert_eq!(e.product_terms().len(), 0);
    }

    #[test]
    fn empty_conjunction_product_term_is_true_at_expression_level() {
        // A TRUE product term anywhere makes the whole disjunction TRUE.
        let e = Expression::new(vec![
            literal(0, 1, true),
            ProductTerm {
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
        // A multi-character name is prefixed with `$` so the output re-parses.
        assert_eq!(e.to_algebraic(), "$rain");
        assert_eq!(e.to_json().to_string(), r#"{"var":"rain"}"#);
    }

    #[test]
    fn to_json_drops_false_sentinel_product_terms() {
        // A real literal ORed with a FALSE sentinel keeps only the literal.
        let e = Expression::new(vec![
            literal(0, 1, true),
            ProductTerm {
                terms: vec![],
                mask: vec![],
            },
        ]);
        assert_eq!(e.to_json().to_string(), r#"{"var":"x0"}"#);
    }

    #[test]
    fn multi_product_term_expression_wraps_in_or() {
        let e = Expression::new(vec![literal(0, 2, true), literal(1, 2, false)]);
        assert_eq!(e.to_json().to_string(), r#"{"or":[{"var":"x0"},{"!":[{"var":"x1"}]}]}"#);
    }

    #[test]
    fn implicant_to_product_term_maps_care_bits_only() {
        // Implicant "x1" over 2 vars: bit 1 fixed to 1, bit 0 don't-care.
        let imp = Implicant { v: 0b10, d: 0b01 };
        let c = implicant_to_product_term(&imp, 2);
        assert_eq!(c.mask, vec![false, true]);
        assert!(c.terms[1]);
    }

    #[test]
    fn expression_equality_ignores_table() {
        // Logical identity is product term equality; the table is serialization metadata only.
        let a = Expression::new(vec![literal(0, 1, true)]);
        let mut table = VariableTable::new();
        table.index_of("a").unwrap();
        let b = Expression::new(vec![literal(0, 1, true)]).with_table(Rc::new(table));
        assert_eq!(a, b);
    }

    #[test]
    fn simplify_collapses_redundant_product_terms() {
        // x0 | x0 -> x0 (table with one variable so num_vars == 1).
        let mut table = VariableTable::new();
        table.index_of("a").unwrap();
        let e = Expression::new(vec![literal(0, 1, true), literal(0, 1, true)]).with_table(Rc::new(table));
        let s = simplify(&e);
        assert_eq!(s.to_algebraic(), "a");
    }

    #[test]
    fn algebraic_styles_pick_operators_and_whitespace() {
        // (a & !b) | c over single-letter vars a, b, c.
        let mut table = VariableTable::new();
        for n in ["a", "b", "c"] {
            table.index_of(n).unwrap();
        }
        let mut term = literal(0, 3, true);
        term.and_assign(&literal(1, 3, false)); // a & !b
        let e = Expression::new(vec![term, literal(2, 3, true)]).with_table(Rc::new(table));

        assert_eq!(e.to_algebraic_styled(AlgebraicStyle::Common), "ab\u{0305} + c");
        assert_eq!(e.to_algebraic_styled(AlgebraicStyle::Code), "a & !b | c");
        assert_eq!(
            e.to_algebraic_styled(AlgebraicStyle::Logic),
            "a \u{2227} \u{00AC}b \u{2228} c"
        );
        // The default style is `Common`.
        assert_eq!(e.to_algebraic(), "ab\u{0305} + c");
    }

    #[test]
    fn common_style_prefixes_and_spaces_multichar_names() {
        let mut table = VariableTable::new();
        table.index_of("velocity").unwrap(); // 0
        table.index_of("a").unwrap(); // 1
        table.index_of("pressure").unwrap(); // 2

        // velocity & pressure: the leading `$` of `$pressure` delimits, so the two `$`-names need no separating space.
        let mut both = literal(0, 3, true);
        both.and_assign(&literal(2, 3, true));
        let e = Expression::new(vec![both]).with_table(Rc::new(table.clone()));
        assert_eq!(e.to_algebraic_styled(AlgebraicStyle::Common), "$velocity$pressure");

        // velocity & a: a `$`-name before a single letter needs a space, else `$velocity` would swallow the `a`.
        let mut mixed = literal(0, 3, true);
        mixed.and_assign(&literal(1, 3, true));
        let e = Expression::new(vec![mixed]).with_table(Rc::new(table));
        assert_eq!(e.to_algebraic_styled(AlgebraicStyle::Common), "$velocity a");
    }

    #[test]
    fn every_style_round_trips_to_the_same_expression() {
        use crate::algebraic::parse_algebraic;

        // Cover single + multi-character names, negation, both adjacency directions (multi→single and single→multi), and the
        // constants.
        for src in [
            "a + b",
            "ab!c + d",
            "$velocity & !$pressure + a",
            "$x1 $x2 + !$x1 c",
            "a$bc",
            "!a",
            "1",
            "0",
        ] {
            let mut table = VariableTable::new();
            let expr = parse_algebraic(src, &mut table).unwrap().with_table(Rc::new(table));
            for style in [AlgebraicStyle::Common, AlgebraicStyle::Code, AlgebraicStyle::Logic] {
                let rendered = expr.to_algebraic_styled(style);
                let mut t2 = VariableTable::new();
                let reparsed = parse_algebraic(&rendered, &mut t2)
                    .unwrap_or_else(|e| panic!("{style:?} output {rendered:?} of {src:?} failed to parse: {e}"));
                // Expression equality is product term equality (the table is ignored).
                assert_eq!(expr, reparsed, "style {style:?} of {src:?} via {rendered:?}");
            }
        }
    }
}
