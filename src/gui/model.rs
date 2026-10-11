//! Presentation model: report data in, table rows and cards out.
//!
//! Mirrors `python/src/rust_loc/ui_model.py` but only carries what the Win32
//! shell needs.

use std::path::{Path, PathBuf};

use crate::baseline;
use crate::history;
use crate::options::{Format, Options};
use crate::report::{FunctionRow, Report};
use crate::scan;

use crate::gui::i18n::{self, fill};
use crate::gui::theme;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum View {
    Files,
    Modules,
    Functions,
    Smells,
    Excluded,
}

impl View {
    pub fn label_title(self) -> &'static str {
        let t = i18n::t();
        match self {
            View::Files => t.label_file,
            View::Modules => t.label_module,
            View::Functions => t.label_function,
            View::Smells => t.label_file,
            View::Excluded => t.label_test_only_file,
        }
    }

    /// Tab caption in the segmented strip.
    pub fn tab_title(self) -> &'static str {
        let t = i18n::t();
        match self {
            View::Files => t.tab_files,
            View::Modules => t.tab_modules,
            View::Functions => t.tab_functions,
            View::Smells => t.tab_smells,
            View::Excluded => t.tab_excluded,
        }
    }

    /// Card title above the table.
    pub fn table_title(self) -> &'static str {
        let t = i18n::t();
        match self {
            View::Files => t.table_per_file,
            View::Modules => t.table_per_module,
            View::Functions => t.table_functions,
            View::Smells => t.table_smells,
            View::Excluded => t.table_excluded,
        }
    }

    pub fn from_index(index: usize) -> View {
        match index {
            1 => View::Modules,
            2 => View::Functions,
            3 => View::Smells,
            4 => View::Excluded,
            _ => View::Files,
        }
    }

    pub fn index(self) -> usize {
        match self {
            View::Files => 0,
            View::Modules => 1,
            View::Functions => 2,
            View::Smells => 3,
            View::Excluded => 4,
        }
    }
}

/// A table column: stats key and design-pixel width.
#[derive(Clone, Copy)]
pub struct Column {
    pub key: &'static str,
    pub width: i32,
}

/// Header title for a column key, in the active language.
pub fn column_title(key: &str) -> &'static str {
    let t = i18n::t();
    match key {
        "lines" => t.col_lines,
        "blank" => t.col_blank,
        "comment" => t.col_comment,
        "code" => t.col_code,
        "test" => t.col_test,
        "complexity" => t.col_cplx,
        "uloc" => t.col_uloc,
        "args" => t.col_fn_args,
        "depth" => t.col_fn_depth,
        "unwrap" => t.col_unwrap,
        "expect" => t.col_expect,
        "panic" => t.col_panic,
        "todo" => t.col_todo,
        "unimplemented" => t.col_unimplemented,
        "allow" => t.col_allow,
        "smells" => t.col_smells,
        "delta" => t.col_delta,
        _ => "",
    }
}

const TABLE_COLUMNS: [Column; 7] = [
    Column {
        key: "lines",
        width: 78,
    },
    Column {
        key: "blank",
        width: 70,
    },
    Column {
        key: "comment",
        width: 84,
    },
    Column {
        key: "code",
        width: 74,
    },
    Column {
        key: "test",
        width: 70,
    },
    Column {
        key: "complexity",
        width: 68,
    },
    Column {
        key: "uloc",
        width: 74,
    },
];

pub fn table_columns(options: &Options) -> Vec<Column> {
    TABLE_COLUMNS
        .iter()
        .copied()
        .filter(|column| match column.key {
            "complexity" => options.complexity,
            "uloc" => options.uloc,
            _ => true,
        })
        .collect()
}

/// Columns for one tab. The excluded list only ever shows a line count; the
/// function list ranks by complexity and shows the per-function numbers.
pub fn view_columns(view: View, options: &Options, has_delta: bool) -> Vec<Column> {
    let mut columns = match view {
        View::Excluded => vec![Column {
            key: "lines",
            width: 90,
        }],
        View::Functions => vec![
            Column {
                key: "code",
                width: 74,
            },
            Column {
                key: "complexity",
                width: 68,
            },
            Column {
                key: "args",
                width: 64,
            },
            Column {
                key: "depth",
                width: 64,
            },
        ],
        View::Smells => vec![
            Column {
                key: "unwrap",
                width: 72,
            },
            Column {
                key: "expect",
                width: 72,
            },
            Column {
                key: "panic",
                width: 64,
            },
            Column {
                key: "todo",
                width: 64,
            },
            Column {
                key: "unimplemented",
                width: 70,
            },
            Column {
                key: "allow",
                width: 64,
            },
            Column {
                key: "smells",
                width: 70,
            },
        ],
        _ => table_columns(options),
    };
    if has_delta && view == View::Files {
        columns.push(Column {
            key: "delta",
            width: 76,
        });
    }
    columns
}

/// Column a tab sorts on until the user clicks a header: worst first for the
/// function list, most code first for the file and module lists.
pub fn default_sort(view: View) -> (i32, bool) {
    match view {
        View::Functions => (1, true),
        View::Smells => (6, true),
        View::Excluded => (0, true),
        _ => (3, true),
    }
}

pub struct Row {
    pub label: String,
    pub values: Vec<u32>,
    /// File the row points at, when it is more precise than the label.
    pub target: Option<String>,
    /// Change in code lines since the previous scan, when there was one.
    pub delta: Option<i64>,
}

pub struct Card {
    pub key: &'static str,
    pub label: &'static str,
    pub value: String,
    pub hint: String,
}

impl Card {
    pub fn tone(&self) -> u32 {
        match self.key {
            "files" | "lines" => theme::ACCENT,
            "code" => theme::COLOR_CODE,
            "comment" => theme::COLOR_COMMENT,
            "blank" => rgb_muted(),
            "test" => theme::COLOR_TEST,
            "complexity" => theme::COLOR_COMPLEXITY,
            "unsafe" => theme::COLOR_UNSAFE,
            "test_fns" => theme::COLOR_TEST_FNS,
            "uloc" => theme::COLOR_ULOC,
            "cost" => theme::COLOR_TEST,
            "effort" => theme::COLOR_DOC,
            "people" => theme::COLOR_TEST_FNS,
            "schedule" => theme::COLOR_ULOC,
            "delta" => theme::ACCENT,
            _ => theme::TEXT,
        }
    }
}

const fn rgb_muted() -> u32 {
    theme::rgb(0x94A3B8)
}

/// Thousands separated integer.
pub fn human_int(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

pub fn human_share(value: u64, base: u64) -> String {
    if base == 0 {
        return "0.0%".to_string();
    }
    format!("{:.1}%", value as f64 / base as f64 * 100.0)
}

/// Whole US dollars, grouped like every other counter.
pub fn money(value: f64) -> String {
    format!("${}", human_int(value.max(0.0).round() as u64))
}

/// "+180", "-31" or "0", grouped like the other counters.
pub fn delta_text(value: i64) -> String {
    if value == 0 {
        return "0".to_string();
    }
    format!(
        "{}{}",
        if value > 0 { "+" } else { "-" },
        human_int(value.unsigned_abs())
    )
}

/// Split pasted roots: Windows paths may contain commas, so only `;` and
/// newlines separate them.
pub fn parse_paths(text: &str) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    for part in text.split([';', '\r', '\n']) {
        let token = part.trim().trim_matches('"').trim();
        if !token.is_empty() && !result.iter().any(|seen| seen == token) {
            result.push(token.to_string());
        }
    }
    result
}

pub fn display_path(path: &Path, cwd: &Path) -> String {
    scan::relative(path, cwd)
}

/// Build the visible rows for the current view, before filter and limit.
pub fn build_rows(report: &Report, view: View, columns: &[Column], cwd: &Path) -> Vec<Row> {
    let mut rows = Vec::new();
    match view {
        View::Files => {
            for entry in report.business_entries() {
                let delta = report
                    .diff
                    .as_ref()
                    .and_then(|diff| diff.code_delta(&baseline::key(entry, &report.roots, cwd)));
                rows.push(Row {
                    label: display_path(entry.path(), cwd),
                    values: counter_values(entry, columns),
                    target: None,
                    delta,
                });
            }
        }
        View::Modules => {
            for (label, stats, _) in scan::module_groups(&report.business_entries(), cwd) {
                rows.push(Row {
                    label: shorten_module(&label),
                    values: columns.iter().map(|c| stats.column(c.key)).collect(),
                    target: None,
                    delta: None,
                });
            }
        }
        View::Functions => {
            for function in &report.functions {
                rows.push(Row {
                    label: function_label(function),
                    values: columns
                        .iter()
                        .map(|column| function_column(function, column.key))
                        .collect(),
                    target: Some(function.path.clone()),
                    delta: None,
                });
            }
        }
        View::Smells => {
            for entry in report.business_entries() {
                let total = entry.stats.smells.total();
                if total == 0 {
                    continue;
                }
                rows.push(Row {
                    label: display_path(entry.path(), cwd),
                    values: counter_values(entry, columns),
                    target: None,
                    delta: None,
                });
            }
        }
        View::Excluded => {
            let mut pairs: Vec<(String, u32)> = report
                .excluded
                .paths
                .iter()
                .map(|(path, lines)| (path.clone(), *lines))
                .collect();
            pairs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            for (path, lines) in pairs {
                rows.push(Row {
                    label: path,
                    values: vec![lines],
                    target: None,
                    delta: None,
                });
            }
        }
    }
    rows
}

/// Drop the `./` that a root-level module inherits from the relative path.
/// The counters of one file, in column order. The trailing delta column is
/// not a counter: the shell draws the signed change from `Row::delta`.
fn counter_values(entry: &scan::Entry, columns: &[Column]) -> Vec<u32> {
    columns
        .iter()
        .filter(|column| column.key != "delta")
        .map(|column| entry.column(column.key))
        .collect()
}

fn shorten_module(label: &str) -> String {
    label.strip_prefix("./").unwrap_or(label).to_string()
}

/// name + path:line, the label shown in the functions tab.
fn function_label(function: &FunctionRow) -> String {
    format!("{}  ·  {}:{}", function.name, function.path, function.line)
}

fn function_column(function: &FunctionRow, key: &str) -> u32 {
    match key {
        "code" => function.code,
        "complexity" => function.complexity,
        "args" => function.params,
        "depth" => function.depth,
        _ => 0,
    }
}

/// Biggest modules by code, ready for the bar chart.
pub fn top_modules(report: &Report, cwd: &Path, limit: usize) -> Vec<(String, u32)> {
    let rows: Vec<(String, u32)> = scan::module_groups(&report.business_entries(), cwd)
        .iter()
        .take(limit)
        .map(|(label, stats, _)| (label.clone(), stats.code))
        .collect();
    shorten_labels(rows)
}

/// Drop the directory prefix every chart label shares.
fn shorten_labels(rows: Vec<(String, u32)>) -> Vec<(String, u32)> {
    if rows.len() < 2 {
        return rows;
    }
    let prefix = common_prefix(&rows);
    let cut = match prefix.rfind('/') {
        Some(cut) if cut > 0 => cut,
        _ => return rows,
    };
    let keep = &prefix[..cut + 1];
    rows.into_iter()
        .map(|(label, value)| match label.strip_prefix(keep) {
            Some(rest) => (format!("…/{rest}"), value),
            None => (label, value),
        })
        .collect()
}

fn common_prefix(rows: &[(String, u32)]) -> String {
    let first: Vec<char> = rows[0].0.chars().collect();
    let mut end = first.len();
    for (label, _) in rows.iter().skip(1) {
        let mut matched = 0;
        for (left, right) in first.iter().zip(label.chars()) {
            if matched >= end || *left != right {
                break;
            }
            matched += 1;
        }
        end = matched;
    }
    first[..end].iter().collect()
}

/// One line of table chrome: how many rows are shown and how they are sorted.
pub fn table_hint(
    view: View,
    report: Option<&Report>,
    shown: usize,
    matched: usize,
    column: i32,
    descending: bool,
) -> String {
    let t = i18n::t();
    let Some(report) = report else {
        return t.table_empty.to_string();
    };
    if view == View::Excluded {
        if report.excluded.files == 0 {
            return t.hint_no_test_files.to_string();
        }
        return fill(
            t.hint_excluded,
            &[
                ("files", human_int(report.excluded.files as u64)),
                ("lines", human_int(report.excluded.file_lines as u64)),
                ("inner", human_int(report.excluded.item_lines as u64)),
            ],
        );
    }
    let mut hint = if view == View::Functions {
        if report.function_count == 0 {
            return t.hint_no_functions.to_string();
        }
        fill(
            t.hint_functions,
            &[
                ("total", human_int(report.function_count as u64)),
                ("shown", human_int(report.functions.len() as u64)),
            ],
        )
    } else if view == View::Smells {
        let total = report.summary.smells.total();
        if total == 0 {
            return t.hint_no_smells.to_string();
        }
        fill(
            t.hint_smells,
            &[
                ("total", human_int(total as u64)),
                ("shown", human_int(shown as u64)),
                ("matched", human_int(matched as u64)),
            ],
        )
    } else {
        fill(
            t.hint_showing,
            &[
                ("shown", human_int(shown as u64)),
                ("matched", human_int(matched as u64)),
            ],
        )
    };
    hint.push_str(&sorted_clause(view, report, column, descending));
    if report.options.exclude_generated && report.excluded.generated_files > 0 {
        hint.push_str(" · ");
        hint.push_str(&fill(
            t.hint_generated_excluded,
            &[
                ("files", human_int(report.excluded.generated_files as u64)),
                ("lines", human_int(report.excluded.generated_lines as u64)),
            ],
        ));
    }
    hint
}

/// " · sorted by Cplx ▼" for the tab in front of the user.
fn sorted_clause(view: View, report: &Report, column: i32, descending: bool) -> String {
    let title = if column >= 0 {
        match view_columns(view, &report.options, report.diff.is_some()).get(column as usize) {
            Some(entry) => column_title(entry.key),
            None => view.label_title(),
        }
    } else {
        view.label_title()
    };
    format!(
        " · {}",
        fill(
            i18n::t().hint_sorted,
            &[
                ("column", title.to_string()),
                ("arrow", if descending { "▼" } else { "▲" }.to_string()),
            ],
        )
    )
}

/// Split a free-form field into unique globs or directory names.
pub fn parse_list(text: &str) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    for part in text.split([',', ';', ' ', '\t', '\r', '\n']) {
        let token = part.trim();
        if !token.is_empty() && !result.iter().any(|seen| seen == token) {
            result.push(token.to_string());
        }
    }
    result
}

/// Stable value sort on top of an ascending label order.
pub fn sort_rows(rows: &mut [Row], column: i32, descending: bool) {
    if column < 0 {
        rows.sort_by_key(|row| row.label.to_lowercase());
        return;
    }
    let index = column as usize;
    rows.sort_by_key(|row| row.label.to_lowercase());
    rows.sort_by(|a, b| {
        let left = sort_key(a, index);
        let right = sort_key(b, index);
        if descending {
            right.cmp(&left)
        } else {
            left.cmp(&right)
        }
    });
}

/// Value a row sorts on for a column index; the trailing delta column reads
/// the signed change instead of a counter.
fn sort_key(row: &Row, index: usize) -> i64 {
    match row.values.get(index) {
        Some(value) => i64::from(*value),
        None => row.delta.unwrap_or(0),
    }
}

pub fn filter_rows(rows: &[Row], query: &str) -> Vec<usize> {
    let needle = query.trim().to_lowercase();
    rows.iter()
        .enumerate()
        .filter(|(_, row)| needle.is_empty() || row.label.to_lowercase().contains(&needle))
        .map(|(index, _)| index)
        .collect()
}

/// Line composition segments in draw order.
pub fn composition(report: &Report) -> Vec<(&'static str, u32, u32)> {
    let t = i18n::t();
    let totals = &report.summary;
    vec![
        (t.seg_code, totals.code, theme::COLOR_CODE),
        (t.seg_doc, totals.doc, theme::COLOR_DOC),
        (
            t.seg_comment,
            totals.comment.saturating_sub(totals.doc),
            theme::COLOR_COMMENT,
        ),
        (t.seg_blank, totals.blank, theme::COLOR_BLANK),
    ]
}
/// Placeholder cards shown before the first scan, so the strip keeps its shape.
pub fn placeholder_cards(options: &Options) -> Vec<Card> {
    let t = i18n::t();
    let mut cards = vec![
        placeholder("files", t.card_files),
        placeholder("lines", t.card_lines),
        placeholder("code", t.card_code),
        placeholder("comment", t.card_comment),
        placeholder("blank", t.card_blank),
        placeholder("test", t.card_test),
    ];
    if options.complexity {
        cards.push(placeholder("complexity", t.card_complexity));
    }
    cards.push(placeholder("unsafe", t.card_unsafe));
    cards.push(placeholder("test_fns", t.card_test_fns));
    if options.uloc {
        cards.push(placeholder("uloc", t.card_uloc));
    }
    cards
}

fn placeholder(key: &'static str, label: &'static str) -> Card {
    let t = i18n::t();
    Card {
        key,
        label,
        value: t.card_placeholder_value.to_string(),
        hint: t.hint_placeholder.to_string(),
    }
}

/// KPI cards for the headline numbers of a scan.
pub fn summary_cards(report: Option<&Report>, options: &Options) -> Vec<Card> {
    let t = i18n::t();
    let Some(report) = report else {
        return placeholder_cards(options);
    };
    let totals = &report.summary;
    let lines = totals.lines as u64;
    let include_tests = report.options.include_tests;
    let test_lines = report.excluded.file_lines as u64 + report.excluded.item_lines as u64;
    let mut cards = vec![
        Card {
            key: "files",
            label: t.card_files,
            value: human_int(report.business.len() as u64),
            hint: fill(
                t.hint_files_scanned,
                &[("n", human_int(report.entries.len() as u64))],
            ),
        },
        Card {
            key: "lines",
            label: t.card_lines,
            value: human_int(lines),
            hint: if include_tests {
                t.hint_tests_included.to_string()
            } else {
                t.hint_business_only.to_string()
            },
        },
        Card {
            key: "code",
            label: t.card_code,
            value: human_int(totals.code as u64),
            hint: fill(
                t.hint_of_lines,
                &[("share", human_share(totals.code as u64, lines))],
            ),
        },
        Card {
            key: "comment",
            label: t.card_comment,
            value: human_int(totals.comment as u64),
            hint: fill(
                t.hint_comment_doc,
                &[
                    ("share", human_share(totals.comment as u64, lines)),
                    ("n", human_int(totals.doc as u64)),
                ],
            ),
        },
        Card {
            key: "blank",
            label: t.card_blank,
            value: human_int(totals.blank as u64),
            hint: fill(
                t.hint_of_lines,
                &[("share", human_share(totals.blank as u64, lines))],
            ),
        },
        Card {
            key: "test",
            label: t.card_test,
            value: human_int(test_lines),
            hint: if include_tests {
                t.hint_included_totals.to_string()
            } else {
                t.hint_excluded_totals.to_string()
            },
        },
    ];
    if report.options.complexity {
        cards.push(Card {
            key: "complexity",
            label: t.card_complexity,
            value: human_int(totals.complexity as u64),
            hint: t.hint_complexity.to_string(),
        });
    }
    cards.push(Card {
        key: "unsafe",
        label: t.card_unsafe,
        value: human_int(totals.unsafe_kw as u64),
        hint: t.hint_unsafe.to_string(),
    });
    cards.push(Card {
        key: "test_fns",
        label: t.card_test_fns,
        value: human_int(totals.test_fns as u64),
        hint: t.hint_test_fns.to_string(),
    });
    if report.options.uloc {
        let total = report.uloc_total.unwrap_or(0) as u64;
        cards.push(Card {
            key: "uloc",
            label: t.card_uloc,
            value: human_int(total),
            hint: fill(t.hint_uloc, &[("share", human_share(total, lines))]),
        });
    }
    if let Some(row) = &report.cocomo {
        cards.push(Card {
            key: "cost",
            label: t.card_cost,
            value: money(row.estimate.cost),
            hint: fill(t.hint_cocomo_cost, &[("project", row.project.to_string())]),
        });
        cards.push(Card {
            key: "effort",
            label: t.card_effort,
            value: format!("{:.1}", row.estimate.effort),
            hint: t.hint_effort.to_string(),
        });
        cards.push(Card {
            key: "people",
            label: t.card_people,
            value: format!("{:.2}", row.estimate.people),
            hint: t.hint_people.to_string(),
        });
        cards.push(Card {
            key: "schedule",
            label: t.card_schedule,
            value: format!("{:.1}", row.estimate.schedule),
            hint: t.hint_schedule.to_string(),
        });
    }
    if let Some(diff) = &report.diff {
        cards.push(Card {
            key: "delta",
            label: t.card_delta,
            value: delta_text(diff.new.code as i64 - diff.old.code as i64),
            hint: fill(
                t.hint_delta_files,
                &[
                    ("added", human_int(diff.added() as u64)),
                    ("deleted", human_int(diff.deleted() as u64)),
                ],
            ),
        });
    }
    cards
}

/// Test-only file lines plus lines inside `#[cfg(test)]` items.
pub fn test_line_total(report: &Report) -> u64 {
    report.excluded.file_lines as u64 + report.excluded.item_lines as u64
}

/// The status bar line shown after a scan.
pub fn scan_status(report: &Report, seconds: f64) -> String {
    fill(
        i18n::t().status_scanned,
        &[
            ("files", human_int(report.entries.len() as u64)),
            ("seconds", format!("{seconds:.2}")),
            ("test", human_int(test_line_total(report))),
        ],
    )
}

/// The right hand side of the title bar: the headline totals.
pub fn header_summary(report: Option<&Report>) -> String {
    let t = i18n::t();
    let Some(report) = report else {
        return t.no_scan_yet.to_string();
    };
    let totals = &report.summary;
    [
        format!("{} {}", human_int(totals.code as u64), t.header_code),
        format!("{} {}", human_int(totals.comment as u64), t.header_comment),
        format!("{} {}", human_int(totals.blank as u64), t.header_blank),
    ]
    .join(" · ")
}

/// The right hand side of the status bar: what the scan covered.
pub fn scope_line(options: &Options, roots: usize) -> String {
    let t = i18n::t();
    let mut parts = vec![
        t.roots(roots),
        if options.gitignore {
            t.scope_gitignore_on
        } else {
            t.scope_gitignore_off
        }
        .to_string(),
        fill(
            t.scope_ignored_dirs,
            &[("n", options.ignore_dirs.len().to_string())],
        ),
    ];
    if !options.exclude.is_empty() {
        parts.push(fill(
            t.scope_excludes,
            &[("n", options.exclude.len().to_string())],
        ));
    }
    if options.include_tests {
        parts.push(t.scope_tests_included.to_string());
    }
    parts.join(" · ")
}

/// Rebuild a `rust-loc` command line from the current UI state.
pub fn cli_command(report: &Report, cwd: &Path) -> String {
    let options = &report.options;
    let mut parts = vec!["rust-loc".to_string()];
    for root in &report.roots {
        parts.push(quote_arg(&display_path(root, cwd)));
    }
    for pattern in &options.exclude {
        parts.push("--exclude".to_string());
        parts.push(quote_arg(pattern));
    }
    let defaults = scan::default_ignore_dirs();
    if !defaults.is_subset(&options.ignore_dirs) {
        parts.push("--no-ignore-dirs".to_string());
    }
    for name in options.ignore_dirs.difference(&defaults) {
        parts.push("--ignore-dir".to_string());
        parts.push(quote_arg(name));
    }
    if options.include_tests {
        parts.push("--include-tests".to_string());
    }
    if options.uloc {
        parts.push("--uloc".to_string());
    }
    if options.exclude_generated {
        parts.push("--exclude-generated".to_string());
    }
    if options.functions {
        parts.push("--functions".to_string());
    }
    if options.smells {
        parts.push("--smells".to_string());
    }
    if options.cocomo {
        parts.push("--cocomo".to_string());
        if options.cocomo_project != crate::cocomo::ProjectType::Organic {
            parts.push("--cocomo-project-type".to_string());
            parts.push(options.cocomo_project.name().to_string());
        }
    }
    if !options.complexity {
        parts.push("--no-complexity".to_string());
    }
    if !options.gitignore {
        parts.push("--no-gitignore".to_string());
    }
    match options.format {
        Format::Json => parts.push("--json".to_string()),
        Format::Csv => parts.push("--csv".to_string()),
        Format::Markdown => parts.push("--markdown".to_string()),
        Format::Svg => parts.push("--format svg".to_string()),
        Format::Text => {}
    }
    parts.join(" ")
}

fn quote_arg(value: &str) -> String {
    if value.contains([' ', '\t', '"']) {
        format!("\"{}\"", value)
    } else {
        value.to_string()
    }
}

/// The numbers the scan history card draws.
#[derive(Clone)]
pub struct Spark {
    pub title: String,
    pub latest: String,
    pub delta: String,
    /// Code lines per recorded scan, oldest first.
    pub values: Vec<u32>,
}

/// Where the history of a root set lives: beside the settings, one file per
/// tree, so switching projects does not splice two series together.
pub fn history_path(roots: &[String]) -> PathBuf {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for root in roots {
        for byte in root.trim().to_lowercase().bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash ^= u64::from(b'/');
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    if roots.is_empty() {
        return settings_path().with_file_name("history.tsv");
    }
    settings_path().with_file_name(format!("history-{hash:016x}.tsv"))
}

/// The sparkline card for the recorded scans, or None while there is nothing
/// to compare: one sample is a point, not a trend.
pub fn history_trend(history: &history::History) -> Option<Spark> {
    if history.samples.len() < 2 {
        return None;
    }
    let values = history.series(history::Trend::Code, history::LIMIT);
    let latest = values.last().copied().unwrap_or(0);
    let before = values
        .get(values.len().saturating_sub(2))
        .copied()
        .unwrap_or(latest);
    Some(Spark {
        title: format!("{} · {}", i18n::t().chart_history, i18n::t().header_code),
        latest: human_int(u64::from(latest)),
        delta: format!(
            "{} {}",
            delta_text(i64::from(latest) - i64::from(before)),
            i18n::t().chart_history_delta
        ),
        values,
    })
}

/// Persisted UI state; a tiny key=value file, no serialisation dependency.
#[derive(Clone)]
pub struct Settings {
    pub roots: String,
    pub gitignore: bool,
    pub default_ignores: bool,
    pub include_tests: bool,
    pub complexity: bool,
    pub uloc: bool,
    pub exclude_generated: bool,
    pub cocomo: bool,
    pub exclude: String,
    pub extra_dirs: String,
    pub view: usize,
    pub top: usize,
    pub format: usize,
    pub lang: usize,
    pub filter: String,
    pub sort_column: i32,
    pub sort_descending: bool,
    pub window: Option<(i32, i32, i32, i32)>,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            roots: String::new(),
            gitignore: true,
            default_ignores: true,
            include_tests: false,
            complexity: true,
            uloc: false,
            exclude_generated: false,
            cocomo: false,
            exclude: String::new(),
            extra_dirs: String::new(),
            view: 0,
            top: 1,
            format: 0,
            lang: i18n::system_lang().index(),
            filter: String::new(),
            sort_column: 3,
            sort_descending: true,
            window: None,
        }
    }
}

pub fn settings_path() -> PathBuf {
    let base = std::env::var("APPDATA")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::env::var("USERPROFILE")
                .ok()
                .map(|home| format!("{home}\\AppData\\Roaming"))
        })
        .unwrap_or_else(|| ".".to_string());
    Path::new(&base).join("rust-loc").join("ui.ini")
}

pub fn load_settings() -> Settings {
    let mut settings = Settings::default();
    let Ok(text) = std::fs::read_to_string(settings_path()) else {
        return settings;
    };
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let flag = value.trim() == "1";
        match key.trim() {
            "roots" => settings.roots = value.trim().to_string(),
            "gitignore" => settings.gitignore = flag,
            "default_ignores" => settings.default_ignores = flag,
            "include_tests" => settings.include_tests = flag,
            "complexity" => settings.complexity = flag,
            "uloc" => settings.uloc = flag,
            "exclude_generated" => settings.exclude_generated = flag,
            "cocomo" => settings.cocomo = flag,
            "exclude" => settings.exclude = value.trim().to_string(),
            "extra_dirs" => settings.extra_dirs = value.trim().to_string(),
            "view" => settings.view = value.trim().parse().unwrap_or(0),
            "lang" => settings.lang = value.trim().parse().unwrap_or(settings.lang),
            "top" => settings.top = value.trim().parse().unwrap_or(1),
            "format" => settings.format = value.trim().parse().unwrap_or(0),
            "filter" => settings.filter = value.trim().to_string(),
            "sort_column" => settings.sort_column = value.trim().parse().unwrap_or(3),
            "sort_descending" => settings.sort_descending = flag,
            "window" => {
                let parts: Vec<i32> = value
                    .trim()
                    .split(',')
                    .filter_map(|part| part.trim().parse().ok())
                    .collect();
                if parts.len() == 4 {
                    settings.window = Some((parts[0], parts[1], parts[2], parts[3]));
                }
            }
            _ => {}
        }
    }
    settings
}

pub fn save_settings(settings: &Settings) {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut text = String::new();
    text.push_str(&format!("roots={}\n", settings.roots));
    text.push_str(&format!("gitignore={}\n", flag(settings.gitignore)));
    text.push_str(&format!(
        "default_ignores={}\n",
        flag(settings.default_ignores)
    ));
    text.push_str(&format!("include_tests={}\n", flag(settings.include_tests)));
    text.push_str(&format!("complexity={}\n", flag(settings.complexity)));
    text.push_str(&format!("uloc={}\n", flag(settings.uloc)));
    text.push_str(&format!(
        "exclude_generated={}\n",
        flag(settings.exclude_generated)
    ));
    text.push_str(&format!("cocomo={}\n", flag(settings.cocomo)));
    text.push_str(&format!("exclude={}\n", settings.exclude));
    text.push_str(&format!("extra_dirs={}\n", settings.extra_dirs));
    text.push_str(&format!("view={}\n", settings.view));
    text.push_str(&format!("lang={}\n", settings.lang));
    text.push_str(&format!("top={}\n", settings.top));
    text.push_str(&format!("format={}\n", settings.format));
    text.push_str(&format!("filter={}\n", settings.filter));
    text.push_str(&format!("sort_column={}\n", settings.sort_column));
    text.push_str(&format!(
        "sort_descending={}\n",
        flag(settings.sort_descending)
    ));
    if let Some((x, y, width, height)) = settings.window {
        text.push_str(&format!("window={x},{y},{width},{height}\n"));
    }
    let _ = std::fs::write(path, text);
}

fn flag(value: bool) -> u8 {
    if value {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(cards: &[Card]) -> Vec<&'static str> {
        cards.iter().map(|card| card.key).collect()
    }

    #[test]
    fn placeholder_cards_follow_the_enabled_metrics() {
        let bare = Options {
            complexity: false,
            uloc: false,
            ..Options::default()
        };
        assert_eq!(
            keys(&placeholder_cards(&bare)),
            ["files", "lines", "code", "comment", "blank", "test", "unsafe", "test_fns"]
        );

        let full = Options {
            complexity: true,
            uloc: true,
            ..Options::default()
        };
        let cards = placeholder_cards(&full);
        assert_eq!(cards.len(), 10);
        assert!(keys(&cards).contains(&"complexity"));
        assert!(keys(&cards).contains(&"uloc"));
    }

    #[test]
    fn placeholder_values_are_the_dash() {
        let options = Options::default();
        for card in placeholder_cards(&options) {
            assert_eq!(card.value, i18n::t().card_placeholder_value);
            assert!(!card.hint.is_empty());
        }
    }

    /// A scratch project that removes itself when the test ends.
    struct Scratch {
        path: PathBuf,
    }

    impl Scratch {
        fn new(name: &str) -> Scratch {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "rust-loc-gui-{}-{name}-{stamp}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Scratch { path }
        }

        fn write(&self, name: &str, text: &str) {
            let path = self.path.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, text).unwrap();
        }

        fn scan(&self, options: &Options) -> Report {
            crate::report::scan(std::slice::from_ref(&self.path), options, &self.path, None)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    const SOURCE: &str = concat!(
        "fn simple() {\n",
        "    let x = 1;\n",
        "}\n",
        "\n",
        "fn branchy(a: u32, b: u32) -> u32 {\n",
        "    if a > b && b > 0 {\n",
        "        a\n",
        "    } else if a == 0 {\n",
        "        b\n",
        "    } else {\n",
        "        a + b\n",
        "    }\n",
        "}\n",
    );

    #[test]
    fn functions_tab_ranks_by_complexity_and_keeps_the_file_it_came_from() {
        let scratch = Scratch::new("functions");
        scratch.write("src/lib.rs", SOURCE);
        scratch.write("build.rs", "fn main() {}\n");
        let options = Options {
            functions: true,
            ..Options::default()
        };
        let report = scratch.scan(&options);
        let columns = view_columns(View::Functions, &options, false);
        assert_eq!(
            columns.iter().map(|column| column.key).collect::<Vec<_>>(),
            ["code", "complexity", "args", "depth"]
        );
        let rows = build_rows(&report, View::Functions, &columns, &scratch.path);
        assert!(rows.len() >= 3, "{}", rows.len());
        let first = &rows[0];
        assert!(first.label.starts_with("branchy"), "{}", first.label);
        assert!(first.label.contains("src/lib.rs:5"), "{}", first.label);
        assert_eq!(first.values[1], report.functions[0].complexity);
        assert_eq!(first.target.as_deref(), Some("src/lib.rs"));
        let files = build_rows(
            &report,
            View::Files,
            &table_columns(&options),
            &scratch.path,
        );
        assert!(files.iter().all(|row| row.target.is_none()));
    }

    #[test]
    fn cocomo_cards_appear_only_when_the_estimate_was_asked_for() {
        let scratch = Scratch::new("cocomo");
        scratch.write("src/lib.rs", SOURCE);
        let plain = Options {
            functions: true,
            ..Options::default()
        };
        let report = scratch.scan(&plain);
        assert!(!keys(&summary_cards(Some(&report), &plain)).contains(&"cost"));

        let priced = Options {
            functions: true,
            cocomo: true,
            ..Options::default()
        };
        let report = scratch.scan(&priced);
        let cards = summary_cards(Some(&report), &priced);
        let keys = keys(&cards);
        for key in ["cost", "effort", "people", "schedule"] {
            assert!(keys.contains(&key), "{key} missing from {keys:?}");
        }
        let cost = cards.iter().find(|card| card.key == "cost").unwrap();
        assert!(cost.value.starts_with('$'), "{}", cost.value);
    }

    #[test]
    fn table_hints_report_the_function_and_generated_scope() {
        i18n::set_lang(i18n::Lang::En);
        let scratch = Scratch::new("hint");
        scratch.write("src/lib.rs", SOURCE);
        scratch.write(
            "src/tables.rs",
            "// @generated by build.rs\n// do not edit\nfn table() {}\n",
        );
        let options = Options {
            functions: true,
            exclude_generated: true,
            ..Options::default()
        };
        let report = scratch.scan(&options);
        let functions = table_hint(View::Functions, Some(&report), 0, 0, -1, true);
        assert!(functions.contains("2 functions in scope"), "{functions}");
        let files = table_hint(View::Files, Some(&report), 0, 0, 3, true);
        assert!(files.contains("1 generated files"), "{files}");
        assert!(files.contains("3 lines skipped"), "{files}");
        assert!(files.contains("sorted by Code ▼"), "{files}");
        // The hint has to follow the column the user sorted by, not the tab.
        let sorted = table_hint(View::Functions, Some(&report), 0, 0, 3, true);
        assert!(sorted.contains("sorted by Depth ▼"), "{sorted}");
    }

    #[test]
    fn every_tab_has_a_natural_sort_column() {
        assert_eq!(default_sort(View::Functions), (1, true));
        assert_eq!(default_sort(View::Smells), (6, true));
        assert_eq!(default_sort(View::Files), (3, true));
        assert_eq!(default_sort(View::Modules), (3, true));
    }

    #[test]
    fn view_indices_round_trip_with_the_tab_strip() {
        for index in 0..5 {
            assert_eq!(View::from_index(index).index(), index);
        }
        assert!(View::from_index(3) == View::Smells);
        assert_eq!(View::Smells.tab_title(), i18n::t().tab_smells);
    }

    #[test]
    fn smells_tab_keeps_only_files_with_escapes() {
        i18n::set_lang(i18n::Lang::En);
        let scratch = Scratch::new("smells");
        scratch.write(
            "src/lib.rs",
            "fn a(x: Option<u32>) -> u32 {\n    x.unwrap()\n}\n\nfn b() {}\n",
        );
        scratch.write("src/clean.rs", "fn c() {}\n");
        let options = Options::default();
        let report = scratch.scan(&options);
        let columns = view_columns(View::Smells, &options, false);
        assert_eq!(
            columns.iter().map(|column| column.key).collect::<Vec<_>>(),
            [
                "unwrap",
                "expect",
                "panic",
                "todo",
                "unimplemented",
                "allow",
                "smells"
            ]
        );
        let rows = build_rows(&report, View::Smells, &columns, &scratch.path);
        assert_eq!(rows.len(), 1, "{}", rows.len());
        assert_eq!(rows[0].label, "src/lib.rs");
        assert_eq!(rows[0].values[0], 1);
        assert_eq!(rows[0].values[6], 1);
        assert_eq!(column_title("smells"), i18n::t().col_smells);
        let hint = table_hint(View::Smells, Some(&report), 1, 1, 6, true);
        assert!(hint.contains("1 smells"), "{hint}");
        assert!(hint.contains("sorted by Total ▼"), "{hint}");
    }

    #[test]
    fn a_second_scan_shows_what_changed() {
        i18n::set_lang(i18n::Lang::En);
        let scratch = Scratch::new("delta");
        scratch.write("src/a.rs", "fn a() {\n    let x = 1;\n}\n");
        let options = Options::default();
        let mut report = scratch.scan(&options);

        let mut files = std::collections::BTreeMap::new();
        files.insert(
            "src/a.rs".to_string(),
            crate::baseline::Row {
                lines: 1,
                code: 1,
                complexity: 0,
                smells: 0,
            },
        );
        files.insert(
            "src/gone.rs".to_string(),
            crate::baseline::Row {
                lines: 1,
                code: 1,
                complexity: 0,
                smells: 0,
            },
        );
        let snapshot = crate::baseline::Snapshot {
            path: PathBuf::from("last"),
            files,
        };
        report.diff = Some(crate::baseline::Diff::of(&report, &snapshot, &scratch.path));

        let columns = view_columns(View::Files, &options, true);
        assert_eq!(columns.last().unwrap().key, "delta");
        let rows = build_rows(&report, View::Files, &columns, &scratch.path);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "src/a.rs");
        assert_eq!(
            rows[0].values.len(),
            columns.len() - 1,
            "the delta column is drawn from Row::delta, not from values"
        );
        assert_eq!(rows[0].delta, Some(2));

        let cards = summary_cards(Some(&report), &options);
        let card = cards.iter().find(|card| card.key == "delta").unwrap();
        assert_eq!(card.value, "+1");
        assert_eq!(card.hint, "0 added, 1 deleted");

        let mut sorted = rows;
        sort_rows(&mut sorted, columns.len() as i32 - 1, true);
        assert_eq!(sorted[0].delta, Some(2));
    }
}
