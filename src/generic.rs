//! The reader every language without a lexer of its own shares.
//!
//! Rust and Python have grammars worth following: brace spans for test
//! modules, decorators, signatures, nesting. The other thirty languages of
//! [`crate::languages`] owe the report the same answer shape and nothing more,
//! so this scanner reads the comment, literal and branch rules of the language
//! table and keeps every metric that survives without a parser: the line
//! classes, the branch tokens as a complexity estimate, the Halstead tokens and
//! the `TODO` markers in comments.
//!
//! What it cannot do is find a test item inside a file, so every line comes
//! back as business code and test files are recognised by their path alone -
//! the way [`crate::scan::is_test_file`] already does it. What it does count
//! is the test-function declaration itself, one matched line per `test_fn`
//! shape in the language table (Go's `func TestXxx(`), because that is all
//! the runner keys on. Function-level metrics, `unsafe`, `unwrap` and the
//! other method-shaped smells stay zero rather than guess, and the report
//! drops the rows that would only ever print a misleading `0`.

use crate::languages::Language;
use crate::lexer::{physical_lines, Analysis, Metrics, Smells, TokenCounter};

/// Comment markers that make a comment a doc comment: the spellings every
/// C-like language inherited from C, Rust included.
const DOC_MARKERS: [&str; 4] = ["///", "//!", "/**", "/*!"];

/// Markers that make a comment a leftover to finish, the spelling every
/// language shares.
const TODO_MARKERS: [&str; 3] = ["TODO", "FIXME", "XXX"];

/// Whether a byte can start a name: ASCII letters, the underscore and anything
/// above it, so an identifier written in another script is a name too.
fn is_name_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80
}

/// Whether a byte can continue a name.
fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

/// Whether a branch token is a word, so it is matched on word boundaries
/// rather than as plain text.
fn is_word_token(token: &str) -> bool {
    token.starts_with(|c: char| c.is_ascii_alphabetic())
}

/// Mark every comment group that sits directly above a code line as a doc
/// comment, the way godoc reads one. A blank line between the group and the
/// code, or a group at the end of the file, keeps it a plain comment.
fn mark_attached_doc(comment: &mut [bool], doc: &mut [bool], code: &[bool]) {
    let total = comment.len();
    let mut index = 0usize;
    while index < total {
        if !comment[index] {
            index += 1;
            continue;
        }
        let mut end = index;
        while end + 1 < total && comment[end + 1] {
            end += 1;
        }
        if end + 1 < total && code[end + 1] {
            for line in &mut doc[index..=end] {
                *line = true;
            }
        }
        index = end + 1;
    }
}

/// Classify every line of a file and collect the metrics the language offers.
pub fn analyze(text: &str, language: &Language) -> Analysis {
    let total = physical_lines(text);
    let mut code = vec![false; total];
    let mut comment = vec![false; total];
    let mut doc = vec![false; total];
    let mut complexity = 0u32;
    let mut todo = 0u32;
    let mut test_fns = 0u32;
    let mut tokens = TokenCounter::default();
    let mut closing: Option<&'static str> = None;

    for (index, raw) in text.lines().enumerate() {
        if index >= total {
            break;
        }
        let bytes = raw.as_bytes();
        let mut line_code = false;
        let mut line_comment = false;
        // A test-function declaration is one matched line: cheap, and it is
        // exactly what `go test` and the JVM runners themselves key on. The
        // shapes come from the language table, so a language without a shape
        // costs nothing.
        let trimmed = raw.trim_start();
        if !language.test_fn.is_empty()
            && language.test_fn.iter().any(|shape| shape.matches(trimmed))
        {
            test_fns += 1;
        }
        let mut i = 0usize;
        while i < bytes.len() {
            let rest = &bytes[i..];
            if let Some(close) = closing {
                line_comment = true;
                if rest.starts_with(close.as_bytes()) {
                    i += close.len();
                    closing = None;
                } else {
                    i += 1;
                }
                continue;
            }
            if bytes[i].is_ascii_whitespace() {
                i += 1;
                continue;
            }
            if let Some((open, close)) = language.block {
                if rest.starts_with(open.as_bytes()) {
                    line_comment = true;
                    closing = Some(close);
                    i += open.len();
                    continue;
                }
            }
            if language
                .line
                .iter()
                .any(|prefix| rest.starts_with(prefix.as_bytes()))
            {
                line_comment = true;
                let lower = raw.to_ascii_lowercase();
                if TODO_MARKERS
                    .iter()
                    .any(|marker| lower.contains(&marker.to_ascii_lowercase()))
                {
                    todo += 1;
                }
                break;
            }
            if language.quotes.iter().any(|quote| bytes[i] == *quote as u8) {
                line_code = true;
                let start = i;
                let quote = bytes[i];
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i = (i + 2).min(bytes.len());
                    } else if bytes[i] == quote {
                        i += 1;
                        break;
                    } else {
                        i += 1;
                    }
                }
                tokens.operand(&bytes[start..i.min(bytes.len())]);
                continue;
            }
            line_code = true;
            if is_name_start(bytes[i]) {
                let start = i;
                while i < bytes.len() && is_name_byte(bytes[i]) {
                    i += 1;
                }
                let word = &bytes[start..i];
                if language
                    .branch
                    .iter()
                    .any(|token| is_word_token(token) && token.as_bytes() == word)
                {
                    complexity += 1;
                    tokens.operator(word);
                } else {
                    tokens.operand(word);
                }
                continue;
            }
            if bytes[i].is_ascii_digit() {
                let start = i;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.')
                {
                    i += 1;
                }
                tokens.operand(&bytes[start..i]);
                continue;
            }
            // A branch token spelled with punctuation, like the `&&` the
            // C-like languages share.
            let symbolic = language
                .branch
                .iter()
                .find(|token| !is_word_token(token) && rest.starts_with(token.as_bytes()));
            if let Some(token) = symbolic {
                complexity += 1;
                tokens.operator(token.as_bytes());
                i += token.len();
                continue;
            }
            tokens.operator(&bytes[i..i + 1]);
            i += 1;
        }

        if line_code {
            code[index] = true;
            // A documentation format ships prose, not code: its lines are
            // counted as doc so the report never calls a README "code".
            if language.documentation {
                code[index] = false;
                comment[index] = true;
                doc[index] = true;
            }
        } else if line_comment {
            comment[index] = true;
            let trimmed = raw.trim_start();
            if DOC_MARKERS.iter().any(|marker| trimmed.starts_with(marker)) {
                doc[index] = true;
            }
        }
    }

    let smells = Smells {
        todo,
        ..Smells::default()
    };
    // A language whose documentation is a plain comment group - Go's godoc -
    // gets the group marked as doc when it sits directly above what it
    // documents.
    if language.doc_attached {
        mark_attached_doc(&mut comment, &mut doc, &code);
    }
    // Without brace spans the business scope and the whole file agree, and a
    // branch pays for no nesting nobody measured.
    let metrics = Metrics {
        complexity,
        cognitive: complexity,
        test_fns,
        halstead: tokens.finish(),
        ..Metrics::default()
    };
    Analysis {
        total,
        code,
        comment,
        doc,
        is_test: vec![false; total],
        crate_test: false,
        sibling_targets: Vec::new(),
        metrics,
        metrics_all: metrics,
        smells,
        smells_all: smells,
        functions: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::languages;

    fn language(name: &str) -> &'static Language {
        languages::LANGUAGES
            .iter()
            .find(|item| item.name == name)
            .unwrap()
    }

    fn counts(text: &str, name: &str) -> languages::LineCounts {
        languages::count(text, language(name))
    }

    fn analyse(text: &str, name: &str) -> Analysis {
        analyze(text, language(name))
    }

    #[test]
    fn blank_comment_and_code_are_separated() {
        let counts = counts("// a\n\nfn main() {}\n", "Rust");
        assert_eq!(counts.lines, 3);
        assert_eq!(counts.comment, 1);
        assert_eq!(counts.blank, 1);
        assert_eq!(counts.code, 1);
    }

    #[test]
    fn block_comments_cover_every_line_they_span() {
        let counts = counts("/* one\n   two */\nint x;\n", "C");
        assert_eq!(counts.comment, 2);
        assert_eq!(counts.code, 1);
    }

    #[test]
    fn comment_markers_inside_strings_stay_code() {
        let counts = counts("let url = \"http://x\";\n", "JavaScript");
        assert_eq!(counts.code, 1);
        assert_eq!(counts.comment, 0);
    }

    #[test]
    fn trailing_comments_leave_the_line_as_code() {
        let counts = counts("int x = 1; // one\n", "C");
        assert_eq!(counts.code, 1);
        assert_eq!(counts.comment, 0);
    }

    #[test]
    fn go_test_functions_are_counted() {
        let analysis = analyse(
            "package money\n\
             \n\
             func TestAmount(t *testing.T) {\n\
             \tt.Run(\"sub\", func(t *testing.T) {})\n\
             }\n\
             \n\
             func helper(t *testing.T) {}\n\
             func testLower(t *testing.T) {}\n",
            "Go",
        );
        // One declaration: `func TestAmount(`. The subtest runs inside it,
        // and the lowercase helpers never matched the uppercase rule.
        assert_eq!(analysis.metrics.test_fns, 1);
    }

    #[test]
    fn a_go_call_site_is_not_a_test_function() {
        let analysis = analyse(
            "package money\n\
             \n\
             func main() {\n\
             \tTestAmount(nil) // a call, not a declaration\n\
             }\n",
            "Go",
        );
        assert_eq!(analysis.metrics.test_fns, 0);
    }

    #[test]
    fn python_triple_quotes_are_a_block_comment() {
        let counts = counts("\"\"\"doc\nstring\"\"\"\nx = 1\n", "Python");
        assert_eq!(counts.comment, 2);
        assert_eq!(counts.code, 1);
    }

    #[test]
    fn markdown_is_documentation_rather_than_code() {
        let counts = counts("# Title\n\nSome text.\n", "Markdown");
        // Every non-blank line of a documentation format is a doc line.
        assert_eq!(counts.code, 0);
        assert_eq!(counts.comment, 2);
        assert_eq!(counts.blank, 1);
    }

    #[test]
    fn doc_comments_are_marked_where_the_language_spells_them() {
        let analysis = analyse("/// one\n//! two\n// three\nfn a() {}\n", "Rust");
        assert!(analysis.doc[0]);
        assert!(analysis.doc[1]);
        assert!(!analysis.doc[2]);
        assert!(!analysis.doc[3]);
        assert!(analysis.comment[0]);
        assert!(analysis.code[3]);
    }

    #[test]
    fn branch_tokens_are_counted_per_language() {
        let go = analyse("if a && b { for { } } else if c { }\n", "Go");
        assert_eq!(go.metrics.complexity, 5);
        assert_eq!(go.metrics_all.complexity, 5);
        let java = analyse("if (a && b) { } else { }\n", "Java");
        assert_eq!(java.metrics.complexity, 3);
        let json = analyse("{\"a\": 1}\n", "JSON");
        assert_eq!(json.metrics.complexity, 0);
        let shell = analyse("if [ -f a ]; then\n  for f in b; do :; done\nfi\n", "Shell");
        assert_eq!(shell.metrics.complexity, 2);
    }

    #[test]
    fn branch_words_inside_strings_are_not_branches() {
        let analysis = analyse("let s = \"if for while\";\n", "Go");
        assert_eq!(analysis.metrics.complexity, 0);
    }

    #[test]
    fn todo_markers_in_comments_are_counted() {
        let analysis = analyse("// TODO: fix\nlet a = 1; // FIXME soon\n", "Go");
        assert_eq!(analysis.smells.todo, 2);
        assert_eq!(analysis.smells_all.todo, 2);
    }

    #[test]
    fn a_documentation_language_counts_its_lines_as_doc() {
        let analysis = analyse("# Title\n\nSome prose about `code`.\n", "Markdown");
        assert!(analysis.doc[0], "a heading is documentation");
        assert!(analysis.comment[0]);
        assert!(!analysis.code[0]);
        assert!(!analysis.doc[1]);
        assert!(!analysis.code[1]);
        assert!(analysis.doc[2]);
        // The generic line counter agrees, so `--languages` reads the same.
        let counts = counts("# Title\n\nSome prose about `code`.\n", "Markdown");
        assert_eq!(counts.code, 0);
        assert_eq!(counts.comment, 2);
    }

    #[test]
    fn a_go_doc_comment_sits_above_what_it_documents() {
        let analysis = analyse(
            "// Package main is the entry point.\npackage main\n\n// A note, on its own.\n",
            "Go",
        );
        assert!(analysis.doc[0], "the package comment is documentation");
        assert!(analysis.comment[0]);
        assert!(
            !analysis.doc[3],
            "a comment with a blank line under it is not"
        );
        assert!(analysis.comment[3]);
    }

    #[test]
    fn other_languages_only_mark_a_marked_doc_comment() {
        let analysis = analyse("// note\nint x;\n/// doc\nint y;\n", "C");
        assert!(!analysis.doc[0], "C spells documentation with /** */");
        assert!(analysis.doc[2]);
    }

    #[test]
    fn everything_else_stays_at_zero_without_a_parser() {
        let analysis = analyse("func a() error { if true { } return nil }\n", "Go");
        assert!(analysis.functions.is_empty());
        assert_eq!(analysis.smells.unwrap, 0);
        assert_eq!(analysis.metrics.unsafe_kw, 0);
        assert_eq!(analysis.metrics.test_fns, 0);
        assert!(!analysis.is_test.iter().any(|flag| *flag));
        assert!(!analysis.crate_test);
        assert!(analysis.sibling_targets.is_empty());
        assert!(analysis.metrics.halstead.length() > 0);
    }

    #[test]
    fn the_halstead_tokens_separate_names_from_punctuation() {
        let analysis = analyse("let alpha = beta + gamma;\n", "Go");
        let halstead = analysis.metrics.halstead;
        assert!(halstead.operands >= 3, "{halstead:?}");
        assert!(halstead.operators >= 3, "{halstead:?}");
        assert_eq!(analysis.metrics_all.halstead, halstead);
    }
}
