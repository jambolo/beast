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
use clap::{ArgAction, Parser, ValueEnum};

/// Input/output syntax.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Format {
    #[value(alias = "alg")]
    Algebraic,
    Json,
}

/// Algebraic output style. Mirrors [`AlgebraicStyle`] so the CLI surface stays decoupled from the library enum.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Style {
    Common,
    Code,
    Logic,
}

impl From<Style> for AlgebraicStyle {
    fn from(style: Style) -> Self {
        match style {
            Style::Common => AlgebraicStyle::Common,
            Style::Code => AlgebraicStyle::Code,
            Style::Logic => AlgebraicStyle::Logic,
        }
    }
}

/// Boolean expression simplifier (Quine–McCluskey over disjunctive normal form).
///
/// If EXPRESSION is omitted it is read from stdin. Use `--` to end option parsing when an algebraic expression begins with `-`
/// (e.g. `beast -- -a`).
#[derive(Parser)]
#[command(name = "beast", version, about, long_about = None, disable_version_flag = true)]
struct Cli {
    /// Input syntax.
    #[arg(short = 'i', long = "in", value_name = "FORMAT", default_value = "algebraic")]
    input_format: Format,

    /// Output syntax [default: same as --in].
    #[arg(short = 'o', long = "out", value_name = "FORMAT")]
    output_format: Option<Format>,

    /// Algebraic output style (ignored for JSON output).
    #[arg(short = 's', long = "style", value_name = "STYLE", default_value = "common")]
    style: Style,

    /// Print version and exit.
    #[arg(short = 'v', long = "version", action = ArgAction::Version)]
    version: Option<bool>,

    /// Expression to simplify (read from stdin if omitted).
    #[arg(value_name = "EXPRESSION")]
    expression: Option<String>,
}

fn fail(message: &str) -> ! {
    eprintln!("{}", message);
    exit(1)
}

fn main() {
    let cli = Cli::parse();
    let output_format = cli.output_format.unwrap_or(cli.input_format);

    let input = match cli.expression {
        Some(text) => text,
        None => read_stdin(),
    };
    // Tolerate a leading UTF-8 BOM (common on Windows pipes / saved files).
    let input = input.strip_prefix('\u{feff}').map(str::to_string).unwrap_or(input);

    let simplified = match cli.input_format {
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
        Format::Algebraic => simplified.to_algebraic_styled(cli.style.into()),
        Format::Json => simplified.to_json().to_string(),
    };
    println!("{}", output);
}

fn read_stdin() -> String {
    let mut buffer = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut buffer) {
        fail(&e.to_string());
    }
    buffer
}
