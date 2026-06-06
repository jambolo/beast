//! Minimal, dependency-free JSON value type with a recursive-descent parser and
//! a compact serializer.
//!
//! This module provides all of the crate's JSON parsing and serialization, so
//! Beast has no external dependencies and builds with no network access. Object
//! keys preserve insertion order, which keeps serialized output deterministic
//! for golden tests.

use std::fmt;

/// A JSON value.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    /// Object as an ordered list of key/value pairs (insertion order preserved).
    Object(Vec<(String, Json)>),
}

impl Json {
    /// Parses a JSON document from a string. Returns an error message on failure.
    pub fn parse(text: &str) -> Result<Json, String> {
        let mut parser = Parser::new(text);
        parser.skip_whitespace();
        let value = parser.parse_value()?;
        parser.skip_whitespace();
        if parser.peek().is_some() {
            return Err(format!("trailing characters at byte {}", parser.pos));
        }
        Ok(value)
    }

    /// Returns the string value if this is a `String`, otherwise `None`.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(s) => Some(s),
            _ => None,
        }
    }

    /// Returns the array elements if this is an `Array`, otherwise `None`.
    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Array(a) => Some(a),
            _ => None,
        }
    }

    /// Looks up a key if this is an `Object`, otherwise `None`.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
}

/// Compact serialization (no superfluous whitespace).
impl fmt::Display for Json {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Json::Null => f.write_str("null"),
            Json::Bool(b) => write!(f, "{}", b),
            Json::Number(n) => {
                // Emit integers without a trailing ".0".
                if n.fract() == 0.0 && n.is_finite() {
                    write!(f, "{}", *n as i64)
                } else {
                    write!(f, "{}", n)
                }
            }
            Json::String(s) => write_json_string(f, s),
            Json::Array(items) => {
                f.write_str("[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "{}", item)?;
                }
                f.write_str("]")
            }
            Json::Object(pairs) => {
                f.write_str("{")?;
                for (i, (k, v)) in pairs.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?;
                    }
                    write_json_string(f, k)?;
                    f.write_str(":")?;
                    write!(f, "{}", v)?;
                }
                f.write_str("}")
            }
        }
    }
}

fn write_json_string(f: &mut fmt::Formatter<'_>, s: &str) -> fmt::Result {
    f.write_str("\"")?;
    for c in s.chars() {
        match c {
            '"' => f.write_str("\\\"")?,
            '\\' => f.write_str("\\\\")?,
            '\n' => f.write_str("\\n")?,
            '\r' => f.write_str("\\r")?,
            '\t' => f.write_str("\\t")?,
            '\u{08}' => f.write_str("\\b")?,
            '\u{0c}' => f.write_str("\\f")?,
            c if (c as u32) < 0x20 => write!(f, "\\u{:04x}", c as u32)?,
            c => f.write_fmt(format_args!("{}", c))?,
        }
    }
    f.write_str("\"")
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Self {
        Parser {
            bytes: text.as_bytes(),
            pos: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.peek();
        if b.is_some() {
            self.pos += 1;
        }
        b
    }

    fn skip_whitespace(&mut self) {
        while let Some(b) = self.peek() {
            match b {
                b' ' | b'\t' | b'\n' | b'\r' => self.pos += 1,
                _ => break,
            }
        }
    }

    fn parse_value(&mut self) -> Result<Json, String> {
        self.skip_whitespace();
        match self.peek() {
            Some(b'{') => self.parse_object(),
            Some(b'[') => self.parse_array(),
            Some(b'"') => Ok(Json::String(self.parse_string()?)),
            Some(b't') | Some(b'f') => self.parse_bool(),
            Some(b'n') => self.parse_null(),
            Some(b) if b == b'-' || b.is_ascii_digit() => self.parse_number(),
            Some(b) => Err(format!(
                "unexpected character '{}' at byte {}",
                b as char, self.pos
            )),
            None => Err("unexpected end of input".to_string()),
        }
    }

    fn expect(&mut self, b: u8) -> Result<(), String> {
        if self.peek() == Some(b) {
            self.pos += 1;
            Ok(())
        } else {
            Err(format!("expected '{}' at byte {}", b as char, self.pos))
        }
    }

    fn parse_object(&mut self) -> Result<Json, String> {
        self.expect(b'{')?;
        let mut pairs = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Json::Object(pairs));
        }
        loop {
            self.skip_whitespace();
            let key = self.parse_string()?;
            self.skip_whitespace();
            self.expect(b':')?;
            let value = self.parse_value()?;
            pairs.push((key, value));
            self.skip_whitespace();
            match self.bump() {
                Some(b',') => continue,
                Some(b'}') => break,
                _ => return Err(format!("expected ',' or '}}' at byte {}", self.pos)),
            }
        }
        Ok(Json::Object(pairs))
    }

    fn parse_array(&mut self) -> Result<Json, String> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Json::Array(items));
        }
        loop {
            let value = self.parse_value()?;
            items.push(value);
            self.skip_whitespace();
            match self.bump() {
                Some(b',') => continue,
                Some(b']') => break,
                _ => return Err(format!("expected ',' or ']' at byte {}", self.pos)),
            }
        }
        Ok(Json::Array(items))
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            match self.bump() {
                None => return Err("unterminated string".to_string()),
                Some(b'"') => break,
                Some(b'\\') => match self.bump() {
                    Some(b'"') => out.push('"'),
                    Some(b'\\') => out.push('\\'),
                    Some(b'/') => out.push('/'),
                    Some(b'n') => out.push('\n'),
                    Some(b'r') => out.push('\r'),
                    Some(b't') => out.push('\t'),
                    Some(b'b') => out.push('\u{08}'),
                    Some(b'f') => out.push('\u{0c}'),
                    Some(b'u') => out.push(self.parse_unicode_escape()?),
                    _ => return Err(format!("invalid escape at byte {}", self.pos)),
                },
                Some(b) if b < 0x80 => out.push(b as char),
                Some(b) => {
                    // Multi-byte UTF-8 sequence: collect continuation bytes.
                    let mut buf = vec![b];
                    let extra = if b >= 0xF0 {
                        3
                    } else if b >= 0xE0 {
                        2
                    } else {
                        1
                    };
                    for _ in 0..extra {
                        match self.bump() {
                            Some(c) => buf.push(c),
                            None => return Err("truncated UTF-8 sequence".to_string()),
                        }
                    }
                    match std::str::from_utf8(&buf) {
                        Ok(s) => out.push_str(s),
                        Err(_) => return Err("invalid UTF-8 in string".to_string()),
                    }
                }
            }
        }
        Ok(out)
    }

    fn parse_unicode_escape(&mut self) -> Result<char, String> {
        let mut code: u32 = 0;
        for _ in 0..4 {
            let b = self
                .bump()
                .ok_or_else(|| "truncated \\u escape".to_string())?;
            let digit = (b as char)
                .to_digit(16)
                .ok_or_else(|| format!("invalid hex digit in \\u escape at byte {}", self.pos))?;
            code = code * 16 + digit;
        }
        std::char::from_u32(code).ok_or_else(|| "invalid unicode code point".to_string())
    }

    fn parse_bool(&mut self) -> Result<Json, String> {
        if self.bytes[self.pos..].starts_with(b"true") {
            self.pos += 4;
            Ok(Json::Bool(true))
        } else if self.bytes[self.pos..].starts_with(b"false") {
            self.pos += 5;
            Ok(Json::Bool(false))
        } else {
            Err(format!("invalid literal at byte {}", self.pos))
        }
    }

    fn parse_null(&mut self) -> Result<Json, String> {
        if self.bytes[self.pos..].starts_with(b"null") {
            self.pos += 4;
            Ok(Json::Null)
        } else {
            Err(format!("invalid literal at byte {}", self.pos))
        }
    }

    fn parse_number(&mut self) -> Result<Json, String> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        while let Some(b) = self.peek() {
            match b {
                b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-' => self.pos += 1,
                _ => break,
            }
        }
        let slice = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| "invalid number".to_string())?;
        slice
            .parse::<f64>()
            .map(Json::Number)
            .map_err(|_| format!("invalid number '{}'", slice))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_object() {
        let v = Json::parse(r#"{"or":[{"var":"a"},true]}"#).unwrap();
        assert_eq!(v.to_string(), r#"{"or":[{"var":"a"},true]}"#);
    }

    #[test]
    fn parses_array_and_strings() {
        let v = Json::parse(r#"["and",["x0",["not","x1"]]]"#).unwrap();
        assert_eq!(v.to_string(), r#"["and",["x0",["not","x1"]]]"#);
    }

    #[test]
    fn rejects_garbage() {
        assert!(Json::parse("{bad}").is_err());
        assert!(Json::parse("").is_err());
        assert!(Json::parse("[1,2").is_err());
    }

    fn get<'a>(v: &'a Json, k: &str) -> &'a Json {
        v.get(k).unwrap()
    }

    #[test]
    fn accessors_work() {
        let v = Json::parse(r#"{"var":"name"}"#).unwrap();
        assert_eq!(get(&v, "var").as_str(), Some("name"));
    }
}
