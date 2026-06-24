# Architecture

This document describes Beast's internal design and implementation. For build
and usage instructions, see the [README](../README.md).

## Overview

Beast is a thin CLI wrapping two conceptual libraries. Data flows in one
direction:

```text
input → DNF → minimized DNF → output
```

Input and output are each either a compact algebraic syntax or JsonLogic. The
`beast` crate owns input parsing and output serialization; the actual
simplification is delegated to the `quine-mccluskey` crate.

## Workspace layout

Beast is a Cargo workspace of two crates:

- **`beast/`** — the main crate. Owns the end-to-end pipeline: parse the input,
  convert it to disjunctive normal form (DNF), drive the simplification, and
  serialize the minimized result back to the chosen output syntax. Depends on
  `quine-mccluskey`.
  - [`src/lib.rs`](../beast/src/lib.rs) — data model, algebra, and the `simplify*`
    entry points.
  - [`src/converter.rs`](../beast/src/converter.rs) — JsonLogic parser (`to_dnf`).
  - [`src/algebraic.rs`](../beast/src/algebraic.rs) — algebraic tokenizer/parser
    (`parse_algebraic`).
  - [`src/json.rs`](../beast/src/json.rs) — the in-crate JSON implementation.
  - [`src/variable_table.rs`](../beast/src/variable_table.rs) — the variable-name
    table.
  - [`src/main.rs`](../beast/src/main.rs) — the CLI (argument parsing via `clap`).
- **`quine-mccluskey/`** — the simplifier library (crate name
  `quine_mccluskey`). Minimizes a boolean expression with the Quine–McCluskey
  algorithm: DNF in, minimized DNF out. Re-exported from `beast` as
  `beast::quine_mccluskey`.

## Pipeline

1. **Parsers** — two interchangeable front ends, both producing an `Expression`
   in **disjunctive normal form**: the JsonLogic converter
   ([`converter.rs`](../beast/src/converter.rs), `to_dnf`) and the algebraic parser
   ([`algebraic.rs`](../beast/src/algebraic.rs), `parse_algebraic`). Both reuse the
   algebra operators — `BitOr` (`|`) concatenates product terms, `BitAnd` (`&`)
   distributes (product of sums → sum of products), and `Expression::inverse`
   applies De Morgan — so any input collapses to DNF as it is built. They pair
   with the serializers `Expression::to_json` and `Expression::to_algebraic`;
   any input form can be emitted in either output form.
2. **Simplifier** ([`quine-mccluskey`](../quine-mccluskey/src/lib.rs), `minimize`):
   input and output are both DNF. The `beast` crate converts the DNF
   product-term set to minterms (`expression_to_minterms`), runs
   Quine–McCluskey, and converts the selected prime implicants back to product
   terms (`implicant_to_product_term`).

The `simplify` / `simplify_json` / `simplify_algebraic` entry points in
[`lib.rs`](../beast/src/lib.rs) tie these together. `simplify_json` and
`simplify_algebraic` return `Result<Expression, String>` (not `Expression`) so
the CLI can surface parse errors — unknown operator, bad arity, syntax errors,
more than 32 variables — on stderr.

## Core data model (`beast/src/lib.rs`)

- `Expression` = OR of `ProductTerm`s (a `Vec<ProductTerm>`), i.e. DNF.
- `ProductTerm` = AND of literals, stored as two parallel `Vec<bool>`: `terms`
  (literal sign: `true` = unnegated, `false` = negated) and `mask` (`true` = the
  variable is present in this product term).

### Constant conventions

Used throughout the algebra and serialization:

- A product term with **empty `terms`** represents **FALSE** (the conflict
  sentinel).
- A product term with **non-empty `terms` but no `mask` bit set** represents
  **TRUE** (an empty conjunction).
- At the `Expression` level: no product terms ⇒ FALSE; any empty-conjunction
  product term ⇒ TRUE.

`ProductTerm::is_true` / `is_false` and `Expression::is_true` / `is_false`
encapsulate these. Use them rather than checking `terms.is_empty()` directly.
`Expression::inverse` folds from the TRUE identity, so `!false == true` and
`!true == false`.

### Variables

Variables are referenced internally by **bit index**, not name.
`VariableTable` ([`variable_table.rs`](../beast/src/variable_table.rs)) maps
arbitrary user-supplied names to indices and restores them on output.
`Expression` owns an `Rc<VariableTable>` so the serializers stay parameterless;
table-less expressions (e.g. in unit tests) fall back to synthesized
`"x" + index` names.

## The two QM-related representations

- `Implicant { v: Term, d: Term }` (in
  [`quine-mccluskey/src/lib.rs`](../quine-mccluskey/src/lib.rs)): `v` = fixed bit
  values, `d` = don't-care mask. `Term` is a `u32`, which caps the design at
  **`MAX_VARIABLES = 32`** distinct variables.
- The bridge between the `ProductTerm` (terms/mask) representation and the
  `Term`/`Implicant` (bitmask) representation lives in
  [`lib.rs`](../beast/src/lib.rs): `expression_to_minterms` (DNF → ON-set minterms
  over `num_vars`) and `implicant_to_product_term` (selected implicant → product
  term).

## Quine–McCluskey selection

`minimize` does both prime-implicant **generation** and **selection** (essential
prime implicants plus Petrick's method). It falls back to a greedy cover when
the estimated product size exceeds `PETRICK_PRODUCT_LIMIT`, so exact minimality
is only guaranteed below that threshold.

## Serialization gotchas

- `to_json` and `to_algebraic` both drop FALSE-sentinel product terms but render
  constants differently on purpose: `to_json` emits JSON `true` / `false`,
  `to_algebraic` emits `1` / `0` (so algebraic output re-parses as algebraic
  input). Keep the product-term-dropping behavior in sync; the constant
  spellings are format-specific.
- `ProductTerm`'s parallel `terms` / `mask` vectors can differ in length between
  product terms (each is only as wide as the highest variable index it uses);
  the algebra handles mismatched widths. When iterating up to `num_vars` (e.g. in
  `expression_to_minterms`), guard with `i < mask.len()`.

## Locked decisions

- **Two I/O formats: algebraic (default) and JsonLogic**, selected per side by
  the CLI `--in` / `--out` flags. JsonLogic is operator-as-key objects, one key
  per node: `to_json` emits objects (`{"or":[...]}`, negation `{"!":[...]}`,
  variable `{"var":"name"}`), with single-element `and` / `or` collapsed to the
  bare child and constants emitted as JSON `true` / `false`.
- **Variable names are arbitrary** user-supplied strings, mapped to bit indices
  and preserved in output.
- **Accepted operators**: standard `and`, `or`, `!`, `var`, and boolean literals
  `true` / `false`, plus the non-standard Beast extensions `xor`, `nand`, `nor`.
  The extensions are **input-only** — desugared to `and` / `or` / `!` during
  conversion and never emitted on output. All other JsonLogic operators
  (comparison / numeric / array / string) are rejected.
- The algebraic output style is selected by the CLI `--style` flag and emitted
  by `Expression::to_algebraic_styled(AlgebraicStyle)` — `common`, `code`, or
  `logic`. Every style emits a `$` prefix on multi-character names (single ASCII
  letters stay bare), so output in any style re-parses to the same `Expression`.
