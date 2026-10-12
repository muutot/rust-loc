//! Scan options shared by every renderer.

use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::badge;
use crate::baseline;
use crate::cocomo;
use crate::gate;
use crate::generated;
use crate::scan;
use crate::sources;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Text,
    Json,
    Csv,
    Markdown,
    Svg,
}

impl Format {
    /// Whether the renderer draws the author tables. The SVG card is a summary
    /// of the counters, so it carries no blame and never waits for one.
    pub fn carries_authors(self) -> bool {
        !matches!(self, Format::Svg)
    }

    pub fn parse(value: &str) -> Option<Format> {
        match value {
            "text" => Some(Format::Text),
            "json" => Some(Format::Json),
            "csv" => Some(Format::Csv),
            "markdown" | "md" => Some(Format::Markdown),
            "svg" => Some(Format::Svg),
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
    /// Which source languages the scan reads. Defaults to Rust alone, so an
    /// existing invocation keeps the numbers it always reported.
    pub sources: sources::Sources,
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
    /// Report panic-shaped calls and lint escapes (unwrap/expect/panic/allow).
    pub smells: bool,
    /// Rank files by complexity times how often git says they change.
    pub hotspots: bool,
    /// Add cognitive complexity, Halstead volume and the maintainability
    /// index to the report.
    pub metrics: bool,
    /// Add a per-language file and line breakdown for the whole tree.
    pub languages: bool,
    /// Look for blocks of lines that were copied from somewhere else.
    pub duplicates: bool,
    /// Blame every business file and split its lines across the authors.
    pub authors: bool,
    /// Lines in a block before it counts as a copy.
    pub dup_lines: usize,
    /// `--fail-if` rules; any violation exits with code 1.
    pub fail_if: Vec<gate::Gate>,
    /// Print gate failures as GitHub annotations.
    pub ci: bool,
    pub format: Format,
    /// Print a README badge instead of a report.
    pub badge: Option<badge::Metric>,
    /// Caption to use instead of the metric name.
    pub badge_label: Option<String>,
    /// Repository config the run started from, when one was found.
    pub config: Option<PathBuf>,
    /// Write this snapshot of the business files after scanning.
    pub snapshot: Option<PathBuf>,
    /// Compare the scan with this previously written snapshot.
    pub baseline: Option<baseline::Snapshot>,
    /// Append this scan to a history file and chart the series.
    pub history: Option<PathBuf>,
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
            sources: sources::Sources::default(),
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
            smells: false,
            hotspots: false,
            metrics: false,
            languages: false,
            duplicates: false,
            authors: false,
            dup_lines: crate::clones::DEFAULT_WINDOW,
            fail_if: Vec::new(),
            ci: false,
            format: Format::Text,
            badge: None,
            badge_label: None,
            config: None,
            snapshot: None,
            baseline: None,
            history: None,
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
        if self.metrics {
            columns.push("cognitive");
        }
        if self.uloc {
            columns.push("uloc");
        }
        if self.smells {
            columns.push("smells");
        }
        columns
    }

    pub fn column_titles(&self) -> Vec<&'static str> {
        let mut titles = vec!["Lines", "Blank", "Comment", "Code", "Test"];
        if self.complexity {
            titles.push("Cplx");
        }
        if self.metrics {
            titles.push("Cog");
        }
        if self.uloc {
            titles.push("ULOC");
        }
        if self.smells {
            titles.push("Smells");
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
            // `--metrics` reports NOM and NARGS, so it needs the function list
            // even when `--functions` did not ask for the table.
            functions: self.functions || self.metrics || self.wants_function_table(),
            include_tests: self.include_tests,
            min_gen_line_length: self.min_gen_line_length,
        }
    }

    /// True when the function table has to be built: `--functions` shows it,
    /// and a gate on a function scoped counter reads it to point an
    /// annotation at the line of the worst function.
    pub fn wants_function_table(&self) -> bool {
        self.functions
            || self
                .fail_if
                .iter()
                .any(|rule| rule.metric.collects_functions())
    }
}

#[cfg(test)]
mod tests {
    use super::Format;

    #[test]
    fn every_format_but_the_card_carries_the_authors() {
        for format in [Format::Text, Format::Json, Format::Csv, Format::Markdown] {
            assert!(format.carries_authors(), "{format:?} drops the authors");
        }
        assert!(!Format::Svg.carries_authors());
    }
}
