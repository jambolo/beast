//! Quine-McCluskey simplifier (Library B in the architecture).
//!
//! The algorithm is currently **unfinished**: prime-implicant generation is
//! present but prime-implicant *selection* is not, so `minimize` still returns
//! an empty result. Completing it is Phase D of `plan.md`.
//!
//! `Term` is a `u32`, which caps the design at `MAX_VARIABLES = 32` distinct
//! variables.

/// Maximum number of distinct variables (bounded by the width of `Term`).
pub const MAX_VARIABLES: usize = 32;

/// A bit-packed term: bit `i` corresponds to variable `i`.
pub type Term = u32;

/// A product term in the simplification: `v` holds the fixed bit values and `d`
/// is the don't-care mask (a set bit means "this variable is eliminated").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Implicant {
    pub v: Term,
    pub d: Term,
}

type ImplicantList = Vec<Implicant>;
/// One `ImplicantList` per number-of-one-bits bucket.
type Round = Vec<ImplicantList>;

fn is_power_of_2(x: Term) -> bool {
    x != 0 && ((x - 1) & x) == 0
}

fn number_of_one_bits(n: u32) -> u32 {
    n.count_ones()
}

fn differ_by_one_bit(i0: &Implicant, i1: &Implicant) -> bool {
    // NOTE: this currently compares `i0.d` to itself, which is a known bug.
    // plan.md tracks the intended fix (`i0.d == i1.d`) as BUG-2 / task A2.
    #[allow(clippy::eq_op)]
    let same_dont_cares = i0.d == i0.d;
    same_dont_cares && is_power_of_2(i0.v ^ i1.v)
}

fn combine(i0: &Implicant, i1: &Implicant) -> Implicant {
    debug_assert!(differ_by_one_bit(i0, i1));
    let d = i0.d | (i0.v ^ i1.v);
    let v = i0.v & !d;
    Implicant { v, d }
}

fn covers(i0: &Implicant, i1: &Implicant) -> bool {
    (i0.d | i1.d) == i0.d && i0.v == (i1.v & !i0.d)
}

/// Removes duplicate implicants in place, keeping the first occurrence of each
/// (matching the original `removeDuplicates`).
fn remove_duplicates(v: &mut ImplicantList) {
    let mut i = 0;
    while i < v.len() {
        let current = v[i];
        let mut j = i + 1;
        while j < v.len() {
            if v[j] == current {
                v.remove(j);
            } else {
                j += 1;
            }
        }
        i += 1;
    }
}

/// Combines every pair of implicants (one from each list) that differ by a
/// single bit, returning the deduplicated set of combined implicants.
fn find_all_matches(i0: &ImplicantList, i1: &ImplicantList) -> ImplicantList {
    let mut combined = ImplicantList::new();
    for i in i0 {
        for j in i1 {
            if differ_by_one_bit(i, j) {
                combined.push(combine(i, j));
            }
        }
    }
    remove_duplicates(&mut combined);
    combined
}

/// Erases from `lower` every implicant that is covered by an implicant in the
/// `higher` round (i.e. those that were successfully combined upward).
fn remove_combined_implicants(lower: &mut Round, higher: &Round) {
    for list in higher {
        for j in list {
            let j = *j;
            for m in lower.iter_mut() {
                m.retain(|n| !covers(&j, n));
            }
        }
    }
}

/// Returns true if `i` covers none of the given terms.
fn covers_none_of(i: &Implicant, terms: &[Term]) -> bool {
    terms.iter().all(|&t| !covers(i, &Implicant { v: t, d: 0 }))
}

/// Minimizes a boolean function given its on-set (`min_terms`) and don't-care
/// set, returning the selected implicants.
///
/// UNFINISHED: prime implicants are generated but selection is not implemented,
/// so this currently returns an empty vector.
pub fn minimize(min_terms: &[Term], dont_cares: &[Term]) -> Vec<Implicant> {
    // Seed the first round, bucketing each term by its number of one bits.
    let mut r: Round = vec![ImplicantList::new(); MAX_VARIABLES];

    for &t in min_terms {
        r[(number_of_one_bits(t) - 1) as usize].push(Implicant { v: t, d: 0 });
    }
    for &t in dont_cares {
        r[(number_of_one_bits(t) - 1) as usize].push(Implicant { v: t, d: 0 });
    }

    let mut prime_implicants = ImplicantList::new();

    // Combine implicants that differ by one bit until no more combinations can
    // be made; whatever cannot be combined is a prime implicant.
    loop {
        let mut next: Round = Round::new();
        let mut done = true;
        for n in 0..r.len() - 1 {
            let combined = find_all_matches(&r[n], &r[n + 1]);
            if !combined.is_empty() {
                done = false;
            }
            next.push(combined);
        }
        if done {
            break;
        }
        remove_combined_implicants(&mut r, &next);
        for list in &r {
            prime_implicants.extend(list.iter().copied());
        }
        r = next;
    }

    // Drop don't-care-only implicants (those covering none of the minterms).
    prime_implicants.retain(|i| !covers_none_of(i, min_terms));

    // not finished: prime-implicant selection (essential PIs + Petrick's
    // method / greedy cover) is Phase D in plan.md.
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn minimize_is_unfinished() {
        // Selection is not implemented yet, so the result is empty.
        assert!(minimize(&[0b01, 0b11], &[]).is_empty());
    }
}
