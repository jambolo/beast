# Beast — Completion Plan

> Format: optimized for AI/agent consumption. Imperative, explicit, dependency-ordered.
> Each task has: `ID`, `deps`, `files`, `action`, `acceptance`. Do tasks in dependency order.
> Treat `acceptance` as the definition of done; do not mark a task complete until it passes.

## 0. Authoritative constraints (do not violate)

- C0.1 — JSON input AND output format is **JsonLogic** (jsonlogic.com): operator-as-key objects, one key per node. Standard operators in scope: `and`, `or`, `!`, `var`, plus boolean literals `true`/`false`. (An algebraic I/O mode is an explicit FUTURE enhancement, NOT in scope here; see §7.)
- C0.1a — NON-STANDARD EXTENSIONS: the converter ALSO accepts `xor`, `nand`, `nor` on INPUT. These are NOT part of the JsonLogic spec (a standard JsonLogic engine will not understand them); document them clearly as Beast extensions. They are INPUT-ONLY — desugared during conversion and NEVER emitted on output (output is minimized DNF over `and`/`or`/`!`/`var`). Semantics (n-ary): `xor` = true iff an odd number of args are true; `nand` = `!(and args)`; `nor` = `!(or args)`.
- C0.2 — Variable names are **arbitrary user-supplied strings**, mapped to internal bit indices via a shared name↔index table. Original names MUST be restored on output.
- C0.3 — Architecture is two libraries wrapped by a CLI (the Converter, serializer, and CLI live in the `beast` crate; the Simplifier is the `quine-mccluskey` crate). End-to-end flow: `beast` parses JsonLogic input → DNF, delegates simplification to `quine-mccluskey`, then serializes the minimized DNF back out as JsonLogic.
  - **Converter** (`beast`): arbitrary JsonLogic boolean expression → `Expression` in **DNF**.
  - **Simplifier** (`quine-mccluskey`): `Expression` in DNF → minimized `Expression` in DNF (Quine–McCluskey).
  - **Serializer** (`beast`): minimized `Expression` → JsonLogic (`to_json`). (`to_algebraic` exists as an internal/comparison helper; it is not a user-facing output path in the current plan.)
- C0.4 — Internal canonical form is **DNF** (disjunctive normal form): `Expression` = OR of `Clause`s; `Clause` = AND of literals.
- C0.5 — Variable count is bounded by `MAX_VARIABLES = 32` (`Term` is `u32`). Reject inputs exceeding this with a clear error.
- C0.6 — Keep the build portable and offline: Beast is a Cargo workspace that builds with `cargo build` on any supported platform. Do not add steps that require network access or platform-specific tooling.
- C0.7 — Keep the workspace **free of external dependencies**: it is a Cargo workspace of two crates — `beast` (data model, converter, CLI) and `quine-mccluskey` (the simplifier library, which `beast` depends on by path). JSON is `beast`'s in-crate `json` module and tests use Rust's built-in harness. Do not add third-party crates without strong justification.

## 1. Current-state inventory (ground truth as of this plan)

| File | State |
| --- | --- |
| `beast/src/lib.rs` | Data model (`Expression`, `Clause`) + algebra (`Clause::and_assign`, `BitOr`/`BitAnd`/`BitOrAssign`/`BitAndAssign` for `Expression`, `Expression::inverse`) + serializers (`to_json`, `to_algebraic`) implemented. `simplify` and `simplify_json` are STUBS returning an empty `Expression`. `to_json` emits a non-JsonLogic array form `["or",[...]]`; hardcodes `"x"+index` names; does NOT filter empty clauses. `to_algebraic` DOES filter empty clauses (inconsistent with `to_json`). `Expression::inverse` returns FALSE for every input (BUG-10). Re-exports the `quine-mccluskey` crate as `beast::quine_mccluskey`. |
| `quine-mccluskey/src/lib.rs` | `minimize()` UNFINISHED (returns empty `Vec`). Prime-implicant generation present; **selection absent**. BUGS: `differ_by_one_bit` self-compares `i0.d == i0.d` (should be `i0.d == i1.d`, isolated behind a `#[allow(clippy::eq_op)]` binding); minterm `0` underflows `r[number_of_one_bits(t) - 1]`; `Round` sized `MAX_VARIABLES` (32) but needs 33 slots for one-counts 0..=32. |
| `beast/src/json.rs` | In-crate JSON value type + recursive-descent parser + compact serializer. Complete; provides all JSON I/O for the crate. |
| `beast/src/main.rs` | CLI: reads first arg or stdin, parses JSON, calls `simplify_json`, prints `to_json`. Errors → stderr, exit nonzero. Output is always JsonLogic. |
| `Cargo.toml` (root) | Cargo **workspace** with members `beast` and `quine-mccluskey`. |
| `beast/Cargo.toml` | Package `beast`, edition 2021, `lib` + `bin` both named `beast`; one path dependency on `quine-mccluskey`. |
| `quine-mccluskey/Cargo.toml` | Package `quine-mccluskey` (lib name `quine_mccluskey`), edition 2021, zero dependencies. |

## 2. Target type/API contracts

Define these precisely before implementing dependents.

- T2.1 — `pub type Term = u32;` and `pub struct Implicant { pub v: Term, pub d: Term }` (v = fixed bit values, d = don't-care mask). Already in `quine-mccluskey/src/lib.rs`.
- T2.2 — Variable table (new, e.g. `beast/src/variable_table.rs`):
  ```rust
  pub struct VariableTable {
      names: Vec<String>,                                 // index -> name
      indices: std::collections::HashMap<String, usize>,  // name -> index
  }

  impl VariableTable {
      pub fn index_of(&mut self, name: &str) -> Result<usize, String>; // inserts if absent; Err if > MAX_VARIABLES
      pub fn name_of(&self, index: usize) -> &str;                     // index -> original name
      pub fn len(&self) -> usize;
      pub fn is_empty(&self) -> bool;
  }
  ```
- T2.3 — `Expression` gains access to names for serialization. Decision (default): `Expression` holds the table (e.g. an `Rc<VariableTable>`) so the public `to_json()` / `to_algebraic()` stay parameterless. Pick ONE approach and apply consistently.
- T2.4 — Converter API (Library A, new `beast/src/converter.rs`):
  ```rust
  /// Parses a JsonLogic boolean expression into a DNF Expression, building the variable table.
  pub fn to_dnf(json: &Json, table: &mut VariableTable) -> Result<Expression, String>;
  ```
- T2.5 — Simplifier API (Library B, `quine-mccluskey/src/lib.rs`):
  ```rust
  pub fn minimize(min_terms: &[Term], dont_cares: &[Term], num_variables: usize) -> Vec<Implicant>;
  ```
  NOTE: add the `num_variables` parameter; the return type is already `Vec<Implicant>` (carries the don't-care mask).
- T2.6 — `simplify(&Expression) -> Expression` performs DNF→minterms→`minimize`→DNF. `simplify_json(&Json) -> Expression` performs `to_dnf` → `simplify`.

## 3. Phased task list

### Phase A — Baseline (already satisfied by the project)

The crate builds (`cargo build`), `cargo test` / `cargo clippy` are clean, and the CLI runs. Earlier build/portability/compile-blocker items are obsolete. The only carried-over logic defect is the self-comparison in `differ_by_one_bit` (BUG-2), fixed as part of D2.

- A1 — DONE: crate builds via `cargo build`; tests run via `cargo test` (built-in harness, no external framework).
- A2 — see D2: fix `differ_by_one_bit` to `(i0.d == i1.d) && is_power_of_2(i0.v ^ i1.v)`.

### Phase B — Variable table + JsonLogic serialization (output side)

- B1 — deps: none. files: `beast/src/variable_table.rs` (new), `beast/src/lib.rs` (add `mod`). action: implement `VariableTable` per T2.2; return `Err` when an index would exceed `MAX_VARIABLES`. acceptance: unit test — duplicate names → same index; distinct names → increasing indices; 33rd distinct name returns `Err`.
- B2 — deps: B1, T2.3. files: `beast/src/lib.rs`. action: give `Expression` access to a `VariableTable` (per T2.3). Rewrite `Clause::to_json` and `Expression::to_json` to emit **JsonLogic**:
  - positive literal → `{"var": name}`
  - negative literal → `{"!": [{"var": name}]}`
  - clause (AND of literals) → `{"and": [ ... ]}`; single-literal clause → emit the literal directly (no wrapping `and`).
  - expression (OR of clauses) → `{"or": [ ... ]}`; single-clause → emit the clause directly.
  - constant TRUE → JSON `true`; constant FALSE → JSON `false`.
  - filter empty/false clauses consistently with `to_algebraic`.
  acceptance: `to_json` and `to_algebraic` describe the SAME formula for any `Expression`; golden tests in the C-phase.
- B3 — deps: B2. files: `beast/src/lib.rs`. action: define and document the constant-expression representation (e.g. empty clause set ⇒ FALSE; a single empty-mask clause ⇒ TRUE) and ensure `to_json`/`to_algebraic` honor it. acceptance: TRUE→`true`, FALSE→`false` in JSON; documented in `lib.rs`.

### Phase C — Converter library (input side: JsonLogic → DNF)

- C1 — deps: B1, BUG-10. files: `beast/src/converter.rs` (new), `beast/src/lib.rs` (add `mod`). NOTE: `!`/`nand`/`nor`/`xor` below rely on `Expression::inverse`, which is currently broken (BUG-10) — fix that first. action: implement `to_dnf(&Json, &mut VariableTable) -> Result<Expression, String>` as a recursive descent over JsonLogic:
  - `{"var": name}` → single-literal DNF `Expression` (positive).
  - `{"!": [x]}` → `to_dnf(x)?.inverse()` (reuse `Expression::inverse`).
  - `{"and": [a,b,...]}` → fold with `&` (distributes to DNF).
  - `{"or": [a,b,...]}` → fold with `|`.
  - `{"nand": [a,b,...]}` → `(and-fold).inverse()` (desugar = NOT of AND-fold).
  - `{"nor": [a,b,...]}` → `(or-fold).inverse()` (desugar = NOT of OR-fold).
  - `{"xor": [a,b,...]}` → desugar to odd-parity over args: fold pairwise `xor(p,q) = (p & !q) | (!p & q)`, i.e. `acc = (acc & next.inverse()) | (acc.inverse() & next)`. Result must be DNF.
  - literal `true`/`false` → constant TRUE/FALSE `Expression`.
  - Validate: each node has exactly one operator key; unknown operator → `Err` with message. n-ary `and`/`or`/`xor`/`nand`/`nor` accept ≥1 arg (1-arg `xor` = arg; 1-arg `nand`/`nor` = `!arg`); `!` accepts exactly 1; `var` value is a string. The accepted set is `{var, !, and, or, xor, nand, nor}` plus boolean literals.
  acceptance: unit tests; `to_dnf` output passes a DNF-shape invariant check (OR of ANDs of literals only).
- C2 — PARTIAL: `Expression::inverse` is already a `&self` method, so the prior non-const inversion blocker no longer applies. It still has a correctness bug (BUG-10: returns FALSE for every input) that must be fixed before C1 relies on it.
- C3 — deps: C1. files: `beast/src/lib.rs`. action: implement `simplify_json(&Json)` = `to_dnf` then `simplify` then return. acceptance: end-to-end CLI test (Phase E).

### Phase D — Simplifier library (DNF → minimized DNF via Quine–McCluskey)

- D1 — deps: none. files: `quine-mccluskey/src/lib.rs`. action: fix `Round` sizing to `MAX_VARIABLES + 1` and index by `number_of_one_bits(t)` (no `-1`), so minterm `0` lands in slot 0. acceptance: minimizing `{0}` does not panic and yields a correct prime implicant.
- D2 — deps: D1. files: `quine-mccluskey/src/lib.rs`. action: fix `differ_by_one_bit` (BUG-2) and verify prime-implicant generation (combine rounds, `remove_combined_implicants`, dedup) produces the correct prime-implicant set; confirm don't-care handling. acceptance: known textbook case (minterms {0,1,2,5,6,7}) yields the expected prime implicants.
- D3 — deps: D2. files: `quine-mccluskey/src/lib.rs`. action: implement **prime-implicant selection** (currently absent):
  1. Build prime-implicant chart (which PIs cover which minterms).
  2. Select all **essential** prime implicants (minterm covered by exactly one PI).
  3. Cover remaining minterms via **Petrick's method** (exact) or a documented greedy/branch-and-bound fallback for larger cases.
  Return the selected `Vec<Implicant>` (T2.5).
  acceptance: textbook case {0,1,2,5,6,7} → minimal cover matching a known reference; result count is minimal.
- D4 — deps: C1,D3. files: `beast/src/lib.rs` (new helpers). action: implement DNF↔minterm bridge:
  - `Expression`(DNF) + `num_vars` → ON-set minterms: expand each `Clause` over its free (unmasked) variables across all `num_vars` positions; union the minterms.
  - selected `Implicant`s → `Expression`(DNF): each `Implicant` → `Clause` where bit i is a literal iff `(d & (1 << i)) == 0`, sign from `v`, else masked out.
  acceptance: round-trip `Expression → minterms → Implicants → Expression` preserves the truth table.
- D5 — deps: D3,D4. files: `beast/src/lib.rs`. action: implement `simplify(&Expression)`: derive `num_vars` from the variable table, compute minterms, call `minimize`, convert back to `Expression`, attach the variable table. Handle constants: all minterms ⇒ TRUE; no minterms ⇒ FALSE. acceptance: `(a&b)|(a&!b)` ⇒ `a`; `a|!a` ⇒ TRUE; `a&!a` ⇒ FALSE.

### Phase E — Integration, CLI, tests, docs

- E1 — deps: C3,D5. files: `#[cfg(test)]` modules and/or a `tests/` directory. action: write real tests:
  - VariableTable (B1 cases).
  - Converter: parse + DNF-shape invariants + error cases (unknown op, multi-key node, bad arity). Include `xor`/`nand`/`nor` desugaring: verify truth-table equivalence (e.g. `xor[a,b]` ≡ `(a&!b)|(!a&b)`, `nand` ≡ `!(a&b)`, `nor` ≡ `!(a|b)`) and n-ary xor parity.
  - QuineMcCluskey: textbook minimization cases + edges (minterm 0, all-ones, single var, empty).
  - Serializer: golden JsonLogic strings + `to_json`/`to_algebraic` agreement.
  - End-to-end: `simplify_json` golden input→output pairs incl. constants.
  acceptance: `cargo test` all green.
- E2 — deps: C3,D5. files: `beast/src/main.rs`. action: confirm CLI wiring (arg/stdin → `simplify_json` → `to_json` → stdout; errors → stderr, exit nonzero). Add `> MAX_VARIABLES` and parse-error messages. acceptance: shell test — README example input produces a valid simplified JsonLogic output; malformed input exits nonzero with a message on stderr.
- E3 — deps: E1,E2. files: `README.md`. action: reconcile README examples with actual output; if `simplify` of the README example yields `{"var":"a"}`, keep; otherwise update the example. acceptance: README examples reproduce verbatim when run.
- E4 — deps: E1. files: doc comments in `src/`. action: ensure rustdoc comments are accurate for new/changed APIs. acceptance: `cargo doc` builds without warnings about the public API (optional/best-effort).

## 4. Cross-cutting bug fixes (tracked, fold into phases above)

- BUG-2 (D2): `differ_by_one_bit` self-compare `i0.d == i0.d` (should be `i0.d == i1.d`). — wrong combination logic.
- BUG-3 (D1): minterm-0 index underflow + `Round` off-by-one sizing.
- BUG-4 (B2): `to_json` non-JsonLogic array form; must emit operator-as-key objects.
- BUG-5 (B2): `to_json` does not filter empty/false clauses while `to_algebraic` does. — serializer divergence.
- BUG-6 (B3): constant/empty-clause semantics (a clause with no terms means FALSE; ensure an empty `and` is not misread as TRUE).
- BUG-7 (B2): hardcoded `"x"+index` names; replace with `VariableTable` lookup.
- BUG-10 (C2/B3): `Expression::inverse` folds from an empty (FALSE) accumulator instead of the TRUE identity, so it returns FALSE for every input. Depends on the TRUE-constant representation (B3); must be fixed before the converter (C1) relies on it for `!`/`nand`/`nor`/`xor`.

(Three defects tracked in earlier iterations of this plan — a missing-`typename` compile error, exception-slicing in the CLI, and a non-const inversion operator — no longer apply to the current design and have been dropped; BUG-8/BUG-9 were among them, hence the numbering gap.)

## 5. Definition of done (whole project)

- DOD-1 — `cargo build` succeeds clean (no warnings); `cargo clippy` is clean.
- DOD-2 — `cargo test` passes; coverage spans converter, simplifier, serializer, CLI, edge cases.
- DOD-3 — `echo '<jsonlogic>' | beast` returns minimal-DNF JsonLogic for arbitrary valid input; constants return `true`/`false`.
- DOD-4 — Round-trip property holds: for random expressions over ≤8 vars, `simplify` preserves the truth table and output is valid JsonLogic in DNF.
- DOD-5 — Variable names round-trip unchanged; >32 distinct vars rejected with a clear error.
- DOD-6 — README examples reproduce verbatim.

## 6. Open decisions (resolve before/within the phase that needs them)

- Q1 (T2.3) — Does `Expression` own the `VariableTable` (parameterless `to_json`) or is it passed in? Default: own it (e.g. `Rc<VariableTable>`).
- Q2 (D3) — Petrick's method (exact, can be exponential) vs greedy (fast, may be non-minimal) for non-essential PI cover. Default: Petrick below an N-PI threshold, greedy fallback above it; document the threshold.
- Q3 (C1) — RESOLVED: converter accepts boolean operators only — standard `and`, `or`, `!`, `var`, `true`, `false`, PLUS the non-standard Beast extensions `xor`, `nand`, `nor` (input-only, desugared, never emitted). Reject every other operator (comparison/numeric/array/string families) with a clear error.
- Q4 (E2) — Output formatting: compact vs pretty JSON; stable key ordering for golden tests. Default: compact, deterministic ordering.

## 7. Future enhancements (explicitly OUT of scope for this plan)

- FE-1 — **Algebraic I/O mode.** The current plan is JsonLogic-only for both input and output. A future enhancement adds algebraic form on BOTH sides: emitting algebraic *output* (CLI flag selecting `to_algebraic` over `to_json`) and accepting algebraic *input* (a new tokenizer + parser for the algebraic syntax, wired into the CLI as an input-format selector). The `to_algebraic` serializer already exists as an internal helper and would become a user-facing output path; the algebraic input parser does not exist yet. Do not implement as part of the current phases.
