//! A dependency-free JSON reader, just enough to check the hand-written
//! serializer that replaces Python's `json.dumps`.

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    Text(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(entries) => entries
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    pub fn number(&self) -> f64 {
        match self {
            Value::Number(value) => *value,
            other => panic!("expected a number, got {:?}", other),
        }
    }

    pub fn integer(&self) -> u64 {
        self.number() as u64
    }

    pub fn text(&self) -> &str {
        match self {
            Value::Text(value) => value,
            other => panic!("expected a string, got {:?}", other),
        }
    }

    pub fn flag(&self) -> bool {
        match self {
            Value::Bool(value) => *value,
            other => panic!("expected a bool, got {:?}", other),
        }
    }

    pub fn items(&self) -> &[Value] {
        match self {
            Value::Array(values) => values,
            other => panic!("expected an array, got {:?}", other),
        }
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

pub fn parse(text: &str) -> Value {
    let mut reader = Reader {
        bytes: text.as_bytes(),
        at: 0,
    };
    reader.space();
    let value = reader.value();
    reader.space();
    assert_eq!(reader.at, reader.bytes.len(), "trailing junk in JSON");
    value
}

impl<'a> Reader<'a> {
    fn space(&mut self) {
        while self.at < self.bytes.len() && self.bytes[self.at].is_ascii_whitespace() {
            self.at += 1;
        }
    }

    fn peek(&self) -> u8 {
        self.bytes[self.at]
    }

    fn expect(&mut self, byte: u8) {
        assert_eq!(self.peek(), byte, "unexpected byte at {}", self.at);
        self.at += 1;
    }

    fn literal(&mut self, word: &[u8]) {
        assert!(
            self.bytes[self.at..].starts_with(word),
            "bad literal at {}",
            self.at
        );
        self.at += word.len();
    }

    fn value(&mut self) -> Value {
        match self.peek() {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => Value::Text(self.string()),
            b't' => {
                self.literal(b"true");
                Value::Bool(true)
            }
            b'f' => {
                self.literal(b"false");
                Value::Bool(false)
            }
            b'n' => {
                self.literal(b"null");
                Value::Null
            }
            _ => self.number(),
        }
    }

    fn object(&mut self) -> Value {
        self.expect(b'{');
        let mut entries = Vec::new();
        self.space();
        if self.peek() == b'}' {
            self.at += 1;
            return Value::Object(entries);
        }
        loop {
            self.space();
            let key = self.string();
            self.space();
            self.expect(b':');
            self.space();
            entries.push((key, self.value()));
            self.space();
            match self.peek() {
                b',' => self.at += 1,
                b'}' => {
                    self.at += 1;
                    return Value::Object(entries);
                }
                other => panic!("unexpected {:?} in object at {}", other as char, self.at),
            }
        }
    }

    fn array(&mut self) -> Value {
        self.expect(b'[');
        let mut values = Vec::new();
        self.space();
        if self.peek() == b']' {
            self.at += 1;
            return Value::Array(values);
        }
        loop {
            self.space();
            values.push(self.value());
            self.space();
            match self.peek() {
                b',' => self.at += 1,
                b']' => {
                    self.at += 1;
                    return Value::Array(values);
                }
                other => panic!("unexpected {:?} in array at {}", other as char, self.at),
            }
        }
    }

    fn string(&mut self) -> String {
        self.expect(b'"');
        let mut out = String::new();
        loop {
            let byte = self.bytes[self.at];
            self.at += 1;
            match byte {
                b'"' => return out,
                b'\\' => {
                    let escape = self.bytes[self.at];
                    self.at += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'b' => out.push('\u{08}'),
                        b'f' => out.push('\u{0c}'),
                        b'u' => {
                            let hex =
                                std::str::from_utf8(&self.bytes[self.at..self.at + 4]).unwrap();
                            self.at += 4;
                            let code = u32::from_str_radix(hex, 16).unwrap();
                            out.push(char::from_u32(code).unwrap());
                        }
                        other => panic!("bad escape {:?}", other as char),
                    }
                }
                _ => out.push(byte as char),
            }
        }
    }

    fn number(&mut self) -> Value {
        let start = self.at;
        while self.at < self.bytes.len()
            && matches!(
                self.bytes[self.at],
                b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9'
            )
        {
            self.at += 1;
        }
        let text = std::str::from_utf8(&self.bytes[start..self.at]).unwrap();
        Value::Number(text.parse().expect("bad number"))
    }
}
