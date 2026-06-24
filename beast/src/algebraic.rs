//! Algebraic parser: an algebraic boolean expression -> DNF [`Expression`].
//!
//! This is the input-side counterpart to [`Expression::to_algebraic`]. A `tokenize` pass turns the source text into `Token`s, then
//! a recursive descent ([`parse_algebraic`]) builds an [`Expression`] directly in DNF by reusing the boolean algebra on
//! [`Expression`] (`|` ORs product terms, `&` distributes, [`Expression::inverse`] applies De Morgan), exactly like the JsonLogic
//! [`converter`](crate::converter).
//!
//! # Syntax
//!
//! - **and** — `&`, `*`, `.`, `∧`, `⋅`, `·`, `×`, or *juxtaposition* (two adjacent operands, e.g. `ab` = `a & b`),
//! - **or** — `|`, `+`, `∨`,
//! - **xor** — `^`, `⊕`, `⊻`,
//! - **not** — `~`, `-`, `¬`, `!` (all prefix), or a combining overbar `\u{0305}` *immediately following a single-letter
//!   variable*
//! - **parentheses** — `(` and `)`,
//! - **constants** — `1` (true) and `0` (false),
//! - **variables** — a single ASCII letter is one variable; a `$` introduces a multi-character name continuing over
//!   `[0-9a-zA-Z_]` (e.g. `$velocity`).
//!
//! Whitespace is a delimiter but otherwise ignored. Precedence, tightest first: `not`, then `and`, then `or`/`xor` (which share one
//! left-associative tier).

use crate::variable_table::VariableTable;
use crate::{Expression, ProductTerm};

/// Combining overbar (U+0305): a postfix negation when it follows a single letter (`a\u{0305}` = `!a`).
const OVERBAR: char = '\u{0305}';

/// A lexical token of the algebraic syntax.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    LParen,
    RParen,
    Or,
    Xor,
    And,
    Not,
    Var(String),
    Const(bool),
}

/// Splits algebraic source text into [`Token`]s.
///
/// # Errors
///
/// Returns `Err` on a bare `$` with no following identifier character, a combining overbar (`\u{0305}`) that does not immediately
/// follow a single-letter variable, or any character that is not whitespace, an operator, a parenthesis, `0`/`1`, an ASCII letter,
/// or `$` (notably the digits `2`–`9`).
fn tokenize(input: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            c if c.is_whitespace() => {
                chars.next();
            }
            '(' => {
                chars.next();
                tokens.push(Token::LParen);
            }
            ')' => {
                chars.next();
                tokens.push(Token::RParen);
            }
            '|' | '+' | '\u{2228}' => {
                chars.next();
                tokens.push(Token::Or);
            }
            '^' | '\u{2295}' | '\u{22BB}' => {
                chars.next();
                tokens.push(Token::Xor);
            }
            '&' | '*' | '.' | '\u{2227}' | '\u{22C5}' | '\u{00B7}' | '\u{00D7}' => {
                chars.next();
                tokens.push(Token::And);
            }
            '~' | '-' | '\u{00AC}' | '!' => {
                chars.next();
                tokens.push(Token::Not);
            }
            '0' => {
                chars.next();
                tokens.push(Token::Const(false));
            }
            '1' => {
                chars.next();
                tokens.push(Token::Const(true));
            }
            '$' => {
                chars.next();
                let mut name = String::new();
                while let Some(&d) = chars.peek() {
                    if d == '_' || d.is_ascii_alphanumeric() {
                        name.push(d);
                        chars.next();
                    } else {
                        break;
                    }
                }
                if name.is_empty() {
                    return Err("'$' must be followed by an identifier character [0-9a-zA-Z_]".to_string());
                }
                tokens.push(Token::Var(name));
            }
            c if c.is_ascii_alphabetic() => {
                chars.next();
                // A combining overbar immediately after a single letter is a postfix negation of that one variable; desugar to a
                // prefix `Not` so the parser handles it like any other negation.
                if chars.peek() == Some(&OVERBAR) {
                    chars.next();
                    tokens.push(Token::Not);
                }
                tokens.push(Token::Var(c.to_string()));
            }
            OVERBAR => {
                return Err(format!("overbar {:?} may only follow a single-letter variable", OVERBAR));
            }
            other => {
                return Err(format!("unexpected character {:?} in algebraic expression", other));
            }
        }
    }
    Ok(tokens)
}

/// Parses an algebraic boolean expression into a DNF [`Expression`], registering variables in `table`.
///
/// The returned expression carries no table; attach one with [`Expression::with_table`] if you need the original names on output.
///
/// # Errors
///
/// Returns `Err` on a lexing error, an empty expression, a syntax error (unbalanced parentheses, a dangling operator, trailing
/// tokens), or more than [`quine_mccluskey::MAX_VARIABLES`] distinct variables.
///
/// # Examples
///
/// ```
/// use beast::algebraic::parse_algebraic;
/// use beast::variable_table::VariableTable;
///
/// // Juxtaposition is AND and `+` is OR: `ab + c` -> (a & b) | c.
/// let mut table = VariableTable::new();
/// let dnf = parse_algebraic("ab + c", &mut table).unwrap();
/// assert_eq!(dnf.product_terms().len(), 2);
/// ```
pub fn parse_algebraic(input: &str, table: &mut VariableTable) -> Result<Expression, String> {
    let tokens = tokenize(input)?;
    if tokens.is_empty() {
        return Err("empty expression".to_string());
    }
    let mut parser = Parser {
        tokens: &tokens,
        pos: 0,
        table,
    };
    let expr = parser.parse_or_xor()?;
    if parser.pos != parser.tokens.len() {
        return Err(format!("unexpected trailing token {:?}", parser.tokens[parser.pos]));
    }
    Ok(expr)
}

/// Recursive-descent parser state over a token slice.
struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    table: &'a mut VariableTable,
}

impl Parser<'_> {
    /// `or_xor := and ( (OR | XOR) and )*` — left-associative, one shared tier.
    fn parse_or_xor(&mut self) -> Result<Expression, String> {
        let mut left = self.parse_and()?;
        loop {
            match self.tokens.get(self.pos) {
                Some(Token::Or) => {
                    self.pos += 1;
                    let right = self.parse_and()?;
                    left |= &right;
                }
                Some(Token::Xor) => {
                    self.pos += 1;
                    let right = self.parse_and()?;
                    left = xor(left, &right);
                }
                _ => break,
            }
        }
        Ok(left)
    }

    /// `and := not ( AND? not )*` — explicit AND or juxtaposition of adjacent factors, left-associative.
    fn parse_and(&mut self) -> Result<Expression, String> {
        let mut left = self.parse_not()?;
        loop {
            let next_is_factor = match self.tokens.get(self.pos) {
                Some(Token::And) => {
                    self.pos += 1; // explicit AND operator
                    true
                }
                // Juxtaposition: a factor begins where one of these tokens does.
                Some(Token::Not | Token::Var(_) | Token::Const(_) | Token::LParen) => true,
                _ => false,
            };
            if !next_is_factor {
                break;
            }
            let right = self.parse_not()?;
            left &= &right;
        }
        Ok(left)
    }

    /// `not := NOT* primary` — prefix negations.
    fn parse_not(&mut self) -> Result<Expression, String> {
        if let Some(Token::Not) = self.tokens.get(self.pos) {
            self.pos += 1;
            return Ok(self.parse_not()?.inverse());
        }
        self.parse_primary()
    }

    /// `primary := VAR | CONST | '(' or_xor ')'`.
    fn parse_primary(&mut self) -> Result<Expression, String> {
        match self.tokens.get(self.pos) {
            Some(Token::LParen) => {
                self.pos += 1;
                let inner = self.parse_or_xor()?;
                if self.tokens.get(self.pos) == Some(&Token::RParen) {
                    self.pos += 1;
                    Ok(inner)
                } else {
                    Err("expected ')'".to_string())
                }
            }
            Some(Token::Var(name)) => {
                let name = name.clone();
                self.pos += 1;
                let index = self.table.index_of(&name)?;
                Ok(var_literal(index))
            }
            Some(Token::Const(value)) => {
                let value = *value;
                self.pos += 1;
                Ok(if value { Expression::r#true() } else { Expression::r#false() })
            }
            Some(token) => Err(format!("unexpected token {:?}", token)),
            None => Err("unexpected end of expression".to_string()),
        }
    }
}

/// Builds a single positive-literal expression for variable `index`.
fn var_literal(index: usize) -> Expression {
    let width = index + 1;
    let mut terms = vec![false; width];
    let mut mask = vec![false; width];
    terms[index] = true;
    mask[index] = true;
    Expression::new(vec![ProductTerm { terms, mask }])
}

/// `xor(a, b) = (a & !b) | (!a & b)`, kept in DNF (same desugaring the JsonLogic converter uses).
fn xor(a: Expression, b: &Expression) -> Expression {
    let b_inv = b.inverse();
    let a_inv = a.inverse();
    let left = a & &b_inv;
    let right = a_inv & b;
    left | &right
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::*;

    /// Parses `input` and renders the (unsimplified) DNF back to algebraic, with the registered names restored, so encounter order
    /// is deterministic.
    fn alg(input: &str) -> String {
        let mut table = VariableTable::new();
        let expr = parse_algebraic(input, &mut table).unwrap().with_table(Rc::new(table));
        expr.to_algebraic()
    }

    fn err(input: &str) -> String {
        let mut table = VariableTable::new();
        parse_algebraic(input, &mut table).unwrap_err()
    }

    #[test]
    fn juxtaposition_is_and() {
        assert_eq!(alg("ab"), "ab");
    }

    #[test]
    fn whitespace_is_a_delimiter_only() {
        assert_eq!(alg("a b"), "ab");
        assert_eq!(alg("  a   b "), "ab");
    }

    #[test]
    fn dollar_introduces_multichar_names() {
        assert_eq!(alg("$velocity * $pressure"), "$velocity$pressure");
        // The two names are distinct variables, not 16 single letters.
        let mut table = VariableTable::new();
        parse_algebraic("$velocity * $pressure", &mut table).unwrap();
        assert_eq!(table.len(), 2);
    }

    #[test]
    fn spec_example_mixed_operators() {
        // ab!c|d -> (a & b & !c) | d, rendered in the default (common) style.
        assert_eq!(alg("ab!c|d"), "abc\u{0305} + d");
    }

    #[test]
    fn spec_example_unicode_operators() {
        // a$bc ∨ ¬d -> (a & bc) | !d, common style: `bc` is emitted as the `$`-prefixed `$bc`, and `!d` renders as an overbar on
        // the single letter.
        assert_eq!(alg("a$bc \u{2228} \u{00AC}d"), "a$bc + d\u{0305}");
    }

    #[test]
    fn all_and_spellings_agree() {
        for op in ["a&b", "a*b", "a.b", "a\u{2227}b", "a\u{22C5}b", "a\u{00B7}b", "a\u{00D7}b"] {
            assert_eq!(alg(op), "ab", "for {:?}", op);
        }
    }

    #[test]
    fn all_or_spellings_agree() {
        for op in ["a|b", "a+b", "a\u{2228}b"] {
            assert_eq!(alg(op), "a + b", "for {:?}", op);
        }
    }

    #[test]
    fn all_not_spellings_agree() {
        for op in ["~a", "-a", "\u{00AC}a", "!a"] {
            assert_eq!(alg(op), "a\u{0305}", "for {:?}", op);
        }
    }

    #[test]
    fn overbar_is_postfix_not_on_a_single_letter() {
        assert_eq!(alg("a\u{0305}"), "a\u{0305}");
        // Binds to the one letter only: the rest juxtaposes/ORs normally.
        assert_eq!(alg("a\u{0305}b"), "a\u{0305}b");
        assert_eq!(alg("ab\u{0305}"), "ab\u{0305}");
        assert_eq!(alg("a\u{0305} + b"), "a\u{0305} + b");
        // Composes with a prefix not -> double negation cancels.
        assert_eq!(alg("!a\u{0305}"), "a");
    }

    #[test]
    fn overbar_only_on_single_letter_variables() {
        // Not a single-letter variable in each of these positions:
        assert!(err("$abc\u{0305}").contains("overbar")); // multi-char name
        assert!(err("1\u{0305}").contains("overbar")); // constant
        assert!(err("(a+b)\u{0305}").contains("overbar")); // parenthesised group
        assert!(err("\u{0305}a").contains("overbar")); // leading, no operand
        assert!(err("a\u{0305}\u{0305}").contains("overbar")); // double overbar
        assert!(err("a \u{0305}").contains("overbar")); // must be adjacent
    }

    #[test]
    fn double_negation_cancels() {
        assert_eq!(alg("~~a"), "a");
        assert_eq!(alg("!-a"), "a");
    }

    #[test]
    fn constants_parse_to_true_and_false() {
        assert_eq!(alg("1"), "1");
        assert_eq!(alg("0"), "0");
        // AND/OR with constants reduce by the algebra.
        assert_eq!(alg("1a"), "a");
        assert_eq!(alg("0a"), "0");
        assert_eq!(alg("1+a"), "1");
    }

    #[test]
    fn not_binds_tighter_than_and_tighter_than_or() {
        // ~a b + c parses as ((!a) & b) | c.
        assert_eq!(alg("~a b + c"), "a\u{0305}b + c");
    }

    #[test]
    fn parentheses_override_precedence() {
        // ~(a + b) = !a & !b by De Morgan, in DNF.
        assert_eq!(alg("~(a+b)"), "a\u{0305}b\u{0305}");
    }

    #[test]
    fn or_and_xor_share_a_left_associative_tier() {
        // a + b ^ c parses as (a + b) ^ c, not a + (b ^ c).
        let mut left = VariableTable::new();
        let a = parse_algebraic("a + b ^ c", &mut left).unwrap();
        let mut right = VariableTable::new();
        let b = parse_algebraic("(a + b) ^ c", &mut right).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn rejects_empty_expression() {
        assert!(err("").contains("empty"));
        assert!(err("   ").contains("empty"));
    }

    #[test]
    fn rejects_bad_characters_and_digits() {
        assert!(err("a2").contains("unexpected character"));
        assert!(err("$").contains("identifier"));
    }

    #[test]
    fn rejects_syntax_errors() {
        assert!(!err("(a").is_empty()); // unbalanced
        assert!(!err("a)").is_empty()); // trailing
        assert!(!err("&a").is_empty()); // leading binary operator
        assert!(!err("a+").is_empty()); // dangling operator
    }
}
