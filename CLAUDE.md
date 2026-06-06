# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project status

Beast is an early-stage **work in progress**, not a working tool yet. The DNF data model and boolean-algebra operators exist, but the headline feature does not: `simplify` / `simplify_json` are stubs (they return an empty `Expression`, which serializes to `["or",[]]`), the Quine–McCluskey `minimize` is unfinished (no prime-implicant selection), and there is no JsonLogic converter yet.

**`plan.md` is the authoritative roadmap.** It is written for agent consumption with dependency-ordered tasks (`ID / deps / files / action / acceptance`), a current-state inventory, locked decisions, and a definition of done. Read it before making changes and keep it in sync when scope changes.

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
cargo clippy                # lints (clean; one intentional-bug line is annotated)
```

## Architecture (big picture)

Beast is a thin CLI wrapping two conceptual libraries; data flows: **JsonLogic → DNF → minimized DNF → JsonLogic**. The `beast` crate owns input parsing and output serialization; the actual simplification is delegated to the `quine-mccluskey` crate.

Workspace layout — two crates:
- `beast/` — `src/lib.rs` (data model, algebra, `simplify*`), `src/json.rs` (in-crate JSON), `src/main.rs` (CLI). Depends on `quine-mccluskey`.
- `quine-mccluskey/` — `src/lib.rs` (the simplifier; lib name `quine_mccluskey`). Re-exported from `beast` as `beast::quine_mccluskey`.

1. **Converter** (to be written, Phase C): parses an arbitrary JsonLogic boolean expression into an `Expression` in **disjunctive normal form**. It works by recursive descent that reuses the existing algebra operators — `BitOr` (`|`) concatenates clauses, `BitAnd` (`&`) distributes (product of sums → sum of products), and `Expression::inverse` applies De Morgan — so any input tree collapses to DNF as it is built.
2. **Simplifier** (`quine-mccluskey` crate, to be finished, Phase D): input and output are both DNF. It converts the DNF clause set to minterms, runs Quine–McCluskey, and converts the selected prime implicants back to clauses.

### Core data model (`beast/src/lib.rs`)

- `Expression` = OR of `Clause`s (a `Vec<Clause>`), i.e. DNF.
- `Clause` = AND of literals, stored as two parallel `Vec<bool>`: `terms` (literal sign: true = unnegated, false = negated) and `mask` (true = the variable is present in this clause).
- Convention (used throughout the algebra and serialization): **a clause with no terms represents FALSE**.
- Variables are referenced internally by **bit index**, not name. A name↔index table (to be added) maps arbitrary user-supplied names to indices and restores them on output. The current serializers hardcode synthesized `"x"+index` names — these must become table lookups.

### The two QM-related representations

- `Implicant { v: Term, d: Term }` (in `quine-mccluskey/src/lib.rs`): `v` = fixed bit values, `d` = don't-care mask. `Term` is `u32`, which caps the design at **`MAX_VARIABLES = 32`** distinct variables.
- The bridge between the `Clause` (terms/mask) representation and the `Term`/`Implicant` (bitmask) representation does not exist yet and is required to connect the two libraries (Phase D in `plan.md`).

## Locked decisions (do not re-litigate; see `plan.md` §0)

- **I/O format is JsonLogic** (operator-as-key objects, one key per node) for both input and output. The current `to_json` emits a non-JsonLogic array form (`["or",[...]]`) and must be rewritten to objects (`{"or":[...]}`, negation `{"!":[...]}`, variable `{"var":"name"}`).
- **Future enhancement (out of scope for the current plan):** an algebraic I/O mode — emitting algebraic output (via a CLI flag using `to_algebraic`) and accepting algebraic input (via a new algebraic parser, which does not exist yet). See `plan.md` §7. For now, `to_algebraic` is an internal/comparison helper only.
- **Variable names are arbitrary** user-supplied strings, mapped to bit indices and preserved in output.
- **Accepted operators**: standard `and`, `or`, `!`, `var`, and boolean literals `true`/`false`, PLUS non-standard Beast extensions `xor`, `nand`, `nor`. The extensions are **input-only** — desugared to `and`/`or`/`!` during conversion and never emitted on output. All other JsonLogic operators (comparison/numeric/array/string) are rejected.

## Gotchas

- `to_json` (both `Expression` and `Clause`) currently emits the non-JsonLogic **array** form (`["or",[...]]`, `["and",[...]]`, `["not", "x0"]`). This is the root cause of the wrong output shape; Phase B rewrites it to operator-as-key objects.
- The two serializers diverge today: `Expression::to_algebraic` filters empty (false) clauses but `Expression::to_json` does not. Keep them consistent.
- `Expression::inverse` is currently broken — it folds from an empty (FALSE) accumulator instead of the TRUE identity, so it returns FALSE for *every* input (`plan.md` §4, BUG-10). The converter (Phase C) uses it for `!`/`nand`/`nor`/`xor`, so fix it (alongside the TRUE-constant decision in B3) before relying on it. A characterization test pins the current behavior.
- `quine-mccluskey/src/lib.rs` has a known self-comparison bug in `differ_by_one_bit` (`i0.d == i0.d`, isolated behind a named binding with `#[allow(clippy::eq_op)]`) and an off-by-one `Round` sizing / minterm-0 underflow. See `plan.md` §4 (BUG-2, BUG-3) for the fixes.
