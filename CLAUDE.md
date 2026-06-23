# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project status

Beast is **functional**: the full input → DNF → minimized DNF → output pipeline works end to end in **both** an algebraic syntax (the default) and JsonLogic. `simplify` / `simplify_json` / `simplify_algebraic` are implemented, the Quine–McCluskey `minimize` does prime-implicant generation *and* selection (essential PIs + Petrick's method with a greedy fallback for large charts), and both front ends — the JsonLogic converter (`beast/src/converter.rs`) and the algebraic tokenizer/parser (`beast/src/algebraic.rs`) — plus the variable-name table (`beast/src/variable_table.rs`) exist. The CLI selects input syntax with `--in`/`-i` and output syntax with `--out`/`-o` (`algebraic`|`json`; in defaults to algebraic, out defaults to in), and the algebraic output style with `--style`/`-s` (`common`|`code`|`logic`, default `common`). `cargo build`, `cargo test`, and `cargo clippy` are clean.

One intentional design choice: `simplify_json` / `simplify_algebraic` return `Result<Expression, String>` (not `Expression`) so the CLI can surface parse errors (unknown operator, bad arity, syntax errors, >32 variables) on stderr.

## Build & run

Requires a Rust toolchain (Cargo). This is a Cargo **workspace** of two crates — `beast` (data model, converter, CLI) and `quine-mccluskey` (the simplifier library). There are **no external dependencies** (JSON is `beast`'s in-crate `json` module), so builds work fully offline.

```sh
cargo build                              # build the whole workspace
cargo run -p beast -- 'ab + a!b'         # algebraic input (default), algebraic output
cargo run -p beast -- --in json '<jsonlogic-expression>'   # JSON input
```

Run the CLI (binary is named `beast`). Input syntax defaults to algebraic; `--in`/`--out` (`algebraic`|`json`) select each side, output defaulting to the input syntax:

```sh
./target/debug/beast 'ab + a!b'                  # algebraic in/out (the default)
./target/debug/beast --out json 'ab + a!b'       # algebraic in, JSON out
./target/debug/beast --in json < expression.json # JSON in (from stdin)
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

Beast is a thin CLI wrapping two conceptual libraries; data flows: **input → DNF → minimized DNF → output**, where input/output are each either algebraic or JsonLogic. The `beast` crate owns input parsing and output serialization; the actual simplification is delegated to the `quine-mccluskey` crate.

Workspace layout — two crates:

- `beast/` — `src/lib.rs` (data model, algebra, `simplify*`), `src/converter.rs` (JsonLogic parser), `src/algebraic.rs` (algebraic parser), `src/json.rs` (in-crate JSON), `src/main.rs` (CLI). Depends on `quine-mccluskey`.
- `quine-mccluskey/` — `src/lib.rs` (the simplifier; lib name `quine_mccluskey`). Re-exported from `beast` as `beast::quine_mccluskey`.

1. **Parsers** — two interchangeable front ends that both produce an `Expression` in **disjunctive normal form**: the JsonLogic **converter** (`beast/src/converter.rs`, `to_dnf`) and the **algebraic** tokenizer/parser (`beast/src/algebraic.rs`, `parse_algebraic`). Both reuse the algebra operators — `BitOr` (`|`) concatenates product terms, `BitAnd` (`&`) distributes (product of sums → sum of products), and `Expression::inverse` applies De Morgan — so any input collapses to DNF as it is built. They pair with the serializers `Expression::to_json` and `Expression::to_algebraic`; any input form can be emitted in either output form.
2. **Simplifier** (`quine-mccluskey` crate, `minimize`): input and output are both DNF. The `beast` crate converts the DNF product-term set to minterms (`expression_to_minterms`), runs Quine–McCluskey, and converts the selected prime implicants back to product terms (`implicant_to_product_term`).

### Core data model (`beast/src/lib.rs`)

- `Expression` = OR of `ProductTerm`s (a `Vec<ProductTerm>`), i.e. DNF.
- `ProductTerm` = AND of literals, stored as two parallel `Vec<bool>`: `terms` (literal sign: true = unnegated, false = negated) and `mask` (true = the variable is present in this product term).
- Constant conventions (used throughout the algebra and serialization): **a product term with empty `terms` represents FALSE** (the conflict sentinel); **a product term with non-empty `terms` but no `mask` bit set represents TRUE** (an empty conjunction). At the `Expression` level, no product terms ⇒ FALSE and any empty-conjunction product term ⇒ TRUE. `ProductTerm::is_true`/`is_false` and `Expression::is_true`/`is_false` encapsulate these.
- Variables are referenced internally by **bit index**, not name. `VariableTable` (`beast/src/variable_table.rs`) maps arbitrary user-supplied names to indices and restores them on output. `Expression` owns an `Rc<VariableTable>` so the serializers stay parameterless; table-less expressions (e.g. in unit tests) fall back to synthesized `"x"+index` names.

### The two QM-related representations

- `Implicant { v: Term, d: Term }` (in `quine-mccluskey/src/lib.rs`): `v` = fixed bit values, `d` = don't-care mask. `Term` is `u32`, which caps the design at **`MAX_VARIABLES = 32`** distinct variables.
- The bridge between the `ProductTerm` (terms/mask) representation and the `Term`/`Implicant` (bitmask) representation lives in `beast/src/lib.rs`: `expression_to_minterms` (DNF → ON-set minterms over `num_vars`) and `implicant_to_product_term` (selected implicant → product term).

## Locked decisions (do not re-litigate)

- **Two I/O formats: algebraic (default) and JsonLogic**, selected per side by the CLI `--in`/`--out` flags. JsonLogic is operator-as-key objects, one key per node: `to_json` emits objects (`{"or":[...]}`, negation `{"!":[...]}`, variable `{"var":"name"}`), with single-element `and`/`or` collapsed to the bare child and constants emitted as JSON `true`/`false`.
- **Algebraic syntax** (`beast/src/algebraic.rs`, `to_algebraic`): operators have multiple spellings — and `& * . ∧ ⋅ · ×` or juxtaposition, or `| + ∨`, xor `^ ⊕ ⊻`, not (prefix) `~ - ¬ !` or a postfix combining overbar `\u{0305}` that must immediately follow a single-letter variable (`a\u{0305}` = `!a`; an error anywhere else), parens `( )`, constants `1`/`0`. A single ASCII letter is one variable, so adjacent letters are an implicit AND (`ab` = `a & b`); a multi-character name must be prefixed with `$` and continues while the next char is in `[0-9a-zA-Z_]` (e.g. `$velocity * $pressure` = two vars; `a$bc + d` = `a` AND var `bc`, OR `d`). Whitespace delimits but is otherwise ignored. Precedence, tightest first: `not`, `and`, then `or`/`xor` (one shared left-associative tier). Output is emitted by `Expression::to_algebraic_styled(AlgebraicStyle)` (`to_algebraic()` = the default `Common` style) in one of three styles selected by the CLI `--style` flag — `common` (`+` OR, juxtaposition AND, overbar / `~` NOT), `code` (`| & !`), `logic` (`∨ ∧ ¬`) — with constants always `1`/`0`. Every style emits a `$` prefix on multi-character names (single ASCII letters stay bare), so output in any style re-parses to the same `Expression` (`every_style_round_trips_to_the_same_expression`). In `common`, a space is inserted only where a multi-character name is immediately followed by a single letter (a `$`-name otherwise greedily eats trailing identifier chars).
- **Variable names are arbitrary** user-supplied strings, mapped to bit indices and preserved in output.
- **Accepted operators**: standard `and`, `or`, `!`, `var`, and boolean literals `true`/`false`, PLUS non-standard Beast extensions `xor`, `nand`, `nor`. The extensions are **input-only** — desugared to `and`/`or`/`!` during conversion and never emitted on output. All other JsonLogic operators (comparison/numeric/array/string) are rejected.

## Gotchas

- `to_json` and `to_algebraic` both drop FALSE-sentinel product terms but render constants differently on purpose: `to_json` emits JSON `true`/`false`, `to_algebraic` emits `1`/`0` (so algebraic output re-parses as algebraic input). Keep the product-term-dropping behavior in sync; the constant spellings are format-specific.
- The TRUE/FALSE product-term encoding is subtle (see the core data model above): empty `terms` = FALSE, non-empty `terms` with all-false `mask` = TRUE. Use `is_true`/`is_false` rather than checking `terms.is_empty()` directly. `Expression::inverse` folds from the TRUE identity, so `!false == true` and `!true == false`.
- `ProductTerm`'s parallel `terms`/`mask` vectors can differ in length between product terms (each is only as wide as the highest variable index it uses); the algebra handles mismatched widths. When iterating up to `num_vars` (e.g. in `expression_to_minterms`), guard with `i < mask.len()`.
- Quine–McCluskey selection uses Petrick's method but falls back to a greedy cover when the estimated product size exceeds `PETRICK_PRODUCT_LIMIT` — exact minimality is only guaranteed below that threshold.
