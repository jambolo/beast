# Beast — Completion Plan

> Format: optimized for AI/agent consumption. Imperative, explicit, dependency-ordered.
> Each task has: `ID`, `deps`, `files`, `action`, `acceptance`. Do tasks in dependency order.
> Treat `acceptance` as the definition of done; do not mark a task complete until it passes.

## 0. Authoritative constraints (do not violate)

- C0.1 — JSON input AND output format is **JsonLogic** (jsonlogic.com): operator-as-key objects, one key per node. Standard operators in scope: `and`, `or`, `!`, `var`, plus boolean literals `true`/`false`.
- C0.1a — NON-STANDARD EXTENSIONS: the converter ALSO accepts `xor`, `nand`, `nor` on INPUT. These are NOT part of the JsonLogic spec (a standard JsonLogic engine will not understand them); document them clearly as Beast extensions. They are INPUT-ONLY — desugared during conversion and NEVER emitted on output (output is minimized DNF over `and`/`or`/`!`/`var`). Semantics (n-ary): `xor` = true iff an odd number of args are true; `nand` = `!(and args)`; `nor` = `!(or args)`.
- C0.2 — Variable names are **arbitrary user-supplied strings**, mapped to internal bit indices via a shared name↔index table. Original names MUST be restored on output.
- C0.3 — Architecture is two libraries wrapped by a CLI:
  - **Converter**: arbitrary JsonLogic boolean expression → `Expression` in **DNF**.
  - **Simplifier**: `Expression` in DNF → minimized `Expression` in DNF (Quine–McCluskey).
- C0.4 — Internal canonical form is **DNF** (disjunctive normal form): `Expression` = OR of `Clause`s; `Clause` = AND of literals.
- C0.5 — Variable count is bounded by `MAX_VARIABLES = 32` (Term is `uint32_t`). Reject inputs exceeding this with a clear error.
- C0.6 — Build must be portable (Linux/macOS/Windows). Remove the MSVC-only hardcoded dependency.
- C0.7 — Do not introduce dependencies beyond `nlohmann_json` and a portable test framework (Catch2 or GoogleTest).

## 1. Current-state inventory (ground truth as of this plan)

| File | State |
| --- | --- |
| `Beast/include/Beast/Beast.h` | Data model + API declared. `simplify` declared. Mojibake char in comment line ~12. `operator~` non-const. |
| `Beast/Beast.cpp` | `simplify(json)` and `simplify(Expression)` are STUBS returning empty `Expression`. Algebra ops (`Clause::&=`, `Expression::\|=`, `&=`, `~`) implemented. `toJson` emits non-JsonLogic array form `["or",[...]]`; hardcodes `"x"+index` names; does NOT filter empty clauses. `toAlgebraic` DOES filter empty clauses (inconsistent with toJson). |
| `Beast/QuineMcCluskey.cpp` | `minimize()` UNFINISHED (`// not finished`, returns empty). No prime-implicant selection. Free function, no header, not called anywhere. BUGS: `removeDuplicates` missing `typename` (compile error); `differByOneBit` self-compares `i0.d==i0.d` (should be `i0.d==i1.d`); minterm `0` underflows `r[numberOfOneBits(t)-1]`; Round sized `MAX_VARIABLES` (32) but needs 33 slots for one-counts 0..32. |
| `main.cpp` | Reads arg or stdin, parses JSON, calls `simplify`, prints `toJson`. Catches `std::exception` by value (slicing). |
| `CMakeLists.txt` (root) | Builds `main` + `Beast` lib. Does NOT `add_subdirectory(unit-test)`. |
| `Beast/CMakeLists.txt` | Builds lib from `Beast.cpp`, `QuineMcCluskey.cpp`. `QuineMcCluskey.h` referenced but commented out. |
| `unit-test/*` | All tests are `Assert::Fail("Not yet implemented")` stubs using MSVC CppUnitTest with hardcoded path `D:/Program Files (x86)/VC/UnitTest`. Not portable, not wired into build. |

Confirmed compile failures (via `c++ -std=c++17 -fsyntax-only Beast/QuineMcCluskey.cpp`): missing `typename` (2x), tautological self-compare warning.

## 2. Target type/API contracts

Define these precisely before implementing dependents.

- T2.1 — `using Term = uint32_t;` and `struct Implicant { Term v; Term d; };` (v = fixed bit values, d = don't-care mask). Move to `Beast/QuineMcCluskey.h`.
- T2.2 — Variable table:
  ```cpp
  class VariableTable {
  public:
      int indexOf(std::string const & name);      // inserts if absent, returns bit index; throws if > MAX_VARIABLES
      std::string const & nameOf(int index) const; // index -> original name
      int size() const;
  private:
      std::vector<std::string> names_;                 // index -> name
      std::unordered_map<std::string,int> indices_;    // name -> index
  };
  ```
- T2.3 — `Expression` gains access to names for serialization. Decision (default): `Beast::Expression` holds `std::shared_ptr<VariableTable const>` (or the table is passed to `toJson`/`toAlgebraic`). Pick ONE and apply consistently. Recommended: store on `Expression` so the public `toJson()` signature stays parameterless.
- T2.4 — Converter API (Library A):
  ```cpp
  // Parses a JsonLogic boolean expression into a DNF Expression, building the variable table.
  Beast::Expression toDNF(nlohmann::json const & jsonLogic, VariableTable & table);
  ```
- T2.5 — Simplifier API (Library B):
  ```cpp
  // QuineMcCluskey.h
  std::vector<Implicant> minimize(std::vector<Term> const & minterms,
                                  std::vector<Term> const & dontCares,
                                  int numVariables);
  ```
  NOTE: return type changes from `std::vector<Term>` to `std::vector<Implicant>` (must carry don't-care mask).
- T2.6 — `Beast::simplify(Expression const &)` performs DNF→minterms→`minimize`→DNF. `Beast::simplify(json const &)` performs parse(`toDNF`)→`simplify(Expression)`.

## 3. Phased task list

### Phase A — Make it compile & portable (no behavior yet)

- A1 — deps: none. files: `Beast/QuineMcCluskey.cpp:60-61`. action: add `typename` before `std::vector<T>::iterator` in `removeDuplicates`. acceptance: `c++ -std=c++17 -fsyntax-only Beast/QuineMcCluskey.cpp` exits 0.
- A2 — deps: none. files: `Beast/QuineMcCluskey.cpp:40`. action: fix `differByOneBit` to `(i0.d == i1.d) && isPowerOf2(i0.v ^ i1.v)`. acceptance: no tautological-compare warning.
- A3 — deps: none. files: `Beast/include/Beast/Beast.h:~12`. action: replace mojibake with ASCII `Quine-McCluskey`. acceptance: file is valid UTF-8/ASCII, no replacement chars.
- A4 — deps: none. files: `main.cpp:24`. action: catch `std::exception const &` (by reference). acceptance: compiles, no slicing.
- A5 — deps: none. files: `unit-test/CMakeLists.txt`, `unit-test/*.cpp`, `unit-test/targetver.h`, root `CMakeLists.txt`. action: replace MSVC CppUnitTest with a portable framework (Catch2 via `FetchContent` or `find_package`). Remove hardcoded `D:/...` path. Add `enable_testing()` + `add_subdirectory(unit-test)` to root, register tests with `add_test`/`catch_discover_tests`. acceptance: `cmake -B build && cmake --build build && ctest --test-dir build` runs (tests may still be empty placeholders).
- A6 — deps: none. files: `Beast/QuineMcCluskey.h` (new), `Beast/QuineMcCluskey.cpp`, `Beast/CMakeLists.txt`. action: create header declaring `Term`, `Implicant`, `minimize` (per T2.1/T2.5); uncomment the source-list entry; `#include` it from the .cpp. acceptance: builds; symbol `minimize` is externally linkable.

### Phase B — Variable table + JsonLogic serialization (output side)

- B1 — deps: A6. files: `Beast/VariableTable.h`/`.cpp` (new), `Beast/CMakeLists.txt`. action: implement `VariableTable` per T2.2; throw `std::length_error` (or custom) when index would exceed `MAX_VARIABLES`. acceptance: unit test inserts duplicate names → same index; distinct names → increasing indices; 33rd distinct name throws.
- B2 — deps: B1, T2.3. files: `Beast/include/Beast/Beast.h`, `Beast/Beast.cpp`. action: give `Expression` access to a `VariableTable` (per T2.3 decision). action: rewrite `Clause::toJson` and `Expression::toJson` to emit **JsonLogic**:
  - positive literal → `{"var": name}`
  - negative literal → `{"!": [{"var": name}]}`
  - clause (AND of literals) → `{"and": [ ... ]}`; single-literal clause → emit the literal directly (no wrapping `and`).
  - expression (OR of clauses) → `{"or": [ ... ]}`; single-clause → emit the clause directly.
  - constant TRUE → JSON `true`; constant FALSE → JSON `false`.
  - filter empty/false clauses consistently with `toAlgebraic`.
  acceptance: round-trip + golden tests in C-phase; `toJson` and `toAlgebraic` describe the SAME formula for any `Expression`.
- B3 — deps: B2. files: `Beast/Beast.cpp`. action: define and document constant-expression representation internally (e.g. empty `clauses_` ⇒ FALSE; a single empty-mask clause ⇒ TRUE) and ensure `toJson`/`toAlgebraic` honor it. acceptance: TRUE→`true`, FALSE→`false` in JSON; documented in `Beast.h`.

### Phase C — Converter library (input side: JsonLogic → DNF)

- C1 — deps: B1. files: `Beast/Converter.cpp` + decl in a header. action: implement `toDNF(json, VariableTable&)` (T2.4) as a recursive descent over JsonLogic:
  - `{"var": name}` → single-literal DNF `Expression` (positive).
  - `{"!": [x]}` → `~toDNF(x)` (reuse `Expression::operator~`; make it `const`).
  - `{"and": [a,b,...]}` → fold with `Expression::operator&` (distributes to DNF).
  - `{"or": [a,b,...]}` → fold with `Expression::operator|`.
  - `{"nand": [a,b,...]}` → `~(toDNF(a) & toDNF(b) & ...)` (desugar = NOT of AND-fold).
  - `{"nor": [a,b,...]}` → `~(toDNF(a) | toDNF(b) | ...)` (desugar = NOT of OR-fold).
  - `{"xor": [a,b,...]}` → desugar to odd-parity over args: fold pairwise `xor(p,q) = (p & ~q) | (~p & q)`, i.e. `acc = (acc & ~next) | (~acc & next)`. Result must be DNF.
  - literal `true`/`false` → constant TRUE/FALSE `Expression`.
  - Validate: each node has exactly one operator key; unknown operator → throw with message. n-ary `and`/`or`/`xor`/`nand`/`nor` accept ≥1 arg (1-arg `xor` = arg; 1-arg `nand`/`nor` = `!arg`); `!` accepts exactly 1; `var` value is a string. The set of accepted operators is `{var, !, and, or, xor, nand, nor}` plus boolean literals.
  acceptance: C-phase unit tests; `toDNF` output passes a DNF-shape invariant check (OR of ANDs of literals only).
- C2 — deps: C1. files: `Beast/Beast.cpp`. action: make `Expression::operator~` `const` (update header `Beast.h:56`). acceptance: compiles; `~expr` usable on const.
- C3 — deps: C1. files: `Beast/Beast.cpp`. action: implement `Beast::simplify(json const &)` = `toDNF` then `simplify(Expression)` then return. acceptance: end-to-end CLI test (Phase E).

### Phase D — Simplifier library (DNF → minimized DNF via Quine–McCluskey)

- D1 — deps: A6. files: `Beast/QuineMcCluskey.cpp`. action: fix Round sizing to `MAX_VARIABLES + 1` and index by `numberOfOneBits(t)` (no `-1`), so minterm `0` lands in slot 0. acceptance: minimizing `{0}` does not crash and yields a correct prime implicant.
- D2 — deps: A1,A2,D1. files: `Beast/QuineMcCluskey.cpp`. action: verify prime-implicant generation loop (combine rounds, `removeCombinedImplicants`, dedup) produces the correct prime-implicant set; add the missing don't-care handling already partially present. acceptance: known textbook case (e.g. minterms {0,1,2,5,6,7}) yields the expected prime implicants.
- D3 — deps: D2. files: `Beast/QuineMcCluskey.cpp`. action: implement **prime-implicant selection** (currently absent):
  1. Build prime-implicant chart (which PIs cover which minterms).
  2. Select all **essential** prime implicants (minterm covered by exactly one PI).
  3. Cover remaining minterms via **Petrick's method** (exact) or a documented greedy/branch-and-bound fallback for larger cases.
  Return the selected `std::vector<Implicant>` (T2.5).
  acceptance: textbook case {0,1,2,5,6,7} → minimal cover matching a known reference (e.g. `!a!b + ...`); result count is minimal.
- D4 — deps: C1,D3. files: `Beast/Beast.cpp` (new helpers). action: implement DNF↔minterm bridge:
  - `Expression`(DNF) + `numVars` → ON-set minterms: expand each `Clause` over its free (unmasked) variables across all `numVars` positions; union the minterms.
  - selected `Implicant`s → `Expression`(DNF): each `Implicant` → `Clause` where bit i is a literal iff `(d & (1<<i))==0`, sign from `v`, else masked out.
  acceptance: round-trip `Expression → minterms → Implicants(identity if already minimal) → Expression` preserves truth table.
- D5 — deps: D3,D4. files: `Beast/Beast.cpp`. action: implement `Beast::simplify(Expression const &)`: derive `numVars` from variable table, compute minterms, call `minimize`, convert back to `Expression`, attach variable table. Handle constants: all minterms ⇒ TRUE; no minterms ⇒ FALSE. acceptance: `(a&b)|(a&!b)` ⇒ `a`; `a|!a` ⇒ TRUE; `a&!a` ⇒ FALSE.

### Phase E — Integration, CLI, tests, docs

- E1 — deps: C3,D5. files: `unit-test/*` (new, portable). action: write real tests:
  - VariableTable (B1 cases).
  - Converter: parse + DNF-shape invariants + error cases (unknown op, multi-key node, bad arity). Include `xor`/`nand`/`nor` desugaring: verify truth-table equivalence (e.g. `xor[a,b]` ≡ `(a&!b)|(!a&b)`, `nand` ≡ `!(a&b)`, `nor` ≡ `!(a|b)`) and n-ary xor parity.
  - QuineMcCluskey: textbook minimization cases + edge (minterm 0, all-ones, single var, empty).
  - Serializer: golden JsonLogic strings + `toJson`/`toAlgebraic` agreement.
  - End-to-end: `simplify(json)` golden input→output pairs incl. constants.
  acceptance: `ctest` all green.
- E2 — deps: C3,D5. files: `main.cpp`. action: confirm CLI wiring (arg/stdin → `simplify(json)` → `toJson` → stdout; errors → stderr, exit nonzero). Add `> MAX_VARIABLES` and parse-error messages. acceptance: shell test: README example input produces a valid simplified JsonLogic output; malformed input exits nonzero with message on stderr.
- E3 — deps: E1,E2. files: `README.md`. action: reconcile README examples with actual output; if `simplify` of the README example yields `{"var":"a"}`, keep; otherwise update example. acceptance: README examples reproduce verbatim when run.
- E4 — deps: E1. files: `Doxyfile.in`, headers. action: ensure doc comments are accurate for new/changed APIs. acceptance: `doxygen` runs without warnings about the public API (optional/best-effort).

## 4. Cross-cutting bug fixes (tracked, fold into phases above)

- BUG-1 (A1): `removeDuplicates` missing `typename`. — compile blocker.
- BUG-2 (A2): `differByOneBit` self-compare `i0.d==i0.d`. — wrong combination logic.
- BUG-3 (D1): minterm-0 index underflow + Round off-by-one sizing.
- BUG-4 (B2): `toJson` non-JsonLogic array form; must emit operator-as-key objects.
- BUG-5 (B2): `toJson` does not filter empty/false clauses while `toAlgebraic` does. — serializer divergence.
- BUG-6 (B3): empty-clause semantics inverted (no-terms means FALSE per header, but `["and",[]]` reads as TRUE).
- BUG-7 (B2): hardcoded `"x"+index` names; replace with `VariableTable` lookup.
- BUG-8 (A4): `catch (std::exception e)` by value (slicing).
- BUG-9 (C2): `operator~` non-const, blocks use in converter recursion.

## 5. Definition of done (whole project)

- DOD-1 — `cmake -B build && cmake --build build` succeeds clean on macOS/Linux (no warnings in Beast sources).
- DOD-2 — `ctest --test-dir build` passes; coverage spans converter, simplifier, serializer, CLI, edge cases.
- DOD-3 — `echo '<jsonlogic>' | beast` returns minimal-DNF JsonLogic for arbitrary valid input; constants return `true`/`false`.
- DOD-4 — Round-trip property holds: for random expressions over ≤8 vars, `simplify` preserves the truth table and output is valid JsonLogic in DNF.
- DOD-5 — Variable names round-trip unchanged; >32 distinct vars rejected with a clear error.
- DOD-6 — README examples reproduce verbatim.

## 6. Open decisions (resolve before/within the phase that needs them)

- Q1 (T2.3) — Does `Expression` own the `VariableTable` (parameterless `toJson`) or is it passed in? Default: own via `shared_ptr<const VariableTable>`.
- Q2 (D3) — Petrick's method (exact, can be exponential) vs greedy (fast, may be non-minimal) for non-essential PI cover. Default: Petrick for ≤ N PIs threshold, greedy fallback above it; document the threshold.
- Q3 (C1) — RESOLVED: converter accepts boolean operators only — standard `and`, `or`, `!`, `var`, `true`, `false`, PLUS the non-standard Beast extensions `xor`, `nand`, `nor` (input-only, desugared, never emitted). Reject every other operator (comparison/numeric/array/string families) with a clear error.
- Q4 (E2) — Output formatting: compact vs pretty JSON; stable key ordering for golden tests. Default: compact, deterministic ordering.
