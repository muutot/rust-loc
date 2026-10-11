//! Rust source tokeniser.
//!
//! One pass over the bytes classifies every physical line as code, comment
//! (optionally doc) or blank, and collects the cheap Rust metrics: approximate
//! cyclomatic complexity, unsafe keyword use and test functions. It also
//! records the line spans of test-only cfg attributes so callers can exclude
//! test code without parsing the file again.

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Metrics {
    pub complexity: u32,
    pub unsafe_kw: u32,
    pub test_fns: u32,
}

/// One `fn` body with the cheap per-function metrics.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FunctionInfo {
    pub name: String,
    pub line: u32,
    pub code: u32,
    pub complexity: u32,
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
    max_depth: u32,
    test: bool,
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
                i += 1 + raw_hashes;
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
            i += len;
            state = RAW_STRING;
            continue;
        }

        if ch == b'"' {
            if line < total {
                code[line] = true;
            }
            escaped = false;
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
            if word == b"unwrap" {
                smells_all.unwrap += 1;
                if !skipping_test {
                    smells.unwrap += 1;
                }
            } else if word == b"expect" {
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
            }
            i += operator.len();
            continue;
        }

        if line < total {
            code[line] = true;
        }

        if skipping_test {
            if ch == b'{' {
                test_depth += 1;
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
            b';' => pending_fn = None,
            b'{' => {
                brace_depth += 1;
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
                        max_depth: 0,
                        test: pending.test,
                    });
                }
                for open in open_fns.iter_mut() {
                    let depth = brace_depth - open.body_depth + 1;
                    open.max_depth = open.max_depth.max(depth);
                }
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
                        params: open.params,
                        depth: open.max_depth,
                        test: open.test,
                    });
                }
                brace_depth = brace_depth.saturating_sub(1);
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
