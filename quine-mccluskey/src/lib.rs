//! Quine-McCluskey simplifier (Library B in the architecture).
//!
//! [`minimize`] takes a boolean function's on-set (`min_terms`) and don't-care
//! set and returns a minimal set of prime [`Implicant`]s covering the on-set.
//! The algorithm has two stages:
//!
//! 1. **Prime-implicant generation** — repeatedly combine implicants that differ
//!    in a single bit until none can be combined; the uncombined ones are prime.
//! 2. **Selection** — pick all essential prime implicants, then cover any
//!    remaining minterms with Petrick's method (exact) or a greedy fallback for
//!    large charts.
//!
//! `Term` is a `u32`, which caps the design at `MAX_VARIABLES = 32` distinct
//! variables.

use std::collections::BTreeSet;

/// Maximum number of distinct variables (bounded by the width of `Term`).
pub const MAX_VARIABLES: usize = 32;

/// Above this many product terms during Petrick's method, fall back to a greedy
/// cover to avoid exponential blow-up.
const PETRICK_PRODUCT_LIMIT: usize = 65_536;

/// A bit-packed term: bit `i` corresponds to variable `i`.
pub type Term = u32;

/// A product term in the simplification: `v` holds the fixed bit values and `d`
/// is the don't-care mask (a set bit means "this variable is eliminated").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Implicant {
    pub v: Term,
    pub d: Term,
}

fn is_power_of_2(x: Term) -> bool {
    x != 0 && ((x - 1) & x) == 0
}

/// Returns true if two implicants have the same don't-care mask and their fixed
/// values differ in exactly one bit (so they can be combined).
fn differ_by_one_bit(i0: &Implicant, i1: &Implicant) -> bool {
    i0.d == i1.d && is_power_of_2(i0.v ^ i1.v)
}

/// Combines two one-bit-apart implicants into one with the differing bit turned
/// into a don't-care.
fn combine(i0: &Implicant, i1: &Implicant) -> Implicant {
    debug_assert!(differ_by_one_bit(i0, i1));
    let d = i0.d | (i0.v ^ i1.v);
    let v = i0.v & !d;
    Implicant { v, d }
}

/// Returns true if `i0` covers `i1` (every care bit of `i0` is also fixed in
/// `i1` with the same value).
fn covers(i0: &Implicant, i1: &Implicant) -> bool {
    (i0.d | i1.d) == i0.d && i0.v == (i1.v & !i0.d)
}

/// Returns true if implicant `i` covers minterm `t`.
fn covers_term(i: &Implicant, t: Term) -> bool {
    covers(i, &Implicant { v: t, d: 0 })
}

/// Generates the prime implicants of the function whose terms (on-set plus
/// don't-cares) are `all`.
fn prime_implicants(all: &[Term]) -> Vec<Implicant> {
    let mut current: Vec<Implicant> = all.iter().map(|&t| Implicant { v: t, d: 0 }).collect();
    let mut primes: Vec<Implicant> = Vec::new();

    while !current.is_empty() {
        let mut used = vec![false; current.len()];
        let mut next: Vec<Implicant> = Vec::new();
        for i in 0..current.len() {
            for j in (i + 1)..current.len() {
                if differ_by_one_bit(&current[i], &current[j]) {
                    used[i] = true;
                    used[j] = true;
                    let c = combine(&current[i], &current[j]);
                    if !next.contains(&c) {
                        next.push(c);
                    }
                }
            }
        }
        for (i, imp) in current.iter().enumerate() {
            if !used[i] && !primes.contains(imp) {
                primes.push(*imp);
            }
        }
        current = next;
    }

    primes
}

/// Deduplicates `terms` preserving order.
fn unique(terms: &[Term]) -> Vec<Term> {
    let mut out: Vec<Term> = Vec::new();
    for &t in terms {
        if !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

/// Counts the literals (fixed bits) in an implicant over `num_variables`.
fn literal_count(imp: &Implicant, num_variables: usize) -> u32 {
    let mask: Term = if num_variables >= 32 {
        Term::MAX
    } else {
        (1 << num_variables) - 1
    };
    num_variables as u32 - (imp.d & mask).count_ones()
}

/// Selects a minimal-size set of prime implicants covering every minterm.
///
/// Essential prime implicants are taken first; remaining minterms are covered by
/// Petrick's method, or a greedy cover when the chart is large.
fn select(primes: &[Implicant], min_terms: &[Term], num_variables: usize) -> Vec<Implicant> {
    // chart[k] = indices of primes covering min_terms[k].
    let chart: Vec<Vec<usize>> = min_terms
        .iter()
        .map(|&m| {
            primes
                .iter()
                .enumerate()
                .filter(|(_, p)| covers_term(p, m))
                .map(|(i, _)| i)
                .collect()
        })
        .collect();

    let mut selected = vec![false; primes.len()];

    // Essential prime implicants: a minterm covered by exactly one prime.
    for cov in &chart {
        if cov.len() == 1 {
            selected[cov[0]] = true;
        }
    }

    let uncovered: Vec<usize> = (0..min_terms.len())
        .filter(|&k| !chart[k].iter().any(|&pi| selected[pi]))
        .collect();

    if !uncovered.is_empty() {
        cover_remaining(&mut selected, &chart, &uncovered, primes, num_variables);
    }

    primes
        .iter()
        .enumerate()
        .filter(|(i, _)| selected[*i])
        .map(|(_, p)| *p)
        .collect()
}

/// Covers the still-uncovered minterms, mutating `selected`.
fn cover_remaining(
    selected: &mut [bool],
    chart: &[Vec<usize>],
    uncovered: &[usize],
    primes: &[Implicant],
    num_variables: usize,
) {
    // Estimate Petrick's product size; fall back to greedy if it would explode.
    let mut estimate: usize = 1;
    for &k in uncovered {
        estimate = estimate.saturating_mul(chart[k].len().max(1));
        if estimate > PETRICK_PRODUCT_LIMIT {
            break;
        }
    }
    if estimate <= PETRICK_PRODUCT_LIMIT {
        petrick(selected, chart, uncovered, primes, num_variables);
    } else {
        greedy(selected, chart, uncovered);
    }
}

/// Petrick's method: build the product-of-sums "(prime choices for each
/// minterm)", multiply out to sum-of-products with absorption, and pick the
/// product with the fewest primes (ties broken by fewest literals).
fn petrick(
    selected: &mut [bool],
    chart: &[Vec<usize>],
    uncovered: &[usize],
    primes: &[Implicant],
    num_variables: usize,
) {
    let mut products: Vec<BTreeSet<usize>> = vec![BTreeSet::new()];
    for &k in uncovered {
        let mut next: Vec<BTreeSet<usize>> = Vec::new();
        for product in &products {
            for &pi in &chart[k] {
                let mut np = product.clone();
                np.insert(pi);
                next.push(np);
            }
        }
        // Absorption: drop any product that is a superset of another.
        next.sort_by_key(|s| s.len());
        let mut reduced: Vec<BTreeSet<usize>> = Vec::new();
        for s in next {
            if !reduced.iter().any(|r| r.is_subset(&s)) {
                reduced.push(s);
            }
        }
        products = reduced;
        if products.len() > PETRICK_PRODUCT_LIMIT {
            // Pathological growth despite absorption: bail out to greedy.
            greedy(selected, chart, uncovered);
            return;
        }
    }

    let best = products.iter().min_by(|a, b| {
        a.len().cmp(&b.len()).then_with(|| {
            let la: u32 = a.iter().map(|&i| literal_count(&primes[i], num_variables)).sum();
            let lb: u32 = b.iter().map(|&i| literal_count(&primes[i], num_variables)).sum();
            la.cmp(&lb)
        })
    });
    if let Some(best) = best {
        for &pi in best {
            selected[pi] = true;
        }
    }
}

/// Greedy fallback: repeatedly pick the prime covering the most still-uncovered
/// minterms.
fn greedy(selected: &mut [bool], chart: &[Vec<usize>], uncovered: &[usize]) {
    let mut remaining: BTreeSet<usize> = uncovered.iter().copied().collect();
    while !remaining.is_empty() {
        let mut best_pi = None;
        let mut best_count = 0;
        for (pi, &is_selected) in selected.iter().enumerate() {
            if is_selected {
                continue;
            }
            let count = remaining
                .iter()
                .filter(|&&k| chart[k].contains(&pi))
                .count();
            if count > best_count {
                best_count = count;
                best_pi = Some(pi);
            }
        }
        match best_pi {
            Some(pi) => {
                selected[pi] = true;
                remaining.retain(|&k| !chart[k].contains(&pi));
            }
            None => break,
        }
    }
}

/// Minimizes a boolean function given its on-set (`min_terms`) and don't-care
/// set over `num_variables` variables, returning a minimal set of prime
/// implicants covering the on-set.
///
/// An empty on-set yields an empty result (the constant `false`).
pub fn minimize(min_terms: &[Term], dont_cares: &[Term], num_variables: usize) -> Vec<Implicant> {
    let min_terms = unique(min_terms);
    if min_terms.is_empty() {
        return Vec::new();
    }

    // Initial implicants come from the on-set and don't-cares alike.
    let mut all = min_terms.clone();
    for &t in dont_cares {
        if !all.contains(&t) {
            all.push(t);
        }
    }

    let mut primes = prime_implicants(&all);

    // Drop prime implicants that cover none of the required minterms (these can
    // arise from don't-cares only).
    primes.retain(|p| min_terms.iter().any(|&m| covers_term(p, m)));

    select(&primes, &min_terms, num_variables)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn as_set(implicants: &[Implicant]) -> BTreeSet<(Term, Term)> {
        implicants.iter().map(|i| (i.v, i.d)).collect()
    }

    // Evaluates a cover at a given input.
    fn cover_value(implicants: &[Implicant], t: Term) -> bool {
        implicants.iter().any(|i| covers_term(i, t))
    }

    #[test]
    fn power_of_two() {
        assert!(is_power_of_2(1));
        assert!(is_power_of_2(8));
        assert!(!is_power_of_2(0));
        assert!(!is_power_of_2(3));
    }

    #[test]
    fn combine_eliminates_differing_bit() {
        let a = Implicant { v: 0b00, d: 0 };
        let b = Implicant { v: 0b01, d: 0 };
        let c = combine(&a, &b);
        assert_eq!(c.d, 0b01);
        assert_eq!(c.v, 0b00);
    }

    #[test]
    fn minterm_zero_does_not_panic() {
        // Regression for the old minterm-0 index underflow.
        let result = minimize(&[0], &[], 1);
        assert!(!result.is_empty());
        assert!(cover_value(&result, 0));
        assert!(!cover_value(&result, 1));
    }

    #[test]
    fn single_variable() {
        // f = x0 -> minterm {1} over 1 var.
        let result = minimize(&[1], &[], 1);
        assert_eq!(as_set(&result), [(1, 0)].into_iter().collect());
    }

    #[test]
    fn merges_into_one_literal() {
        // minterms {2, 3} over 2 vars -> x1 (v=2, d=1).
        let result = minimize(&[2, 3], &[], 2);
        assert_eq!(as_set(&result), [(2, 1)].into_iter().collect());
    }

    #[test]
    fn all_ones_is_tautology() {
        // Every minterm over 2 vars -> single implicant covering all (v=0, d=11).
        let result = minimize(&[0, 1, 2, 3], &[], 2);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].d, 0b11);
        for t in 0..4 {
            assert!(cover_value(&result, t));
        }
    }

    #[test]
    fn textbook_cyclic_chart() {
        // Classic cyclic prime-implicant chart: {0,1,2,5,6,7} over 3 vars has no
        // essential PIs and a minimal cover of three implicants.
        let min_terms = [0, 1, 2, 5, 6, 7];
        let result = minimize(&min_terms, &[], 3);
        assert_eq!(result.len(), 3, "minimal cover should be 3 implicants");
        // The cover must reproduce the on-set exactly.
        for t in 0..8 {
            assert_eq!(cover_value(&result, t), min_terms.contains(&t));
        }
    }

    #[test]
    fn dont_cares_are_used_but_not_required() {
        // f with minterm {1}, don't-care {3} over 2 vars -> x0 (v=1, d=2).
        let result = minimize(&[1], &[3], 2);
        assert!(cover_value(&result, 1));
        // The result need not cover the don't-care's complement region beyond
        // what helps; but it must cover minterm 1 and not minterm 0.
        assert!(!cover_value(&result, 0));
    }

    #[test]
    fn empty_on_set_is_empty() {
        assert!(minimize(&[], &[], 3).is_empty());
        assert!(minimize(&[], &[1, 2], 3).is_empty());
    }
}
