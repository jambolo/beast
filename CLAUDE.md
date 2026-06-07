# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project status

Beast is **functional**: the full JsonLogic → DNF → minimized DNF → JsonLogic pipeline works end to end. `simplify` / `simplify_json` are implemented, the Quine–McCluskey `minimize` does prime-implicant generation *and* selection (essential PIs + Petrick's method with a greedy fallback for large charts), and the JsonLogic converter (`beast/src/converter.rs`) and variable-name table (`beast/src/variable_table.rs`) exist. `cargo build`, `cargo test` (44 tests), and `cargo clippy` are clean.

One intentional deviation from `plan.md`: `simplify_json` returns `Result<Expression, String>` (not `Expression`) so the CLI can surface converter errors (unknown operator, bad arity, >32 variables) on stderr.

**`plan.md` is the original roadmap.** It is written for agent consumption with dependency-ordered tasks (`ID / deps / files / action / acceptance`), locked decisions, and a definition of done; the phased task list is now complete. Consult it for the rationale behind design decisions.

## Build & run

Requires a Rust toolchain (Cargo). This is a Cargo **workspace** of two crates — `beast` (data model, converter, CLI) and `quine-mccluskey` (the simplifier library). There are **no external dependencies** (JSON is `beast`'s in-crate `json` module), so builds work fully offline.

```sh
cargo build                              # build the whole workspace
cargo run -p beast -- '<jsonlogic-expression>'   # build and run the CLI with an arg
```

Run the CLI (binary is named `beast`):

```sh
./target/debug/beast '<jsonlogic-expression>'   # expression as first argument
./target/debug/beast < expression.json          # or read from stdin
```

Quick type-check without linking (fast iteration):

```sh
cargo check
```

## Tests

Tests use Rust's built-in test harness (no external framework needed):

```sh
cargo test                  # run all unit/doc tests
cargo test <name>           # run a single test by name substring
cargo clippy                # lints (clean)
```

## Architecture (big picture)

Beast is a thin CLI wrapping two conceptual libraries; data flows: **JsonLogic → DNF → minimized DNF → JsonLogic**. The `beast` crate owns input parsing and output serialization; the actual simplification is delegated to the `quine-mccluskey` crate.

Workspace layout — two crates:
- `beast/` — `src/lib.rs` (data model, algebra, `simplify*`), `src/json.rs` (in-crate JSON), `src/main.rs` (CLI). Depends on `quine-mccluskey`.
- `quine-mccluskey/` — `src/lib.rs` (the simplifier; lib name `quine_mccluskey`). Re-exported from `beast` as `beast::quine_mccluskey`.

1. **Converter** (`beast/src/converter.rs`, `to_dnf`): parses an arbitrary JsonLogic boolean expression into an `Expression` in **disjunctive normal form**. It works by recursive descent that reuses the algebra operators — `BitOr` (`|`) concatenates clauses, `BitAnd` (`&`) distributes (product of sums → sum of products), and `Expression::inverse` applies De Morgan — so any input tree collapses to DNF as it is built.
2. **Simplifier** (`quine-mccluskey` crate, `minimize`): input and output are both DNF. The `beast` crate converts the DNF clause set to minterms (`expression_to_minterms`), runs Quine–McCluskey, and converts the selected prime implicants back to clauses (`implicant_to_clause`).

### Core data model (`beast/src/lib.rs`)

- `Expression` = OR of `Clause`s (a `Vec<Clause>`), i.e. DNF.
- `Clause` = AND of literals, stored as two parallel `Vec<bool>`: `terms` (literal sign: true = unnegated, false = negated) and `mask` (true = the variable is present in this clause).
- Constant conventions (used throughout the algebra and serialization): **a clause with empty `terms` represents FALSE** (the conflict sentinel); **a clause with non-empty `terms` but no `mask` bit set represents TRUE** (an empty conjunction). At the `Expression` level, no clauses ⇒ FALSE and any empty-conjunction clause ⇒ TRUE. `Clause::is_true`/`is_false` and `Expression::is_true`/`is_false` encapsulate these.
- Variables are referenced internally by **bit index**, not name. `VariableTable` (`beast/src/variable_table.rs`) maps arbitrary user-supplied names to indices and restores them on output. `Expression` owns an `Rc<VariableTable>` so the serializers stay parameterless; table-less expressions (e.g. in unit tests) fall back to synthesized `"x"+index` names.

### The two QM-related representations

- `Implicant { v: Term, d: Term }` (in `quine-mccluskey/src/lib.rs`): `v` = fixed bit values, `d` = don't-care mask. `Term` is `u32`, which caps the design at **`MAX_VARIABLES = 32`** distinct variables.
- The bridge between the `Clause` (terms/mask) representation and the `Term`/`Implicant` (bitmask) representation lives in `beast/src/lib.rs`: `expression_to_minterms` (DNF → ON-set minterms over `num_vars`) and `implicant_to_clause` (selected implicant → clause).

## Locked decisions (do not re-litigate; see `plan.md` §0)

- **I/O format is JsonLogic** (operator-as-key objects, one key per node) for both input and output: `to_json` emits objects (`{"or":[...]}`, negation `{"!":[...]}`, variable `{"var":"name"}`), with single-element `and`/`or` collapsed to the bare child and constants emitted as JSON `true`/`false`.
- **Future enhancement (out of scope for the current plan):** an algebraic I/O mode — emitting algebraic output (via a CLI flag using `to_algebraic`) and accepting algebraic input (via a new algebraic parser, which does not exist yet). See `plan.md` §7. For now, `to_algebraic` is an internal/comparison helper only.
- **Variable names are arbitrary** user-supplied strings, mapped to bit indices and preserved in output.
- **Accepted operators**: standard `and`, `or`, `!`, `var`, and boolean literals `true`/`false`, PLUS non-standard Beast extensions `xor`, `nand`, `nor`. The extensions are **input-only** — desugared to `and`/`or`/`!` during conversion and never emitted on output. All other JsonLogic operators (comparison/numeric/array/string) are rejected.

## Gotchas

- `to_json` and `to_algebraic` must stay consistent: both emit `true`/`false` for constants and both drop FALSE-sentinel clauses. When changing one, change the other.
- The TRUE/FALSE clause encoding is subtle (see the core data model above): empty `terms` = FALSE, non-empty `terms` with all-false `mask` = TRUE. Use `is_true`/`is_false` rather than checking `terms.is_empty()` directly. `Expression::inverse` folds from the TRUE identity, so `!false == true` and `!true == false`.
- `Clause`'s parallel `terms`/`mask` vectors can differ in length between clauses (each is only as wide as the highest variable index it uses); the algebra handles mismatched widths. When iterating up to `num_vars` (e.g. in `expression_to_minterms`), guard with `i < mask.len()`.
- Quine–McCluskey selection uses Petrick's method but falls back to a greedy cover when the estimated product size exceeds `PETRICK_PRODUCT_LIMIT` — exact minimality is only guaranteed below that threshold.
