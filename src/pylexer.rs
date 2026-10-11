//! Python source tokeniser.
//!
//! The twin of [`crate::lexer::analyze`]: the same [`Analysis`], read from a
//! different grammar. Python needs a reader of its own because a body is
//! opened by a trailing `:` and closed by indentation rather than by braces, a
//! docstring stands in for `///`, and a test is a name that starts with `test_`
//! rather than an attribute.
//!
//! The line classes follow the Rust reader: a line is `code` when any token on
//! it is not a comment, `comment` when a `#` comment or a docstring sits on it,
//! and `doc` when that comment is a docstring, so `doc` stays a subset of
//! `comment`. A triple quoted string counts as a docstring when it opens a
//! logical line and either the line above it ends in `:` or no code line has
//! been read yet - which is where every module, class and function docstring
//! sits.
//!
//! Test code is a `def` whose name starts with `test_`, a `class` whose name
//! starts with `Test`, and anything carrying a `@pytest.`, `@unittest.`,
//! `@mock.`, `@patch` or `@parameterized` decorator, body included.
//!
//! Three Rust metrics have no Python counterpart and stay zero: `unsafe` is a
//! keyword of the other language, and `unwrap`/`expect`/`panic` are Rust
//! methods and macros. What Python does have lands in the slots that fit it:
//! `unimplemented` for `NotImplementedError`, `todo` for a `TODO`/`FIXME`
//! marker in a comment, and `allow` for a lint escape (`# noqa`,
//! `# type: ignore`, `# pylint: disable`).

use crate::lexer::{physical_lines, Analysis, FunctionInfo, Metrics, Smells, TokenCounter};

const CODE: u8 = 0;
const STRING: u8 = 1;
const TRIPLE: u8 = 2;

/// Branch points: a keyword that splits control flow, and the two boolean
/// operators that do the same inside an expression.
const BRANCH_KEYWORDS: [&[u8]; 6] = [b"elif", b"else", b"except", b"if", b"for", b"while"];
/// `case` is a soft keyword, so it only counts when it opens a logical line -
/// which is exactly where a `match` arm lives.
const SOFT_BRANCH: &[u8] = b"case";
/// Headers that open a body without adding a branch point of their own.
const BLOCK_KEYWORDS: [&[u8]; 3] = [b"try", b"with", b"match"];
const BRANCH_OPERATORS: [&[u8]; 2] = [b"and", b"or"];
/// Comparison operators, the Python twin of the `==` the Rust reader counts.
const MATCH_OPERATORS: [&[u8]; 2] = [b"==", b"!="];

/// Python keywords, sorted, counted as Halstead operators rather than
/// operands. A unit test keeps the sort order honest.
const KEYWORDS: [&[u8]; 35] = [
    b"False",
    b"None",
    b"True",
    b"and",
    b"as",
    b"assert",
    b"async",
    b"await",
    b"break",
    b"class",
    b"continue",
    b"def",
    b"del",
    b"elif",
    b"else",
    b"except",
    b"finally",
    b"for",
    b"from",
    b"global",
    b"if",
    b"import",
    b"in",
    b"is",
    b"lambda",
    b"nonlocal",
    b"not",
    b"or",
    b"pass",
    b"raise",
    b"return",
    b"try",
    b"while",
    b"with",
    b"yield",
];

/// The prefixes a string literal may carry, lower cased before the lookup, so
/// `r"""..."""` still counts as the docstring it is.
const STRING_PREFIXES: [&str; 8] = ["b", "br", "f", "fr", "r", "rb", "rf", "u"];

/// The first name of a decorator that means "this is test code".
const TEST_DECORATORS: [&str; 6] = [
    "pytest",
    "unittest",
    "mock",
    "patch",
    "parameterized",
    "hypothesis",
];

/// Comment markers that count as a lint escape.
const ALLOW_MARKERS: [&str; 5] = [
    "noqa",
    "type: ignore",
    "pylint: disable",
    "pyright: ignore",
    "mypy: ignore",
];

/// Comment markers that count as a leftover to finish.
const TODO_MARKERS: [&str; 3] = ["TODO", "FIXME", "XXX"];

/// What the logical line above opened, consumed when the line below turns out
/// to be indented deeper.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Header {
    /// A plain statement, or the top of the file.
    Other,
    /// `if`, `for`, `try`, ... - a branch that opens a body.
    Control,
    /// A `def` whose entry is already in `functions`.
    Def { index: usize },
    /// A `class`; nothing is recorded but the body inherits its test-ness.
    Class,
}

/// One level of indentation, the Python stand-in for the Rust brace stack.
struct Frame {
    indent: usize,
    /// The nesting a branch directly inside this body pays for: the number of
    /// enclosing control headers.
    control_depth: u32,
    /// The body being read inside this frame, indexed into `functions`.
    function: Option<usize>,
    /// Where the test item this frame opened started, decorators included.
    test_span: Option<usize>,
    test: bool,
    /// Deepest indentation reached inside this frame.
    max_depth: u32,
    /// Where the header that opened this frame sits.
    start: usize,
}

/// A `def` header being read, from the keyword to the closing parenthesis.
struct DefHeader {
    name: String,
    line: usize,
    commas: u32,
    depth: u32,
    filled: bool,
    /// The last token of the parameter list was a comma, the way idiomatic
    /// multi-line signatures end.
    trailing: bool,
    first: String,
    first_done: bool,
}

impl DefHeader {
    fn new(line: usize) -> DefHeader {
        DefHeader {
            name: String::new(),
            line,
            commas: 0,
            depth: 0,
            filled: false,
            trailing: false,
            first: String::new(),
            first_done: false,
        }
    }

    /// Parameters the way the Rust reader counts them: the outer level only,
    /// with the receiver every method carries dropped.
    fn params(&self) -> u32 {
        if !self.filled {
            return 0;
        }
        let count = self.commas + 1 - u32::from(self.trailing);
        if self.first == "self" || self.first == "cls" {
            count - 1
        } else {
            count
        }
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    total: usize,
    code: Vec<bool>,
    comment: Vec<bool>,
    doc: Vec<bool>,
    metrics: Metrics,
    metrics_all: Metrics,
    smells: Smells,
    smells_all: Smells,
    functions: Vec<FunctionInfo>,
    /// `(function index, first line, last line)` for every body that closed.
    fn_spans: Vec<(usize, usize, usize)>,
    /// Line ranges that hold test code.
    spans: Vec<(usize, usize)>,
    tokens: TokenCounter<'a>,
    tokens_all: TokenCounter<'a>,
    frames: Vec<Frame>,
    pending: Header,
    /// Whether the header `pending` describes is a test item of its own.
    pending_own_test: bool,
    /// The line the item `pending` describes started on.
    pending_span: usize,
    decorators: Vec<usize>,
    decorator_test: bool,
    def: Option<DefHeader>,
    line: usize,
    /// A code line has been read, so a string is no longer a module docstring.
    seen_code: bool,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8], total: usize) -> Reader<'a> {
        Reader {
            bytes,
            total,
            code: vec![false; total],
            comment: vec![false; total],
            doc: vec![false; total],
            metrics: Metrics::default(),
            metrics_all: Metrics::default(),
            smells: Smells::default(),
            smells_all: Smells::default(),
            functions: Vec::new(),
            fn_spans: Vec::new(),
            spans: Vec::new(),
            tokens: TokenCounter::default(),
            tokens_all: TokenCounter::default(),
            frames: vec![Frame {
                indent: 0,
                control_depth: 0,
                function: None,
                test_span: None,
                test: false,
                max_depth: 0,
                start: 0,
            }],
            pending: Header::Other,
            pending_own_test: false,
            pending_span: 0,
            decorators: Vec::new(),
            decorator_test: false,
            def: None,
            line: 0,
            seen_code: false,
        }
    }

    /// Inside a test body: counted, but never as business code.
    fn test_now(&self) -> bool {
        self.frames.last().is_some_and(|frame| frame.test)
    }

    fn mark_code(&mut self, line: usize) {
        if line < self.total {
            self.code[line] = true;
        }
        self.seen_code = true;
    }

    fn mark_comment(&mut self, line: usize, is_doc: bool) {
        if line < self.total {
            self.comment[line] = true;
            if is_doc {
                self.doc[line] = true;
            }
        }
    }

    fn top(&self) -> &Frame {
        self.frames.last().expect("the root frame never pops")
    }

    /// Open the body the line above announced, at `indent`.
    fn open_body(&mut self, indent: usize) {
        let header = self.pending;
        let own_test = self.pending_own_test;
        let start = self.pending_span;
        let control = header == Header::Control;
        let test = own_test || self.top().test;
        self.frames.push(Frame {
            indent,
            control_depth: self.top().control_depth + u32::from(control),
            function: match header {
                Header::Def { index } => Some(index),
                _ => None,
            },
            test_span: if own_test { Some(start) } else { None },
            test,
            max_depth: 0,
            start,
        });
        self.pending = Header::Other;
        self.pending_own_test = false;
        let depth = self.frames.len() - 1;
        for (index, frame) in self.frames.iter_mut().enumerate() {
            frame.max_depth = frame.max_depth.max((depth - index) as u32);
        }
    }

    /// Close the frame the current line has dedented out of.
    fn pop_frame(&mut self, end: usize) {
        let Some(frame) = self.frames.pop() else {
            return;
        };
        // The blank lines a body is followed by belong to nobody, so a test
        // item does not swallow the gap under it.
        let end = if end > frame.start {
            let mut last = end;
            while last > frame.start && !self.code[last] && !self.comment[last] {
                last -= 1;
            }
            last
        } else {
            end
        };
        if let Some(start) = frame.test_span {
            self.spans.push((start, end));
        }
        if let Some(index) = frame.function {
            self.fn_spans.push((index, frame.start, end));
            if let Some(item) = self.functions.get_mut(index) {
                item.depth = frame.max_depth;
            }
        }
    }

    /// The header above opened nothing: `def f(): pass`, or a class whose body
    /// sits on the header line itself.
    fn close_header(&mut self, end: usize) {
        if let Header::Def { index } = self.pending {
            self.fn_spans.push((index, self.pending_span, end));
            if self.pending_own_test {
                self.spans.push((self.pending_span, end));
            }
        }
        self.pending = Header::Other;
        self.pending_own_test = false;
    }

    /// Register the `def` whose closing parenthesis was just read.
    fn open_def(&mut self) {
        let Some(header) = self.def.take() else {
            return;
        };
        let test = header.name.starts_with("test_") || self.decorator_test;
        if test {
            self.metrics_all.test_fns += 1;
            if !self.test_now() {
                self.metrics.test_fns += 1;
            }
        }
        let params = header.params();
        self.functions.push(FunctionInfo {
            name: header.name,
            line: header.line as u32 + 1,
            code: 0,
            complexity: 0,
            cognitive: 0,
            params,
            depth: 0,
            test,
        });
        self.pending = Header::Def {
            index: self.functions.len() - 1,
        };
        self.pending_own_test = test;
        self.pending_span = self.item_start(header.line);
        self.decorators.clear();
        self.decorator_test = false;
    }

    fn open_class(&mut self, name: &str) {
        let test = name.starts_with("Test") || self.decorator_test;
        self.pending = Header::Class;
        self.pending_own_test = test;
        self.pending_span = self.item_start(self.line);
        self.decorators.clear();
        self.decorator_test = false;
    }

    /// The line an item starts on: the decorator above it when there is one.
    fn item_start(&self, line: usize) -> usize {
        self.decorators.first().copied().unwrap_or(line)
    }

    /// The line of the innermost open body, if it belongs to a function.
    fn open_function(&self) -> Option<(usize, u32)> {
        self.frames
            .iter()
            .rev()
            .find_map(|frame| frame.function.map(|index| (index, frame.control_depth)))
    }

    /// A branch point: one for the branch, plus one for every level of control
    /// flow it is nested in.
    fn branch(&mut self) {
        let depth = self.top().control_depth;
        self.metrics_all.complexity += 1;
        self.metrics_all.cognitive += 1 + depth;
        if self.test_now() {
            return;
        }
        self.metrics.complexity += 1;
        self.metrics.cognitive += 1 + depth;
        if let Some((index, base)) = self.open_function() {
            if let Some(item) = self.functions.get_mut(index) {
                item.complexity += 1;
                item.cognitive += 1 + depth.saturating_sub(base);
            }
        }
    }

    /// A boolean operator, counted the way the Rust reader counts `&&`: a
    /// branch point, and a Halstead operator in the token pass that already
    /// ran for it.
    fn matching_operator(&mut self) {
        self.metrics_all.complexity += 1;
        if self.test_now() {
            return;
        }
        self.metrics.complexity += 1;
        if let Some((index, _)) = self.open_function() {
            if let Some(item) = self.functions.get_mut(index) {
                item.complexity += 1;
            }
        }
    }

    fn operand(&mut self, token: &'a [u8]) {
        if self.test_now() {
            self.tokens_all.operand(token);
        } else {
            self.tokens.operand(token);
            self.tokens_all.operand(token);
        }
    }

    /// Count a smell in both scopes, the way the Rust reader does.
    fn smell(&mut self, slot: fn(&mut Smells) -> &mut u32) {
        *slot(&mut self.smells_all) += 1;
        if !self.test_now() {
            *slot(&mut self.smells) += 1;
        }
    }

    /// Read a `#` comment: its markers, and the line it sits on.
    fn read_comment(&mut self, start: usize, line: usize) -> usize {
        let end = self.bytes[start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(self.bytes.len(), |offset| start + offset);
        let text = String::from_utf8_lossy(&self.bytes[start..end]);
        if ALLOW_MARKERS.iter().any(|marker| text.contains(marker)) {
            self.smell(|smells| &mut smells.allow);
        }
        if TODO_MARKERS.iter().any(|marker| text.contains(marker)) {
            self.smell(|smells| &mut smells.todo);
        }
        self.mark_comment(line, false);
        end
    }

    /// The first token of a logical line: open the body the line above
    /// announced, or close the one the dedent just ended.
    fn start_logical_line(&mut self, indent: usize) {
        if indent > self.top().indent {
            self.open_body(indent);
            return;
        }
        if self.pending != Header::Other {
            self.close_header(self.line.saturating_sub(1));
        }
        while indent < self.top().indent {
            self.pop_frame(self.line.saturating_sub(1));
        }
    }
}

/// Classify every line of a Python file and collect its cheap metrics.
pub fn analyze(text: &str) -> Analysis {
    let bytes = text.as_bytes();
    let total = physical_lines(text);
    let mut reader = Reader::new(bytes, total);
    let n = bytes.len();

    let mut state = CODE;
    let mut quote = 0u8;
    let mut escaped = false;
    let mut literal_doc = false;
    let mut literal_start = 0usize;

    // Logical line bookkeeping.
    let mut at_start = true;
    let mut indent = 0usize;
    let mut code_before = false;
    let mut first_token = false;
    let mut prefix = false;
    let mut on_decorator = false;
    let mut decorated_before = false;
    let mut decorator_named = false;
    let mut class_pending = false;
    let mut paren = 0u32;
    let mut continued = false;
    let mut prev_colon = false;
    let mut line_last = 0u8;

    let mut i = 0usize;
    while i < n {
        let ch = bytes[i];

        if state == STRING || state == TRIPLE {
            if state == TRIPLE {
                if literal_doc {
                    reader.mark_comment(reader.line, true);
                } else {
                    reader.mark_code(reader.line);
                }
            } else {
                reader.mark_code(reader.line);
            }
            if ch == b'\n' {
                reader.line += 1;
                escaped = false;
                i += 1;
                continue;
            }
            if escaped {
                escaped = false;
                i += 1;
                continue;
            }
            if ch == b'\\' {
                escaped = true;
                i += 1;
                continue;
            }
            if ch == quote {
                if state == TRIPLE {
                    if !bytes[i..].starts_with(&[quote, quote, quote]) {
                        i += 1;
                        continue;
                    }
                    let literal = &bytes[literal_start..i + 3];
                    if reader.test_now() {
                        reader.tokens_all.operand(literal);
                    } else {
                        reader.tokens.operand(literal);
                        reader.tokens_all.operand(literal);
                    }
                    i += 3;
                } else {
                    let literal = &bytes[literal_start..=i];
                    if reader.test_now() {
                        reader.tokens_all.operand(literal);
                    } else {
                        reader.tokens.operand(literal);
                        reader.tokens_all.operand(literal);
                    }
                    i += 1;
                }
                state = CODE;
                continue;
            }
            i += 1;
            continue;
        }

        if at_start {
            match ch {
                b' ' => {
                    indent += 1;
                    i += 1;
                    continue;
                }
                b'\t' => {
                    indent = (indent / 8 + 1) * 8;
                    i += 1;
                    continue;
                }
                b'\x0c' => {
                    indent = 0;
                    i += 1;
                    continue;
                }
                b'\r' => {
                    i += 1;
                    continue;
                }
                b'\n' => {
                    reader.line += 1;
                    indent = 0;
                    i += 1;
                    continue;
                }
                _ => {}
            }
            at_start = false;
            first_token = true;
            code_before = reader.seen_code;
            on_decorator = ch == b'@';
            decorator_named = false;
            if on_decorator {
                reader.decorators.push(reader.line);
            } else if !decorated_before {
                reader.decorators.clear();
                reader.decorator_test = false;
            }
            decorated_before = on_decorator;
            reader.start_logical_line(indent);
        }

        if ch == b'\n' {
            reader.line += 1;
            if paren == 0 && !continued {
                prev_colon = line_last == b':';
                at_start = true;
                indent = 0;
                on_decorator = false;
                prefix = false;
                line_last = 0;
                class_pending = false;
                reader.def = None;
            }
            continued = false;
            i += 1;
            continue;
        }

        if ch == b'\\' {
            let rest = &bytes[i + 1..];
            let joined = match rest.iter().position(|byte| *byte != b' ' && *byte != b'\t') {
                Some(offset) => rest.get(offset) == Some(&b'\n'),
                None => true,
            };
            if joined {
                continued = true;
            }
            i += 1;
            continue;
        }

        if ch == b'#' {
            i = reader.read_comment(i, reader.line);
            continue;
        }

        if ch.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        line_last = ch;

        if ch == b'"' || ch == b'\'' {
            if bytes[i..].starts_with(&[ch, ch, ch]) {
                let docstring = (first_token || prefix) && (prev_colon || !code_before);
                literal_doc = docstring;
                if docstring {
                    // Nothing else is on the line but a possible prefix
                    // identifier, which is not code once it opens a docstring.
                    if reader.line < reader.total {
                        reader.code[reader.line] = false;
                    }
                    reader.mark_comment(reader.line, true);
                } else {
                    reader.mark_code(reader.line);
                }
                literal_start = i;
                quote = ch;
                state = TRIPLE;
                escaped = false;
                i += 3;
            } else {
                reader.mark_code(reader.line);
                literal_doc = false;
                literal_start = i;
                quote = ch;
                state = STRING;
                escaped = false;
                i += 1;
            }
            first_token = false;
            prefix = false;
            continue;
        }

        if ch.is_ascii_digit() {
            let mut end = i;
            while end < n
                && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_' || bytes[end] == b'.')
            {
                end += 1;
            }
            reader.mark_code(reader.line);
            reader.operand(&bytes[i..end]);
            first_token = false;
            prefix = false;
            i = end;
            continue;
        }

        if ch.is_ascii_alphabetic() || ch == b'_' || ch >= 0x80 {
            let mut end = i;
            while end < n {
                let byte = bytes[end];
                if byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80 {
                    end += 1;
                } else {
                    break;
                }
            }
            let word = &bytes[i..end];
            reader.mark_code(reader.line);
            let mut word_is_prefix = false;
            if KEYWORDS.binary_search(&word).is_ok() {
                if reader.test_now() {
                    reader.tokens_all.operator(word);
                } else {
                    reader.tokens.operator(word);
                    reader.tokens_all.operator(word);
                }
            } else {
                reader.operand(word);
                if first_token {
                    let name = String::from_utf8_lossy(word).to_lowercase();
                    word_is_prefix = STRING_PREFIXES.contains(&name.as_str());
                    if on_decorator && !decorator_named {
                        decorator_named = true;
                        if TEST_DECORATORS.contains(&name.as_ref()) {
                            reader.decorator_test = true;
                        }
                    }
                }
                if class_pending {
                    class_pending = false;
                    let name = String::from_utf8_lossy(word).into_owned();
                    reader.open_class(&name);
                } else if word == b"NotImplementedError" {
                    reader.smell(|smells| &mut smells.unimplemented);
                } else if let Some(header) = reader.def.as_mut() {
                    if header.name.is_empty() && header.depth == 0 {
                        header.name = String::from_utf8_lossy(word).into_owned();
                    } else if header.depth >= 1 {
                        if !header.first_done {
                            header.first = String::from_utf8_lossy(word).into_owned();
                            header.first_done = true;
                        }
                        header.filled = true;
                        header.trailing = false;
                    }
                }
            }
            if word == b"def" {
                reader.def = Some(DefHeader::new(reader.line));
            } else if word == b"class" {
                class_pending = true;
            } else if BRANCH_OPERATORS.contains(&word) {
                reader.matching_operator();
            }
            let soft = first_token && word == SOFT_BRANCH;
            if BRANCH_KEYWORDS.contains(&word) || soft {
                if matches!(reader.pending, Header::Other) {
                    reader.pending = Header::Control;
                }
                reader.branch();
            } else if BLOCK_KEYWORDS.contains(&word) && matches!(reader.pending, Header::Other) {
                reader.pending = Header::Control;
            }
            first_token = false;
            prefix = word_is_prefix;
            i = end;
            continue;
        }

        if matches!(ch, b'(' | b'[' | b'{') {
            reader.mark_code(reader.line);
            paren += 1;
            if let Some(header) = reader.def.as_mut() {
                header.depth += 1;
                if header.depth == 1 {
                    header.filled = false;
                    header.commas = 0;
                    header.trailing = false;
                    header.first.clear();
                    header.first_done = false;
                }
            }
            first_token = false;
            prefix = false;
            i += 1;
            continue;
        }

        if matches!(ch, b')' | b']' | b'}') {
            reader.mark_code(reader.line);
            paren = paren.saturating_sub(1);
            let mut closed = false;
            if let Some(header) = reader.def.as_mut() {
                header.depth = header.depth.saturating_sub(1);
                closed = header.depth == 0;
            }
            if closed {
                reader.open_def();
            }
            first_token = false;
            prefix = false;
            i += 1;
            continue;
        }

        if ch == b',' {
            reader.mark_code(reader.line);
            if let Some(header) = reader.def.as_mut() {
                if header.depth == 1 {
                    header.commas += 1;
                    header.trailing = true;
                }
            }
            first_token = false;
            prefix = false;
            i += 1;
            continue;
        }

        reader.mark_code(reader.line);
        if let Some(header) = reader.def.as_mut() {
            if header.depth == 1 {
                header.filled = true;
                header.trailing = false;
            }
        }
        if let Some(operator) = MATCH_OPERATORS
            .iter()
            .find(|operator| bytes[i..].starts_with(operator))
        {
            if reader.test_now() {
                reader.tokens_all.operator(operator);
            } else {
                reader.tokens.operator(operator);
                reader.tokens_all.operator(operator);
            }
            reader.matching_operator();
            first_token = false;
            prefix = false;
            i += 2;
            continue;
        }
        if reader.test_now() {
            reader.tokens_all.operator(&bytes[i..i + 1]);
        } else {
            reader.tokens.operator(&bytes[i..i + 1]);
            reader.tokens_all.operator(&bytes[i..i + 1]);
        }
        first_token = false;
        prefix = false;
        i += 1;
    }

    // A file can end inside a body; close what is still open at the last line.
    let last = total.saturating_sub(1);
    if reader.pending != Header::Other {
        reader.close_header(last);
    }
    while reader.frames.len() > 1 {
        reader.pop_frame(last);
    }

    let mut is_test = vec![false; total];
    for &(start, end) in &reader.spans {
        let from = start.min(last);
        let to = end.min(last);
        if total > 0 {
            for flag in is_test.iter_mut().take(to + 1).skip(from) {
                *flag = true;
            }
        }
    }

    let mut functions = reader.functions;
    if total > 0 {
        for &(index, start, end) in &reader.fn_spans {
            let from = start.min(last);
            let to = end.min(last);
            if let Some(item) = functions.get_mut(index) {
                item.code = (from..=to)
                    .filter(|line_index| reader.code[*line_index] && !is_test[*line_index])
                    .count() as u32;
                if is_test[from..=to].iter().any(|flag| *flag) {
                    item.test = true;
                }
            }
        }
    }

    let mut metrics = reader.metrics;
    metrics.halstead = reader.tokens.finish();
    let mut metrics_all = reader.metrics_all;
    metrics_all.halstead = reader.tokens_all.finish();

    Analysis {
        total,
        code: reader.code,
        comment: reader.comment,
        doc: reader.doc,
        is_test,
        crate_test: false,
        sibling_targets: Vec::new(),
        metrics,
        metrics_all,
        smells: reader.smells,
        smells_all: reader.smells_all,
        functions,
    }
}
#[cfg(test)]
mod tests {
    use super::KEYWORDS;

    #[test]
    fn keywords_are_sorted_for_the_binary_search() {
        let mut sorted = KEYWORDS.to_vec();
        sorted.sort_unstable();
        assert_eq!(KEYWORDS.to_vec(), sorted);
    }
}
