# Beast

**Beast** is a boolean expression simplifier. It reads an arbitrary boolean expression, expressed as JSON, and writes back an equivalent expression that has been logically simplified.

## Overview

Given a boolean expression of any shape — arbitrarily nested `and`, `or`, and `not` operations over named variables — Beast produces the simplest equivalent expression. For example, `(a & b) | (a & !b)` reduces to `a`.

Internally the simplification is performed in disjunctive normal form (DNF) using the Quine–McCluskey algorithm. The input may be in any form; Beast converts it to DNF, minimizes it, and emits the result.

## Building

Beast is a Cargo workspace with no external dependencies, so it builds offline with a standard Rust toolchain:

```sh
cargo build                            # build the workspace (library + `beast` binary)
cargo test                             # run the test suite
cargo run -p beast -- '<expression>'   # build and run the CLI
```

The compiled binary is `target/debug/beast` (or `target/release/beast` after `cargo build --release`).

## Command-line syntax

```
beast '<expression>'
beast < expression.json
```

- If an expression is supplied as the first command-line argument, it is parsed as the input.
- Otherwise, the expression is read from standard input.

The simplified expression is written to standard output. Parse errors and other failures are reported on standard error, and the program exits with a non-zero status.

Examples:

```sh
# Expression passed as an argument
beast '{"or":[{"and":[{"var":"a"},{"var":"b"}]},{"and":[{"var":"a"},{"!":{"var":"b"}}]}]}'

# Expression read from a file via stdin
beast < expression.json
```

## JSON format

Beast uses the [JsonLogic](https://jsonlogic.com) format for both input and output.

A JsonLogic expression is an object with a single key — the operator — whose value is the operator's argument(s). Expressions nest to form the full formula. Beast uses the following subset:

| Operator | Meaning | Example |
| --- | --- | --- |
| `and` | logical AND | `{"and": [a, b, ...]}` |
| `or`  | logical OR  | `{"or": [a, b, ...]}` |
| `!`   | logical NOT | `{"!": [a]}` |
| `var` | variable reference | `{"var": "name"}` |

The boolean literals `true` and `false` are also accepted.

You can name variables whatever you like — `{"var": "raining"}`, `{"var": "x0"}`, anything — and the names you use are preserved in the simplified output.

### Extension operators

For convenience, Beast also accepts the following operators on input:

| Operator | Meaning | Example |
| --- | --- | --- |
| `xor`  | exclusive OR (true when an odd number of operands are true) | `{"xor": [a, b, ...]}` |
| `nand` | NOT AND | `{"nand": [a, b, ...]}` |
| `nor`  | NOT OR  | `{"nor": [a, b, ...]}` |

> **Note:** `xor`, `nand`, and `nor` are *not* part of the standard JsonLogic specification — they are Beast extensions. Other JsonLogic implementations will not recognize them. They are accepted on input only; Beast rewrites them in terms of `and`, `or`, and `!`, so the simplified output never contains these operators.

Example input — an arbitrary boolean expression:

```json
{
  "or": [
    { "and": [ { "var": "a" }, { "var": "b" } ] },
    { "and": [ { "var": "a" }, { "!": { "var": "b" } } ] }
  ]
}
```

Corresponding simplified output, in disjunctive normal form:

```json
{ "var": "a" }
```

## Architecture

Beast is organized as a Cargo workspace of two crates:

1. **The `beast` crate** — the main crate. It owns the end-to-end pipeline: it parses the JsonLogic input and converts it to an equivalent expression in disjunctive normal form (DNF), drives the simplification, and serializes the simplified result back to JsonLogic. It depends on `quine-mccluskey` for the minimization step.
2. **The `quine-mccluskey` crate** — the simplifier library. It minimizes a boolean expression with the Quine–McCluskey algorithm, taking DNF as its input and producing minimized DNF as its output.

So `beast` does the full job — convert, simplify, and emit the simplified expression — delegating only the core Quine–McCluskey minimization to the `quine-mccluskey` crate. The command-line program parses the JsonLogic input, converts it to DNF, hands that to the simplifier, and serializes the minimized DNF result back to JsonLogic.

> **Future enhancement:** an algebraic I/O mode — reading and writing expressions in algebraic form (e.g. `(a & b) | !c`) in addition to JsonLogic — is planned but not yet part of Beast. For now, both input and output are JsonLogic.
