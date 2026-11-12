//! A small language table and a comment-aware line counter.
//!
//! `--languages` answers "what is this repository made of", which a Rust-only
//! counter cannot. The table is hand written: one entry per language, no
//! dependencies, and only the extensions that show up in mixed repositories.
//! Files whose extension is not in the table are never read.
//!
//! The table is also the grammar the shared reader in [`crate::generic`] reads
//! with, so every language here can be selected as a source language with
//! `--sources`: the comment and literal rules classify the lines, and the
//! `branch` tokens estimate complexity the way the language's own tools count
//! branches.

use crate::generic;

/// One language: its extensions and how it marks comments and literals.
pub struct Language {
    pub name: &'static str,
    pub extensions: &'static [&'static str],
    /// Prefixes that comment out the rest of a line.
    pub line: &'static [&'static str],
    /// Opening and closing token of a block comment, when there is one.
    pub block: Option<(&'static str, &'static str)>,
    /// Quotes that open and close a single-line literal.
    pub quotes: &'static [char],
    /// Tokens that split control flow, so a scan can estimate complexity
    /// without a parser. Alphabetic entries are matched on word boundaries,
    /// the rest as plain tokens; an empty list marks a language with no
    /// branches to count.
    pub branch: &'static [&'static str],
    /// Whether a plain comment group sitting directly above what it documents
    /// is a doc comment. Go is the language that spells documentation this
    /// way; everywhere else a doc comment carries a marker of its own.
    pub doc_attached: bool,
    /// Whether the language is a documentation format rather than a code one.
    /// Every non-blank line of a documentation file counts as a doc line, the
    /// way a reference write-up is read rather than as shipped code.
    pub documentation: bool,
    /// Line shapes that declare a test function, matched against the
    /// whitespace-trimmed line start. `{}` is the test function's name; a
    /// `*` marks the parameter list where C-like languages spell it. The
    /// shared reader in [`crate::generic`] counts every match so the report's
    /// test-function total covers languages without a lexer of their own -
    /// Go's `func TestXxx(`, Java's `void testXxx()`, Python-style `def
    /// test_xxx` spellings their lexers already count.
    pub test_fn: &'static [TestFn],
}

/// One test-function declaration shape. `prefix` must match the trimmed line
/// start; `name_upper` requires the captured name to start upper- (true) or
/// lowercase (false), which keeps `func helper(` out while `func TestXxx(`
/// counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TestFn {
    pub prefix: &'static str,
    pub name_upper: bool,
}

impl TestFn {
    /// Whether the trimmed line declares a test function in this shape: the
    /// prefix matches, the name is an identifier of the required case, and a
    /// `(` follows - so a call site or a comment never counts.
    pub fn matches(&self, trimmed: &str) -> bool {
        let Some(rest) = trimmed.strip_prefix(self.prefix) else {
            return false;
        };
        let rest = rest.trim_start();
        let bytes = rest.as_bytes();
        if bytes.is_empty() {
            return false;
        }
        let ok_case = if self.name_upper {
            bytes[0].is_ascii_uppercase() || bytes[0] == b'_'
        } else {
            bytes[0].is_ascii_lowercase() || bytes[0] == b'_'
        };
        if !ok_case {
            return false;
        }
        let mut end = 1;
        while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
            end += 1;
        }
        let tail = &bytes[end..];
        let mut i = 0;
        while i < tail.len() && (tail[i] == b' ' || tail[i] == b'\t') {
            i += 1;
        }
        tail[i..].starts_with(b"(")
    }
}

const SLASH: &[&str] = &["//"];
const HASH: &[&str] = &["#"];
const DASH: &[&str] = &["--"];
const PERCENT: &[&str] = &["%"];
const NONE: &[&str] = &[];
const C_BLOCK: Option<(&str, &str)> = Some(("/*", "*/"));
const HTML_BLOCK: Option<(&str, &str)> = Some(("<!--", "-->"));
const ML_BLOCK: Option<(&str, &str)> = Some(("(*", "*)"));
/// The three double quotes around a Python docstring, spelled with unicode
/// escapes so this file needs no quote gymnastics.
const TRIPLE: &str = "\u{22}\u{22}\u{22}";
const PY_BLOCK: Option<(&str, &str)> = Some((TRIPLE, TRIPLE));
/// A single quote, spelled the same way.
const SINGLE_QUOTE: char = '\u{27}';
const QUOTES_C: &[char] = &['"', SINGLE_QUOTE];
const QUOTES_NONE: &[char] = &[];
const QUOTES_GO: &[char] = &['"', SINGLE_QUOTE, '`'];
/// Branch tokens of the C family: every language that spells `if` and `&&`
/// the way C does.
const BRANCH_C: &[&str] = &["if", "else", "for", "while", "case", "&&", "||", "==", "!="];
/// The JavaScript spelling, which TypeScript and Svelte share.
const BRANCH_JS: &[&str] = &["if", "else", "for", "while", "case", "&&", "||", "==", "!="];
/// Languages whose branches are words rather than operators.
const BRANCH_LUA: &[&str] = &[
    "if", "elseif", "else", "for", "while", "and", "or", "==", "~=",
];
const BRANCH_SHELL: &[&str] = &[
    "if", "elif", "else", "for", "while", "until", "case", "&&", "||",
];
const BRANCH_RUBY: &[&str] = &[
    "if", "elsif", "else", "unless", "for", "while", "until", "case", "when", "&&", "||", "==",
    "!=",
];
/// Languages with no branch token worth counting.
const BRANCH_NONE: &[&str] = &[];
/// No test-function shape: the languages whose reader never sees one, or
/// whose lexers already count them (Rust, Python).
const TEST_FN_NONE: &[TestFn] = &[];
/// Go: `func TestXxx(t *testing.T)` - the name must start uppercase, the way
/// `go test` requires. Only the top-level declaration counts; subtests run
/// inside it and are not separate declarations.
const TEST_FN_GO: &[TestFn] = &[TestFn {
    prefix: "func Test",
    name_upper: true,
}];
pub const LANGUAGES: [Language; 32] = [
    Language {
        name: "C",
        extensions: &["c", "h"],
        line: SLASH,
        block: C_BLOCK,
        quotes: QUOTES_C,
        branch: BRANCH_C,
        doc_attached: false,
        documentation: false,
        test_fn: TEST_FN_NONE,
    },
    Language {
        name: "C#",
        extensions: &["cs"],
        line: SLASH,
        block: C_BLOCK,
        quotes: QUOTES_C,
        branch: BRANCH_C,
        doc_attached: false,
        documentation: false,
        test_fn: TEST_FN_NONE,
    },
    Language {
        name: "C++",
        extensions: &["cc", "cpp", "cxx", "hpp", "hh", "hxx"],
        line: SLASH,
        block: C_BLOCK,
        quotes: QUOTES_C,
        branch: BRANCH_C,
        doc_attached: false,
        documentation: false,
        test_fn: TEST_FN_NONE,
    },
    Language {
        name: "CMake",
        extensions: &["cmake"],
        line: HASH,
        block: None,
        quotes: QUOTES_C,
        branch: &["if", "elseif", "else", "foreach", "while", "and", "or"],
        doc_attached: false,
        documentation: false,
        test_fn: TEST_FN_NONE,
    },
    Language {
        name: "CSS",
        extensions: &["css", "scss", "less", "sass"],
        line: SLASH,
        block: C_BLOCK,
        quotes: QUOTES_C,
        branch: BRANCH_NONE,
        doc_attached: false,
        documentation: false,
        test_fn: TEST_FN_NONE,
    },
    Language {
        name: "Dart",
        extensions: &["dart"],
        line: SLASH,
        block: C_BLOCK,
        quotes: QUOTES_C,
        branch: BRANCH_C,
        doc_attached: false,
        documentation: false,
        test_fn: TEST_FN_NONE,
    },
    Language {
        name: "Go",
        extensions: &["go"],
        line: SLASH,
        block: C_BLOCK,
        quotes: QUOTES_GO,
        branch: &[
            "if", "else", "for", "switch", "case", "select", "&&", "||", "==", "!=",
        ],
        doc_attached: true,
        documentation: false,
        test_fn: TEST_FN_GO,
    },
    Language {
        name: "Haskell",
        extensions: &["hs", "lhs"],
        line: DASH,
        block: Some(("{-", "-}")),
        quotes: QUOTES_C,
        branch: &["if", "case", "&&", "||", "==", "/="],
        doc_attached: false,
        documentation: false,
        test_fn: TEST_FN_NONE,
    },
    Language {
        name: "HTML",
        extensions: &["html", "htm", "xhtml", "xml", "svg"],
        line: NONE,
        block: HTML_BLOCK,
        quotes: QUOTES_C,
        branch: BRANCH_NONE,
        doc_attached: false,
        documentation: false,
        test_fn: TEST_FN_NONE,
    },
    Language {
        name: "Java",
        extensions: &["java"],
        line: SLASH,
        block: C_BLOCK,
        quotes: QUOTES_C,
        branch: BRANCH_C,
        doc_attached: false,
        documentation: false,
        test_fn: TEST_FN_NONE,
    },
    Language {
        name: "JavaScript",
        extensions: &["js", "mjs", "cjs", "jsx"],
        line: SLASH,
        block: C_BLOCK,
        quotes: QUOTES_GO,
        branch: BRANCH_JS,
        doc_attached: false,
        documentation: false,
        test_fn: TEST_FN_NONE,
    },
    Language {
        name: "JSON",
        extensions: &["json", "jsonc", "json5"],
        line: SLASH,
        block: C_BLOCK,
        quotes: QUOTES_C,
        branch: BRANCH_NONE,
        doc_attached: false,
        documentation: false,
        test_fn: TEST_FN_NONE,
    },
    Language {
        name: "Kotlin",
        extensions: &["kt", "kts"],
        line: SLASH,
        block: C_BLOCK,
        quotes: QUOTES_C,
        branch: &[
            "if", "else", "for", "while", "when", "case", "&&", "||", "==", "!=",
        ],
        doc_attached: false,
        documentation: false,
        test_fn: TEST_FN_NONE,
    },
    Language {
        name: "Lua",
        extensions: &["lua"],
        line: DASH,
        block: Some(("--[[", "]]")),
        quotes: QUOTES_C,
        branch: BRANCH_LUA,
        doc_attached: false,
        documentation: false,
        test_fn: TEST_FN_NONE,
    },
    Language {
        name: "Markdown",
        extensions: &["md", "markdown", "mdx"],
        line: NONE,
        block: HTML_BLOCK,
        quotes: QUOTES_NONE,
        branch: BRANCH_NONE,
        doc_attached: false,
        documentation: true,
        test_fn: TEST_FN_NONE,
    },
    Language {
        name: "OCaml",
        extensions: &["ml", "mli", "fs", "fsx"],
        line: SLASH,
        block: ML_BLOCK,
        quotes: QUOTES_C,
        branch: &["if", "match", "when", "&&", "||", "==", "<>"],
        doc_attached: false,
        documentation: false,
        test_fn: TEST_FN_NONE,
    },
    Language {
        name: "PHP",
        extensions: &["php"],
        line: SLASH,
        block: C_BLOCK,
        quotes: QUOTES_C,
        branch: &[
            "if", "elseif", "else", "foreach", "for", "while", "switch", "case", "&&", "||", "==",
            "!=",
        ],
        doc_attached: false,
        documentation: false,
        test_fn: TEST_FN_NONE,
    },
    Language {
        name: "PowerShell",
        extensions: &["ps1", "psm1", "psd1"],
        line: HASH,
        block: Some(("<#", "#>")),
        quotes: QUOTES_C,
        branch: &[
            "if", "elseif", "else", "for", "foreach", "while", "switch", "-and", "-or", "-eq",
            "-ne",
        ],
        doc_attached: false,
        documentation: false,
        test_fn: TEST_FN_NONE,
    },
    Language {
        name: "Python",
        extensions: &["py", "pyi", "pyw"],
        line: HASH,
        block: PY_BLOCK,
        quotes: QUOTES_C,
        branch: &[
            "if", "elif", "else", "except", "for", "while", "and", "or", "==", "!=", "case",
        ],
        doc_attached: false,
        documentation: false,
        test_fn: TEST_FN_NONE,
    },
    Language {
        name: "Ruby",
        extensions: &["rb", "rake", "gemspec"],
        line: HASH,
        block: Some(("=begin", "=end")),
        quotes: QUOTES_C,
        branch: BRANCH_RUBY,
        doc_attached: false,
        documentation: false,
        test_fn: TEST_FN_NONE,
    },
    Language {
        name: "Rust",
        extensions: &["rs"],
        line: SLASH,
        block: C_BLOCK,
        quotes: QUOTES_C,
        branch: &[
            "if", "else", "for", "while", "loop", "match", "&&", "||", "==", "!=",
        ],
        doc_attached: false,
        documentation: false,
        test_fn: TEST_FN_NONE,
    },
    Language {
        name: "SQL",
        extensions: &["sql"],
        line: DASH,
        block: C_BLOCK,
        quotes: QUOTES_C,
        branch: BRANCH_NONE,
        doc_attached: false,
        documentation: false,
        test_fn: TEST_FN_NONE,
    },
    Language {
        name: "Shell",
        extensions: &["sh", "bash", "zsh", "fish", "ksh", "env"],
        line: HASH,
        block: None,
        quotes: QUOTES_C,
        branch: BRANCH_SHELL,
        doc_attached: false,
        documentation: false,
        test_fn: TEST_FN_NONE,
    },
    Language {
        name: "Svelte",
        extensions: &["svelte", "svx"],
        line: SLASH,
        block: HTML_BLOCK,
        quotes: QUOTES_C,
        branch: &["if", "else", "each", "case", "&&", "||", "==", "!="],
        doc_attached: false,
        documentation: false,
        test_fn: TEST_FN_NONE,
    },
    Language {
        name: "Swift",
        extensions: &["swift"],
        line: SLASH,
        block: C_BLOCK,
        quotes: QUOTES_C,
        branch: &[
            "if", "else", "for", "while", "case", "guard", "switch", "&&", "||", "==", "!=",
        ],
        doc_attached: false,
        documentation: false,
        test_fn: TEST_FN_NONE,
    },
    Language {
        name: "TOML",
        extensions: &["toml"],
        line: HASH,
        block: None,
        quotes: QUOTES_C,
        branch: BRANCH_NONE,
        doc_attached: false,
        documentation: false,
        test_fn: TEST_FN_NONE,
    },
    Language {
        name: "TypeScript",
        extensions: &["ts", "tsx", "mts", "cts"],
        line: SLASH,
        block: C_BLOCK,
        quotes: QUOTES_GO,
        branch: BRANCH_JS,
        doc_attached: false,
        documentation: false,
        test_fn: TEST_FN_NONE,
    },
    Language {
        name: "TeX",
        extensions: &["tex", "sty", "cls", "bib"],
        line: PERCENT,
        block: None,
        quotes: QUOTES_NONE,
        branch: BRANCH_NONE,
        doc_attached: false,
        documentation: true,
        test_fn: TEST_FN_NONE,
    },
    Language {
        name: "YAML",
        extensions: &["yml", "yaml"],
        line: HASH,
        block: None,
        quotes: QUOTES_C,
        branch: BRANCH_NONE,
        doc_attached: false,
        documentation: false,
        test_fn: TEST_FN_NONE,
    },
    Language {
        name: "Zig",
        extensions: &["zig"],
        line: SLASH,
        block: C_BLOCK,
        quotes: QUOTES_C,
        branch: BRANCH_C,
        doc_attached: false,
        documentation: false,
        test_fn: TEST_FN_NONE,
    },
    Language {
        name: "Makefile",
        extensions: &["mk", "mak"],
        line: HASH,
        block: None,
        quotes: QUOTES_C,
        branch: &["ifeq", "ifneq", "ifdef", "ifndef", "else"],
        doc_attached: false,
        documentation: false,
        test_fn: TEST_FN_NONE,
    },
    Language {
        name: "Dockerfile",
        extensions: &["dockerfile"],
        line: HASH,
        block: None,
        quotes: QUOTES_C,
        branch: BRANCH_NONE,
        doc_attached: false,
        documentation: false,
        test_fn: TEST_FN_NONE,
    },
];

/// Files with no extension that are still worth counting.
const FILENAMES: [(&str, &str); 4] = [
    ("makefile", "Makefile"),
    ("dockerfile", "Dockerfile"),
    ("cmakelists.txt", "CMake"),
    ("rakefile", "Ruby"),
];

/// The language a path belongs to: by extension first, then by file name.
pub fn detect(path: &std::path::Path) -> Option<&'static Language> {
    if let Some(extension) = path.extension() {
        let extension = extension.to_string_lossy().to_ascii_lowercase();
        let found = LANGUAGES
            .iter()
            .find(|language| language.extensions.iter().any(|known| *known == extension));
        if let Some(found) = found {
            return Some(found);
        }
    }
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let known = FILENAMES.iter().find(|(file, _)| *file == name)?;
    LANGUAGES.iter().find(|language| language.name == known.1)
}

/// What one file contributed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LineCounts {
    pub lines: u32,
    pub blank: u32,
    pub comment: u32,
    pub code: u32,
}

/// Classify every physical line as blank, comment-only or code, the way tokei
/// and cloc do: a line holding both code and a trailing comment counts as code,
/// and a block comment marks every line it covers.
///
/// The walk is the one [`crate::generic::analyze`] does, so the numbers a
/// `--languages` table shows and the numbers a `--sources` report shows for the
/// same file cannot drift apart.
pub fn count(text: &str, language: &Language) -> LineCounts {
    let analysis = generic::analyze(text, language);
    let mut counts = LineCounts {
        lines: analysis.total as u32,
        ..LineCounts::default()
    };
    for index in 0..analysis.total {
        if analysis.code[index] {
            counts.code += 1;
        } else if analysis.comment[index] {
            counts.comment += 1;
        } else {
            counts.blank += 1;
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn language(name: &str) -> &'static Language {
        LANGUAGES.iter().find(|item| item.name == name).unwrap()
    }

    #[test]
    fn no_extension_is_claimed_twice() {
        let mut seen: Vec<&'static str> = Vec::new();
        for item in &LANGUAGES {
            for extension in item.extensions {
                assert!(!seen.contains(extension), "{extension} is claimed twice");
                seen.push(extension);
            }
        }
    }

    #[test]
    fn files_are_recognised_by_extension_and_by_name() {
        assert_eq!(detect(Path::new("src/a.rs")).unwrap().name, "Rust");
        assert_eq!(detect(Path::new("web/App.TSX")).unwrap().name, "TypeScript");
        assert_eq!(detect(Path::new("Makefile")).unwrap().name, "Makefile");
        assert!(detect(Path::new("nix/Cargo.lock")).is_none());
    }

    #[test]
    fn blank_comment_and_code_are_separated() {
        let counts = count("// a\n\nfn main() {}\n", language("Rust"));
        assert_eq!(counts.lines, 3);
        assert_eq!(counts.comment, 1);
        assert_eq!(counts.blank, 1);
        assert_eq!(counts.code, 1);
    }

    #[test]
    fn block_comments_cover_every_line_they_span() {
        let counts = count("/* one\n   two */\nint x;\n", language("C"));
        assert_eq!(counts.comment, 2);
        assert_eq!(counts.code, 1);
    }

    #[test]
    fn comment_markers_inside_strings_stay_code() {
        let counts = count("let url = \"http://x\";\n", language("JavaScript"));
        assert_eq!(counts.code, 1);
        assert_eq!(counts.comment, 0);
    }

    #[test]
    fn trailing_comments_leave_the_line_as_code() {
        let counts = count("int x = 1; // one\n", language("C"));
        assert_eq!(counts.code, 1);
        assert_eq!(counts.comment, 0);
    }

    #[test]
    fn python_triple_quotes_are_a_block_comment() {
        let counts = count("\"\"\"doc\nstring\"\"\"\nx = 1\n", language("Python"));
        assert_eq!(counts.comment, 2);
        assert_eq!(counts.code, 1);
    }

    #[test]
    fn markdown_is_counted_as_documentation() {
        // A documentation format ships prose, not code: --languages reads it
        // the way the main report does.
        let counts = count("# Title\n\nSome text.\n", language("Markdown"));
        assert_eq!(counts.code, 0);
        assert_eq!(counts.comment, 2);
        assert_eq!(counts.blank, 1);
    }
}
