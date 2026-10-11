//! A small JSON reader and writer.
//!
//! The MCP server has to read what a client sends and write what it answers,
//! and the crate links nothing, so this is the smallest reader the protocol
//! needs: one complete value at a time, with the nesting capped so a hostile
//! request cannot blow the stack.

/// Nesting allowed before the reader gives up.
const DEPTH_LIMIT: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

impl Value {
    /// The value of `key`, when this value is an object that has it.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(fields) => fields
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    /// The elements of an array; empty for anything else, so a caller can
    /// walk a missing list without a match of its own.
    pub fn items(&self) -> &[Value] {
        match self {
            Value::Array(items) => items,
            _ => &[],
        }
    }

    pub fn text(&self) -> Option<&str> {
        match self {
            Value::String(text) => Some(text),
            _ => None,
        }
    }

    pub fn number(&self) -> Option<f64> {
        match self {
            Value::Number(value) => Some(*value),
            _ => None,
        }
    }

    pub fn boolean(&self) -> Option<bool> {
        match self {
            Value::Bool(value) => Some(*value),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// The value as compact JSON, for echoing an id or a small object back.
    pub fn render(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut String) {
        match self {
            Value::Null => out.push_str("null"),
            Value::Bool(true) => out.push_str("true"),
            Value::Bool(false) => out.push_str("false"),
            Value::Number(value) => {
                if value.fract() == 0.0 && value.abs() < 1e15 {
                    out.push_str(&format!("{:.0}", value));
                } else {
                    out.push_str(&format!("{}", value));
                }
            }
            Value::String(text) => out.push_str(&quote(text)),
            Value::Array(items) => {
                out.push('[');
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    item.write(out);
                }
                out.push(']');
            }
            Value::Object(fields) => {
                out.push('{');
                for (index, (name, value)) in fields.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    out.push_str(&quote(name));
                    out.push(':');
                    value.write(out);
                }
                out.push('}');
            }
        }
    }
}

/// A quoted string with the characters JSON reserves escaped.
pub fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other if (other as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", other as u32));
            }
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// Parse one complete JSON value; trailing text is refused.
pub fn parse(text: &str) -> Result<Value, String> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        at: 0,
    };
    parser.skip_space();
    let value = parser.value(0)?;
    parser.skip_space();
    if parser.at != parser.bytes.len() {
        return Err(parser.error("trailing characters"));
    }
    Ok(value)
}

struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn error(&self, message: &str) -> String {
        format!("{} at byte {}", message, self.at)
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn skip_space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), String> {
        if self.peek() == Some(byte) {
            self.at += 1;
            Ok(())
        } else {
            Err(self.error(&format!("expected '{}'", char::from(byte))))
        }
    }

    fn word(&mut self, word: &str, value: Value) -> Result<Value, String> {
        if self.bytes[self.at..].starts_with(word.as_bytes()) {
            self.at += word.len();
            Ok(value)
        } else {
            Err(self.error(&format!("expected {}", word)))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > DEPTH_LIMIT {
            return Err(self.error("too deeply nested"));
        }
        match self.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => Ok(Value::String(self.string()?)),
            Some(b't') => self.word("true", Value::Bool(true)),
            Some(b'f') => self.word("false", Value::Bool(false)),
            Some(b'n') => self.word("null", Value::Null),
            Some(_) => self.number(),
            None => Err(self.error("unexpected end of input")),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value, String> {
        self.expect(b'{')?;
        let mut fields: Vec<(String, Value)> = Vec::new();
        self.skip_space();
        if self.peek() == Some(b'}') {
            self.at += 1;
            return Ok(Value::Object(fields));
        }
        loop {
            self.skip_space();
            let key = self.string()?;
            self.skip_space();
            self.expect(b':')?;
            self.skip_space();
            let value = self.value(depth + 1)?;
            fields.push((key, value));
            self.skip_space();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b'}') => {
                    self.at += 1;
                    break;
                }
                _ => return Err(self.error("expected ',' or '}'")),
            }
        }
        Ok(Value::Object(fields))
    }

    fn array(&mut self, depth: usize) -> Result<Value, String> {
        self.expect(b'[')?;
        let mut items: Vec<Value> = Vec::new();
        self.skip_space();
        if self.peek() == Some(b']') {
            self.at += 1;
            return Ok(Value::Array(items));
        }
        loop {
            self.skip_space();
            items.push(self.value(depth + 1)?);
            self.skip_space();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b']') => {
                    self.at += 1;
                    break;
                }
                _ => return Err(self.error("expected ',' or ']'")),
            }
        }
        Ok(Value::Array(items))
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let byte = self
                .peek()
                .ok_or_else(|| self.error("unterminated string"))?;
            match byte {
                b'"' => {
                    self.at += 1;
                    break;
                }
                b'\\' => {
                    self.at += 1;
                    self.escape(&mut out)?;
                }
                0x00..=0x1f => return Err(self.error("control character in string")),
                _ => {
                    let rest = std::str::from_utf8(&self.bytes[self.at..])
                        .map_err(|_| self.error("invalid UTF-8"))?;
                    let character = rest.chars().next().unwrap();
                    out.push(character);
                    self.at += character.len_utf8();
                }
            }
        }
        Ok(out)
    }

    fn escape(&mut self, out: &mut String) -> Result<(), String> {
        let byte = self
            .peek()
            .ok_or_else(|| self.error("unterminated escape"))?;
        self.at += 1;
        match byte {
            b'"' => out.push('"'),
            b'\\' => out.push('\\'),
            b'/' => out.push('/'),
            b'b' => out.push('\u{8}'),
            b'f' => out.push('\u{c}'),
            b'n' => out.push('\n'),
            b'r' => out.push('\r'),
            b't' => out.push('\t'),
            b'u' => {
                let start = self.hex4()?;
                let code = if (0xd800..0xdc00).contains(&start) {
                    // A surrogate pair: the low half follows in another escape.
                    self.expect(b'\\')?;
                    self.expect(b'u')?;
                    let low = self.hex4()?;
                    if (0xdc00..0xe000).contains(&low) {
                        0x1_0000 + ((start - 0xd800) << 10) + (low - 0xdc00)
                    } else {
                        0xfffd
                    }
                } else if (0xdc00..0xe000).contains(&start) {
                    0xfffd
                } else {
                    start
                };
                out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
            }
            _ => return Err(self.error("unknown escape")),
        }
        Ok(())
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let slice = self
            .bytes
            .get(self.at..self.at + 4)
            .ok_or_else(|| self.error("short \\u escape"))?;
        let text = std::str::from_utf8(slice).map_err(|_| self.error("invalid escape"))?;
        let code = u32::from_str_radix(text, 16).map_err(|_| self.error("invalid escape"))?;
        self.at += 4;
        Ok(code)
    }

    fn number(&mut self) -> Result<Value, String> {
        let start = self.at;
        if matches!(self.peek(), Some(b'-' | b'+')) {
            self.at += 1;
        }
        while matches!(
            self.peek(),
            Some(b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
        ) {
            self.at += 1;
        }
        let text = std::str::from_utf8(&self.bytes[start..self.at])
            .map_err(|_| self.error("invalid number"))?;
        text.parse::<f64>()
            .map(Value::Number)
            .map_err(|_| self.error("invalid number"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_round_trip_through_render_and_parse() {
        let text = r#"{"a":[1,2.5,true,false,null],"b":{"c":"x\ny"}}"#;
        let value = parse(text).unwrap();
        let items = value.get("a").unwrap().items();
        assert_eq!(items.len(), 5);
        assert_eq!(items[0].number(), Some(1.0));
        assert_eq!(items[1].number(), Some(2.5));
        assert_eq!(items[2].boolean(), Some(true));
        assert_eq!(items[3].boolean(), Some(false));
        assert!(items[4].is_null());
        assert_eq!(
            value.get("b").unwrap().get("c").unwrap().text(),
            Some("x\ny")
        );
        assert_eq!(parse(&value.render()).unwrap(), value);
    }

    #[test]
    fn missing_keys_and_wrong_types_read_as_empty() {
        let value = parse(r#"{"n":1}"#).unwrap();
        assert!(value.get("nope").is_none());
        assert_eq!(value.get("n").unwrap().text(), None);
        assert_eq!(value.get("n").unwrap().items(), [] as [Value; 0]);
        assert_eq!(parse("[1,2]").unwrap().get("a"), None);
        assert!(parse("true").unwrap().boolean().unwrap());
        assert_eq!(parse("\"hi\"").unwrap().text(), Some("hi"));
    }

    #[test]
    fn every_escape_is_understood() {
        let value = parse(r#"["\"\\\/\b\f\n\r\t","\u00e9","\ud83d\ude00"]"#).unwrap();
        let items = value.items();
        assert_eq!(items[0].text(), Some("\"\\/\u{8}\u{c}\n\r\t"));
        assert_eq!(items[1].text(), Some("é"));
        assert_eq!(items[2].text(), Some("😀"));
    }

    #[test]
    fn numbers_render_without_a_pointless_fraction() {
        assert_eq!(Value::Number(3.0).render(), "3");
        assert_eq!(Value::Number(-1.5).render(), "-1.5");
    }

    #[test]
    fn quotes_escape_what_json_reserves() {
        assert_eq!(quote("a\"b\\c\nd\u{1}"), "\"a\\\"b\\\\c\\nd\\u0001\"");
        assert_eq!(parse(&quote("héllo")).unwrap().text(), Some("héllo"));
    }

    #[test]
    fn broken_input_is_refused_with_a_position() {
        for text in [
            "{\"a\":1",
            "{\"a\" 1}",
            "[1,]",
            "tru",
            "\"open",
            "{\"a\":\"\\q\"}",
            "1 2",
            "[1,2]x",
        ] {
            let error = parse(text).unwrap_err();
            assert!(error.contains(" at byte "), "{}: {}", text, error);
        }
        assert!(parse("[1,2,]").is_err());
        assert!(parse("").unwrap_err().contains("unexpected end of input"));
    }

    #[test]
    fn a_control_character_in_a_string_is_refused() {
        assert!(parse("\"a\nb\"").unwrap_err().contains("control character"));
    }

    #[test]
    fn nesting_is_capped() {
        let deep = "[".repeat(DEPTH_LIMIT + 4) + &"]".repeat(DEPTH_LIMIT + 4);
        assert!(parse(&deep).unwrap_err().contains("too deeply nested"));
        let allowed = "[".repeat(DEPTH_LIMIT) + &"]".repeat(DEPTH_LIMIT);
        assert!(parse(&allowed).is_ok());
    }

    #[test]
    fn whitespace_between_tokens_is_fine() {
        let value = parse(" {\n\t\"a\" : [ 1 , 2 ] , \"b\" : { } } ").unwrap();
        assert_eq!(value.get("a").unwrap().items().len(), 2);
        assert!(value.get("b").unwrap().items().is_empty());
        assert_eq!(value.get("b").unwrap().render(), "{}");
    }
}
