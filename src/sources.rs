//! Which source languages a scan reads.
//!
//! The counters started out reading Rust and nothing else, so the default
//! selection keeps that promise: a scan sees the `.rs` files it always saw.
//! `--sources python` reads a Python tree instead, `--sources go,java` a
//! polyglot one, and `--sources all` every language the table knows. The flag
//! takes a comma separated list and may be repeated, because a repository that
//! ships a Rust core with a Go service wants both from one run.
//!
//! A language brings its own reader and its own idea of what a test is. Rust
//! and Python have lexers of their own ([`crate::lexer`],
//! [`crate::pylexer`]) that follow brace spans, attributes and decorators;
//! every other language shares the reader in [`crate::generic`], which reads
//! the comment and literal rules of [`crate::languages`] and keeps the metrics
//! that survive without a parser. Everything downstream - the report, the
//! tables, the gates - is shared.

use std::path::Path;

use crate::generic;
use crate::languages;
use crate::lexer::{self, Analysis};
use crate::pylexer;

/// Files the language table knows by name rather than by extension, spelled
/// the way they sit in a repository, so a `git ls-files` pathspec finds them.
const NAMED_FILES: [(&str, Lang); 4] = [
    ("Makefile", Lang::Makefile),
    ("Dockerfile", Lang::Dockerfile),
    ("CMakeLists.txt", Lang::CMake),
    ("Rakefile", Lang::Ruby),
];

/// A language the counters can read.///
/// The discriminant doubles as the bit the [`Sources`] mask sets, so a
/// selection is one `u64` and adding a language never renumbers the others.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum Lang {
    Rust = 1 << 0,
    Python = 1 << 1,
    C = 1 << 2,
    Cpp = 1 << 3,
    CSharp = 1 << 4,
    CMake = 1 << 5,
    Css = 1 << 6,
    Dart = 1 << 7,
    Go = 1 << 8,
    Haskell = 1 << 9,
    Html = 1 << 10,
    Java = 1 << 11,
    JavaScript = 1 << 12,
    Json = 1 << 13,
    Kotlin = 1 << 14,
    Lua = 1 << 15,
    Markdown = 1 << 16,
    OCaml = 1 << 17,
    Php = 1 << 18,
    PowerShell = 1 << 19,
    Ruby = 1 << 20,
    Sql = 1 << 21,
    Shell = 1 << 22,
    Svelte = 1 << 23,
    Swift = 1 << 24,
    Toml = 1 << 25,
    TypeScript = 1 << 26,
    Tex = 1 << 27,
    Yaml = 1 << 28,
    Zig = 1 << 29,
    Makefile = 1 << 30,
    Dockerfile = 1 << 31,
}

impl Lang {
    /// Every language, in the order the help lists them and the window draws
    /// the check boxes: the two with a reader of their own first, then the
    /// rest in the alphabetical order of the language table.
    pub const ALL: [Lang; 32] = [
        Lang::Rust,
        Lang::Python,
        Lang::C,
        Lang::Cpp,
        Lang::CSharp,
        Lang::CMake,
        Lang::Css,
        Lang::Dart,
        Lang::Go,
        Lang::Haskell,
        Lang::Html,
        Lang::Java,
        Lang::JavaScript,
        Lang::Json,
        Lang::Kotlin,
        Lang::Lua,
        Lang::Markdown,
        Lang::OCaml,
        Lang::Php,
        Lang::PowerShell,
        Lang::Ruby,
        Lang::Sql,
        Lang::Shell,
        Lang::Svelte,
        Lang::Swift,
        Lang::Toml,
        Lang::TypeScript,
        Lang::Tex,
        Lang::Yaml,
        Lang::Zig,
        Lang::Makefile,
        Lang::Dockerfile,
    ];

    /// The name the language table and the reports use.
    pub fn name(self) -> &'static str {
        match self {
            Lang::Rust => "Rust",
            Lang::Python => "Python",
            Lang::C => "C",
            Lang::Cpp => "C++",
            Lang::CSharp => "C#",
            Lang::CMake => "CMake",
            Lang::Css => "CSS",
            Lang::Dart => "Dart",
            Lang::Go => "Go",
            Lang::Haskell => "Haskell",
            Lang::Html => "HTML",
            Lang::Java => "Java",
            Lang::JavaScript => "JavaScript",
            Lang::Json => "JSON",
            Lang::Kotlin => "Kotlin",
            Lang::Lua => "Lua",
            Lang::Markdown => "Markdown",
            Lang::OCaml => "OCaml",
            Lang::Php => "PHP",
            Lang::PowerShell => "PowerShell",
            Lang::Ruby => "Ruby",
            Lang::Sql => "SQL",
            Lang::Shell => "Shell",
            Lang::Svelte => "Svelte",
            Lang::Swift => "Swift",
            Lang::Toml => "TOML",
            Lang::TypeScript => "TypeScript",
            Lang::Tex => "TeX",
            Lang::Yaml => "YAML",
            Lang::Zig => "Zig",
            Lang::Makefile => "Makefile",
            Lang::Dockerfile => "Dockerfile",
        }
    }

    /// The value that names the language on a command line, in a config file
    /// and in the JSON scope: lower case, without punctuation.
    pub fn slug(self) -> &'static str {
        match self {
            Lang::Rust => "rust",
            Lang::Python => "python",
            Lang::C => "c",
            Lang::Cpp => "cpp",
            Lang::CSharp => "csharp",
            Lang::CMake => "cmake",
            Lang::Css => "css",
            Lang::Dart => "dart",
            Lang::Go => "go",
            Lang::Haskell => "haskell",
            Lang::Html => "html",
            Lang::Java => "java",
            Lang::JavaScript => "javascript",
            Lang::Json => "json",
            Lang::Kotlin => "kotlin",
            Lang::Lua => "lua",
            Lang::Markdown => "markdown",
            Lang::OCaml => "ocaml",
            Lang::Php => "php",
            Lang::PowerShell => "powershell",
            Lang::Ruby => "ruby",
            Lang::Sql => "sql",
            Lang::Shell => "shell",
            Lang::Svelte => "svelte",
            Lang::Swift => "swift",
            Lang::Toml => "toml",
            Lang::TypeScript => "typescript",
            Lang::Tex => "tex",
            Lang::Yaml => "yaml",
            Lang::Zig => "zig",
            Lang::Makefile => "makefile",
            Lang::Dockerfile => "dockerfile",
        }
    }

    /// The main file extension, without the dot.
    pub fn extension(self) -> &'static str {
        self.language().extensions[0]
    }

    /// The bit this language occupies in a [`Sources`] mask.
    pub const fn bit(self) -> u64 {
        self as u64
    }

    /// The language a path belongs to, by extension or by file name. Unknown
    /// extensions belong to nothing, the way they always did.
    pub fn of(path: &Path) -> Option<Lang> {
        languages::detect(path).and_then(|language| {
            Lang::ALL
                .into_iter()
                .find(|lang| lang.language().name == language.name)
        })
    }

    /// The language a `--sources` value names, when it names one. The name, the
    /// slug, the extension and the handful of spellings people actually type
    /// are all accepted, in any case.
    pub fn parse(value: &str) -> Option<Lang> {
        let wanted = value.trim().to_ascii_lowercase();
        if wanted.is_empty() {
            return None;
        }
        let alias = |alias: &str| Lang::ALL.into_iter().find(|lang| lang.slug() == alias);
        if let Some(found) = alias(&wanted) {
            return Some(found);
        }
        if let Some(found) = Lang::ALL
            .into_iter()
            .find(|lang| lang.name().eq_ignore_ascii_case(&wanted))
        {
            return Some(found);
        }
        let spelled = match wanted.as_str() {
            "golang" => Some("go"),
            "c++" | "cxx" | "cplusplus" => Some("cpp"),
            "c#" | "cs" => Some("csharp"),
            "f#" | "fs" | "fsharp" => Some("ocaml"),
            "py" | "python3" => Some("python"),
            "rs" => Some("rust"),
            "ts" | "tsx" => Some("typescript"),
            "js" | "jsx" => Some("javascript"),
            "yml" => Some("yaml"),
            "md" | "mdx" => Some("markdown"),
            "bash" | "sh" => Some("shell"),
            "ps1" | "pwsh" => Some("powershell"),
            "make" => Some("makefile"),
            "docker" => Some("dockerfile"),
            "latex" => Some("tex"),
            "kt" | "kts" => Some("kotlin"),
            "rb" => Some("ruby"),
            "hs" => Some("haskell"),
            "dart" => Some("dart"),
            _ => None,
        };
        if let Some(slug) = spelled {
            return alias(slug);
        }
        // Any extension the language table knows resolves to its language.
        Lang::ALL
            .into_iter()
            .find(|lang| lang.language().extensions.contains(&wanted.as_str()))
    }

    /// The comment and literal rules behind this language.
    pub fn language(self) -> &'static languages::Language {
        languages::LANGUAGES
            .iter()
            .find(|language| language.name == self.name())
            .expect("every language is in the table")
    }

    /// Read this file with the language's own reader. Rust and Python follow
    /// their grammar; everything else shares the generic scanner, which has
    /// the same answer shape but no brace, attribute or indentation spans.
    pub fn analyze(self, text: &str) -> Analysis {
        match self {
            Lang::Rust => lexer::analyze(text),
            Lang::Python => pylexer::analyze(text),
            other => generic::analyze(text, other.language()),
        }
    }

    /// Whether this language has a lexer of its own, so per-function metrics
    /// and in-file test items exist for it.
    pub fn deep(self) -> bool {
        matches!(self, Lang::Rust | Lang::Python)
    }
}

/// The selection `--sources` carries: the set of languages a scan reads.
///
/// One bit per language keeps a selection as cheap to pass around as the
/// three-variant enum it replaced, while covering any combination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sources(u64);

impl Sources {
    /// Rust alone: today's behaviour, and the only one the Python reference
    /// in `python/` implements.
    pub const RUST: Sources = Sources(Lang::Rust.bit());

    /// Python alone.
    pub const PYTHON: Sources = Sources(Lang::Python.bit());

    /// Every language the table knows, the way `--sources all` asks for it.
    pub const ALL: Sources = Sources(Sources::every_bit());

    /// No language at all: a selection nothing is read with.
    pub const NONE: Sources = Sources(0);

    /// The bit mask with every language in it.
    const fn every_bit() -> u64 {
        let mut mask = 0u64;
        let mut index = 0usize;
        while index < Lang::ALL.len() {
            mask |= Lang::ALL[index] as u64;
            index += 1;
        }
        mask
    }

    /// Parse a comma separated list of names, or a single `all`/`both`. An
    /// empty value selects nothing.
    pub fn parse(value: &str) -> Option<Sources> {
        let mut selection = Sources::NONE;
        for part in value.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            if matches!(part.to_ascii_lowercase().as_str(), "all" | "both" | "every") {
                selection = selection.union(Sources::ALL);
                continue;
            }
            selection = selection.union(Sources::of(Lang::parse(part)?));
        }
        Some(selection)
    }

    /// The selection that reads one language.
    pub fn of(lang: Lang) -> Sources {
        Sources(lang.bit())
    }

    /// The two selections a legacy saved setting names: a single language, or
    /// every one of them.
    pub fn union(self, other: Sources) -> Sources {
        Sources(self.0 | other.0)
    }

    /// Add a language to the selection.
    pub fn insert(&mut self, lang: Lang) {
        self.0 |= lang.bit();
    }

    /// Drop a language from the selection.
    pub fn remove(&mut self, lang: Lang) {
        self.0 &= !lang.bit();
    }

    /// Flip a language in the selection.
    pub fn toggle(&mut self, lang: Lang) {
        self.0 ^= lang.bit();
    }

    /// Whether a language is part of the selection.
    pub fn contains(self, lang: Lang) -> bool {
        self.0 & lang.bit() != 0
    }

    /// Whether nothing is selected, so a scan would read no file at all.
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The languages this selection reads, in the order `Lang::ALL` lists them.
    pub fn languages(self) -> Vec<Lang> {
        Lang::ALL
            .into_iter()
            .filter(|lang| self.contains(*lang))
            .collect()
    }

    /// The one language the selection reads, when it reads exactly one. The
    /// reports use it to name the language their counters belong to.
    pub fn single(self) -> Option<Lang> {
        if self.0 == 0 || self.0 & (self.0 - 1) != 0 {
            return None;
        }
        Lang::ALL.into_iter().find(|lang| self.contains(*lang))
    }

    /// Whether a discovered file is in scope.
    pub fn accepts(self, path: &Path) -> bool {
        Lang::of(path).is_some_and(|lang| self.contains(lang))
    }

    /// Whether a language is in scope, for report wording.
    pub fn covers(self, lang: Lang) -> bool {
        self.contains(lang)
    }

    /// The `git ls-files` pathspecs that cover this selection. A Rust-only
    /// scan keeps the single `*.rs` glob it always passed to git; a selection
    /// names every extension its languages claim, plus the files the language
    /// table knows by name rather than by extension.
    pub fn pathspecs(self) -> Vec<String> {
        let mut globs: Vec<String> = self
            .languages()
            .into_iter()
            .flat_map(|lang| {
                lang.language()
                    .extensions
                    .iter()
                    .map(|extension| format!("*.{extension}"))
            })
            .collect();
        for (name, lang) in NAMED_FILES {
            if self.contains(lang) {
                globs.push(format!("*{name}"));
            }
        }
        globs
    }

    /// The value `--sources` was given, and the one the config and the JSON
    /// scope take: `all` when everything is selected, otherwise the slugs.
    pub fn slug(self) -> String {
        if self == Sources::ALL {
            return "all".to_string();
        }
        let mut out = String::new();
        for lang in self.languages() {
            if !out.is_empty() {
                out.push(',');
            }
            out.push_str(lang.slug());
        }
        out
    }

    /// The wording a report uses when it has to name the scope. One language
    /// is named, a handful are listed, and a wider selection is counted.
    pub fn label(self) -> String {
        if self == Sources::ALL {
            return "all languages".to_string();
        }
        match self.languages().as_slice() {
            [] => "no language".to_string(),
            [lang] => lang.name().to_string(),
            [first, second] => format!("{} and {}", first.name(), second.name()),
            [first, second, third] => {
                format!("{}, {} and {}", first.name(), second.name(), third.name())
            }
            many => format!("{} languages", many.len()),
        }
    }
}

impl Default for Sources {
    /// Rust, so an invocation that never names a language reads what it always
    /// read.
    fn default() -> Self {
        Sources::RUST
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_the_rust_only_scan() {
        assert_eq!(Sources::default(), Sources::RUST);
        assert_eq!(Sources::default().slug(), "rust");
    }

    #[test]
    fn every_bit_tells_the_languages_apart() {
        let mut seen: Vec<u64> = Vec::new();
        for lang in Lang::ALL {
            assert!(lang.bit() != 0, "{} shares a bit with nobody", lang.name());
            assert!(
                !seen.contains(&lang.bit()),
                "{} has a duplicate bit",
                lang.name()
            );
            seen.push(lang.bit());
        }
    }

    #[test]
    fn the_language_table_agrees_with_the_enum() {
        for lang in Lang::ALL {
            assert_eq!(lang.language().name, lang.name());
            assert!(lang.language().extensions.contains(&lang.extension()));
        }
        for language in languages::LANGUAGES.iter() {
            let found = Lang::ALL
                .into_iter()
                .find(|lang| lang.name() == language.name);
            assert!(found.is_some(), "{} is missing a Lang", language.name);
        }
    }

    #[test]
    fn a_language_is_read_from_the_extension() {
        assert_eq!(Lang::of(Path::new("src/main.rs")), Some(Lang::Rust));
        assert_eq!(Lang::of(Path::new("tools/cli.py")), Some(Lang::Python));
        assert_eq!(Lang::of(Path::new("src/main.RS")), Some(Lang::Rust));
        assert_eq!(Lang::of(Path::new("web/App.TSX")), Some(Lang::TypeScript));
        assert_eq!(Lang::of(Path::new("cmd/main.go")), Some(Lang::Go));
        assert_eq!(Lang::of(Path::new("Makefile")), Some(Lang::Makefile));
        assert_eq!(Lang::of(Path::new("Cargo.lock")), None);
    }

    #[test]
    fn a_selection_only_takes_the_files_it_names() {
        assert!(Sources::RUST.accepts(Path::new("a.rs")));
        assert!(!Sources::RUST.accepts(Path::new("a.py")));
        assert!(Sources::PYTHON.accepts(Path::new("a.py")));
        assert!(!Sources::PYTHON.accepts(Path::new("a.rs")));
        assert!(Sources::ALL.accepts(Path::new("a.py")));
        assert!(Sources::ALL.accepts(Path::new("a.rs")));
        // The whole table is the whole table: a Markdown file is one of the
        // languages `--sources all` reads, and only a narrower selection
        // leaves it out.
        assert!(Sources::ALL.accepts(Path::new("a.md")));
        let mixed = Sources::RUST.union(Sources::of(Lang::Go));
        assert!(mixed.accepts(Path::new("a.rs")));
        assert!(mixed.accepts(Path::new("a.go")));
        assert!(!mixed.accepts(Path::new("a.py")));
        assert!(!mixed.accepts(Path::new("a.md")));
    }

    #[test]
    fn every_selection_round_trips_through_its_own_value() {
        assert_eq!(Sources::parse("rust"), Some(Sources::RUST));
        assert_eq!(Sources::parse("PY"), Some(Sources::PYTHON));
        assert_eq!(Sources::parse("rs"), Some(Sources::RUST));
        assert_eq!(Sources::parse("both"), Some(Sources::ALL));
        assert_eq!(Sources::parse("all"), Some(Sources::ALL));
        assert_eq!(Sources::parse("cobol"), None);
        let mixed = Sources::parse("rust,go,java").unwrap();
        assert_eq!(mixed.languages(), vec![Lang::Rust, Lang::Go, Lang::Java]);
        assert_eq!(mixed.slug(), "rust,go,java");
        assert_eq!(mixed.label(), "Rust, Go and Java");
        assert_eq!(Sources::parse(mixed.slug().as_str()), Some(mixed));
        assert_eq!(
            Sources::parse(" Rust , GO "),
            Some(Sources::RUST.union(Sources::of(Lang::Go)))
        );
        assert_eq!(Sources::parse("golang").unwrap(), Sources::of(Lang::Go));
        assert_eq!(Sources::parse("c++").unwrap(), Sources::of(Lang::Cpp));
    }

    #[test]
    fn a_wide_selection_is_counted_rather_than_listed() {
        let mut wide = Sources::ALL;
        wide.remove(Lang::Rust);
        wide.remove(Lang::Python);
        assert_eq!(wide.single(), None);
        assert!(wide.label().ends_with("languages"), "{}", wide.label());
        assert_eq!(Sources::ALL.label(), "all languages");
        assert_eq!(Sources::ALL.slug(), "all");
        assert_eq!(Sources::NONE.label(), "no language");
        assert!(Sources::NONE.is_empty());
        assert!(!Sources::ALL.is_empty());
    }

    #[test]
    fn a_language_can_be_flipped_in_and_out() {
        let mut selection = Sources::RUST;
        selection.toggle(Lang::Go);
        assert!(selection.contains(Lang::Go));
        assert_eq!(selection.single(), None);
        selection.toggle(Lang::Rust);
        assert_eq!(selection.single(), Some(Lang::Go));
    }

    #[test]
    fn covers_answers_for_the_wording_of_a_report() {
        assert!(Sources::ALL.covers(Lang::Python));
        assert!(!Sources::RUST.covers(Lang::Python));
        let mixed = Sources::RUST.union(Sources::PYTHON);
        assert_eq!(mixed.label(), "Rust and Python");
        assert_eq!(mixed.slug(), "rust,python");
    }

    #[test]
    fn pathspecs_cover_every_selected_language() {
        assert_eq!(Sources::RUST.pathspecs(), vec!["*.rs"]);
        assert_eq!(Sources::PYTHON.pathspecs(), vec!["*.py", "*.pyi", "*.pyw"]);
        let mixed = Sources::parse("rust,go").unwrap();
        assert_eq!(mixed.pathspecs(), vec!["*.rs", "*.go"]);
        let make = Sources::of(Lang::Makefile);
        assert_eq!(make.pathspecs(), vec!["*.mk", "*.mak", "*Makefile"]);
        assert!(Sources::ALL.pathspecs().len() >= Lang::ALL.len());
    }
}
