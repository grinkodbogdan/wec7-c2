//! Flare configuration syntax.
//!
//! A deliberately small, TOML-like syntax shared by Flare Hardware
//! Descriptions (`*.fhd`), component manifests (`component.toml`) and boot
//! compositions (`*.boot.toml`). It is `no_std` so the same parser can later
//! run inside the user-space component manager.
//!
//! Supported:
//! - `# comments`
//! - `[section]` and `[dotted.section-name]` headers
//! - `key = value` where value is a `"string"`, an integer (`42`, `0x4000_0000`),
//!   `true`/`false`, or an `[array, of, values]` (may span lines, may nest,
//!   trailing comma allowed).
//!
//! Duplicate sections and duplicate keys inside a section are errors: a
//! hardware description must never silently say two things.

#![no_std]

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Str(String),
    Int(u64),
    Bool(bool),
    Array(Vec<Value>),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_int(&self) -> Option<u64> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }
    /// A string, or an array consisting only of strings.
    pub fn as_str_list(&self) -> Option<Vec<String>> {
        match self {
            Value::Str(s) => Some(alloc::vec![s.clone()]),
            Value::Array(a) => a.iter().map(|v| v.as_str().map(|s| s.to_string())).collect(),
            _ => None,
        }
    }
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Str(_) => "string",
            Value::Int(_) => "integer",
            Value::Bool(_) => "boolean",
            Value::Array(_) => "array",
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Str(s) => write!(f, "{:?}", s),
            Value::Int(i) if *i > 0xffff => write!(f, "{:#x}", i),
            Value::Int(i) => write!(f, "{}", i),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Array(a) => {
                f.write_str("[")?;
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                f.write_str("]")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub key: String,
    pub value: Value,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// Empty for the root (keys before the first header).
    pub name: String,
    pub line: u32,
    pub entries: Vec<Entry>,
}

impl Section {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entry(key).map(|e| &e.value)
    }
    pub fn entry(&self, key: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.key == key)
    }
    /// Line of `key` if present, else the section header line.
    pub fn line_of(&self, key: &str) -> u32 {
        self.entry(key).map(|e| e.line).unwrap_or(self.line)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// `sections[0]` is always the root section.
    pub sections: Vec<Section>,
}

impl Document {
    pub fn root(&self) -> &Section {
        &self.sections[0]
    }
    pub fn section(&self, name: &str) -> Option<&Section> {
        self.sections.iter().find(|s| s.name == name)
    }
    /// Non-root sections, in file order.
    pub fn named_sections(&self) -> impl Iterator<Item = &Section> {
        self.sections.iter().skip(1)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: u32,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    line: u32,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
    fn bump(&mut self) -> Option<u8> {
        let c = self.peek()?;
        self.i += 1;
        if c == b'\n' {
            self.line += 1;
        }
        Some(c)
    }
    fn err<T>(&self, message: impl Into<String>) -> Result<T, ParseError> {
        Err(ParseError { line: self.line, message: message.into() })
    }
    fn skip_inline_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r')) {
            self.bump();
        }
    }
    fn skip_comment(&mut self) {
        if self.peek() == Some(b'#') {
            while !matches!(self.peek(), None | Some(b'\n')) {
                self.bump();
            }
        }
    }
    /// Skip whitespace, comments and newlines.
    fn skip_all_ws(&mut self) {
        loop {
            self.skip_inline_ws();
            self.skip_comment();
            if self.peek() == Some(b'\n') {
                self.bump();
            } else {
                return;
            }
        }
    }
    fn expect(&mut self, c: u8) -> Result<(), ParseError> {
        if self.peek() == Some(c) {
            self.bump();
            Ok(())
        } else {
            self.err(format!("expected '{}'", c as char))
        }
    }
    fn ident(&mut self) -> Result<String, ParseError> {
        let start = self.i;
        while matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || c == b'_' || c == b'-') {
            self.bump();
        }
        if start == self.i {
            return self.err("expected a name");
        }
        Ok(String::from_utf8_lossy(&self.s[start..self.i]).into_owned())
    }
    fn ident_path(&mut self) -> Result<String, ParseError> {
        let mut name = self.ident()?;
        while self.peek() == Some(b'.') {
            self.bump();
            name.push('.');
            name.push_str(&self.ident()?);
        }
        Ok(name)
    }
    fn value(&mut self) -> Result<Value, ParseError> {
        match self.peek() {
            Some(b'"') => self.string(),
            Some(b'[') => {
                self.bump();
                let mut items = Vec::new();
                loop {
                    self.skip_all_ws();
                    if self.peek() == Some(b']') {
                        self.bump();
                        break;
                    }
                    items.push(self.value()?);
                    self.skip_all_ws();
                    match self.peek() {
                        Some(b',') => {
                            self.bump();
                        }
                        Some(b']') => {
                            self.bump();
                            break;
                        }
                        _ => return self.err("expected ',' or ']' in array"),
                    }
                }
                Ok(Value::Array(items))
            }
            Some(c) if c.is_ascii_digit() => self.integer(),
            Some(c) if c.is_ascii_alphabetic() => {
                let word = self.ident()?;
                match word.as_str() {
                    "true" => Ok(Value::Bool(true)),
                    "false" => Ok(Value::Bool(false)),
                    _ => self.err(format!("unexpected word '{}' (strings must be quoted)", word)),
                }
            }
            _ => self.err("expected a value"),
        }
    }
    fn string(&mut self) -> Result<Value, ParseError> {
        self.expect(b'"')?;
        let mut out = Vec::new();
        loop {
            match self.bump() {
                None | Some(b'\n') => return self.err("unterminated string"),
                Some(b'"') => break,
                Some(b'\\') => match self.bump() {
                    Some(b'n') => out.push(b'\n'),
                    Some(b't') => out.push(b'\t'),
                    Some(b'"') => out.push(b'"'),
                    Some(b'\\') => out.push(b'\\'),
                    _ => return self.err("unknown escape sequence"),
                },
                Some(c) => out.push(c),
            }
        }
        match String::from_utf8(out) {
            Ok(s) => Ok(Value::Str(s)),
            Err(_) => self.err("string is not valid UTF-8"),
        }
    }
    fn integer(&mut self) -> Result<Value, ParseError> {
        let (radix, start) = if self.s[self.i..].starts_with(b"0x") || self.s[self.i..].starts_with(b"0X") {
            self.i += 2;
            (16u64, self.i)
        } else {
            (10u64, self.i)
        };
        let mut v: u64 = 0;
        let mut digits = 0;
        while let Some(c) = self.peek() {
            let d = match c {
                b'_' => {
                    self.bump();
                    continue;
                }
                b'0'..=b'9' => (c - b'0') as u64,
                b'a'..=b'f' if radix == 16 => (c - b'a' + 10) as u64,
                b'A'..=b'F' if radix == 16 => (c - b'A' + 10) as u64,
                c if c.is_ascii_alphanumeric() => return self.err("invalid digit in integer"),
                _ => break,
            };
            v = match v.checked_mul(radix).and_then(|v| v.checked_add(d)) {
                Some(v) => v,
                None => return self.err("integer does not fit in 64 bits"),
            };
            digits += 1;
            self.bump();
        }
        if digits == 0 || self.i == start {
            return self.err("expected digits");
        }
        Ok(Value::Int(v))
    }
}

/// Parse a configuration document.
pub fn parse(src: &str) -> Result<Document, ParseError> {
    let mut p = Parser { s: src.as_bytes(), i: 0, line: 1 };
    let mut doc = Document { sections: alloc::vec![Section { name: String::new(), line: 1, entries: Vec::new() }] };
    loop {
        p.skip_all_ws();
        let Some(c) = p.peek() else { break };
        let line = p.line;
        if c == b'[' {
            p.bump();
            p.skip_inline_ws();
            let name = p.ident_path()?;
            p.skip_inline_ws();
            p.expect(b']')?;
            if doc.sections.iter().any(|s| s.name == name) {
                return p.err(format!("duplicate section [{}]", name));
            }
            doc.sections.push(Section { name, line, entries: Vec::new() });
        } else {
            let key = p.ident()?;
            p.skip_inline_ws();
            p.expect(b'=')?;
            p.skip_inline_ws();
            let value = p.value()?;
            let section = doc.sections.last_mut().unwrap();
            if section.entries.iter().any(|e| e.key == key) {
                return Err(ParseError { line, message: format!("duplicate key '{}'", key) });
            }
            section.entries.push(Entry { key, value, line });
        }
        p.skip_inline_ws();
        p.skip_comment();
        match p.peek() {
            None => break,
            Some(b'\n') => {
                p.bump();
            }
            Some(_) => return p.err("unexpected trailing characters"),
        }
    }
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn parses_all_value_kinds() {
        let doc = parse(
            r#"
            # root keys
            fhd = 0
            name = "qemu-virt" # trailing comment
            enabled = true

            [device.uart0]
            compatible = ["arm,pl011", "arm,primecell"]
            reg = [[0x0900_0000, 0x1000],
                   [0x0901_0000, 0x1000],]
            "#,
        )
        .unwrap();
        assert_eq!(doc.root().get("fhd"), Some(&Value::Int(0)));
        assert_eq!(doc.root().get("name").unwrap().as_str(), Some("qemu-virt"));
        assert_eq!(doc.root().get("enabled"), Some(&Value::Bool(true)));
        let uart = doc.section("device.uart0").unwrap();
        assert_eq!(uart.line, 7);
        assert_eq!(
            uart.get("compatible").unwrap().as_str_list().unwrap(),
            vec!["arm,pl011".to_string(), "arm,primecell".to_string()]
        );
        let reg = uart.get("reg").unwrap().as_array().unwrap();
        assert_eq!(reg[1], Value::Array(vec![Value::Int(0x0901_0000), Value::Int(0x1000)]));
        assert_eq!(uart.line_of("reg"), 9);
    }

    #[test]
    fn rejects_duplicates() {
        assert_eq!(parse("a = 1\na = 2").unwrap_err().line, 2);
        assert!(parse("[x]\n[x]").unwrap_err().message.contains("duplicate section"));
    }

    #[test]
    fn reports_errors_with_lines() {
        let e = parse("a = 1\nb = nope\n").unwrap_err();
        assert_eq!(e.line, 2);
        let e = parse("a = \"open\n").unwrap_err();
        assert!(e.message.contains("unterminated"));
        assert!(parse("a = 0x1_0000_0000_0000_0000").is_err());
        assert!(parse("a = 1 2").is_err());
        assert!(parse("a = 12abc").is_err());
    }

    #[test]
    fn escapes_and_display() {
        let doc = parse(r#"s = "a\"b\\c""#).unwrap();
        assert_eq!(doc.root().get("s").unwrap().as_str(), Some("a\"b\\c"));
        let v = Value::Array(vec![Value::Int(0x40000000), Value::Str("x".into())]);
        assert_eq!(v.to_string(), "[0x40000000, \"x\"]");
    }
}
