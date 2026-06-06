# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project status

Beast is an early-stage **work in progress**, not a working tool yet. The DNF data model and boolean-algebra operators exist, but the headline feature does not: `Beast::simplify` is a stub, the Quine–McCluskey `minimize` is unfinished (no prime-implicant selection), there is no JsonLogic parser, and the unit tests are unportable MSVC stubs that aren't wired into the build.

**`plan.md` is the authoritative roadmap.** It is written for agent consumption with dependency-ordered tasks (`ID / deps / files / action / acceptance`), a current-state inventory, locked decisions, and a definition of done. Read it before making changes and keep it in sync when scope changes.

## Build & run

Requires a C++17 compiler, CMake ≥ 3.10, and `nlohmann_json` (found via `find_package`; installed on this machine under `/usr/local`).

```sh
cmake -B build              # configure
cmake --build build         # build the Beast library + `beast` executable
```

Run the CLI (executable is named `beast`, not `main`):

```sh
./build/beast '<jsonlogic-expression>'   # expression as first argument
./build/beast < expression.json          # or read from stdin
```

Quick syntax-check of the dependency-free translation unit (useful while iterating on the algorithm):

```sh
c++ -std=c++17 -fsyntax-only -Wall Beast/QuineMcCluskey.cpp
```

## Tests

There is **no working test command yet**. `unit-test/` uses the MSVC `CppUnitTest` framework with a hardcoded Windows path (`D:/Program Files (x86)/VC/UnitTest`), every test is an `Assert::Fail("Not yet implemented")` stub, and the root `CMakeLists.txt` does not `add_subdirectory(unit-test)`. Replacing this with a portable framework (Catch2/GoogleTest) and wiring `ctest` is Phase A in `plan.md`. Once done, the intended flow is `ctest --test-dir build` (and `ctest -R <name>` for a single test).

## Architecture (big picture)

Beast is a thin CLI wrapping two conceptual libraries; data flows: **JsonLogic → DNF → minimized DNF → JsonLogic**.

1. **Converter** (to be written, Phase C): parses an arbitrary JsonLogic boolean expression into an `Expression` in **disjunctive normal form**. It works by recursive descent that reuses the existing algebra operators — `operator|` concatenates clauses, `operator&` distributes (product of sums → sum of products), and `operator~` applies De Morgan — so any input tree collapses to DNF as it is built.
2. **Simplifier** (`Beast/QuineMcCluskey.cpp`, to be finished, Phase D): input and output are both DNF. It converts the DNF clause set to minterms, runs Quine–McCluskey, and converts the selected prime implicants back to clauses.

### Core data model (`Beast/include/Beast/Beast.h`)

- `Beast::Expression` = OR of `Clause`s (a `std::vector<Clause>`), i.e. DNF.
- `Beast::Expression::Clause` = AND of literals, stored as two parallel `std::vector<bool>`: `terms` (literal sign: true = unnegated, false = negated) and `mask` (true = the variable is present in this clause).
- Convention (used throughout the algebra and serialization): **a clause with no terms represents FALSE**.
- Variables are referenced internally by **bit index**, not name. A name↔index table (to be added) maps arbitrary user-supplied names to indices and restores them on output. The current serializers hardcode synthesized `"x"+index` names — these must become table lookups.

### The two QM-related representations

- `Implicant { Term v; Term d; }` (in `QuineMcCluskey.cpp`): `v` = fixed bit values, `d` = don't-care mask. `Term` is `uint32_t`, which caps the design at **`MAX_VARIABLES = 32`** distinct variables.
- The bridge between the `Clause` (terms/mask) representation and the `Term`/`Implicant` (bitmask) representation does not exist yet and is required to connect the two libraries (Phase D in `plan.md`).

## Locked decisions (do not re-litigate; see `plan.md` §0)

- **I/O format is JsonLogic** (operator-as-key objects, one key per node) for both input and output. The current `toJson` emits a non-JsonLogic array form (`["or",[...]]`) and must be rewritten to objects (`{"or":[...]}`, negation `{"!":[...]}`, variable `{"var":"name"}`).
- **Variable names are arbitrary** user-supplied strings, mapped to bit indices and preserved in output.
- **Accepted operators**: standard `and`, `or`, `!`, `var`, and boolean literals `true`/`false`, PLUS non-standard Beast extensions `xor`, `nand`, `nor`. The extensions are **input-only** — desugared to `and`/`or`/`!` during conversion and never emitted on output. All other JsonLogic operators (comparison/numeric/array/string) are rejected.

## Gotchas

- `json({ "or", a })` in nlohmann produces a JSON **array**, not an object; use `json{{ "or", a }}` for an object. This quirk is the root cause of the current wrong output shape.
- The two serializers diverge today: `Expression::toAlgebraic` filters empty (false) clauses but `Expression::toJson` does not. Keep them consistent.
- `QuineMcCluskey.cpp` does not currently compile under a conforming compiler (missing `typename` in `removeDuplicates`) and has a self-comparison bug in `differByOneBit` (`i0.d == i0.d`). See `plan.md` §4 for the full bug list with line references.
