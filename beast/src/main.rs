//! Beast command-line interface.
//!
//! Reads a boolean expression (from the first non-flag argument, or from stdin if none is given), simplifies it, and writes the
//! result to stdout. The input syntax is selected with `--in` (`algebraic`, the default, or `json`) and the output syntax with
//! `--out` (defaults to the input syntax). The algebraic output style is selected with `--style` (`common`, the default, `code`, or
//! `logic`). Parse errors are reported on stderr with a non-zero exit status.

use std::io::Read;
use std::process::exit;

use beast::AlgebraicStyle;
use beast::json::Json;

const USAGE: &str = "\
usage: beast [--in algebraic|json] [--out algebraic|json] [--style common|code|logic] [EXPRESSION]

  -i, --in    FORMAT   input syntax: algebraic (default) or json
  -o, --out   FORMAT   output syntax: defaults to the input syntax
  -s, --style STYLE    algebraic output style: common (default), code, or logic
  -h, --help           show this help

The --style flag affects algebraic output only (it is ignored for JSON output):
  common  a + b, juxtaposition for AND, overbar/~ for NOT  (the default)
  code    a | b, a & b, !a
  logic   a \u{2228} b, a \u{2227} b, \u{00AC}a

If EXPRESSION is omitted it is read from stdin. Use `--` to end option parsing
when an algebraic expression begins with `-` (e.g. beast -- -a).";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Format {
    Algebraic,
    Json,
}

fn fail(message: &str) -> ! {
    eprintln!("{}", message);
    exit(1)
}

fn parse_format(value: &str) -> Format {
    match value {
        "algebraic" | "alg" => Format::Algebraic,
        "json" => Format::Json,
        other => fail(&format!("unknown format {:?} (expected \"algebraic\" or \"json\")", other)),
    }
}

fn parse_style(value: &str) -> AlgebraicStyle {
    match value {
        "common" => AlgebraicStyle::Common,
        "code" => AlgebraicStyle::Code,
        "logic" => AlgebraicStyle::Logic,
        other => fail(&format!(
            "unknown style {:?} (expected \"common\", \"code\", or \"logic\")",
            other
        )),
    }
}

fn main() {
    let mut input_format: Option<Format> = None;
    let mut output_format: Option<Format> = None;
    let mut style: Option<AlgebraicStyle> = None;
    let mut expression: Option<String> = None;
    let mut options_ended = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if options_ended {
            set_expression(&mut expression, arg);
            continue;
        }
        if let Some(value) = arg.strip_prefix("--in=") {
            input_format = Some(parse_format(value));
        } else if let Some(value) = arg.strip_prefix("--out=") {
            output_format = Some(parse_format(value));
        } else if let Some(value) = arg.strip_prefix("--style=") {
            style = Some(parse_style(value));
        } else if arg == "--in" || arg == "-i" {
            input_format = Some(parse_format(&next_value(&mut args, &arg)));
        } else if arg == "--out" || arg == "-o" {
            output_format = Some(parse_format(&next_value(&mut args, &arg)));
        } else if arg == "--style" || arg == "-s" {
            style = Some(parse_style(&next_value(&mut args, &arg)));
        } else if arg == "--help" || arg == "-h" {
            println!("{}", USAGE);
            return;
        } else if arg == "--" {
            options_ended = true;
        } else if arg.starts_with("--") {
            fail(&format!("unknown option {:?}\n\n{}", arg, USAGE));
        } else {
            // Anything else (including a single `-` or `-a`) is the expression.
            set_expression(&mut expression, arg);
        }
    }

    let input_format = input_format.unwrap_or(Format::Algebraic);
    let output_format = output_format.unwrap_or(input_format);

    let input = match expression {
        Some(text) => text,
        None => read_stdin(),
    };
    // Tolerate a leading UTF-8 BOM (common on Windows pipes / saved files).
    let input = input.strip_prefix('\u{feff}').map(str::to_string).unwrap_or(input);

    let simplified = match input_format {
        Format::Algebraic => beast::simplify_algebraic(&input),
        Format::Json => match Json::parse(&input) {
            Ok(value) => beast::simplify_json(&value),
            Err(e) => fail(&e),
        },
    };
    let simplified = match simplified {
        Ok(expression) => expression,
        Err(e) => fail(&e),
    };

    let output = match output_format {
        Format::Algebraic => simplified.to_algebraic_styled(style.unwrap_or_default()),
        Format::Json => simplified.to_json().to_string(),
    };
    println!("{}", output);
}

fn set_expression(slot: &mut Option<String>, value: String) {
    if slot.is_some() {
        fail("expected a single expression argument");
    }
    *slot = Some(value);
}

fn next_value(args: &mut impl Iterator<Item = String>, flag: &str) -> String {
    args.next().unwrap_or_else(|| fail(&format!("{} requires a value", flag)))
}

fn read_stdin() -> String {
    let mut buffer = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut buffer) {
        fail(&e.to_string());
    }
    buffer
}
