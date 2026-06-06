//! Beast command-line interface.
//!
//! Reads a boolean expression as JSON (from the first argument, or from stdin
//! if no argument is given), simplifies it, and writes the result as JSON to
//! stdout. Parse errors are reported on stderr with a non-zero exit status.

use std::io::Read;
use std::process::exit;

use beast::json::Json;

fn main() {
    // First command-line argument, if any, is the expression text.
    let expression_text: Option<String> = std::env::args().nth(1);

    let input = match expression_text {
        Some(text) => text,
        None => {
            let mut buffer = String::new();
            if let Err(e) = std::io::stdin().read_to_string(&mut buffer) {
                eprintln!("{}", e);
                exit(1);
            }
            buffer
        }
    };

    let expression_json = match Json::parse(&input) {
        Ok(value) => value,
        Err(e) => {
            eprintln!("{}", e);
            exit(1);
        }
    };

    let simplified = beast::simplify_json(&expression_json);
    println!("{}", simplified.to_json());
}
