//! Presentation model: report data in, table rows and cards out.
//!
//! Mirrors `python/src/rust_loc/ui_model.py` but only carries what the Win32
//! shell needs.

use std::path::{Path, PathBuf};

use crate::options::{Format, Options};
use crate::report::Report;
use crate::scan;

use crate::gui::i18n::{self, fill};
use crate::gui::theme;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum View {
    Files,
    Modules,
    Excluded,
}

impl View {
    pub fn label_title(self) -> &'static str {
        let t = i18n::t();
        match self {
            View::Files => t.label_file,
            View::Modules => t.label_module,
            View::Excluded => t.label_test_only_file,
        }
    }

    /// Tab caption in the segmented strip.
    pub fn tab_title(self) -> &'static str {
        let t = i18n::t();
        match self {
            View::Files => t.tab_files,
            View::Modules => t.tab_modules,
            View::Excluded => t.tab_excluded,
        }
    }

    /// Card title above the table.
    pub fn table_title(self) -> &'static str {
        let t = i18n::t();
        match self {
            View::Files => t.table_per_file,
            View::Modules => t.table_per_module,
            View::Excluded => t.table_excluded,
        }
    }

    pub fn from_index(index: usize) -> View {
        match index {
            1 => View::Modules,
            2 => View::Excluded,
            _ => View::Files,
        }
    }

    pub fn index(self) -> usize {
        match self {
            View::Files => 0,
            View::Modules => 1,
            View::Excluded => 2,
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

/// Columns for one tab: the excluded list only ever shows a line count.
pub fn view_columns(view: View, options: &Options) -> Vec<Column> {
    match view {
        View::Excluded => vec![Column {
            key: "lines",
            width: 90,
        }],
        _ => table_columns(options),
    }
}

pub struct Row {
    pub label: String,
    pub values: Vec<u32>,
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
                rows.push(Row {
                    label: display_path(entry.path(), cwd),
                    values: columns.iter().map(|c| entry.column(c.key)).collect(),
                });
            }
        }
        View::Modules => {
            for (label, stats, _) in scan::module_groups(&report.business_entries(), cwd) {
                rows.push(Row {
                    label: shorten_module(&label),
                    values: columns.iter().map(|c| stats.column(c.key)).collect(),
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
                });
            }
        }
    }
    rows
}

/// Drop the `./` that a root-level module inherits from the relative path.
fn shorten_module(label: &str) -> String {
    label.strip_prefix("./").unwrap_or(label).to_string()
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
    let title = if column >= 0 {
        match view_columns(view, &report.options).get(column as usize) {
            Some(entry) => column_title(entry.key),
            None => view.label_title(),
        }
    } else {
        view.label_title()
    };
    fill(
        t.hint_showing,
        &[
            ("shown", human_int(shown as u64)),
            ("matched", human_int(matched as u64)),
            ("column", title.to_string()),
            ("arrow", if descending { "▼" } else { "▲" }.to_string()),
        ],
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
        let left = a.values.get(index).copied().unwrap_or(0);
        let right = b.values.get(index).copied().unwrap_or(0);
        if descending {
            right.cmp(&left)
        } else {
            left.cmp(&right)
        }
    });
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

/// Persisted UI state; a tiny key=value file, no serialisation dependency.
#[derive(Clone)]
pub struct Settings {
    pub roots: String,
    pub gitignore: bool,
    pub default_ignores: bool,
    pub include_tests: bool,
    pub complexity: bool,
    pub uloc: bool,
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
}
