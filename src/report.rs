//! The scan pipeline plus the text, JSON, CSV and Markdown renderers.

use std::path::{Path, PathBuf};

use crate::baseline;
use crate::clones;
use crate::cocomo;
use crate::git;
use crate::history;
use crate::metrics;
use crate::options::{Format, Options};
use crate::scan::{self, Entry, FileFacts, Progress, Totals};
use crate::sources::{Lang, Sources};

pub struct Excluded {
    pub files: usize,
    pub file_lines: u32,
    pub item_lines: u32,
    pub paths: Vec<(String, u32)>,
    pub generated_files: usize,
    pub generated_lines: u32,
}

/// One function in the ranked list, flattened across every scanned file.
pub struct FunctionRow {
    pub path: String,
    pub name: String,
    pub line: u32,
    pub code: u32,
    pub complexity: u32,
    pub cognitive: u32,
    pub params: u32,
    pub depth: u32,
}

/// The COCOMO estimate for the business scope.
pub struct CocomoRow {
    pub project: &'static str,
    pub params: cocomo::Params,
    pub estimate: cocomo::Estimate,
}

/// One file in the git hotspots ranking.
pub struct HotspotRow {
    pub path: String,
    pub code: u32,
    pub complexity: u32,
    pub commits: u64,
    pub churn: u64,
    pub score: u64,
}

/// Rows in the hotspot ranking unless `--top` says otherwise.
pub const DEFAULT_HOTSPOTS: usize = 15;

/// One language in the `--languages` breakdown.
pub struct LanguageRow {
    pub name: &'static str,
    pub files: u32,
    pub lines: u32,
    pub blank: u32,
    pub comment: u32,
    pub code: u32,
}

/// The maintainability numbers for the business scope, built by `--metrics`.
pub struct MetricsRow {
    /// Code-line weighted mean of the per-file index, so a big file counts
    /// more than a small one.
    pub index: f64,
    /// Sum of the per-file Halstead volumes.
    pub volume: f64,
    pub cognitive: u32,
    pub sloc: u32,
    pub operators: u32,
    pub operands: u32,
    /// `NOM`: functions in scope.
    pub methods: usize,
    /// `NARGS`: their parameters, added up.
    pub arguments: u64,
}

impl MetricsRow {
    pub fn tokens(&self) -> u32 {
        self.operators + self.operands
    }

    pub fn band(&self) -> &'static str {
        metrics::band(self.index)
    }
}

pub struct Report {
    pub roots: Vec<PathBuf>,
    pub options: Options,
    pub entries: Vec<Entry>,
    pub business: Vec<usize>,
    pub excluded_entries: Vec<usize>,
    pub summary: Totals,
    pub uloc_total: Option<usize>,
    pub all_lines: u32,
    pub excluded: Excluded,
    /// Business functions ranked by complexity; empty unless requested.
    pub functions: Vec<FunctionRow>,
    /// Every business function found before the ranking was truncated.
    pub function_count: usize,
    /// Cost estimate for the business code lines, when requested.
    pub cocomo: Option<CocomoRow>,
    /// Comparison with a saved snapshot, when --baseline asked for one.
    pub diff: Option<baseline::Diff>,
    /// Files ranked by complexity and git churn, when asked for.
    pub hotspots: Vec<HotspotRow>,
    /// Recorded scans, when `--history` asked for the series.
    pub history: Option<history::History>,
    /// Cognitive complexity, Halstead volume and the MI score, when asked for.
    pub metrics: Option<MetricsRow>,
    /// Every language in the scanned tree, largest first.
    pub languages: Vec<LanguageRow>,
    /// Blocks of copied lines, when `--duplicates` asked for them.
    pub duplicates: Option<clones::Dupes>,
    /// Author attribution from `git blame`, when `--authors` asked for it.
    pub authors: Option<crate::authors::Authors>,
}

impl Report {
    pub fn business_entries(&self) -> Vec<&Entry> {
        self.business
            .iter()
            .map(|index| &self.entries[*index])
            .collect()
    }

    /// What the author pass blames: the business files, ready to be handed to a
    /// thread of its own. The entries stay here, so a report can render its
    /// other panes while the blame runs.
    pub fn blame_targets(&self) -> Vec<crate::authors::Target> {
        self.business_entries()
            .into_iter()
            .map(crate::authors::Target::of)
            .collect()
    }

    /// Re-render this report in another format without scanning again.
    pub fn render_as(&self, format: Format, cwd: &Path) -> String {
        let mut options = self.options.clone();
        options.format = format;
        render_with(self, &options, cwd)
    }
}

/// Analyse every root and reduce it to the numbers all renderers share.
pub fn scan(
    roots: &[PathBuf],
    options: &Options,
    cwd: &Path,
    progress: Option<&Progress<'_>>,
) -> Report {
    let mut discovered: Vec<(PathBuf, Vec<PathBuf>)> = Vec::with_capacity(roots.len());
    let mut total = 0usize;
    for root in roots {
        let files = scan::apply_gitignore(
            root,
            scan::discover(root, options),
            options.gitignore,
            options.sources,
        );
        total += files.len();
        discovered.push((root.clone(), files));
    }

    let mut facts: Vec<FileFacts> = Vec::new();
    let mut done = 0usize;
    for (root, files) in &discovered {
        let offset = done;
        done += files.len();
        let forward: Option<Box<Progress<'_>>> = match progress {
            Some(callback) => Some(Box::new(move |stage, index, _total, path| {
                // Shown relative to the root, the way the tables name files.
                let shown = scan::relative(path, root);
                callback(stage, offset + index, total, Path::new(&shown))
            })),
            None => None,
        };
        facts.extend(scan::collect_files(
            root,
            files,
            options.read_options(),
            options.worker_count(),
            forward.as_deref(),
        ));
    }

    let siblings = scan::resolve_siblings(&facts);
    let entries: Vec<Entry> = facts
        .into_iter()
        .map(|facts| {
            let stats = scan::summarize(&facts, &siblings, options.include_tests);
            Entry { facts, stats }
        })
        .collect();

    let drop_generated =
        |entry: &Entry| options.exclude_generated && entry.stats.generated.is_some();
    let business: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| {
            (options.include_tests || !entry.stats.forced_test) && !drop_generated(entry)
        })
        .map(|(index, _)| index)
        .collect();
    let excluded_entries: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.stats.forced_test)
        .map(|(index, _)| index)
        .collect();

    let business_refs: Vec<&Entry> = business.iter().map(|index| &entries[*index]).collect();
    let excluded_refs: Vec<&Entry> = excluded_entries
        .iter()
        .map(|index| &entries[*index])
        .collect();
    let summary = scan::totals(&business_refs);
    let uloc_total = if options.uloc {
        scan::union_uloc(&business_refs)
    } else {
        None
    };
    let all_lines = entries.iter().map(|entry| entry.stats.physical).sum();
    let generated_refs: Vec<&Entry> = if options.exclude_generated {
        entries
            .iter()
            .filter(|entry| entry.stats.generated.is_some() && !entry.stats.forced_test)
            .collect()
    } else {
        Vec::new()
    };
    let generated_files = generated_refs.len();
    let generated_lines: u32 = generated_refs
        .iter()
        .map(|entry| entry.stats.physical)
        .sum();
    let mut paths: Vec<(String, u32)> = excluded_refs
        .iter()
        .map(|entry| (scan::relative(entry.path(), cwd), entry.stats.physical))
        .collect();
    paths.sort();
    let excluded = Excluded {
        files: excluded_refs.len(),
        file_lines: excluded_refs.iter().map(|entry| entry.stats.physical).sum(),
        item_lines: business_refs.iter().map(|entry| entry.stats.test).sum(),
        paths,
        generated_files,
        generated_lines,
    };

    let mut function_count = 0usize;
    let mut functions: Vec<FunctionRow> = Vec::new();
    if options.wants_function_table() {
        for entry in &business_refs {
            for function in &entry.facts.functions {
                if function.test && !options.include_tests {
                    continue;
                }
                function_count += 1;
                functions.push(FunctionRow {
                    path: scan::relative(entry.path(), cwd),
                    name: function.name.clone(),
                    line: function.line,
                    code: function.code,
                    complexity: function.complexity,
                    cognitive: function.cognitive,
                    params: function.params,
                    depth: function.depth,
                });
            }
        }
        functions.sort_by(|left, right| {
            right
                .complexity
                .cmp(&left.complexity)
                .then(right.code.cmp(&left.code))
                .then(left.path.cmp(&right.path))
                .then(left.line.cmp(&right.line))
        });
        if options.top_functions > 0 {
            functions.truncate(options.top_functions);
        }
    }

    let cocomo_row = if options.cocomo {
        Some(CocomoRow {
            project: options.cocomo_project.name(),
            params: options.cocomo_params,
            estimate: cocomo::estimate(summary.code, options.cocomo_params),
        })
    } else {
        None
    };

    let metrics_row = if options.metrics {
        Some(metrics_row(&business_refs, &summary, options))
    } else {
        None
    };

    let languages = if options.languages {
        language_rows(roots, options)
    } else {
        Vec::new()
    };

    let duplicates = if options.duplicates {
        let files: Vec<PathBuf> = business
            .iter()
            .map(|index| entries[*index].path().to_path_buf())
            .collect();
        Some(clones::detect(
            &files,
            cwd,
            options.dup_lines,
            options.include_tests,
            options.worker_count(),
        ))
    } else {
        None
    };

    let authors = if options.authors {
        Some(crate::authors::Authors::collect(
            &business_refs,
            options,
            progress,
        ))
    } else {
        None
    };

    // Ranked before the entries move into the report, because the ranking
    // borrows them through `business_refs`.
    let hotspots = if options.hotspots {
        rank_hotspots(&business_refs, roots, options, cwd)
    } else {
        Vec::new()
    };

    let mut report = Report {
        roots: roots.to_vec(),
        options: options.clone(),
        entries,
        business,
        excluded_entries,
        summary,
        uloc_total,
        all_lines,
        excluded,
        functions,
        function_count,
        cocomo: cocomo_row,
        diff: None,
        hotspots,
        history: None,
        metrics: metrics_row,
        languages,
        duplicates,
        authors,
    };
    report.diff = options
        .baseline
        .as_ref()
        .map(|snapshot| baseline::Diff::of(&report, snapshot, cwd));
    report
}

/// Every language in the tree, largest by code first. This reads files the
/// Rust pass never touches, so it only runs when the flag is given.
fn language_rows(roots: &[PathBuf], options: &Options) -> Vec<LanguageRow> {
    let mut files: Vec<PathBuf> = Vec::new();
    for root in roots {
        let found = scan::discover_languages(root, options);
        files.extend(scan::apply_gitignore_all(root, found, options.gitignore));
    }
    scan::language_totals(&files, options.worker_count())
        .into_iter()
        .map(|(name, tally)| LanguageRow {
            name,
            files: tally.files,
            lines: tally.counts.lines,
            blank: tally.counts.blank,
            comment: tally.counts.comment,
            code: tally.counts.code,
        })
        .collect()
}

/// Reduce the business scope to one maintainability card. The index is the
/// code-line weighted mean of the per-file scores, the way
/// `rust-code-analysis` reports a project figure, while the volume and the
/// token counts are plain sums (Halstead volume is additive, the distinct
/// counts behind it are not, so they stay per-file).
fn metrics_row(entries: &[&Entry], summary: &Totals, options: &Options) -> MetricsRow {
    let mut weighted = 0.0f64;
    let mut weight = 0u64;
    let mut operators = 0u32;
    let mut operands = 0u32;
    let mut methods = 0usize;
    let mut arguments = 0u64;
    for entry in entries {
        let stats = &entry.stats;
        operators += stats.halstead.operators;
        operands += stats.halstead.operands;
        let volume = stats.halstead.volume();
        let mut scored = 0u32;
        for function in &entry.facts.functions {
            if function.test && !options.include_tests {
                continue;
            }
            methods += 1;
            arguments += u64::from(function.params);
            if function.code == 0 || stats.code == 0 {
                continue;
            }
            // The formula is calibrated for units, not for thousand-line
            // files, so score each function and let the code lines weigh
            // them. Its Halstead volume is the file volume shared out by the
            // function's share of the code.
            let share = volume * f64::from(function.code) / f64::from(stats.code);
            weighted += metrics::maintainability_index(share, function.complexity, function.code)
                * f64::from(function.code);
            weight += u64::from(function.code);
            scored += function.code;
        }
        if scored == 0 && stats.code > 0 {
            // A file with no functions to score falls back to its own totals.
            weighted += metrics::maintainability_index(volume, stats.complexity, stats.code)
                * f64::from(stats.code);
            weight += u64::from(stats.code);
        }
    }
    let index = if weight > 0 {
        weighted / weight as f64
    } else {
        metrics::maintainability_index(summary.volume, summary.complexity, summary.code)
    };
    MetricsRow {
        index,
        volume: summary.volume,
        cognitive: summary.cognitive,
        sloc: summary.code,
        operators,
        operands,
        methods,
        arguments,
    }
}

/// Rank files by complexity times git churn, the way `scc --hotspots` does.
/// The first root that lives in a git repository decides which history to
/// use; files from other roots have no churn and simply drop out.
fn rank_hotspots(
    entries: &[&Entry],
    roots: &[PathBuf],
    options: &Options,
    cwd: &Path,
) -> Vec<HotspotRow> {
    let Some((toplevel, churn)) = roots.iter().find_map(|root| git::churn(root)) else {
        return Vec::new();
    };
    let mut rows: Vec<HotspotRow> = entries
        .iter()
        .filter_map(|entry| {
            let key = git::repo_relative(entry.path(), &toplevel)?;
            let change = churn.get(&key).copied().unwrap_or_default();
            Some(HotspotRow {
                path: scan::relative(entry.path(), cwd),
                code: entry.stats.code,
                complexity: entry.stats.complexity,
                commits: change.commits,
                churn: change.lines(),
                score: u64::from(entry.stats.complexity) * change.lines(),
            })
        })
        .collect();
    rows.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then(b.churn.cmp(&a.churn))
            .then(a.path.cmp(&b.path))
    });
    let limit = if options.top > 0 {
        options.top
    } else {
        DEFAULT_HOTSPOTS
    };
    rows.truncate(limit);
    rows
}

/// 1234567 -> 1,234,567, matching Python string formatting.
pub fn group<T: Into<u64>>(value: T) -> String {
    let digits = value.into().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    let bytes = digits.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if index > 0 && (bytes.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(*byte as char);
    }
    out
}

/// The heading a report puts over the scope it counted. Rust keeps the wording
/// every published report already used.
pub fn scope_title(sources: Sources, include_tests: bool) -> &'static str {
    match (sources, include_tests) {
        (Sources::Rust, false) => "Rust business code (tests excluded)",
        (Sources::Rust, true) => "Rust source lines",
        (Sources::Python, false) => "Python business code (tests excluded)",
        (Sources::Python, true) => "Python source lines",
        (Sources::All, false) => "Business code (tests excluded)",
        (Sources::All, true) => "Source lines",
    }
}

/// "Rust" while the scope is one language, and nothing when it is both, so the
/// block titles can name the language that owns their counters.
fn scope_word(sources: Sources) -> Option<&'static str> {
    match sources {
        Sources::Rust => Some("Rust"),
        Sources::Python => Some("Python"),
        Sources::All => None,
    }
}

/// What the language calls the item level test code it excludes.
fn test_item_phrase(sources: Sources) -> &'static str {
    match sources {
        Sources::Rust => "lines inside `#[cfg(test)]` items",
        Sources::Python => "lines inside `def test_` items",
        Sources::All => "lines inside test items",
    }
}

/// The caption over the "every line, tests included" figure.
fn all_lines_label(sources: Sources) -> &'static str {
    match sources {
        Sources::Rust => "All Rust lines",
        Sources::Python => "All Python lines",
        Sources::All => "All source lines",
    }
}

/// The row caption for the item-level test lines in the excluded block. Rust
/// keeps the wording published reports already use.
fn test_item_label(sources: Sources) -> &'static str {
    match sources {
        Sources::Rust => "#[cfg(test)] items",
        Sources::Python => "def test_ functions",
        Sources::All => "in-code tests",
    }
}

/// The smell row captions. Rust keeps the wording published reports already use;
/// the other languages name their own items.
fn smell_labels(sources: Sources) -> (&'static str, &'static str, &'static str) {
    match sources {
        Sources::Rust => ("todo!()", "unimplemented!()", "#[allow(..)]"),
        Sources::Python => ("TODO/FIXME", "NotImplementedError", "noqa/type:ignore"),
        Sources::All => ("todo/unimplemented", "TODO comment", "allow/ignore"),
    }
}

/// One-decimal percentage of a base.
pub fn share(value: usize, base: usize) -> String {
    if base == 0 {
        return "0.0%".to_string();
    }
    format!("{:.1}%", value as f64 / base as f64 * 100.0)
}

fn pad(text: &str, width: usize) -> String {
    format!("{:<width$}", text, width = width)
}

/// A file path for a listing, tagged when generated code is only marked.
fn file_label(entry: &Entry, options: &Options, cwd: &Path) -> String {
    let mut label = scan::relative(entry.path(), cwd);
    if options.mark_generated {
        label.push_str(entry.stats.generated.tag());
    }
    label
}

fn number(value: u32, width: usize) -> String {
    format!("{:>width$}", group(value), width = width)
}

/// Same as [`number`] for the counters that outgrew `u32`, like a Halstead
/// volume on a very large code base.
fn wide_number(value: u64, width: usize) -> String {
    format!("{:>width$}", group(value), width = width)
}

/// Whole dollars, grouped the way every other counter is.
fn money(value: f64) -> String {
    let dollars = group(value.max(0.0).round() as u64);
    let mut out = String::with_capacity(dollars.len() + 1);
    out.push('$');
    out.push_str(&dollars);
    out
}

fn table_row(label: &str, values: &[u32], width: usize) -> String {
    let mut row = pad(label, width);
    for value in values {
        row.push_str(&number(*value, 10));
    }
    row
}

/// Render a report in the requested format.
/// "+86", "-1,204" or "0", for a change against the baseline.
fn delta_plain(value: i64) -> String {
    if value == 0 {
        return "0".to_string();
    }
    format!(
        "{}{}",
        if value > 0 { "+" } else { "-" },
        group(value.unsigned_abs())
    )
}

/// `delta_plain` right aligned, for a report column.
fn delta_cell(value: i64) -> String {
    format!("{:>10}", delta_plain(value))
}

/// The biggest code moves, biggest first.
fn diff_movers(diff: &baseline::Diff, limit: usize) -> Vec<(String, i64)> {
    diff.files
        .iter()
        .take(limit)
        .map(|delta| (delta.path.clone(), delta.code_delta()))
        .collect()
}

/// The "what changed" block the text report prints for `--baseline`.
fn render_baseline_text(diff: &baseline::Diff, cwd: &Path, out: &mut Vec<String>) {
    out.push(format!(
        "Since baseline {} ({} added, {} deleted, {} changed)",
        scan::relative(&diff.path, cwd),
        group(diff.added() as u32),
        group(diff.deleted() as u32),
        group(diff.changed() as u32)
    ));
    for (label, before, after) in diff.counters() {
        out.push(
            pad(&format!("  {}", label), 22)
                + &number(after, 12)
                + "  "
                + &delta_cell(after as i64 - before as i64),
        );
    }
    let movers = diff_movers(diff, 5);
    if !movers.is_empty() {
        out.push("  Largest moves".to_string());
        for (path, delta) in movers {
            out.push(format!("    {:>10}  {}", delta_plain(delta), path));
        }
    }
    out.push(String::new());
}

/// The "how the tree grew" block the text report prints for `--history`.
fn render_history_text(history: &history::History, out: &mut Vec<String>) {
    let count = history.samples.len();
    out.push(format!(
        "Scan history - {} scan{} ({})",
        group(count as u32),
        if count == 1 { "" } else { "s" },
        history.span()
    ));
    for trend in history::Trend::all() {
        let series = history.series(*trend, history::SPARK_WIDTH);
        let latest = series.last().copied().unwrap_or(0);
        let before = if series.len() < 2 {
            latest
        } else {
            series[series.len() - 2]
        };
        out.push(
            pad(&format!("  {}", trend.as_str()), 24)
                + &history::sparkline(&series)
                + "  "
                + &number(latest, 10)
                + "  "
                + &delta_cell(latest as i64 - before as i64),
        );
    }
    out.push(String::new());
}

pub fn render(report: &Report, cwd: &Path) -> String {
    render_with(report, &report.options, cwd)
}

/// Render an existing report with a different set of options, so a caller can
/// re-export without scanning again.
pub fn render_with(report: &Report, options: &Options, cwd: &Path) -> String {
    match options.format {
        Format::Text => render_text(report, options, cwd),
        Format::Json => render_json(report, options, cwd),
        Format::Csv => render_csv(report, options, cwd),
        Format::Markdown => render_markdown(report, options, cwd),
        Format::Svg => crate::svg::render(report, options, cwd),
    }
}

fn render_text(report: &Report, options: &Options, cwd: &Path) -> String {
    let business = report.business_entries();
    let columns = options.column_keys();
    let titles = options.column_titles();
    let mut out: Vec<String> = Vec::new();

    if options.by_file {
        let mut ordered = scan::sort_entries(business.clone(), &options.sort);
        if options.top > 0 {
            ordered.truncate(options.top);
        }
        let labels: Vec<String> = ordered
            .iter()
            .map(|entry| file_label(entry, options, cwd))
            .collect();
        let width = label_width(&labels, "File");
        let mut head = pad("File", width);
        for title in &titles {
            head.push_str(&format!("{:>10}", title));
        }
        let diffs = report.diff.as_ref();
        if diffs.is_some() {
            head.push_str(&format!("{:>10}", "Delta"));
        }
        out.push(head);
        let mut rule = width + 10 * titles.len();
        if diffs.is_some() {
            rule += 10;
        }
        out.push("-".repeat(rule));
        for (label, entry) in labels.iter().zip(ordered.iter()) {
            let values: Vec<u32> = columns.iter().map(|key| entry.column(key)).collect();
            let mut row = table_row(label, &values, width);
            if let Some(diff) = diffs {
                let path = baseline::key(entry, &report.roots, cwd);
                row.push_str(&delta_cell(diff.code_delta(&path).unwrap_or(0)));
            }
            out.push(row);
        }
        out.push(String::new());
    }

    if options.by_module {
        let groups = scan::module_groups(&business, cwd);
        let labels: Vec<String> = groups.iter().map(|(label, _, _)| label.clone()).collect();
        let width = label_width(&labels, "Module");
        let mut head = pad("Module", width);
        for title in &titles {
            head.push_str(&format!("{:>10}", title));
        }
        out.push(head);
        out.push("-".repeat(width + 10 * titles.len()));
        for (label, stats, _) in groups.iter() {
            let values: Vec<u32> = columns.iter().map(|key| stats.column(key)).collect();
            out.push(table_row(label, &values, width));
        }
        out.push(String::new());
    }

    out.push(scope_title(options.sources, options.include_tests).to_string());
    let roots: Vec<String> = report
        .roots
        .iter()
        .map(|root| scan::relative(root, cwd))
        .collect();
    out.push("Roots: ".to_string() + &roots.join(", "));
    if let Some(path) = &options.config {
        out.push("Config: ".to_string() + &scan::relative(path, cwd));
    }
    if let Some(path) = &options.snapshot {
        out.push("Snapshot written: ".to_string() + &scan::relative(path, cwd));
    }
    if let Some(path) = &options.history {
        out.push("History written: ".to_string() + &scan::relative(path, cwd));
    }
    out.push(format!(
        "Scope: {}, {} ignored directory names",
        if options.gitignore {
            "gitignore-aware"
        } else {
            "no gitignore filtering"
        },
        options.ignore_dirs.len()
    ));
    if options.sources != Sources::Rust {
        out.push(format!("Sources: {}", options.sources.label()));
    }
    out.push(String::new());

    let summary = &report.summary;
    let lines = summary.lines as usize;
    out.push(pad("Files", 22) + &number(report.business.len() as u32, 12));
    out.push(pad("Lines", 22) + &number(summary.lines, 12));
    out.push(
        pad("  Blank", 22)
            + &number(summary.blank, 12)
            + "   "
            + &share(summary.blank as usize, lines),
    );
    let mut comment_line = pad("  Comment", 22)
        + &number(summary.comment, 12)
        + "   "
        + &share(summary.comment as usize, lines);
    if summary.doc > 0 {
        comment_line.push_str(&format!("   (doc {})", group(summary.doc)));
    }
    out.push(comment_line);
    out.push(
        pad("  Code", 22)
            + &number(summary.code, 12)
            + "   "
            + &share(summary.code as usize, lines),
    );
    out.push(String::new());
    out.push(match scope_word(options.sources) {
        Some(word) => format!("{} metrics", word),
        None => "Metrics".to_string(),
    });
    if options.complexity {
        out.push(pad("  Complexity (est.)", 22) + &number(summary.complexity, 12));
    }
    if options.sources.covers(Lang::Rust) {
        out.push(pad("  Unsafe", 22) + &number(summary.unsafe_kw, 12));
    }
    out.push(pad("  Test functions", 22) + &number(summary.test_fns, 12));
    if options.uloc {
        let uloc = report.uloc_total.unwrap_or(0);
        out.push(
            pad("  ULOC", 22)
                + &number(uloc as u32, 12)
                + "   "
                + &share(uloc, lines)
                + " of lines (DRYness)",
        );
    }
    out.push(String::new());

    if let Some(diff) = &report.diff {
        render_baseline_text(diff, cwd, &mut out);
    }

    if let Some(history) = &report.history {
        render_history_text(history, &mut out);
    }

    if options.smells {
        out.push(match scope_word(options.sources) {
            Some(word) => format!("{} smells", word),
            None => "Smells".to_string(),
        });
        if options.sources.covers(Lang::Rust) {
            out.push(pad("  unwrap()", 22) + &number(summary.smells.unwrap, 12));
            out.push(pad("  expect()", 22) + &number(summary.smells.expect, 12));
            out.push(pad("  panic!()", 22) + &number(summary.smells.panic, 12));
        }
        let (todo_label, unimpl_label, allow_label) = smell_labels(options.sources);
        out.push(pad(&format!("  {}", todo_label), 22) + &number(summary.smells.todo, 12));
        out.push(
            pad(&format!("  {}", unimpl_label), 22) + &number(summary.smells.unimplemented, 12),
        );
        out.push(pad(&format!("  {}", allow_label), 22) + &number(summary.smells.allow, 12));
        out.push(pad("  Total", 22) + &number(summary.smells.total(), 12));
        out.push(String::new());
    }

    if let Some(row) = &report.metrics {
        render_metrics_text(row, &mut out);
        out.push(String::new());
    }

    if let Some(row) = &report.cocomo {
        render_cocomo_text(row, &mut out);
        out.push(String::new());
    }

    if options.include_tests {
        out.push("Test code included (--include-tests)".to_string());
    } else {
        out.push("Excluded test code".to_string());
        out.push(format!(
            "  Test-only files          {:>6} file(s), {} lines",
            group(report.excluded.files as u32),
            group(report.excluded.file_lines)
        ));
        out.push(format!(
            "  {:<25}{:>6}          {} lines",
            test_item_label(options.sources),
            "",
            group(report.excluded.item_lines)
        ));
    }
    out.push(String::new());
    out.push(pad(all_lines_label(options.sources), 22) + &number(report.all_lines, 12));
    if options.exclude_generated {
        out.push(String::new());
        out.push("Excluded generated code".to_string());
        out.push(format!(
            "  Generated files          {:>6} file(s), {} lines",
            group(report.excluded.generated_files as u32),
            group(report.excluded.generated_lines)
        ));
    }
    if options.show_excluded && !report.excluded.paths.is_empty() {
        out.push(String::new());
        out.push("Excluded test-only files".to_string());
        for (path, count) in &report.excluded.paths {
            out.push(format!("  {} ({} lines)", path, group(*count)));
        }
    }

    if options.languages {
        out.push(String::new());
        render_languages_text(report, &mut out);
    }
    if options.functions {
        out.push(String::new());
        render_functions_text(report, options.metrics, &mut out);
    }
    if options.hotspots {
        out.push(String::new());
        render_hotspots_text(report, &mut out);
    }
    if options.duplicates {
        out.push(String::new());
        render_clones_text(report, &mut out);
    }
    if options.authors {
        out.push(String::new());
        render_authors_text(report, options, cwd, &mut out);
    }
    out.join("\n")
}

fn label_width(labels: &[String], title: &str) -> usize {
    let mut width = 44usize.max(title.chars().count());
    for label in labels {
        width = width.max(label.chars().count());
    }
    width
}

/// Cut a label to width characters, marking the loss with a trailing "..".
fn clip(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    if width <= 2 {
        return text.chars().take(width).collect();
    }
    let mut out: String = text.chars().take(width - 2).collect();
    out.push_str("..");
    out
}

fn column_width(values: &[String], title: &str, minimum: usize, maximum: usize) -> usize {
    let mut width = minimum.max(title.chars().count());
    for value in values {
        width = width.max(value.chars().count());
    }
    width.min(maximum)
}

/// Cognitive complexity, Halstead volume and the maintainability index.
fn render_metrics_text(row: &MetricsRow, out: &mut Vec<String>) {
    out.push("Maintainability".to_string());
    out.push(
        pad("  MI score", 22) + &format!("{:>12}", format!("{:.1}", row.index)) + "  " + row.band(),
    );
    out.push(pad("  Halstead volume", 22) + &wide_number(row.volume.round() as u64, 12));
    out.push(
        pad("  Tokens", 22)
            + &number(row.tokens(), 12)
            + &format!(
                "   ({} operators, {} operands)",
                group(row.operators),
                group(row.operands)
            ),
    );
    out.push(pad("  Cognitive complexity", 22) + &number(row.cognitive, 12));
    out.push(
        pad("  Methods / arguments", 22)
            + &format!(
                "{:>12}",
                format!("{} / {}", group(row.methods as u32), group(row.arguments))
            ),
    );
}

/// Who wrote the business lines, biggest share first.
fn render_authors_text(report: &Report, options: &Options, cwd: &Path, out: &mut Vec<String>) {
    let Some(authors) = &report.authors else {
        return;
    };
    out.push("Authors".to_string());
    if !authors.available {
        out.push("  not a git repository, so nothing can be blamed".to_string());
        return;
    }
    let shown = authors.shown(options.top);
    let total: u32 = authors.rows.iter().map(|row| row.lines.code).sum();
    out.push(format!(
        "  {} author(s), {} code lines across {} blamed file(s)",
        group(authors.rows.len() as u32),
        group(total),
        group(authors.files.len() as u32)
    ));
    if authors.untracked > 0 {
        out.push(format!(
            "  {} file(s) are not in git, so their lines are not counted",
            group(authors.untracked as u32)
        ));
    }
    if shown.is_empty() {
        return;
    }
    out.push(String::new());
    let names: Vec<String> = shown.iter().map(|row| row.name.clone()).collect();
    let width = column_width(&names, "Author", 12, 30);
    let mut head = pad("Author", width);
    head.push_str(&format!(
        "{:>8}{:>11}{:>12}{:>10}{:>9}{:>8}",
        "Files", "Lines", "Code", "Comment", "Blank", "Share"
    ));
    out.push(head);
    out.push("-".repeat(width + 58));
    for row in shown {
        let mut line = pad(&clip(&row.name, width), width);
        line.push_str(&format!(
            "{:>8}{:>11}{:>12}{:>10}{:>9}{:>8}",
            group(row.files as u32),
            group(row.lines.business()),
            group(row.lines.code),
            group(row.lines.comment),
            group(row.lines.blank),
            share(row.lines.code as usize, total as usize)
        ));
        out.push(line);
    }
    if shown.len() < authors.rows.len() {
        out.push(format!(
            "  showing {} of {} author(s), use --top to show more",
            group(shown.len() as u32),
            group(authors.rows.len() as u32)
        ));
    }
    if !options.by_file {
        return;
    }
    let mut ordered = scan::sort_entries(report.business_entries(), &options.sort);
    if options.top > 0 {
        ordered.truncate(options.top);
    }
    let detailed: Vec<&Entry> = ordered
        .into_iter()
        .filter(|entry| {
            authors
                .file(entry.path())
                .is_some_and(|file| !file.authors.is_empty())
        })
        .collect();
    if detailed.is_empty() {
        return;
    }
    out.push(String::new());
    out.push("Per-file authors".to_string());
    let mut head = pad("  File / author", width + 2);
    head.push_str(&format!(
        "{:>11}{:>12}{:>10}{:>9}",
        "Lines", "Code", "Comment", "Blank"
    ));
    out.push(head);
    for entry in detailed {
        let Some(file) = authors.file(entry.path()) else {
            continue;
        };
        out.push("  ".to_string() + &scan::relative(entry.path(), cwd));
        for item in &file.authors {
            let mut line = pad(&format!("    {}", clip(&item.name, width)), width + 2);
            line.push_str(&format!(
                "{:>11}{:>12}{:>10}{:>9}",
                group(item.lines.business()),
                group(item.lines.code),
                group(item.lines.comment),
                group(item.lines.blank)
            ));
            out.push(line);
        }
    }
}

/// Every language in the tree, biggest by code first.
fn render_languages_text(report: &Report, out: &mut Vec<String>) {
    let rows = &report.languages;
    let total: u32 = rows.iter().map(|row| row.code).sum();
    out.push("Languages".to_string());
    out.push(format!(
        "  {} language(s), {} code lines in {} file(s)",
        group(rows.len() as u32),
        group(total),
        group(rows.iter().map(|row| row.files).sum::<u32>())
    ));
    if rows.is_empty() {
        return;
    }
    out.push(String::new());
    let names: Vec<String> = rows.iter().map(|row| row.name.to_string()).collect();
    let name_width = column_width(&names, "Language", 12, 24);
    let mut head = pad("Language", name_width);
    head.push_str(&format!(
        "{:>8}{:>10}{:>10}{:>10}{:>11}{:>8}",
        "Files", "Lines", "Blank", "Comment", "Code", "Share"
    ));
    out.push(head);
    out.push("-".repeat(name_width + 57));
    for (row, name) in rows.iter().zip(names.iter()) {
        let mut line = pad(&clip(name, name_width), name_width);
        line.push_str(&format!(
            "{:>8}{:>10}{:>10}{:>10}{:>11}{:>8}",
            group(row.files),
            group(row.lines),
            group(row.blank),
            group(row.comment),
            group(row.code),
            share(row.code as usize, total as usize)
        ));
        out.push(line);
    }
}

/// The worst functions by complexity, one row per function.
fn render_functions_text(report: &Report, cog: bool, out: &mut Vec<String>) {
    let rows = &report.functions;
    out.push("Functions".to_string());
    let mut scope = format!(
        "  {} function(s) in scope, ranked by complexity",
        group(report.function_count as u32)
    );
    if rows.len() < report.function_count {
        scope.push_str(&format!(", showing {}", group(rows.len() as u32)));
    }
    out.push(scope);
    if rows.is_empty() {
        return;
    }
    out.push(String::new());
    let names: Vec<String> = rows.iter().map(|row| row.name.clone()).collect();
    let locations: Vec<String> = rows
        .iter()
        .map(|row| format!("{}:{}", row.path, row.line))
        .collect();
    let name_width = column_width(&names, "Function", 20, 40);
    let location_width = column_width(&locations, "Location", 24, 64);
    let mut head = pad("Function", name_width);
    head.push_str(&pad("Location", location_width));
    head.push_str(&format!("{:>8}{:>8}", "Code", "Cplx"));
    if cog {
        head.push_str(&format!("{:>7}", "Cog"));
    }
    head.push_str(&format!("{:>7}{:>7}", "Args", "Depth"));
    out.push(head);
    out.push("-".repeat(name_width + location_width + if cog { 37 } else { 30 }));
    for (row, name) in rows.iter().zip(names.iter()) {
        let mut line = pad(&clip(name, name_width), name_width);
        line.push_str(&pad(
            &clip(&format!("{}:{}", row.path, row.line), location_width),
            location_width,
        ));
        line.push_str(&format!(
            "{:>8}{:>8}",
            group(row.code),
            group(row.complexity)
        ));
        if cog {
            line.push_str(&format!("{:>7}", group(row.cognitive)));
        }
        line.push_str(&format!("{:>7}{:>7}", row.params, row.depth));
        out.push(line);
    }
}

/// Files that are both complex and often changed, worst first.
fn render_hotspots_text(report: &Report, out: &mut Vec<String>) {
    let rows = &report.hotspots;
    out.push("Git hotspots".to_string());
    out.push(format!(
        "  {} file(s) ranked by complexity x churn",
        group(rows.len() as u32)
    ));
    if rows.is_empty() {
        out.push("  no git history for these files".to_string());
        return;
    }
    out.push(String::new());
    let paths: Vec<String> = rows.iter().map(|row| row.path.clone()).collect();
    let path_width = column_width(&paths, "File", 24, 64);
    let mut head = pad("File", path_width);
    head.push_str(&format!(
        "{:>8}{:>8}{:>9}{:>12}",
        "Code", "Cplx", "Commits", "Churn"
    ));
    out.push(head);
    out.push("-".repeat(path_width + 37));
    for (row, path) in rows.iter().zip(paths.iter()) {
        let mut line = pad(&clip(path, path_width), path_width);
        line.push_str(&format!(
            "{:>8}{:>8}{:>9}{:>12}",
            group(row.code),
            group(row.complexity),
            group(row.commits),
            group(row.churn)
        ));
        out.push(line);
    }
}

/// Blocks of lines that appear more than once in the tree.
fn render_clones_text(report: &Report, out: &mut Vec<String>) {
    let Some(dupes) = &report.duplicates else {
        return;
    };
    out.push("Duplicated code".to_string());
    out.push(format!(
        "  {} copy(ies) of {} lines or more, {} of {} normalised lines",
        group(dupes.clusters.len() as u32),
        group(dupes.window as u32),
        share(dupes.duplicated_lines as usize, dupes.code_lines as usize),
        group(dupes.code_lines)
    ));
    if dupes.clusters.is_empty() {
        return;
    }
    out.push(String::new());
    for (index, cluster) in dupes.clusters.iter().enumerate() {
        out.push(format!(
            "  {}. {} lines, {} site(s)",
            index + 1,
            cluster.lines,
            cluster.sites.len()
        ));
        for site in cluster.sites.iter().take(clones::SITES_SHOWN) {
            out.push(format!("     {}:{}", site.path, site.line));
        }
        if cluster.sites.len() > clones::SITES_SHOWN {
            out.push(format!(
                "     ... and {} more",
                cluster.sites.len() - clones::SITES_SHOWN
            ));
        }
    }
}

fn render_markdown(report: &Report, options: &Options, cwd: &Path) -> String {
    let business = report.business_entries();
    let columns = options.column_keys();
    let titles = options.column_titles();
    let title = scope_title(options.sources, options.include_tests);
    let mut out: Vec<String> = vec![format!("### {}", title), String::new()];
    let roots: Vec<String> = report
        .roots
        .iter()
        .map(|root| format!("`{}`", scan::relative(root, cwd)))
        .collect();
    out.push("Roots: ".to_string() + &roots.join(", "));
    out.push(String::new());
    out.push("| Metric | Value |".to_string());
    out.push("| --- | ---: |".to_string());
    let summary = &report.summary;
    let mut rows: Vec<(&str, u32)> = vec![
        ("Files", report.business.len() as u32),
        ("Lines", summary.lines),
        ("Blank", summary.blank),
        ("Comment", summary.comment),
        ("Code", summary.code),
    ];
    if options.complexity {
        rows.push(("Complexity (est.)", summary.complexity));
    }
    rows.push(("Unsafe", summary.unsafe_kw));
    rows.push(("Test functions", summary.test_fns));
    if options.uloc {
        rows.push(("ULOC", report.uloc_total.unwrap_or(0) as u32));
    }
    if options.smells {
        rows.push(("unwrap()", summary.smells.unwrap));
        rows.push(("expect()", summary.smells.expect));
        rows.push(("panic!()", summary.smells.panic));
        rows.push(("todo!()", summary.smells.todo));
        rows.push(("unimplemented!()", summary.smells.unimplemented));
        rows.push(("#[allow]", summary.smells.allow));
        rows.push(("Smells (total)", summary.smells.total()));
    }
    for (name, value) in rows {
        out.push(format!("| {} | {} |", name, group(value)));
    }
    out.push(String::new());
    if let Some(diff) = &report.diff {
        out.push(format!(
            "### Since baseline `{}`",
            scan::relative(&diff.path, cwd)
        ));
        out.push(String::new());
        out.push("| Metric | Before | After | Change |".to_string());
        out.push("| --- | ---: | ---: | ---: |".to_string());
        for (label, before, after) in diff.counters() {
            out.push(format!(
                "| {} | {} | {} | {} |",
                label,
                group(before),
                group(after),
                delta_plain(after as i64 - before as i64)
            ));
        }
        out.push(format!(
            "| Files | | {} | {} added, {} deleted, {} changed |",
            report.business.len(),
            diff.added(),
            diff.deleted(),
            diff.changed()
        ));
        out.push(String::new());
    }
    if let Some(history) = &report.history {
        out.push(format!("### Scan history ({})", history.span()));
        out.push(String::new());
        out.push("| Metric | Latest | Change | Trend |".to_string());
        out.push("| --- | ---: | ---: | :--- |".to_string());
        for trend in history::Trend::all() {
            let series = history.series(*trend, history::SPARK_WIDTH);
            let latest = series.last().copied().unwrap_or(0);
            let before = if series.len() < 2 {
                latest
            } else {
                series[series.len() - 2]
            };
            out.push(format!(
                "| {} | {} | {} | {} |",
                trend.label(),
                group(latest),
                delta_plain(latest as i64 - before as i64),
                history::sparkline(&series)
            ));
        }
        out.push(String::new());
    }
    if let Some(row) = &report.metrics {
        out.push("### Maintainability".to_string());
        out.push(String::new());
        out.push("| Metric | Value |".to_string());
        out.push("| --- | ---: |".to_string());
        out.push(format!("| MI score | {:.1} ({}) |", row.index, row.band()));
        out.push(format!(
            "| Halstead volume | {} |",
            group(row.volume.round() as u64)
        ));
        out.push(format!("| Tokens | {} |", group(row.tokens())));
        out.push(format!(
            "| Cognitive complexity | {} |",
            group(row.cognitive)
        ));
        out.push(format!(
            "| Methods / arguments | {} / {} |",
            group(row.methods as u32),
            group(row.arguments)
        ));
        out.push(String::new());
    }
    if let Some(row) = &report.cocomo {
        out.push(format!("### COCOMO estimate ({})", row.project));
        out.push(String::new());
        out.push("| Metric | Value |".to_string());
        out.push("| --- | ---: |".to_string());
        out.push(format!(
            "| Cost to develop | {} |",
            money(row.estimate.cost)
        ));
        out.push(format!(
            "| Schedule effort | {:.2} months |",
            row.estimate.schedule
        ));
        out.push(format!("| People required | {:.2} |", row.estimate.people));
        out.push(String::new());
    }
    if !options.include_tests {
        out.push(format!(
            "Excluded tests: {} test-only files ({} lines), {} {}. {}: {}.",
            group(report.excluded.files as u32),
            group(report.excluded.file_lines),
            group(report.excluded.item_lines),
            test_item_phrase(options.sources),
            all_lines_label(options.sources),
            group(report.all_lines)
        ));
    }
    out.push(String::new());
    if options.by_file {
        let mut ordered = scan::sort_entries(business.clone(), &options.sort);
        if options.top > 0 {
            ordered.truncate(options.top);
        }
        let diffs = report.diff.as_ref();
        let mut head = titles.join(" | ");
        let mut rule: Vec<&str> = vec!["---:"; titles.len()];
        if diffs.is_some() {
            head.push_str(" | Delta");
            rule.push("---:");
        }
        out.push(format!("| File | {} |", head));
        out.push(format!("| --- | {} |", rule.join(" | ")));
        for entry in ordered {
            let values: Vec<String> = columns.iter().map(|key| group(entry.column(key))).collect();
            let mut row = format!(
                "| `{}` | {}",
                file_label(entry, options, cwd),
                values.join(" | ")
            );
            match diffs {
                Some(diff) => {
                    let path = baseline::key(entry, &report.roots, cwd);
                    row.push_str(&format!(
                        " | {} |",
                        delta_plain(diff.code_delta(&path).unwrap_or(0))
                    ));
                }
                None => row.push_str(" |"),
            }
            out.push(row);
        }
        out.push(String::new());
    }
    if options.by_module {
        out.push(format!("| Module | {} |", titles.join(" | ")));
        out.push(format!(
            "| --- | {} |",
            vec!["---:"; titles.len()].join(" | ")
        ));
        for (label, stats, _) in scan::module_groups(&business, cwd) {
            let values: Vec<String> = columns.iter().map(|key| group(stats.column(key))).collect();
            out.push(format!("| `{}` | {} |", label, values.join(" | ")));
        }
        out.push(String::new());
    }
    if options.languages {
        out.push("### Languages".to_string());
        out.push(String::new());
        let total: u32 = report.languages.iter().map(|row| row.code).sum();
        out.push(format!(
            "{} language(s), {} code lines",
            group(report.languages.len() as u32),
            group(total)
        ));
        out.push(String::new());
        out.push("| Language | Files | Lines | Blank | Comment | Code | Share |".to_string());
        out.push("| --- | ---: | ---: | ---: | ---: | ---: | ---: |".to_string());
        for row in &report.languages {
            out.push(format!(
                "| {} | {} | {} | {} | {} | {} | {} |",
                row.name,
                group(row.files),
                group(row.lines),
                group(row.blank),
                group(row.comment),
                group(row.code),
                share(row.code as usize, total as usize)
            ));
        }
        out.push(String::new());
    }
    if let Some(authors) = &report.authors {
        out.push("### Authors".to_string());
        out.push(String::new());
        if !authors.available {
            out.push("Not a git repository, so nothing can be blamed.".to_string());
            out.push(String::new());
        } else {
            let total: u32 = authors.rows.iter().map(|row| row.lines.code).sum();
            out.push(format!(
                "{} author(s), {} code lines across {} blamed file(s)",
                group(authors.rows.len() as u32),
                group(total),
                group(authors.files.len() as u32)
            ));
            out.push(String::new());
            out.push("| Author | Files | Lines | Code | Comment | Blank | Share |".to_string());
            out.push("| --- | ---: | ---: | ---: | ---: | ---: | ---: |".to_string());
            for row in authors.shown(options.top) {
                out.push(format!(
                    "| {} | {} | {} | {} | {} | {} | {} |",
                    row.name,
                    group(row.files as u32),
                    group(row.lines.business()),
                    group(row.lines.code),
                    group(row.lines.comment),
                    group(row.lines.blank),
                    share(row.lines.code as usize, total as usize)
                ));
            }
            out.push(String::new());
            if options.by_file {
                let mut ordered = scan::sort_entries(report.business_entries(), &options.sort);
                if options.top > 0 {
                    ordered.truncate(options.top);
                }
                for entry in ordered {
                    let Some(file) = authors.file(entry.path()) else {
                        continue;
                    };
                    if file.authors.is_empty() {
                        continue;
                    }
                    out.push(format!("`{}`", scan::relative(entry.path(), cwd)));
                    out.push(String::new());
                    for item in &file.authors {
                        out.push(format!(
                            "- {}: {} lines ({} code, {} comment, {} blank)",
                            item.name,
                            group(item.lines.business()),
                            group(item.lines.code),
                            group(item.lines.comment),
                            group(item.lines.blank)
                        ));
                    }
                    out.push(String::new());
                }
            }
        }
    }
    if options.functions {
        out.push("### Worst functions by complexity".to_string());
        out.push(String::new());
        out.push(format!(
            "{} function(s) in scope, showing {}",
            group(report.function_count as u32),
            group(report.functions.len() as u32)
        ));
        out.push(String::new());
        let cog = if options.metrics { " Cog |" } else { "" };
        let cog_rule = if options.metrics { " ---: |" } else { "" };
        out.push(format!(
            "| Function | Location | Code | Cplx |{cog} Args | Depth |"
        ));
        out.push(format!(
            "| --- | --- | ---: | ---: |{cog_rule} ---: | ---: |"
        ));
        for row in &report.functions {
            let cog = if options.metrics {
                format!(" {} |", row.cognitive)
            } else {
                String::new()
            };
            out.push(format!(
                "| `{}` | `{}:{}` | {} | {} |{cog} {} | {} |",
                row.name, row.path, row.line, row.code, row.complexity, row.params, row.depth
            ));
        }
        out.push(String::new());
    }
    if options.hotspots {
        out.push("### Git hotspots".to_string());
        out.push(String::new());
        out.push(format!(
            "{} file(s) ranked by complexity x churn",
            group(report.hotspots.len() as u32)
        ));
        out.push(String::new());
        out.push("| File | Code | Cplx | Commits | Churn |".to_string());
        out.push("| --- | ---: | ---: | ---: | ---: |".to_string());
        for row in &report.hotspots {
            out.push(format!(
                "| `{}` | {} | {} | {} | {} |",
                row.path, row.code, row.complexity, row.commits, row.churn
            ));
        }
        out.push(String::new());
    }
    if let Some(dupes) = &report.duplicates {
        out.push("### Duplicated code".to_string());
        out.push(String::new());
        out.push(format!(
            "{} copy(ies) of {} lines or more, {} of {} normalised lines",
            group(dupes.clusters.len() as u32),
            group(dupes.window as u32),
            share(dupes.duplicated_lines as usize, dupes.code_lines as usize),
            group(dupes.code_lines)
        ));
        out.push(String::new());
        if !dupes.clusters.is_empty() {
            out.push("| # | Lines | Sites | Locations |".to_string());
            out.push("| ---: | ---: | ---: | --- |".to_string());
            for (index, cluster) in dupes.clusters.iter().enumerate() {
                let sites: Vec<String> = cluster
                    .sites
                    .iter()
                    .take(clones::SITES_SHOWN)
                    .map(|site| format!("`{}:{}`", site.path, site.line))
                    .collect();
                let mut listed = sites.join(", ");
                if cluster.sites.len() > clones::SITES_SHOWN {
                    listed.push_str(&format!(
                        " (+{} more)",
                        cluster.sites.len() - clones::SITES_SHOWN
                    ));
                }
                out.push(format!(
                    "| {} | {} | {} | {} |",
                    index + 1,
                    cluster.lines,
                    cluster.sites.len(),
                    listed
                ));
            }
            out.push(String::new());
        }
    }
    out.join("\n")
}

fn csv_field(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') || value.contains('\r') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn render_csv(report: &Report, options: &Options, cwd: &Path) -> String {
    let business = report.business_entries();
    let mut columns: Vec<&str> = vec!["lines", "blank", "comment", "doc", "code", "test"];
    if options.complexity {
        columns.push("complexity");
    }
    if options.metrics {
        columns.push("cognitive");
    }
    columns.push("unsafe");
    columns.push("test_fns");
    if options.uloc {
        columns.push("uloc");
    }
    if options.smells {
        columns.push("unwrap");
        columns.push("expect");
        columns.push("panic");
        columns.push("todo");
        columns.push("unimplemented");
        columns.push("allow");
        columns.push("smells");
    }
    let mut out: Vec<String> = Vec::new();
    let mut header: Vec<String> = vec!["path".to_string()];
    header.extend(columns.iter().map(|key| key.to_string()));
    if report.diff.is_some() {
        header.push("code_delta".to_string());
    }
    out.push(header.join(","));
    if options.by_file {
        let mut ordered = scan::sort_entries(business.clone(), &options.sort);
        if options.top > 0 {
            ordered.truncate(options.top);
        }
        for entry in ordered {
            let mut row: Vec<String> = vec![csv_field(&scan::relative(entry.path(), cwd))];
            row.extend(columns.iter().map(|key| entry.column(key).to_string()));
            if let Some(diff) = &report.diff {
                let path = baseline::key(entry, &report.roots, cwd);
                row.push(diff.code_delta(&path).unwrap_or(0).to_string());
            }
            out.push(row.join(","));
        }
    } else {
        let stats = scan::totals(&business);
        let uloc = scan::union_uloc(&business).unwrap_or(0);
        let mut row: Vec<String> = vec!["TOTAL".to_string()];
        row.extend(columns.iter().map(|key| {
            if *key == "uloc" {
                uloc.to_string()
            } else {
                stats.column(key).to_string()
            }
        }));
        if let Some(diff) = &report.diff {
            row.push((diff.new.code as i64 - diff.old.code as i64).to_string());
        }
        out.push(row.join(","));
    }
    out.join("\n") + "\n"
}

fn json_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            ch if (ch as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out
}

fn json_string(value: &str) -> String {
    format!("\"{}\"", json_escape(value))
}

fn json_number(value: f64) -> String {
    format!("{:.1}", (value * 10.0).round() / 10.0)
}

fn optional_number<T: std::fmt::Display>(value: Option<T>) -> String {
    match value {
        Some(number) => number.to_string(),
        None => "null".to_string(),
    }
}

/// Render a JSON array the way Python's `json.dumps(..., indent=2)` does:
/// inline when empty, one element per line otherwise.
fn json_array(items: &[String], indent: usize) -> String {
    if items.is_empty() {
        return "[]".to_string();
    }
    let item_pad = " ".repeat(indent + 2);
    let body: Vec<String> = items
        .iter()
        .map(|item| format!("{}{}", item_pad, item))
        .collect();
    format!("[\n{}\n{}]", body.join(",\n"), " ".repeat(indent))
}

/// The history block of the JSON report: the series and its sparklines.
fn json_history(history: &history::History, cwd: &Path, width: usize) -> String {
    let series: Vec<String> = history::Trend::all()
        .iter()
        .map(|trend| {
            let values: Vec<String> = history
                .series(*trend, width)
                .iter()
                .map(|value| value.to_string())
                .collect();
            format!("      \"{}\": {}", trend.as_str(), json_array(&values, 6))
        })
        .collect();
    let sparks: Vec<String> = history::Trend::all()
        .iter()
        .map(|trend| {
            format!(
                "      \"{}\": {}",
                trend.as_str(),
                json_string(&history::sparkline(&history.series(*trend, width)))
            )
        })
        .collect();
    let mut out = String::new();
    out.push_str("  \"history\": {\n");
    out.push_str(&format!(
        "    \"path\": {},\n",
        json_string(&scan::relative(&history.path, cwd))
    ));
    out.push_str(&format!("    \"samples\": {},\n", history.samples.len()));
    out.push_str(&format!(
        "    \"span\": {},\n",
        json_string(&history.span())
    ));
    out.push_str("    \"series\": {\n");
    out.push_str(&series.join(",\n"));
    out.push_str("\n    },\n");
    out.push_str("    \"sparkline\": {\n");
    out.push_str(&sparks.join(",\n"));
    out.push_str("\n    }\n");
    out.push_str("  },\n");
    out
}

fn render_json(report: &Report, options: &Options, cwd: &Path) -> String {
    let roots: Vec<String> = report
        .roots
        .iter()
        .map(|root| json_string(&scan::relative(root, cwd)))
        .collect();
    let ignore_dirs: Vec<String> = options
        .ignore_dirs
        .iter()
        .map(|name| json_string(name))
        .collect();
    let exclude: Vec<String> = options
        .exclude
        .iter()
        .map(|item| json_string(item))
        .collect();
    let summary = &report.summary;
    let dryness = if options.uloc && summary.lines > 0 {
        Some(json_number(
            report.uloc_total.unwrap_or(0) as f64 / summary.lines as f64 * 100.0,
        ))
    } else {
        None
    };

    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(&format!("  \"roots\": {},\n", json_array(&roots, 2)));
    out.push_str(&format!(
        "  \"config\": {},\n",
        match &options.config {
            Some(path) => json_string(&scan::relative(path, cwd)),
            None => "null".to_string(),
        }
    ));
    out.push_str(&format!(
        "  \"snapshot\": {},\n",
        match &options.snapshot {
            Some(path) => json_string(&scan::relative(path, cwd)),
            None => "null".to_string(),
        }
    ));
    out.push_str("  \"scope\": {\n");
    out.push_str(&format!("    \"gitignore\": {},\n", options.gitignore));
    out.push_str(&format!(
        "    \"sources\": {},\n",
        json_string(options.sources.slug())
    ));
    out.push_str(&format!(
        "    \"ignore_dirs\": {},\n",
        json_array(&ignore_dirs, 4)
    ));
    out.push_str(&format!("    \"exclude\": {},\n", json_array(&exclude, 4)));
    out.push_str(&format!(
        "    \"include_tests\": {}\n",
        options.include_tests
    ));
    out.push_str("  },\n");
    out.push_str("  \"summary\": {\n");
    out.push_str(&format!("    \"files\": {},\n", report.business.len()));
    out.push_str(&format!("    \"lines\": {},\n", summary.lines));
    out.push_str(&format!("    \"blank\": {},\n", summary.blank));
    out.push_str(&format!("    \"comment\": {},\n", summary.comment));
    out.push_str(&format!("    \"doc\": {},\n", summary.doc));
    out.push_str(&format!("    \"code\": {},\n", summary.code));
    out.push_str(&format!("    \"test\": {},\n", summary.test));
    out.push_str(&format!("    \"physical\": {},\n", summary.physical));
    out.push_str(&format!("    \"complexity\": {},\n", summary.complexity));
    out.push_str(&format!("    \"unsafe\": {},\n", summary.unsafe_kw));
    out.push_str(&format!("    \"test_fns\": {},\n", summary.test_fns));
    if options.smells {
        out.push_str(&format!(
            "    \"smells\": {{\"unwrap\": {}, \"expect\": {}, \"panic\": {}, \"todo\": {}, \"unimplemented\": {}, \"allow\": {}, \"total\": {}}},\n",
            summary.smells.unwrap,
            summary.smells.expect,
            summary.smells.panic,
            summary.smells.todo,
            summary.smells.unimplemented,
            summary.smells.allow,
            summary.smells.total()
        ));
    }
    out.push_str(&format!(
        "    \"uloc\": {},\n",
        optional_number(report.uloc_total)
    ));
    out.push_str(&format!(
        "    \"dryness_percent\": {}\n",
        match dryness {
            Some(value) => value,
            None => "null".to_string(),
        }
    ));
    out.push_str("  },\n");
    if let Some(row) = &report.metrics {
        out.push_str("  \"maintainability\": {\n");
        out.push_str(&format!("    \"mi\": {},\n", json_number(row.index)));
        out.push_str(&format!("    \"band\": {},\n", json_string(row.band())));
        out.push_str(&format!(
            "    \"halstead_volume\": {},\n",
            json_number(row.volume)
        ));
        out.push_str(&format!("    \"operators\": {},\n", row.operators));
        out.push_str(&format!("    \"operands\": {},\n", row.operands));
        out.push_str(&format!("    \"cognitive\": {},\n", row.cognitive));
        out.push_str(&format!("    \"sloc\": {},\n", row.sloc));
        out.push_str(&format!("    \"methods\": {},\n", row.methods));
        out.push_str(&format!("    \"arguments\": {}\n", row.arguments));
        out.push_str("  },\n");
    }
    if let Some(row) = &report.cocomo {
        out.push_str("  \"cocomo\": {\n");
        out.push_str(&format!("    \"project\": {},\n", json_string(row.project)));
        out.push_str(&format!("    \"a\": {},\n", json_number(row.params.a)));
        out.push_str(&format!("    \"b\": {},\n", json_number(row.params.b)));
        out.push_str(&format!("    \"c\": {},\n", json_number(row.params.c)));
        out.push_str(&format!("    \"d\": {},\n", json_number(row.params.d)));
        out.push_str(&format!(
            "    \"effort_person_months\": {},\n",
            json_number(row.estimate.effort)
        ));
        out.push_str(&format!(
            "    \"schedule_months\": {},\n",
            json_number(row.estimate.schedule)
        ));
        out.push_str(&format!(
            "    \"people\": {},\n",
            json_number(row.estimate.people)
        ));
        out.push_str(&format!(
            "    \"cost_usd\": {}\n",
            json_number(row.estimate.cost)
        ));
        out.push_str("  },\n");
    }
    if options.include_tests {
        out.push_str("  \"excluded\": null,\n");
    } else {
        out.push_str(&format!(
            "  \"excluded\": {{\n    \"files\": {},\n    \"file_lines\": {},\n    \"item_lines\": {},\n    \"paths\": null\n  }},\n",
            report.excluded.files, report.excluded.file_lines, report.excluded.item_lines
        ));
    }
    if options.exclude_generated {
        out.push_str(&format!(
            "  \"generated_excluded\": {{\"files\": {}, \"lines\": {}}},\n",
            report.excluded.generated_files, report.excluded.generated_lines
        ));
    }
    out.push_str(&format!("  \"all_rust_lines\": {},\n", report.all_lines));
    if options.functions {
        out.push_str(&format!(
            "  \"functions_total\": {},\n",
            report.function_count
        ));
        let mut items: Vec<String> = Vec::with_capacity(report.functions.len());
        for row in &report.functions {
            items.push(format!(
                "{{\"name\": {}, \"path\": {}, \"line\": {}, \"code\": {}, \"complexity\": {}, \"cognitive\": {}, \"params\": {}, \"depth\": {}}}",
                json_string(&row.name),
                json_string(&row.path),
                row.line,
                row.code,
                row.complexity,
                row.cognitive,
                row.params,
                row.depth
            ));
        }
        out.push_str(&format!("  \"functions\": {},\n", json_array(&items, 2)));
    }
    if options.hotspots {
        let mut items: Vec<String> = Vec::with_capacity(report.hotspots.len());
        for row in &report.hotspots {
            items.push(format!(
                "{{\"path\": {}, \"code\": {}, \"complexity\": {}, \"commits\": {}, \"churn\": {}, \"score\": {}}}",
                json_string(&row.path),
                row.code,
                row.complexity,
                row.commits,
                row.churn,
                row.score
            ));
        }
        out.push_str(&format!(
            "  \"hotspot_total\": {},\n",
            report.hotspots.len()
        ));
        out.push_str(&format!("  \"hotspots\": {},\n", json_array(&items, 2)));
    }
    if let Some(diff) = &report.diff {
        let mut items: Vec<String> = Vec::with_capacity(diff.files.len());
        for delta in &diff.files {
            items.push(format!(
                "{{\"path\": {}, \"code\": {}, \"code_delta\": {}, \"added\": {}, \"deleted\": {}}}",
                json_string(&delta.path),
                delta.new.map(|row| row.code).unwrap_or(0),
                delta.code_delta(),
                delta.added(),
                delta.deleted()
            ));
        }
        out.push_str("  \"baseline\": {\n");
        out.push_str(&format!(
            "    \"path\": {},\n",
            json_string(&scan::relative(&diff.path, cwd))
        ));
        out.push_str(&format!("    \"added\": {},\n", diff.added()));
        out.push_str(&format!("    \"deleted\": {},\n", diff.deleted()));
        out.push_str(&format!("    \"changed\": {},\n", diff.changed()));
        out.push_str("    \"totals\": {\n");
        out.push_str(&format!(
            "      \"lines\": [{}, {}],\n",
            diff.old.lines, diff.new.lines
        ));
        out.push_str(&format!(
            "      \"code\": [{}, {}],\n",
            diff.old.code, diff.new.code
        ));
        out.push_str(&format!(
            "      \"complexity\": [{}, {}],\n",
            diff.old.complexity, diff.new.complexity
        ));
        out.push_str(&format!(
            "      \"smells\": [{}, {}]\n",
            diff.old.smells, diff.new.smells
        ));
        out.push_str("    },\n");
        out.push_str(&format!("    \"files\": {}\n", json_array(&items, 4)));
        out.push_str("  },\n");
    }
    if let Some(history) = &report.history {
        out.push_str(&json_history(history, cwd, history::SPARK_WIDTH));
    }
    let mut rows: Vec<String> = Vec::with_capacity(report.entries.len());
    for entry in &report.entries {
        let stats = &entry.stats;
        let mut row = String::new();
        row.push_str("    {\n");
        row.push_str(&format!(
            "      \"path\": {},\n",
            json_string(&scan::relative(entry.path(), cwd))
        ));
        row.push_str(&format!("      \"test_only\": {},\n", stats.forced_test));
        if options.mark_generated {
            row.push_str(&format!(
                "      \"generated\": {},\n",
                json_string(stats.generated.label())
            ));
        }
        row.push_str(&format!("      \"lines\": {},\n", stats.lines));
        row.push_str(&format!("      \"blank\": {},\n", stats.blank));
        row.push_str(&format!("      \"comment\": {},\n", stats.comment));
        row.push_str(&format!("      \"doc\": {},\n", stats.doc));
        row.push_str(&format!("      \"code\": {},\n", stats.code));
        if let Some(diff) = &report.diff {
            row.push_str(&format!(
                "      \"code_delta\": {},\n",
                optional_number(diff.code_delta(&baseline::key(entry, &report.roots, cwd)))
            ));
        }
        row.push_str(&format!("      \"test\": {},\n", stats.test));
        row.push_str(&format!("      \"physical\": {},\n", stats.physical));
        row.push_str(&format!("      \"complexity\": {},\n", stats.complexity));
        row.push_str(&format!("      \"unsafe\": {},\n", stats.unsafe_kw));
        row.push_str(&format!("      \"test_fns\": {},\n", stats.test_fns));
        if options.smells {
            row.push_str(&format!(
                "      \"smells\": {{\"unwrap\": {}, \"expect\": {}, \"panic\": {}, \"todo\": {}, \"unimplemented\": {}, \"allow\": {}, \"total\": {}}},\n",
                stats.smells.unwrap,
                stats.smells.expect,
                stats.smells.panic,
                stats.smells.todo,
                stats.smells.unimplemented,
                stats.smells.allow,
                stats.smells.total()
            ));
        }
        row.push_str(&format!(
            "      \"uloc\": {}\n",
            optional_number(stats.uloc)
        ));
        row.push_str("    }");
        rows.push(row);
    }
    if options.languages {
        let mut items: Vec<String> = Vec::with_capacity(report.languages.len());
        for row in &report.languages {
            items.push(format!(
                "{{\"name\": {}, \"files\": {}, \"lines\": {}, \"blank\": {}, \"comment\": {}, \"code\": {}}}",
                json_string(row.name),
                row.files,
                row.lines,
                row.blank,
                row.comment,
                row.code
            ));
        }
        let total: u32 = report.languages.iter().map(|row| row.code).sum();
        out.push_str(&format!("  \"language_code\": {},\n", total));
        out.push_str(&format!("  \"languages\": {},\n", json_array(&items, 2)));
    }
    if let Some(authors) = &report.authors {
        let mut items: Vec<String> = Vec::with_capacity(authors.rows.len());
        for row in &authors.rows {
            items.push(format!(
                "{{\"name\": {}, \"email\": {}, \"files\": {}, \"lines\": {}, \"blank\": {}, \"comment\": {}, \"doc\": {}, \"code\": {}, \"test\": {}}}",
                json_string(&row.name),
                json_string(&row.email),
                row.files,
                row.lines.business(),
                row.lines.blank,
                row.lines.comment,
                row.lines.doc,
                row.lines.code,
                row.lines.test
            ));
        }
        out.push_str(&format!(
            "  \"authors_available\": {},\n",
            authors.available
        ));
        out.push_str(&format!(
            "  \"authors_untracked\": {},\n",
            authors.untracked
        ));
        out.push_str(&format!("  \"authors\": {},\n", json_array(&items, 2)));
        if options.by_file {
            let mut files: Vec<String> = Vec::with_capacity(authors.files.len());
            for file in &authors.files {
                let shares: Vec<String> = file
                    .authors
                    .iter()
                    .map(|item| {
                        format!(
                            "{{\"name\": {}, \"email\": {}, \"lines\": {}, \"blank\": {}, \"comment\": {}, \"code\": {}, \"test\": {}}}",
                            json_string(&item.name),
                            json_string(&item.email),
                            item.lines.business(),
                            item.lines.blank,
                            item.lines.comment,
                            item.lines.code,
                            item.lines.test
                        )
                    })
                    .collect();
                files.push(format!(
                    "{{\"path\": {}, \"authors\": {}}}",
                    json_string(&scan::relative(&file.path, cwd)),
                    json_array(&shares, 4)
                ));
            }
            out.push_str(&format!("  \"file_authors\": {},\n", json_array(&files, 2)));
        }
    }
    if let Some(dupes) = &report.duplicates {
        let mut items: Vec<String> = Vec::with_capacity(dupes.clusters.len());
        for cluster in &dupes.clusters {
            let sites: Vec<String> = cluster
                .sites
                .iter()
                .map(|site| {
                    format!(
                        "{{\"path\": {}, \"line\": {}}}",
                        json_string(&site.path),
                        site.line
                    )
                })
                .collect();
            items.push(format!(
                "{{\"lines\": {}, \"sites\": {}}}",
                cluster.lines,
                json_array(&sites, 6)
            ));
        }
        out.push_str("  \"duplicates\": {\n");
        out.push_str(&format!("    \"window\": {},\n", dupes.window));
        out.push_str(&format!("    \"clone_count\": {},\n", dupes.clusters.len()));
        out.push_str(&format!(
            "    \"duplicated_lines\": {},\n",
            dupes.duplicated_lines
        ));
        out.push_str(&format!(
            "    \"normalised_lines\": {},\n",
            dupes.code_lines
        ));
        out.push_str(&format!(
            "    \"percent\": {},\n",
            json_number(dupes.percent())
        ));
        out.push_str(&format!("    \"clusters\": {}\n", json_array(&items, 4)));
        out.push_str("  },\n");
    }
    if rows.is_empty() {
        out.push_str("  \"files\": []\n}");
    } else {
        out.push_str("  \"files\": [\n");
        out.push_str(&rows.join(",\n"));
        out.push_str("\n  ]\n}");
    }
    out
}

/// The COCOMO estimate block, in the shape `scc` prints it.
fn render_cocomo_text(row: &CocomoRow, out: &mut Vec<String>) {
    let estimate = &row.estimate;
    out.push(format!("COCOMO estimate ({})", row.project));
    out.push(pad("  Cost to develop", 22) + &format!("{:>12}", money(estimate.cost)));
    out.push(
        pad("  Schedule effort", 22)
            + &format!("{:>12} months", format!("{:.2}", estimate.schedule)),
    );
    out.push(pad("  People required", 22) + &format!("{:>12}", format!("{:.2}", estimate.people)));
}
