# Beast

[![CI](https://github.com/jambolo/beast/actions/workflows/ci.yml/badge.svg)](https://github.com/jambolo/beast/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/jambolo/beast/graph/badge.svg)](https://codecov.io/gh/jambolo/beast)

**Beast** is a boolean expression simplifier. It reads an arbitrary boolean expression — written either in a compact algebraic syntax (the default) or as JSON — and writes back an equivalent expression that has been logically simplified.

## Overview

Given a boolean expression of any shape — arbitrarily nested `and`, `or`, and `not` operations over named variables — Beast produces the simplest equivalent expression. For example, `(a & b) | (a & !b)` reduces to `a`.

Internally the simplification is performed in disjunctive normal form (DNF) using the Quine–McCluskey algorithm. The input may be in any form; Beast converts it to DNF, minimizes it, and emits the result.

## Building

Beast is a Cargo workspace built with a standard Rust toolchain. Its only third-party dependency is [`clap`](https://crates.io/crates/clap) (command-line parsing), fetched from crates.io on the first build; thereafter it builds offline:

```sh
cargo build                            # build the workspace (library + `beast` binary)
cargo test                             # run the test suite
cargo run -p beast -- '<expression>'   # build and run the CLI
```

The compiled binary is `target/debug/beast` (or `target/release/beast` after `cargo build --release`).

## Command-line syntax

```text
beast [--in algebraic|json] [--out algebraic|json] [--style common|code|logic] '<expression>'
beast [--in algebraic|json] [--out algebraic|json] [--style common|code|logic] < expression.txt
```

| Option | Values | Default | Description |
| --- | --- | --- | --- |
| `--in` / `-i` | `algebraic`, `json` | `algebraic` | Input syntax. |
| `--out` / `-o` | `algebraic`, `json` | input syntax | Output syntax. So `algebraic` in yields `algebraic` out and `json` in yields `json` out unless you ask otherwise. |
| `--style` / `-s` | `common`, `code`, `logic` | `common` | Algebraic output style. Affects only the operator glyphs and whitespace, and is ignored for JSON output. See [Output styles](#output-styles). |
| `--version` / `-v` | — | — | Print the version and exit. |
| `--help` / `-h` | — | — | Print usage and exit. |

- If an expression is supplied as the first non-flag argument, it is parsed as the input. Otherwise, the expression is read from standard input.
- Use `--` to end option parsing when an algebraic expression begins with `-` (e.g. `beast -- '-a'`), or pass it on stdin.

The simplified expression is written to standard output. Parse errors and other failures are reported on standard error, and the program exits with a non-zero status.

Examples:

```sh
# Algebraic input (the default): (a & b) | (a & !b) reduces to a
beast 'ab + a!b'

# Algebraic input, JSON output
beast --out json 'ab + a!b'

# JSON input (round-trips to JSON output by default)
beast --in json '{"or":[{"and":[{"var":"a"},{"var":"b"}]},{"and":[{"var":"a"},{"!":{"var":"b"}}]}]}'

# JSON input, algebraic output
beast -i json -o algebraic < expression.json
```

## Algebraic format

The default syntax is a compact algebraic notation. Each operator has several spellings so you can use whichever your keyboard or notation prefers:

| Operation | Spellings |
| --- | --- |
| and | `&`, `*`, `.`, `∧`, `⋅`, `·`, `×`, or *juxtaposition* (two adjacent operands) |
| or | `\|`, `+`, `∨` |
| xor | `^`, `⊕`, `⊻` |
| not | `~`, `-`, `¬`, `!` (all prefix), or a combining overbar (U+0305) *immediately following a single-letter variable* |
| grouping | `(` `)` |
| constants | `1` (true), `0` (false) |

Variables:

- A single ASCII letter is one variable, so adjacent letters are an implicit AND: `ab` means `a & b`.
- A name longer than one character must be introduced with `$` and start with a letter; after the first letter the name continues over `[0-9a-zA-Z_]`. So `$velocity * $pressure` is two variables, `velocity` and `pressure`. `$0` and `$_` are not valid.

Whitespace is a delimiter for variable names, but is otherwise ignored. Precedence, tightest first, is `not`, then `and`, then `or`/`xor` (which share one left-associative tier).

Examples:

| Input | Meaning (code style) |
| --- | --- |
| `ab!c\|d` | `a & b & !c \| d` |
| `a$bc d` | `a & bc & d` |
| `$velocity*$pressure` | `velocity & pressure` |
| `ab̅c` | `a & !b & c` |
| `~a*-b` | `!a & !b` |
| `a ⊕ b` | `a ^ b` |
| `p·q r` | `p & q & r` |
| `¬(a + b)` | `!(a \| b)` |
| `~a b + c` | `!a & b \| c` |
| `1a + 0` | `1 & a \| 0` |
| `a & b . c` | `a & b & c` |
| `a ∧ b ⋅ c × d` | `a & b & c & d` |
| `a ∨ b` | `a \| b` |
| `a ^ b ⊻ c` | `a ^ b ^ c` |

### Output styles

Algebraic output is emitted in one of three styles, selected with `--style` / `-s`. The style changes only the operator glyphs and the whitespace around them; the constants always render as `1` / `0`.

| Style | OR | AND | NOT | Example |
| --- | --- | --- | --- | --- |
| `common` (default) | ` + ` | juxtaposition | overbar on a single letter, `~` prefix otherwise | `ab̅c + d` |
| `code` | ` \| ` | ` & ` | `!` prefix | `a & b & !c \| d` |
| `logic` | ` ∨ ` | ` ∧ ` | `¬` prefix | `a ∧ b ∧ ¬c ∨ d` |

A multi-character variable name is emitted with its `$` prefix.

In `common` style, adjacent operands are written next to each other (`ab` is `a & b`); because a `$`-name greedily consumes following identifier characters, a space is inserted only where a multi-character name is immediately followed by a single letter (so `velocity & pressure` prints as `$velocity$pressure`, but `velocity & a` prints as `$velocity a`).

## JSON format

Beast also reads and writes the [JsonLogic](https://jsonlogic.com) format (select it with `--in json` / `--out json`).

A JsonLogic expression is an object with a single key — the operator — whose value is the operator's argument(s). Expressions nest to form the full formula. Beast uses the following subset:

| Operator | Meaning | Example |
| --- | --- | --- |
| `and` | logical AND | `{"and": [a, b, ...]}` |
| `or` | logical OR | `{"or": [a, b, ...]}` |
| `!` | logical NOT | `{"!": [a]}` |
| `var` | variable reference | `{"var": "name"}` |

The boolean literals `true` and `false` are also accepted.

You can name variables whatever you like — `{"var": "raining"}`, `{"var": "x0"}`, anything — and the names you use are preserved in the simplified output.

### Extension operators

For convenience, Beast also accepts the following operators on input:

| Operator | Meaning | Example |
| --- | --- | --- |
| `xor` | exclusive OR (true when an odd number of operands are true) | `{"xor": [a, b, ...]}` |
| `nand` | NOT AND | `{"nand": [a, b, ...]}` |
| `nor` | NOT OR | `{"nor": [a, b, ...]}` |

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

Beast is a Cargo workspace of two crates: `beast` (parsing, serialization, and the end-to-end pipeline) and `quine-mccluskey` (the Quine–McCluskey simplifier library). The pipeline is `input → DNF → minimized DNF → output`, with algebraic or JsonLogic on each side.

For the internal design — data model, the DNF core, the two Quine–McCluskey representations, and implementation notes — see [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).
