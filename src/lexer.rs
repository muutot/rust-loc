//! Rust source tokeniser.
//!
//! One pass over the bytes classifies every physical line as code, comment
//! (optionally doc) or blank, and collects the cheap Rust metrics: approximate
//! cyclomatic complexity, unsafe keyword use and test functions. It also
//! records the line spans of test-only cfg attributes so callers can exclude
//! test code without parsing the file again.

use std::collections::HashSet;

/// Halstead token counts for one scope: the business code, or the whole file
/// when `--include-tests` asks for everything.
///
/// "Operators" are Rust punctuation and keywords, "operands" are names,
/// numbers and literals. Punctuation is counted per character, so `->` counts
/// as two operators; that is an approximation of the token-tree tools the
/// volume formula comes from, and it is the volume alone that feeds the index.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Halstead {
    pub operators: u32,
    pub operands: u32,
    pub distinct_operators: u32,
    pub distinct_operands: u32,
}

impl Halstead {
    /// `N`, every token seen.
    pub fn length(self) -> u32 {
        self.operators + self.operands
    }

    /// `n`, the tokens that are distinct.
    pub fn vocabulary(self) -> u32 {
        self.distinct_operators + self.distinct_operands
    }

    /// `V = N log2 n`, the program volume.
    pub fn volume(self) -> f64 {
        let vocabulary = self.vocabulary();
        if vocabulary == 0 {
            0.0
        } else {
            f64::from(self.length()) * f64::from(vocabulary).log2()
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Metrics {
    pub complexity: u32,
    /// Control-flow nesting counted the way Sonar does: one per branch, plus
    /// one for every level of nesting it sits inside.
    pub cognitive: u32,
    pub unsafe_kw: u32,
    pub test_fns: u32,
    pub halstead: Halstead,
}

/// One `fn` body with the cheap per-function metrics.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FunctionInfo {
    pub name: String,
    pub line: u32,
    pub code: u32,
    pub complexity: u32,
    /// Cognitive complexity inside this body, independent of the enclosing
    /// function, so a closure does not inherit its parent's nesting.
    pub cognitive: u32,
    pub params: u32,
    pub depth: u32,
    pub test: bool,
}

/// Panic-shaped calls and lint escapes a reviewer usually wants counted.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Smells {
    pub unwrap: u32,
    pub expect: u32,
    pub panic: u32,
    pub todo: u32,
    pub unimplemented: u32,
    pub allow: u32,
}

impl Smells {
    pub fn total(self) -> u32 {
        self.unwrap + self.expect + self.panic + self.todo + self.unimplemented + self.allow
    }
}

/// A `fn` signature being read; the body opens at the next `{`.
#[derive(Debug, Default, Clone)]
struct PendingFn {
    name: String,
    line: usize,
    commas: u32,
    paren: u32,
    angle: u32,
    seen_param: bool,
    test: bool,
}

/// A `fn` body currently open on the brace stack.
#[derive(Debug, Clone)]
struct OpenFn {
    name: String,
    line: usize,
    body_depth: u32,
    params: u32,
    complexity: u32,
    cognitive: u32,
    /// Control-flow depth when the body opened, so the nesting penalty can
    /// be counted from the function boundary.
    control_base: u32,
    max_depth: u32,
    test: bool,
}

/// Accumulates one Halstead scope while the lexer walks the file. The token
/// slices borrow the file text, so nothing outlives `analyze`. The Python
/// reader in `crate::pylexer` borrows it too, so both languages feed the
/// volume formula the same way.
#[derive(Default)]
pub(crate) struct TokenCounter<'a> {
    operators: u32,
    operands: u32,
    distinct_operators: HashSet<&'a [u8]>,
    distinct_operands: HashSet<&'a [u8]>,
}

impl<'a> TokenCounter<'a> {
    pub(crate) fn operator(&mut self, token: &'a [u8]) {
        self.operators += 1;
        self.distinct_operators.insert(token);
    }

    pub(crate) fn operand(&mut self, token: &'a [u8]) {
        self.operands += 1;
        self.distinct_operands.insert(token);
    }

    pub(crate) fn finish(&self) -> Halstead {
        Halstead {
            operators: self.operators,
            operands: self.operands,
            distinct_operators: self.distinct_operators.len() as u32,
            distinct_operands: self.distinct_operands.len() as u32,
        }
    }
}

#[derive(Debug)]
pub struct Analysis {
    pub total: usize,
    pub code: Vec<bool>,
    pub comment: Vec<bool>,
    pub doc: Vec<bool>,
    pub is_test: Vec<bool>,
    pub crate_test: bool,
    pub sibling_targets: Vec<String>,
    pub metrics: Metrics,
    pub metrics_all: Metrics,
    pub smells: Smells,
    pub smells_all: Smells,
    pub functions: Vec<FunctionInfo>,
}

pub const COMPLEXITY_KEYWORDS: [&str; 6] = ["if", "else", "for", "while", "loop", "match"];
const COMPLEXITY_OPERATORS: [&[u8]; 4] = [b"&&", b"||", b"==", b"!="];

/// Rust keywords, sorted, counted as Halstead operators rather than operands.
/// A unit test keeps the sort order honest.
const KEYWORDS: [&[u8]; 39] = [
    b"Self",
    b"as",
    b"async",
    b"await",
    b"box",
    b"break",
    b"const",
    b"continue",
    b"crate",
    b"dyn",
    b"else",
    b"enum",
    b"extern",
    b"false",
    b"fn",
    b"for",
    b"if",
    b"impl",
    b"in",
    b"let",
    b"loop",
    b"match",
    b"mod",
    b"move",
    b"mut",
    b"pub",
    b"ref",
    b"return",
    b"self",
    b"static",
    b"struct",
    b"super",
    b"trait",
    b"true",
    b"type",
    b"unsafe",
    b"use",
    b"where",
    b"while",
];

const CODE: u8 = 0;
const LINE_COMMENT: u8 = 1;
const BLOCK_COMMENT: u8 = 2;
const STRING: u8 = 3;
const RAW_STRING: u8 = 4;
const CHAR: u8 = 5;

#[inline]
fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

#[inline]
fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80
}

#[inline]
fn is_ident_continue(byte: u8) -> bool {
    is_ident_start(byte) || byte.is_ascii_digit()
}

/// Physical line count, wc-style: a trailing newline does not add a line.
pub fn physical_lines(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    let newlines = text
        .as_bytes()
        .iter()
        .filter(|byte| **byte == b'\n')
        .count();
    if text.ends_with('\n') {
        newlines
    } else {
        newlines + 1
    }
}

/// Translate CRLF and lone CR to LF, matching a text-mode read.
pub fn normalize_newlines(text: &str) -> std::borrow::Cow<'_, str> {
    if text.contains('\r') {
        std::borrow::Cow::Owned(text.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        std::borrow::Cow::Borrowed(text)
    }
}

/// Match an attribute at index i, returning its trimmed body and end offset.
fn match_attr(bytes: &[u8], i: usize) -> Option<(String, usize)> {
    let n = bytes.len();
    if i >= n || bytes[i] != b'#' {
        return None;
    }
    let mut j = i + 1;
    while j < n && is_space(bytes[j]) {
        j += 1;
    }
    if j < n && bytes[j] == b'!' {
        j += 1;
    }
    while j < n && is_space(bytes[j]) {
        j += 1;
    }
    if j >= n || bytes[j] != b'[' {
        return None;
    }
    j += 1;
    let start = j;
    while j < n && bytes[j] != b']' {
        j += 1;
    }
    if j >= n {
        return None;
    }
    let body = String::from_utf8_lossy(&bytes[start..j]).trim().to_string();
    Some((body, j + 1))
}

/// True for cfg(test) and cfg(all(test, ..)); any(test, ..) and not(test) stay.
fn cfg_selects_test(body: &str) -> bool {
    let trimmed = body.trim();
    let rest = match trimmed.strip_prefix("cfg") {
        Some(rest) => rest.trim_start(),
        None => return false,
    };
    let inner = match rest.strip_prefix('(') {
        Some(rest) => match rest.rfind(')') {
            Some(end) if rest[end + 1..].trim().is_empty() => &rest[..end],
            _ => return false,
        },
        None => return false,
    };
    let inner = inner.trim();
    if inner == "test" {
        return true;
    }
    if let Some(body) = inner.strip_prefix("all(") {
        if let Some(end) = body.rfind(')') {
            return body[..end].split(',').any(|part| part.trim() == "test");
        }
    }
    false
}

/// Match a path attribute body and return the referenced file.
fn path_attribute(body: &str) -> Option<&str> {
    let rest = body.trim().strip_prefix("path")?.trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    if end == 0 || !rest[end + 1..].is_empty() {
        return None;
    }
    Some(&rest[..end])
}

/// True for test, rstest and for qualified paths such as foo::bar::test.
fn is_test_attr(attr: &str) -> bool {
    if attr.is_empty() {
        return false;
    }
    let parts: Vec<&str> = attr.split("::").collect();
    let (last, head) = match parts.split_last() {
        Some(split) => split,
        None => return false,
    };
    if *last != "test" && *last != "rstest" {
        return false;
    }
    head.iter().all(|part| {
        !part.is_empty() && {
            let bytes = part.as_bytes();
            is_ident_start(bytes[0]) && bytes[1..].iter().all(|byte| is_ident_continue(*byte))
        }
    })
}

/// True for lint escapes such as #[allow(..)] and #![allow(..)].
fn attr_is_allow(body: &str) -> bool {
    let rest = match body.trim().strip_prefix("allow") {
        Some(rest) => rest,
        None => return false,
    };
    rest.is_empty() || rest.starts_with('(')
}

/// Detect a raw string opener starting exactly at i.
///
/// Returns the opener length (prefix + `r` + hashes + quote) and the hash
/// count so the caller can find the matching terminator. The prefix must sit
/// at i itself; looking ahead for an `r` would misread `(r"..."` as a
/// `cr`-prefixed literal.
fn raw_string_len(bytes: &[u8], i: usize) -> Option<(usize, usize)> {
    let mut j = match bytes.get(i) {
        Some(b'r') => i + 1,
        Some(b'b') | Some(b'c') if bytes.get(i + 1) == Some(&b'r') => i + 2,
        _ => return None,
    };
    let mut hashes = 0usize;
    while bytes.get(j) == Some(&b'#') {
        hashes += 1;
        j += 1;
    }
    if bytes.get(j) == Some(&b'"') {
        Some((j + 1 - i, hashes))
    } else {
        None
    }
}

#[inline]
fn identifier_end(bytes: &[u8], i: usize) -> Option<usize> {
    if i >= bytes.len() || !is_ident_start(bytes[i]) {
        return None;
    }
    let mut j = i + 1;
    while j < bytes.len() && is_ident_continue(bytes[j]) {
        j += 1;
    }
    Some(j)
}

/// End of a numeric literal. Underscores and the base prefixes count as part
/// of it, a `.` only when a digit follows (so the range `1..2` is two
/// numbers), and a trailing type suffix like `u32` comes along.
fn number_end(bytes: &[u8], start: usize) -> usize {
    let n = bytes.len();
    let mut i = start;
    if bytes[i] == b'0'
        && i + 1 < n
        && matches!(bytes[i + 1], b'x' | b'X' | b'b' | b'B' | b'o' | b'O')
    {
        i += 2;
        while i < n && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
            i += 1;
        }
        return i;
    }
    let digits = |index: &mut usize| {
        while *index < n && (bytes[*index].is_ascii_digit() || bytes[*index] == b'_') {
            *index += 1;
        }
    };
    digits(&mut i);
    if i < n && bytes[i] == b'.' && i + 1 < n && bytes[i + 1].is_ascii_digit() {
        i += 1;
        digits(&mut i);
    }
    if i < n && matches!(bytes[i], b'e' | b'E') {
        let mut exponent = i + 1;
        if exponent < n && matches!(bytes[exponent], b'+' | b'-') {
            exponent += 1;
        }
        if exponent < n && bytes[exponent].is_ascii_digit() {
            i = exponent;
            digits(&mut i);
        }
    }
    while i < n && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
        i += 1;
    }
    i
}

/// Classify one file. The text must already have normalised newlines.
pub fn analyze(text: &str) -> Analysis {
    let bytes = text.as_bytes();
    let total = physical_lines(text);
    let mut code = vec![false; total];
    let mut comment = vec![false; total];
    let mut doc = vec![false; total];
    let mut spans: Vec<(usize, usize)> = Vec::new();

    let mut metrics = Metrics::default();
    let mut metrics_all = Metrics::default();

    let mut line = 0usize;
    let mut i = 0usize;
    let n = bytes.len();
    let mut state = CODE;
    let mut block_depth = 0usize;
    let mut raw_hashes = 0usize;
    let mut escaped = false;
    let mut comment_is_doc = false;
    let mut pending_test: Option<usize> = None;
    let mut pending_is_mod = false;
    let mut pending_mod_name: Option<String> = None;
    let mut pending_path: Option<String> = None;
    let mut pending_attrs: Vec<String> = Vec::new();
    let mut skipping_test = false;
    let mut test_depth = 0usize;
    let mut test_start = 0usize;
    let mut crate_test = false;
    let mut sibling_targets: Vec<String> = Vec::new();
    let mut smells = Smells::default();
    let mut smells_all = Smells::default();
    let mut functions: Vec<FunctionInfo> = Vec::new();
    let mut fn_spans: Vec<(usize, usize)> = Vec::new();
    let mut open_fns: Vec<OpenFn> = Vec::new();
    let mut pending_fn: Option<PendingFn> = None;
    let mut brace_depth: u32 = 0;
    // Cognitive complexity needs to know how deep the control flow is, which
    // the brace stack alone cannot say (a `struct {` nests no control flow).
    let mut control_depth: u32 = 0;
    let mut block_control: Vec<bool> = Vec::new();
    let mut pending_control = false;
    let mut after_else = false;
    // Where the literal being read started, and whether it is business code.
    let mut literal_start = 0usize;
    let mut literal_business = false;
    let mut tokens: TokenCounter<'_> = TokenCounter::default();
    let mut tokens_all: TokenCounter<'_> = TokenCounter::default();

    while i < n {
        let ch = bytes[i];

        if state == LINE_COMMENT {
            if ch == b'\n' {
                state = CODE;
                line += 1;
            } else if line < total {
                comment[line] = true;
                if comment_is_doc {
                    doc[line] = true;
                }
            }
            i += 1;
            continue;
        }

        if state == BLOCK_COMMENT {
            if line < total {
                comment[line] = true;
                if comment_is_doc {
                    doc[line] = true;
                }
            }
            if ch == b'\n' {
                line += 1;
                i += 1;
                continue;
            }
            if ch == b'/' && bytes.get(i + 1) == Some(&b'*') {
                block_depth += 1;
                i += 2;
                continue;
            }
            if ch == b'*' && bytes.get(i + 1) == Some(&b'/') {
                block_depth -= 1;
                i += 2;
                if block_depth == 0 {
                    state = CODE;
                    comment_is_doc = false;
                }
                continue;
            }
            i += 1;
            continue;
        }

        if state == STRING {
            if line < total {
                code[line] = true;
            }
            if ch == b'\n' {
                line += 1;
            } else if escaped {
                escaped = false;
            } else if ch == b'\\' {
                escaped = true;
            } else if ch == b'"' {
                state = CODE;
                let literal = &bytes[literal_start..=i];
                if literal_business {
                    tokens.operand(literal);
                }
                tokens_all.operand(literal);
            }
            i += 1;
            continue;
        }

        if state == RAW_STRING {
            if line < total {
                code[line] = true;
            }
            if ch == b'\n' {
                line += 1;
                i += 1;
                continue;
            }
            if ch == b'"'
                && i + 1 + raw_hashes <= n
                && bytes[i + 1..i + 1 + raw_hashes]
                    .iter()
                    .all(|byte| *byte == b'#')
            {
                let end = i + 1 + raw_hashes;
                let literal = &bytes[literal_start..end];
                if literal_business {
                    tokens.operand(literal);
                }
                tokens_all.operand(literal);
                i = end;
                state = CODE;
                continue;
            }
            i += 1;
            continue;
        }

        if state == CHAR {
            if line < total {
                code[line] = true;
            }
            if escaped {
                escaped = false;
            } else if ch == b'\\' {
                escaped = true;
            } else if ch == b'\'' {
                state = CODE;
                let literal = &bytes[literal_start..=i];
                if literal_business {
                    tokens.operand(literal);
                }
                tokens_all.operand(literal);
            }
            i += 1;
            continue;
        }

        // ---- code state ----
        if ch == b'\n' {
            line += 1;
            i += 1;
            continue;
        }

        if ch == b'#' {
            if let Some((body, end)) = match_attr(bytes, i) {
                if attr_is_allow(&body) {
                    smells_all.allow += 1;
                    if !skipping_test {
                        smells.allow += 1;
                    }
                }
                if !skipping_test {
                    if cfg_selects_test(&body) {
                        if bytes[i..].starts_with(b"#!") {
                            crate_test = true;
                        } else {
                            pending_test = Some(line);
                            pending_fn = None;
                        }
                    } else if let Some(target) = path_attribute(&body) {
                        pending_path = Some(target.to_string());
                    } else {
                        pending_attrs.push(body);
                    }
                }
                let extra = bytes[i..end].iter().filter(|byte| **byte == b'\n').count();
                for flag in code
                    .iter_mut()
                    .take((line + extra + 1).min(total))
                    .skip(line)
                {
                    *flag = true;
                }
                line += extra;
                i = end;
            } else {
                i += 1;
            }
            continue;
        }

        if is_space(ch) {
            i += 1;
            continue;
        }

        let comment_open = bytes[i..].starts_with(b"///") && !bytes[i..].starts_with(b"////")
            || bytes[i..].starts_with(b"//!");

        if bytes[i..].starts_with(b"//") {
            if line < total {
                comment[line] = true;
                if comment_open {
                    doc[line] = true;
                }
            }
            comment_is_doc = comment_open;
            state = LINE_COMMENT;
            i += 2;
            continue;
        }

        if bytes[i..].starts_with(b"/*") {
            let block_doc = bytes[i..].starts_with(b"/**") && !bytes[i..].starts_with(b"/***")
                || bytes[i..].starts_with(b"/*!");
            if line < total {
                comment[line] = true;
                if block_doc {
                    doc[line] = true;
                }
            }
            comment_is_doc = block_doc;
            block_depth = 1;
            state = BLOCK_COMMENT;
            i += 2;
            continue;
        }

        if let Some((len, hashes)) = raw_string_len(bytes, i) {
            if line < total {
                code[line] = true;
            }
            raw_hashes = hashes;
            literal_start = i;
            literal_business = !skipping_test;
            i += len;
            state = RAW_STRING;
            continue;
        }

        if ch == b'"' {
            if line < total {
                code[line] = true;
            }
            escaped = false;
            literal_start = i;
            literal_business = !skipping_test;
            state = STRING;
            i += 1;
            continue;
        }

        if ch == b'\'' {
            if line < total {
                code[line] = true;
            }
            let char_like = i + 1 < n
                && (bytes[i + 1] == b'\\'
                    || (i + 2 < n && bytes[i + 2] == b'\'' && bytes[i + 1] != b'\n'));
            if char_like {
                escaped = false;
                literal_start = i;
                literal_business = !skipping_test;
                state = CHAR;
            }
            i += 1;
            continue;
        }

        if let Some(end) = identifier_end(bytes, i) {
            if line < total {
                code[line] = true;
            }
            let word = &bytes[i..end];
            let macro_call = bytes.get(end) == Some(&b'!');
            // `unwrap`/`expect` are only smells when they are the method being
            // called; a struct field that happens to share the name is not one.
            let call_like = bytes.get(end) == Some(&b'(') || bytes[end..].starts_with(b"::<");
            let is_keyword = KEYWORDS.binary_search(&word).is_ok();
            if !skipping_test {
                if is_keyword {
                    tokens.operator(word);
                } else {
                    tokens.operand(word);
                }
            }
            if is_keyword {
                tokens_all.operator(word);
            } else {
                tokens_all.operand(word);
            }
            if COMPLEXITY_KEYWORDS
                .iter()
                .any(|keyword| keyword.as_bytes() == word)
            {
                metrics_all.complexity += 1;
                if !skipping_test {
                    metrics.complexity += 1;
                    if let Some(open) = open_fns.last_mut() {
                        open.complexity += 1;
                    }
                }
            } else if word == b"unsafe" {
                metrics_all.unsafe_kw += 1;
                if !skipping_test {
                    metrics.unsafe_kw += 1;
                }
            }
            if word == b"unwrap" && call_like {
                smells_all.unwrap += 1;
                if !skipping_test {
                    smells.unwrap += 1;
                }
            } else if word == b"expect" && call_like {
                smells_all.expect += 1;
                if !skipping_test {
                    smells.expect += 1;
                }
            } else if word == b"panic" && macro_call {
                smells_all.panic += 1;
                if !skipping_test {
                    smells.panic += 1;
                }
            } else if word == b"todo" && macro_call {
                smells_all.todo += 1;
                if !skipping_test {
                    smells.todo += 1;
                }
            } else if word == b"unimplemented" && macro_call {
                smells_all.unimplemented += 1;
                if !skipping_test {
                    smells.unimplemented += 1;
                }
            }
            if COMPLEXITY_KEYWORDS
                .iter()
                .any(|keyword| keyword.as_bytes() == word)
            {
                let is_else_if = word == b"if" && after_else;
                after_else = word == b"else";
                if !is_else_if {
                    metrics_all.cognitive += 1 + control_depth;
                }
                if !skipping_test {
                    if !is_else_if {
                        metrics.cognitive += 1 + control_depth;
                        if let Some(open) = open_fns.last_mut() {
                            open.cognitive += 1 + control_depth.saturating_sub(open.control_base);
                        }
                    }
                    pending_control = true;
                }
            } else {
                after_else = false;
            }
            if word == b"fn" {
                let is_test_fn = pending_attrs.iter().any(|attr| is_test_attr(attr));
                if is_test_fn {
                    metrics_all.test_fns += 1;
                    if !skipping_test {
                        metrics.test_fns += 1;
                    }
                }
                pending_attrs.clear();
                pending_fn = if skipping_test || pending_test.is_some() {
                    None
                } else {
                    Some(PendingFn {
                        name: String::new(),
                        line,
                        commas: 0,
                        paren: 0,
                        angle: 0,
                        seen_param: false,
                        test: is_test_fn,
                    })
                };
            } else if let Some(pending) = pending_fn.as_mut() {
                if pending.name.is_empty() {
                    pending.name = String::from_utf8_lossy(word).into_owned();
                } else if pending.paren >= 1 {
                    pending.seen_param = true;
                }
            } else if pending_test.is_some() {
                if word == b"mod" {
                    pending_is_mod = true;
                    pending_mod_name = None;
                } else if pending_is_mod && pending_mod_name.is_none() {
                    pending_mod_name = Some(String::from_utf8_lossy(word).into_owned());
                }
            }
            i = end;
            continue;
        }

        if ch.is_ascii_digit() {
            let end = number_end(bytes, i);
            if line < total {
                code[line] = true;
            }
            let number = &bytes[i..end];
            if !skipping_test {
                tokens.operand(number);
            }
            tokens_all.operand(number);
            i = end;
            continue;
        }

        if let Some(operator) = COMPLEXITY_OPERATORS
            .iter()
            .find(|operator| bytes[i..].starts_with(operator))
        {
            if line < total {
                code[line] = true;
            }
            metrics_all.complexity += 1;
            if !skipping_test {
                metrics.complexity += 1;
                if let Some(open) = open_fns.last_mut() {
                    open.complexity += 1;
                }
                tokens.operator(operator);
            }
            tokens_all.operator(operator);
            i += operator.len();
            continue;
        }

        if line < total {
            code[line] = true;
        }

        // Everything left is punctuation or a single operator byte; counting
        // a byte at a time is the documented Halstead approximation.
        if !skipping_test {
            tokens.operator(&bytes[i..i + 1]);
        }
        tokens_all.operator(&bytes[i..i + 1]);

        if skipping_test {
            if ch == b'{' {
                test_depth += 1;
                pending_control = false;
                after_else = false;
            } else if ch == b'}' {
                test_depth -= 1;
                if test_depth == 0 {
                    spans.push((test_start, line));
                    skipping_test = false;
                    pending_test = None;
                    pending_is_mod = false;
                    pending_mod_name = None;
                    pending_path = None;
                    pending_attrs.clear();
                }
            }
            i += 1;
            continue;
        }

        if let Some(start) = pending_test {
            if ch == b'{' {
                test_start = start;
                skipping_test = true;
                test_depth = 1;
            } else if ch == b';' {
                spans.push((start, line));
                if pending_is_mod {
                    let target = match (&pending_path, &pending_mod_name) {
                        (Some(path), _) => Some(path.clone()),
                        (None, Some(name)) => Some(format!("{}.rs", name)),
                        _ => None,
                    };
                    if let Some(target) = target {
                        sibling_targets.push(target);
                    }
                }
                pending_test = None;
                pending_is_mod = false;
                pending_mod_name = None;
                pending_path = None;
                pending_attrs.clear();
            }
            i += 1;
            continue;
        }

        if matches!(ch, b';' | b'{' | b'}') {
            pending_is_mod = false;
            pending_mod_name = None;
            pending_path = None;
            pending_attrs.clear();
        }

        // Punctuation feeds the per-function metrics. Only code that survived
        // the test skip above reaches here, so test bodies open no entry.
        match ch {
            b'(' => {
                if pending_fn.as_ref().is_some_and(|item| item.name.is_empty()) {
                    pending_fn = None;
                } else if let Some(pending) = pending_fn.as_mut() {
                    pending.paren += 1;
                }
            }
            b')' => {
                if let Some(pending) = pending_fn.as_mut() {
                    if pending.paren <= 1 {
                        pending.paren = 0;
                        pending.angle = 0;
                    } else {
                        pending.paren -= 1;
                    }
                }
            }
            b',' => {
                if let Some(pending) = pending_fn.as_mut() {
                    if pending.paren >= 1 && pending.angle == 0 {
                        pending.commas += 1;
                    }
                }
            }
            b'<' => {
                if let Some(pending) = pending_fn.as_mut() {
                    if pending.paren >= 1 {
                        pending.angle += 1;
                    }
                }
            }
            b'>' => {
                if let Some(pending) = pending_fn.as_mut() {
                    pending.angle = pending.angle.saturating_sub(1);
                }
            }
            b';' => {
                pending_fn = None;
                pending_control = false;
                after_else = false;
            }
            b'{' => {
                brace_depth += 1;
                let control = pending_control;
                pending_control = false;
                after_else = false;
                if let Some(pending) = pending_fn.take() {
                    open_fns.push(OpenFn {
                        name: pending.name,
                        line: pending.line,
                        body_depth: brace_depth,
                        params: if pending.seen_param {
                            pending.commas + 1
                        } else {
                            0
                        },
                        complexity: 0,
                        cognitive: 0,
                        control_base: control_depth,
                        max_depth: 0,
                        test: pending.test,
                    });
                }
                for open in open_fns.iter_mut() {
                    let depth = brace_depth - open.body_depth + 1;
                    open.max_depth = open.max_depth.max(depth);
                }
                if control {
                    control_depth += 1;
                }
                block_control.push(control);
            }
            b'}' => {
                while open_fns
                    .last()
                    .is_some_and(|open| open.body_depth == brace_depth)
                {
                    let open = open_fns.pop().expect("body_depth just matched");
                    fn_spans.push((open.line, line));
                    functions.push(FunctionInfo {
                        name: open.name,
                        line: open.line as u32 + 1,
                        code: 0,
                        complexity: open.complexity,
                        cognitive: open.cognitive,
                        params: open.params,
                        depth: open.max_depth,
                        test: open.test,
                    });
                }
                brace_depth = brace_depth.saturating_sub(1);
                pending_control = false;
                after_else = false;
                if block_control.pop().unwrap_or(false) {
                    control_depth = control_depth.saturating_sub(1);
                }
            }
            _ => {}
        }

        i += 1;
    }

    if skipping_test {
        spans.push((test_start, total.saturating_sub(1)));
    } else if let Some(start) = pending_test {
        spans.push((start, total.saturating_sub(1)));
    }

    let mut is_test = vec![false; total];
    if total > 0 {
        for &(start, end) in &spans {
            let from = start.min(total - 1);
            let to = end.min(total - 1);
            for flag in is_test.iter_mut().take(to + 1).skip(from) {
                *flag = true;
            }
        }
    }

    // A truncated file can leave function bodies open; close them at EOF.
    let last_line = total.saturating_sub(1);
    while let Some(open) = open_fns.pop() {
        fn_spans.push((open.line, last_line));
        functions.push(FunctionInfo {
            name: open.name,
            line: open.line as u32 + 1,
            code: 0,
            complexity: open.complexity,
            cognitive: open.cognitive,
            params: open.params,
            depth: open.max_depth,
            test: open.test,
        });
    }

    if total > 0 {
        // Business code lines per function: code lines minus test-only lines.
        for (index, &(start, end)) in fn_spans.iter().enumerate() {
            let from = start.min(total - 1);
            let to = end.min(total - 1);
            functions[index].code = (from..=to)
                .filter(|line_index| code[*line_index] && !is_test[*line_index])
                .count() as u32;
            if is_test[from..=to].iter().any(|flag| *flag) {
                functions[index].test = true;
            }
        }
    }

    metrics.halstead = tokens.finish();
    metrics_all.halstead = tokens_all.finish();

    Analysis {
        total,
        code,
        comment,
        doc,
        is_test,
        crate_test,
        sibling_targets,
        metrics,
        metrics_all,
        smells,
        smells_all,
        functions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keyword_table_stays_sorted() {
        for pair in KEYWORDS.windows(2) {
            assert!(
                pair[0] < pair[1],
                "{} should sort before {}",
                String::from_utf8_lossy(pair[0]),
                String::from_utf8_lossy(pair[1])
            );
        }
    }

    #[test]
    fn numeric_literals_come_back_whole() {
        for (text, end) in [
            ("1_000u32", 8usize),
            ("0xff", 4),
            ("1.5e-3", 6),
            ("3.", 1),
            ("1..2", 1),
            ("0b1010_i8", 9),
        ] {
            assert_eq!(number_end(text.as_bytes(), 0), end, "{text}");
        }
    }
}
