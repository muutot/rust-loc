//! A small language table and a comment-aware line counter.
//!
//! `--languages` answers "what is this repository made of", which a Rust-only
//! counter cannot. The table is hand written: one entry per language, no
//! dependencies, and only the extensions that show up in mixed repositories.
//! Files whose extension is not in the table are never read.

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
pub const LANGUAGES: [Language; 32] = [
    Language {
        name: "C",
        extensions: &["c", "h"],
        line: SLASH,
        block: C_BLOCK,
        quotes: QUOTES_C,
    },
    Language {
        name: "C#",
        extensions: &["cs"],
        line: SLASH,
        block: C_BLOCK,
        quotes: QUOTES_C,
    },
    Language {
        name: "C++",
        extensions: &["cc", "cpp", "cxx", "hpp", "hh", "hxx"],
        line: SLASH,
        block: C_BLOCK,
        quotes: QUOTES_C,
    },
    Language {
        name: "CMake",
        extensions: &["cmake"],
        line: HASH,
        block: None,
        quotes: QUOTES_C,
    },
    Language {
        name: "CSS",
        extensions: &["css", "scss", "less", "sass"],
        line: SLASH,
        block: C_BLOCK,
        quotes: QUOTES_C,
    },
    Language {
        name: "Dart",
        extensions: &["dart"],
        line: SLASH,
        block: C_BLOCK,
        quotes: QUOTES_C,
    },
    Language {
        name: "Go",
        extensions: &["go"],
        line: SLASH,
        block: C_BLOCK,
        quotes: QUOTES_GO,
    },
    Language {
        name: "Haskell",
        extensions: &["hs", "lhs"],
        line: DASH,
        block: Some(("{-", "-}")),
        quotes: QUOTES_C,
    },
    Language {
        name: "HTML",
        extensions: &["html", "htm", "xhtml", "xml", "svg"],
        line: NONE,
        block: HTML_BLOCK,
        quotes: QUOTES_C,
    },
    Language {
        name: "Java",
        extensions: &["java"],
        line: SLASH,
        block: C_BLOCK,
        quotes: QUOTES_C,
    },
    Language {
        name: "JavaScript",
        extensions: &["js", "mjs", "cjs", "jsx"],
        line: SLASH,
        block: C_BLOCK,
        quotes: QUOTES_GO,
    },
    Language {
        name: "JSON",
        extensions: &["json", "jsonc", "json5"],
        line: SLASH,
        block: C_BLOCK,
        quotes: QUOTES_C,
    },
    Language {
        name: "Kotlin",
        extensions: &["kt", "kts"],
        line: SLASH,
        block: C_BLOCK,
        quotes: QUOTES_C,
    },
    Language {
        name: "Lua",
        extensions: &["lua"],
        line: DASH,
        block: Some(("--[[", "]]")),
        quotes: QUOTES_C,
    },
    Language {
        name: "Markdown",
        extensions: &["md", "markdown", "mdx"],
        line: NONE,
        block: HTML_BLOCK,
        quotes: QUOTES_NONE,
    },
    Language {
        name: "OCaml",
        extensions: &["ml", "mli", "fs", "fsx"],
        line: SLASH,
        block: ML_BLOCK,
        quotes: QUOTES_C,
    },
    Language {
        name: "PHP",
        extensions: &["php"],
        line: SLASH,
        block: C_BLOCK,
        quotes: QUOTES_C,
    },
    Language {
        name: "PowerShell",
        extensions: &["ps1", "psm1", "psd1"],
        line: HASH,
        block: Some(("<#", "#>")),
        quotes: QUOTES_C,
    },
    Language {
        name: "Python",
        extensions: &["py", "pyi", "pyw"],
        line: HASH,
        block: PY_BLOCK,
        quotes: QUOTES_C,
    },
    Language {
        name: "Ruby",
        extensions: &["rb", "rake", "gemspec"],
        line: HASH,
        block: Some(("=begin", "=end")),
        quotes: QUOTES_C,
    },
    Language {
        name: "Rust",
        extensions: &["rs"],
        line: SLASH,
        block: C_BLOCK,
        quotes: QUOTES_C,
    },
    Language {
        name: "SQL",
        extensions: &["sql"],
        line: DASH,
        block: C_BLOCK,
        quotes: QUOTES_C,
    },
    Language {
        name: "Shell",
        extensions: &["sh", "bash", "zsh", "fish", "ksh", "env"],
        line: HASH,
        block: None,
        quotes: QUOTES_C,
    },
    Language {
        name: "Svelte",
        extensions: &["svelte", "svx"],
        line: SLASH,
        block: HTML_BLOCK,
        quotes: QUOTES_C,
    },
    Language {
        name: "Swift",
        extensions: &["swift"],
        line: SLASH,
        block: C_BLOCK,
        quotes: QUOTES_C,
    },
    Language {
        name: "TOML",
        extensions: &["toml"],
        line: HASH,
        block: None,
        quotes: QUOTES_C,
    },
    Language {
        name: "TypeScript",
        extensions: &["ts", "tsx", "mts", "cts"],
        line: SLASH,
        block: C_BLOCK,
        quotes: QUOTES_GO,
    },
    Language {
        name: "TeX",
        extensions: &["tex", "sty", "cls", "bib"],
        line: PERCENT,
        block: None,
        quotes: QUOTES_NONE,
    },
    Language {
        name: "YAML",
        extensions: &["yml", "yaml"],
        line: HASH,
        block: None,
        quotes: QUOTES_C,
    },
    Language {
        name: "Zig",
        extensions: &["zig"],
        line: SLASH,
        block: C_BLOCK,
        quotes: QUOTES_C,
    },
    Language {
        name: "Makefile",
        extensions: &["mk", "mak"],
        line: HASH,
        block: None,
        quotes: QUOTES_C,
    },
    Language {
        name: "Dockerfile",
        extensions: &["dockerfile"],
        line: HASH,
        block: None,
        quotes: QUOTES_C,
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
pub fn count(text: &str, language: &Language) -> LineCounts {
    let mut counts = LineCounts::default();
    let mut closing: Option<&'static str> = None;
    for raw in text.lines() {
        counts.lines += 1;
        let bytes = raw.as_bytes();
        let mut i = 0usize;
        let mut code = false;
        let mut comment = false;
        while i < bytes.len() {
            let rest = &bytes[i..];
            if let Some(close) = closing {
                comment = true;
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
                    comment = true;
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
                comment = true;
                break;
            }
            if language.quotes.iter().any(|quote| bytes[i] == *quote as u8) {
                code = true;
                let quote = bytes[i];
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i += 2;
                    } else if bytes[i] == quote {
                        i += 1;
                        break;
                    } else {
                        i += 1;
                    }
                }
                continue;
            }
            code = true;
            i += 1;
        }
        if code {
            counts.code += 1;
        } else if comment {
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
    fn markdown_without_comments_is_all_code() {
        let counts = count("# Title\n\nSome text.\n", language("Markdown"));
        assert_eq!(counts.code, 2);
        assert_eq!(counts.blank, 1);
    }
}
