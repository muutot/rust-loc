//! Scan options shared by every renderer.

use std::collections::BTreeSet;

use crate::cocomo;
use crate::generated;
use crate::scan;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Text,
    Json,
    Csv,
    Markdown,
}

impl Format {
    pub fn parse(value: &str) -> Option<Format> {
        match value {
            "text" => Some(Format::Text),
            "json" => Some(Format::Json),
            "csv" => Some(Format::Csv),
            "markdown" | "md" => Some(Format::Markdown),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Options {
    pub by_file: bool,
    pub by_module: bool,
    pub sort: String,
    pub top: usize,
    pub show_excluded: bool,
    pub include_tests: bool,
    pub exclude: Vec<String>,
    pub ignore_dirs: BTreeSet<String>,
    pub gitignore: bool,
    pub uloc: bool,
    pub complexity: bool,
    /// Collect per-function metrics and list them ranked by complexity.
    pub functions: bool,
    /// Rows kept in the ranked function list; 0 keeps every function.
    pub top_functions: usize,
    /// Drop detected generated/minified files from the business scope.
    pub exclude_generated: bool,
    /// Tag file listings with (gen)/(min) instead of dropping them.
    pub mark_generated: bool,
    /// Average line length that marks a file minified; 0 disables the check.
    pub min_gen_line_length: usize,
    /// Add a COCOMO cost estimate for the business code.
    pub cocomo: bool,
    pub cocomo_project: cocomo::ProjectType,
    pub cocomo_params: cocomo::Params,
    pub format: Format,
    /// 0 means "use every available core".
    pub jobs: usize,
}

/// Rows in the ranked function list unless --top-functions says otherwise.
pub const DEFAULT_TOP_FUNCTIONS: usize = 20;

impl Default for Options {
    fn default() -> Self {
        Options {
            by_file: false,
            by_module: false,
            sort: "code".to_string(),
            top: 0,
            show_excluded: false,
            include_tests: false,
            exclude: Vec::new(),
            ignore_dirs: scan::default_ignore_dirs(),
            gitignore: true,
            uloc: false,
            complexity: true,
            functions: false,
            top_functions: DEFAULT_TOP_FUNCTIONS,
            exclude_generated: false,
            mark_generated: false,
            min_gen_line_length: generated::DEFAULT_MIN_LINE_LENGTH,
            cocomo: false,
            cocomo_project: cocomo::ProjectType::Organic,
            cocomo_params: cocomo::ORGANIC,
            format: Format::Text,
            jobs: 0,
        }
    }
}

impl Options {
    pub fn column_keys(&self) -> Vec<&'static str> {
        let mut columns = vec!["lines", "blank", "comment", "code", "test"];
        if self.complexity {
            columns.push("complexity");
        }
        if self.uloc {
            columns.push("uloc");
        }
        columns
    }

    pub fn column_titles(&self) -> Vec<&'static str> {
        let mut titles = vec!["Lines", "Blank", "Comment", "Code", "Test"];
        if self.complexity {
            titles.push("Cplx");
        }
        if self.uloc {
            titles.push("ULOC");
        }
        titles
    }

    /// Number of worker threads to use for file analysis.
    pub fn worker_count(&self) -> usize {
        if self.jobs > 0 {
            return self.jobs;
        }
        std::thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(1)
    }

    /// The per-file work `scan::collect_files` should do for this run.
    pub fn read_options(&self) -> scan::ReadOptions {
        scan::ReadOptions {
            uloc: self.uloc,
            functions: self.functions,
            include_tests: self.include_tests,
            min_gen_line_length: self.min_gen_line_length,
        }
    }
}
