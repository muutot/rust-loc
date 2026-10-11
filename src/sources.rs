//! Which source languages a scan reads.
//!
//! The counters started out reading Rust and nothing else, so the default
//! selection keeps that promise: a scan sees the `.rs` files it always saw.
//! `--sources python` reads a Python tree instead, and `--sources all` reads
//! both, which is what a repository that ships a Rust core and a Python tool
//! wants from one run.
//!
//! A language brings its own reader ([`crate::lexer`] or [`crate::pylexer`])
//! and its own idea of what a test is; everything downstream - the report, the
//! tables, the gates - is shared.

use std::path::Path;

/// A language the counters can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Rust,
    Python,
}

impl Lang {
    /// Every language, in the order the help lists them.
    pub const ALL: [Lang; 2] = [Lang::Rust, Lang::Python];

    /// The name the language table and the reports use.
    pub fn name(self) -> &'static str {
        match self {
            Lang::Rust => "Rust",
            Lang::Python => "Python",
        }
    }

    /// The file extension, without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            Lang::Rust => "rs",
            Lang::Python => "py",
        }
    }

    /// The language a path belongs to, by extension.
    pub fn of(path: &Path) -> Option<Lang> {
        let extension = path.extension()?.to_str()?;
        Lang::ALL
            .into_iter()
            .find(|lang| extension.eq_ignore_ascii_case(lang.extension()))
    }

    /// The language a `--sources` value names, when it names one.
    pub fn parse(value: &str) -> Option<Lang> {
        let value = value.trim().to_ascii_lowercase();
        match value.as_str() {
            "rust" | "rs" => Some(Lang::Rust),
            "python" | "py" => Some(Lang::Python),
            _ => None,
        }
    }
}

/// The selection `--sources` carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sources {
    /// The default: today's behaviour, and the only one the Python reference
    /// in `python/` implements.
    #[default]
    Rust,
    Python,
    All,
}

impl Sources {
    /// The values `--sources` accepts, for the help text and the config gate.
    pub const ALL: [Sources; 3] = [Sources::Rust, Sources::Python, Sources::All];

    pub fn parse(value: &str) -> Option<Sources> {
        let value = value.trim().to_ascii_lowercase();
        match value.as_str() {
            "all" | "both" => Some(Sources::All),
            other => Lang::parse(other).map(Sources::of),
        }
    }

    /// The selection that reads one language.
    pub fn of(lang: Lang) -> Sources {
        match lang {
            Lang::Rust => Sources::Rust,
            Lang::Python => Sources::Python,
        }
    }

    /// The value `--sources` was given, and the one the config takes.
    pub fn slug(self) -> &'static str {
        match self {
            Sources::Rust => "rust",
            Sources::Python => "python",
            Sources::All => "all",
        }
    }

    /// The languages this selection reads.
    pub fn languages(self) -> &'static [Lang] {
        match self {
            Sources::Rust => &[Lang::Rust],
            Sources::Python => &[Lang::Python],
            Sources::All => &Lang::ALL,
        }
    }

    /// Whether a discovered file is in scope.
    pub fn accepts(self, path: &Path) -> bool {
        Lang::of(path).is_some_and(|lang| self.languages().contains(&lang))
    }

    /// Whether a language is in scope, for report wording.
    pub fn covers(self, lang: Lang) -> bool {
        self.languages().contains(&lang)
    }

    /// The `git ls-files` pathspecs that cover this selection. A Rust-only
    /// scan keeps the single `*.rs` glob it always passed to git.
    pub fn pathspecs(self) -> Vec<&'static str> {
        self.languages()
            .iter()
            .map(|lang| match lang {
                Lang::Rust => "*.rs",
                Lang::Python => "*.py",
            })
            .collect()
    }

    /// The wording a report uses when it has to name the scope.
    pub fn label(self) -> &'static str {
        match self {
            Sources::Rust => "Rust",
            Sources::Python => "Python",
            Sources::All => "Rust and Python",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_the_rust_only_scan() {
        assert_eq!(Sources::default(), Sources::Rust);
        assert_eq!(Sources::default().slug(), "rust");
    }

    #[test]
    fn a_language_is_read_from_the_extension() {
        assert_eq!(Lang::of(Path::new("src/main.rs")), Some(Lang::Rust));
        assert_eq!(Lang::of(Path::new("tools/cli.py")), Some(Lang::Python));
        assert_eq!(Lang::of(Path::new("src/main.RS")), Some(Lang::Rust));
        assert_eq!(Lang::of(Path::new("README.md")), None);
        assert_eq!(Lang::of(Path::new("Makefile")), None);
    }

    #[test]
    fn a_selection_only_takes_the_files_it_names() {
        assert!(Sources::Rust.accepts(Path::new("a.rs")));
        assert!(!Sources::Rust.accepts(Path::new("a.py")));
        assert!(Sources::Python.accepts(Path::new("a.py")));
        assert!(!Sources::Python.accepts(Path::new("a.rs")));
        assert!(Sources::All.accepts(Path::new("a.py")));
        assert!(Sources::All.accepts(Path::new("a.rs")));
        assert!(!Sources::All.accepts(Path::new("a.md")));
    }

    #[test]
    fn every_selection_round_trips_through_its_own_value() {
        for selection in Sources::ALL {
            assert_eq!(Sources::parse(selection.slug()), Some(selection));
        }
        assert_eq!(Sources::parse("PY"), Some(Sources::Python));
        assert_eq!(Sources::parse("rs"), Some(Sources::Rust));
        assert_eq!(Sources::parse("both"), Some(Sources::All));
        assert_eq!(Sources::parse("cobol"), None);
    }

    #[test]
    fn covers_answers_for_the_wording_of_a_report() {
        assert!(Sources::All.covers(Lang::Python));
        assert!(!Sources::Rust.covers(Lang::Python));
        assert_eq!(Sources::All.label(), "Rust and Python");
    }
}
