//! Presentation model: report data in, table rows and cards out.
//!
//! Mirrors `python/src/rust_loc/ui_model.py` but only carries what the Win32
//! shell needs.

use std::path::{Path, PathBuf};

use crate::authors::{AuthorRow, Share};
use crate::baseline;
use crate::history;
use crate::options::{Format, Options};
use crate::report::{FunctionRow, Report};
use crate::scan;

use crate::gui::i18n::{self, fill};
use crate::gui::theme;

/// The three panes the right side switches between.
///
/// Each pane owns the whole content area, so the charts and the tables never
/// share a column of pixels: picking a pane is picking what the window shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Group {
    /// Who still owns the lines: the `git blame` tables.
    Authors,
    /// Line composition, the module chart and the KPI cards.
    Overview,
    /// The line counters: per file, module, function and smell.
    Code,
    /// The settings screen: a second-level grouping on the left, the settings
    /// of the group in front on the right.
    Settings,
}

impl Group {
    /// Every pane, in the order its strip shows them. The three that came
    /// first keep their index, so a settings file an older build wrote still
    /// opens on the pane it named.
    pub const ALL: [Group; 4] = [
        Group::Overview,
        Group::Authors,
        Group::Code,
        Group::Settings,
    ];

    /// Caption of one pane in the strip above the content.
    pub fn tab_title(self) -> &'static str {
        let t = i18n::t();
        match self {
            Group::Authors => t.tab_authors,
            Group::Overview => t.group_overview,
            Group::Code => t.group_code,
            Group::Settings => t.group_settings,
        }
    }

    pub fn from_index(index: usize) -> Group {
        *Group::ALL.get(index).unwrap_or(&Group::Code)
    }

    pub fn index(self) -> usize {
        Group::ALL
            .iter()
            .position(|group| *group == self)
            .unwrap_or(2)
    }
}

/// The second-level grouping inside the settings screen. Each page owns the
/// settings that belong together, so a reader finds one thing where they look
/// for it instead of scrolling a form that grew.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SettingsPage {
    /// Which languages a scan reads.
    Languages,
    /// What a scan reads and what it skips.
    Filters,
    /// What the report adds on top of the counters.
    Report,
}

impl SettingsPage {
    /// Every page, in the order the settings screen lists them.
    pub const ALL: [SettingsPage; 3] = [
        SettingsPage::Languages,
        SettingsPage::Filters,
        SettingsPage::Report,
    ];

    /// Caption of one page in the settings screen's own strip.
    pub fn tab_title(self) -> &'static str {
        let t = i18n::t();
        match self {
            SettingsPage::Languages => t.group_languages,
            SettingsPage::Filters => t.group_filters,
            SettingsPage::Report => t.group_report,
        }
    }

    /// The sentence under the page caption, naming what the page holds.
    pub fn hint(self) -> &'static str {
        let t = i18n::t();
        match self {
            SettingsPage::Languages => t.languages_hint,
            SettingsPage::Filters => t.filters_hint,
            SettingsPage::Report => t.report_hint,
        }
    }

    pub fn from_index(index: usize) -> SettingsPage {
        *SettingsPage::ALL
            .get(index)
            .unwrap_or(&SettingsPage::Languages)
    }

    pub fn index(self) -> usize {
        SettingsPage::ALL
            .iter()
            .position(|page| *page == self)
            .unwrap_or(0)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Files,
    Modules,
    Authors,
    AuthorTotals,
    Functions,
    Smells,
    Excluded,
}

impl View {
    /// The author pane's strip: the whole-scan tally, then the file breakdown
    /// it is folded from.
    pub const AUTHOR_STRIP: [View; 2] = [View::AuthorTotals, View::Authors];
    /// The code pane's strip, in the order the tables appear.
    pub const CODE_STRIP: [View; 5] = [
        View::Files,
        View::Modules,
        View::Functions,
        View::Smells,
        View::Excluded,
    ];
    /// Longest strip, so the strip rectangles can be a fixed size array.
    pub const MAX_STRIP: usize = 5;
    /// Every view, in the order the settings file records them.
    ///
    /// The six tables an older `view=` could already name come first, so a
    /// file written by an older build still selects the table it always did.
    pub const ALL: [View; 7] = [
        View::Files,
        View::Modules,
        View::Authors,
        View::Functions,
        View::Smells,
        View::Excluded,
        View::AuthorTotals,
    ];

    /// The tables one pane can show, in strip order. The overview and the
    /// settings screens have none: their content is not a table.
    pub fn strip(group: Group) -> &'static [View] {
        match group {
            Group::Authors => &View::AUTHOR_STRIP,
            Group::Code => &View::CODE_STRIP,
            Group::Overview | Group::Settings => &[],
        }
    }

    pub fn from_strip(group: Group, index: usize) -> View {
        let strip = View::strip(group);
        *strip
            .get(index)
            .unwrap_or_else(|| strip.first().unwrap_or(&View::Files))
    }

    /// Where this view sits in its own pane's strip.
    pub fn strip_index(self) -> usize {
        View::strip(self.group())
            .iter()
            .position(|view| *view == self)
            .unwrap_or(0)
    }

    /// The pane this view is shown in.
    pub fn group(self) -> Group {
        match self {
            View::Authors | View::AuthorTotals => Group::Authors,
            _ => Group::Code,
        }
    }

    /// Whether a row of this view points at something the shell can open: the
    /// author tally has one row per person, so there is no file behind it.
    pub fn opens_files(self) -> bool {
        self != View::AuthorTotals
    }

    /// Whether the table leads with a column of row numbers, so a reader can
    /// point at a row the way a report or a chart names one.
    pub fn numbers_rows(self) -> bool {
        self == View::AuthorTotals
    }

    /// The column a click on a header item sorts on, or `None` when the item
    /// sorts nothing.
    ///
    /// The label column owns item zero and the report's own columns follow it.
    /// A table that numbers its rows puts the number column in front of the
    /// label, and that column counts the rows instead of ranking them, so the
    /// one click it takes is the one the table ignores.
    pub fn header_item_column(self, item: i32) -> Option<i32> {
        if self.numbers_rows() {
            if item == 0 {
                None
            } else {
                Some(item - 2)
            }
        } else {
            Some(item - 1)
        }
    }

    pub fn label_title(self) -> &'static str {
        let t = i18n::t();
        match self {
            View::Files => t.label_file,
            View::Modules => t.label_module,
            View::Authors => t.label_file,
            View::AuthorTotals => t.label_author,
            View::Functions => t.label_function,
            View::Smells => t.label_file,
            View::Excluded => t.label_test_only_file,
        }
    }

    /// Tab caption in the pane's segmented strip.
    pub fn tab_title(self) -> &'static str {
        let t = i18n::t();
        match self {
            View::Files => t.tab_files,
            View::Modules => t.tab_modules,
            View::Authors => t.tab_author_files,
            View::AuthorTotals => t.tab_author_totals,
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
            View::Authors => t.table_authors,
            View::AuthorTotals => t.table_author_totals,
            View::Functions => t.table_functions,
            View::Smells => t.table_smells,
            View::Excluded => t.table_excluded,
        }
    }

    /// Cue text for the filter box. Every table filters on a path except the
    /// author tally, whose first column is the author name.
    pub fn filter_cue(self) -> &'static str {
        match self {
            View::AuthorTotals => i18n::t().filter_cue_author,
            _ => i18n::t().filter_cue,
        }
    }

    pub fn from_index(index: usize) -> View {
        *View::ALL.get(index).unwrap_or(&View::Files)
    }

    pub fn index(self) -> usize {
        View::ALL.iter().position(|view| *view == self).unwrap_or(0)
    }
}

/// A table column: stats key and design-pixel width.
#[derive(Clone, Copy)]
pub struct Column {
    pub key: &'static str,
    pub width: i32,
}

impl Column {
    /// Design-pixel width the column asks for in the active language. The
    /// Chinese smell headers carry the code token plus a measure word, so they
    /// need more room than the bare English tokens do.
    pub fn pixels(self) -> i32 {
        if i18n::lang() == i18n::Lang::En {
            return self.width;
        }
        match self.key {
            "unwrap" => 100,
            "expect" => 96,
            "panic" => 92,
            "todo" => 88,
            "unimplemented" => 92,
            "allow" => 88,
            _ => self.width,
        }
    }
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
        "location" => t.col_fn_where,
        "unwrap" => t.col_unwrap,
        "expect" => t.col_expect,
        "panic" => t.col_panic,
        "todo" => t.col_todo,
        "unimplemented" => t.col_unimplemented,
        "allow" => t.col_allow,
        "smells" => t.col_smells,
        "files" => t.col_files,
        "author" => t.col_author,
        "delta" => t.col_delta,
        "no" => t.col_index,
        _ => "",
    }
}

/// Key of the row-number column the tables that number their rows lead with.
///
/// It is the one column whose text is table state rather than report data: a
/// number is the row's place in the table, which the filter and the limit
/// say, so the shell writes it while it fills the table instead of reading it
/// out of a `Row`.
pub const NUMBER_KEY: &str = "no";

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
        View::AuthorTotals => vec![
            Column {
                key: "files",
                width: 64,
            },
            Column {
                key: "lines",
                width: 74,
            },
            Column {
                key: "code",
                width: 74,
            },
            Column {
                key: "comment",
                width: 78,
            },
            Column {
                key: "blank",
                width: 70,
            },
            Column {
                key: "test",
                width: 70,
            },
        ],
        View::Authors => vec![
            Column {
                key: "author",
                width: 176,
            },
            Column {
                key: "lines",
                width: 74,
            },
            Column {
                key: "code",
                width: 74,
            },
            Column {
                key: "comment",
                width: 78,
            },
            Column {
                key: "blank",
                width: 70,
            },
            Column {
                key: "test",
                width: 70,
            },
        ],
        View::Functions => vec![
            Column {
                key: "location",
                width: 200,
            },
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

/// Columns that carry text instead of a counter: the file and line a
/// function was found at, and the author name the blame credited a line to.
pub fn is_text_column(key: &str) -> bool {
    key == "location" || key == "author"
}

/// Widths for the columns that hold text, in draw order: the label column
/// first, then one per text column. `natural` is the width each one needs for
/// the widest text it holds, `available` the room they all share.
///
/// Every column keeps the width its own content asks for and the longest one
/// takes what the rest leave, so a row of short author names never grows a
/// column wider than the paths beside it, and the table still fills the
/// window. When the content needs more room than there is, every column gives
/// way in proportion to the text it holds instead of one of them carrying the
/// shortage alone.
pub fn flex_widths(natural: &[i32], available: i32) -> Vec<i32> {
    /// Nothing shrinks past the room the narrowest readable cell needs.
    const MINIMUM: i32 = 96;
    let mut widths = natural.to_vec();
    if widths.is_empty() {
        return widths;
    }
    let total: i32 = widths.iter().sum();
    if total > available {
        // Not enough room: every column gives way in proportion to the text it
        // holds, so the long ones pay for the shortage together instead of one
        // of them carrying it alone.
        let scale = f64::from(available.max(0)) / f64::from(total);
        for width in widths.iter_mut() {
            *width = ((f64::from(*width) * scale) as i32).max(MINIMUM);
        }
        return widths;
    }
    let mut widest = 0usize;
    for (index, width) in widths.iter().enumerate() {
        if *width > widths[widest] {
            widest = index;
        }
    }
    widths[widest] += available - total;
    widths
}

/// Column a tab sorts on until the user clicks a header: worst first for the
/// function list, most code first for the file and module lists.
pub fn default_sort(view: View) -> (i32, bool) {
    match view {
        View::Authors => (-1, false),
        View::AuthorTotals => (1, true),
        View::Functions => (2, true),
        View::Smells => (6, true),
        View::Excluded => (0, true),
        _ => (3, true),
    }
}

/// One cell of a table row. The functions tab keeps the file and line a
/// function was found at as text; every other cell is a counter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cell {
    Number(u32),
    Text(String),
}

impl Cell {
    /// The counter a cell holds; a text cell counts as zero.
    pub fn number(&self) -> i64 {
        match self {
            Cell::Number(number) => i64::from(*number),
            Cell::Text(_) => 0,
        }
    }

    /// The text a cell holds; a counter cell reads as empty.
    pub fn text(&self) -> &str {
        match self {
            Cell::Text(value) => value,
            Cell::Number(_) => "",
        }
    }
}

pub struct Row {
    pub label: String,
    pub cells: Vec<Cell>,
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
pub use crate::settings::parse_paths;

pub fn display_path(path: &Path, cwd: &Path) -> String {
    scan::relative(path, cwd)
}

/// Where the table measures paths from: the scan root itself when the run has
/// one, so a report reads from the tree the user picked instead of the drive
/// letter wherever the window was started from. A run with several roots keeps
/// the working directory, because that is what tells the roots apart.
pub fn label_base(report: &Report, cwd: &Path) -> PathBuf {
    match report.roots.as_slice() {
        [root] => root.clone(),
        _ => cwd.to_path_buf(),
    }
}

/// Path text a row prints: below the base when it can be, and the path itself
/// when the two have nothing in common. The report may have printed the path
/// against the working directory, so a relative one is resolved there first.
fn label_text(text: &str, base: &Path, cwd: &Path) -> String {
    let path = Path::new(text);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    scan::relative(&absolute, base)
}

/// Absolute path a row points at: the path it carries when it has one, and
/// otherwise its label read as a path below the scan root the table is drawn
/// from. The label and this agree by construction, so opening a row reaches
/// the file the row names.
pub fn row_path(row: &Row, report: Option<&Report>, cwd: &Path) -> PathBuf {
    if let Some(target) = row.target.as_deref() {
        return cwd.join(target);
    }
    let base = report
        .map(|report| label_base(report, cwd))
        .unwrap_or_else(|| cwd.to_path_buf());
    let label = row.label.trim_start_matches("./").trim();
    if label.is_empty() || label == "." {
        base
    } else {
        base.join(label)
    }
}

/// Build the visible rows for the current view, before filter and limit.
///
/// Labels are drawn from the scan root, so the same tree reads the same way no
/// matter which directory the window was started from.
pub fn build_rows(report: &Report, view: View, columns: &[Column], cwd: &Path) -> Vec<Row> {
    let base = label_base(report, cwd);
    let base = base.as_path();
    let mut rows = Vec::new();
    match view {
        View::Files => {
            for entry in report.business_entries() {
                let delta = report
                    .diff
                    .as_ref()
                    .and_then(|diff| diff.code_delta(&baseline::key(entry, &report.roots, cwd)));
                rows.push(Row {
                    label: scan::relative(entry.path(), base),
                    cells: counter_cells(entry, columns),
                    target: None,
                    delta,
                });
            }
        }
        View::Modules => {
            for (label, stats, _) in scan::module_groups(&report.business_entries(), base) {
                rows.push(Row {
                    label: shorten_module(&label),
                    cells: columns
                        .iter()
                        .map(|c| Cell::Number(stats.column(c.key)))
                        .collect(),
                    target: None,
                    delta: None,
                });
            }
        }
        View::Authors => {
            if let Some(authors) = &report.authors {
                for file in &authors.files {
                    let label = scan::relative(&file.path, base);
                    for share in &file.authors {
                        if share.lines.business() == 0 {
                            continue;
                        }
                        rows.push(Row {
                            label: label.clone(),
                            cells: columns
                                .iter()
                                .map(|column| share_cell(share, column.key))
                                .collect(),
                            target: Some(file.path.display().to_string()),
                            delta: None,
                        });
                    }
                }
            }
        }
        View::AuthorTotals => {
            if let Some(authors) = &report.authors {
                for author in &authors.rows {
                    rows.push(Row {
                        label: author.name.clone(),
                        cells: columns
                            .iter()
                            .map(|column| author_cell(author, column.key))
                            .collect(),
                        target: None,
                        delta: None,
                    });
                }
            }
        }
        View::Functions => {
            for function in &report.functions {
                let location = label_text(&function.path, base, cwd);
                rows.push(Row {
                    label: function.name.clone(),
                    cells: columns
                        .iter()
                        .map(|column| function_cell(function, &location, column.key))
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
                    label: scan::relative(entry.path(), base),
                    cells: counter_cells(entry, columns),
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
                    label: label_text(&path, base, cwd),
                    cells: vec![Cell::Number(lines)],
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
fn counter_cells(entry: &scan::Entry, columns: &[Column]) -> Vec<Cell> {
    columns
        .iter()
        .filter(|column| column.key != "delta")
        .map(|column| Cell::Number(entry.column(column.key)))
        .collect()
}

fn shorten_module(label: &str) -> String {
    label.strip_prefix("./").unwrap_or(label).to_string()
}

/// One cell of the functions tab: the name is the row label, the file and line
/// is the text column, the counters follow.
fn function_cell(function: &FunctionRow, location: &str, key: &str) -> Cell {
    match key {
        "location" => Cell::Text(format!("{}:{}", location, function.line)),
        "code" => Cell::Number(function.code),
        "complexity" => Cell::Number(function.complexity),
        "args" => Cell::Number(function.params),
        "depth" => Cell::Number(function.depth),
        _ => Cell::Number(0),
    }
}

/// One cell of the author tally: what one person still holds, by line class.
fn author_cell(author: &AuthorRow, key: &str) -> Cell {
    match key {
        "files" => Cell::Number(author.files as u32),
        "lines" => Cell::Number(author.lines.business()),
        "code" => Cell::Number(author.lines.code),
        "comment" => Cell::Number(author.lines.comment),
        "blank" => Cell::Number(author.lines.blank),
        "test" => Cell::Number(author.lines.test),
        _ => Cell::Number(0),
    }
}

/// One cell of a per-file author row: the name is the text column, then the
/// same line classes the tally shows.
fn share_cell(share: &Share, key: &str) -> Cell {
    match key {
        "author" => Cell::Text(share.name.clone()),
        "lines" => Cell::Number(share.lines.business()),
        "code" => Cell::Number(share.lines.code),
        "comment" => Cell::Number(share.lines.comment),
        "blank" => Cell::Number(share.lines.blank),
        "test" => Cell::Number(share.lines.test),
        _ => Cell::Number(0),
    }
}

/// Biggest modules by code, ready for the bar chart.
pub fn top_modules(report: &Report, cwd: &Path, limit: usize) -> Vec<(String, u32)> {
    let base = label_base(report, cwd);
    let rows: Vec<(String, u32)> = scan::module_groups(&report.business_entries(), &base)
        .iter()
        .take(limit)
        // The chart and the table name a module the same way, so the label
        // shortening below only ever drops a prefix both really share.
        .map(|(label, stats, _)| (shorten_module(label), stats.code))
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
    } else if matches!(view, View::Authors | View::AuthorTotals) {
        let files = report
            .authors
            .as_ref()
            .map(|data| data.files.len())
            .unwrap_or(0);
        let authors = report
            .authors
            .as_ref()
            .map(|data| data.rows.len())
            .unwrap_or(0);
        if files == 0 {
            return t.hint_no_authors.to_string();
        }
        fill(
            t.hint_authors,
            &[
                ("authors", human_int(authors as u64)),
                ("files", human_int(files as u64)),
                ("shown", human_int(shown as u64)),
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
pub use crate::settings::parse_list;

/// Stable value sort on top of an ascending label order. A text cell sorts by
/// its text, which is how the location column reads; the trailing delta column
/// has no cell of its own and sorts on the signed change.
pub fn sort_rows(rows: &mut [Row], column: i32, descending: bool) {
    if column < 0 {
        rows.sort_by_key(|row| row.label.to_lowercase());
        return;
    }
    let index = column as usize;
    rows.sort_by_key(|row| row.label.to_lowercase());
    rows.sort_by(|a, b| {
        let order = compare_rows(a, b, index);
        if descending {
            order.reverse()
        } else {
            order
        }
    });
}

fn compare_rows(left: &Row, right: &Row, index: usize) -> std::cmp::Ordering {
    match (left.cells.get(index), right.cells.get(index)) {
        (Some(Cell::Text(a)), Some(Cell::Text(b))) => a.to_lowercase().cmp(&b.to_lowercase()),
        (Some(a), Some(b)) => a.number().cmp(&b.number()),
        _ => left.delta.unwrap_or(0).cmp(&right.delta.unwrap_or(0)),
    }
}

pub fn filter_rows(rows: &[Row], query: &str) -> Vec<usize> {
    let needle = query.trim().to_lowercase();
    rows.iter()
        .enumerate()
        .filter(|(_, row)| {
            needle.is_empty()
                || row.label.to_lowercase().contains(&needle)
                || row
                    .cells
                    .iter()
                    .any(|cell| cell.text().to_lowercase().contains(&needle))
        })
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
    let sources = report.options.sources;
    let test_lines = report.excluded.file_lines as u64 + report.excluded.item_lines as u64;
    let mut cards = vec![
        Card {
            key: "files",
            label: t.card_files,
            value: human_int(report.business.len() as u64),
            hint: fill(
                t.hint_files_scanned,
                &[
                    ("n", human_int(report.entries.len() as u64)),
                    ("ext", i18n::extensions(sources)),
                ],
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
        hint: i18n::test_item(sources),
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
    let t = i18n::t();
    let template = if report.options.include_tests {
        t.status_scanned_tests
    } else {
        t.status_scanned
    };
    fill(
        template,
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
    // A scan that reads something other than Rust says so, the way the report
    // does, so the status bar never claims the wrong language.
    if options.sources != crate::sources::Sources::RUST {
        parts.push(options.sources.label());
    }
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
    // A selection other than Rust-only has to be spelled out, or the command
    // a reader copies would scan a different tree than the one on screen.
    if options.sources != crate::sources::Sources::RUST {
        parts.push("--sources".to_string());
        parts.push(options.sources.slug());
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
    if options.authors {
        parts.push("--authors".to_string());
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
        Format::Html => parts.push("--format html".to_string()),
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
pub use crate::settings::history_path;

/// Persisted UI state, shared with the web front end: the struct and its
/// key=value file live in `crate::settings` so both shells read the same file.
pub use crate::settings::{
    load_settings, save_settings, settings_path, sources_of, sources_slugs, Settings,
    SOURCE_LANGS,
};

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
            ["location", "code", "complexity", "args", "depth"]
        );
        let rows = build_rows(&report, View::Functions, &columns, &scratch.path);
        assert!(rows.len() >= 3, "{}", rows.len());
        let first = &rows[0];
        assert_eq!(first.label, report.functions[0].name);
        assert_eq!(
            first.cells[0],
            Cell::Text("src/lib.rs:5".to_string()),
            "the file and line sit in their own column"
        );
        assert_eq!(
            first.cells[2].number(),
            i64::from(report.functions[0].complexity)
        );
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
    fn labels_start_at_the_scan_root_not_the_drive_letter() {
        i18n::set_lang(i18n::Lang::En);
        let scratch = Scratch::new("label-base");
        scratch.write("src/lib.rs", SOURCE);
        let options = Options {
            functions: true,
            ..Options::default()
        };
        // A window started from a shortcut runs somewhere else entirely, so the
        // scan is asked for its paths from there and the table still names the
        // files from the tree that was scanned.
        let elsewhere = Path::new("C:/elsewhere");
        let report = crate::report::scan(
            std::slice::from_ref(&scratch.path),
            &options,
            elsewhere,
            None,
        );

        let columns = view_columns(View::Files, &options, false);
        let rows = build_rows(&report, View::Files, &columns, elsewhere);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "src/lib.rs");
        assert_eq!(
            row_path(&rows[0], Some(&report), elsewhere),
            scratch.path.join("src/lib.rs"),
            "opening a row has to reach the file the row names"
        );

        let columns = view_columns(View::Modules, &options, false);
        let rows = build_rows(&report, View::Modules, &columns, elsewhere);
        assert_eq!(rows[0].label, "src");

        let columns = view_columns(View::Functions, &options, false);
        let rows = build_rows(&report, View::Functions, &columns, elsewhere);
        assert!(!rows.is_empty());
        for row in &rows {
            let location = row.cells[0].text().to_string();
            assert!(location.starts_with("src/lib.rs:"), "{location}");
        }
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
    fn the_card_hints_name_the_language_that_was_scanned() {
        i18n::set_lang(i18n::Lang::En);
        let scratch = Scratch::new("hints_lang");
        scratch.write("src/lib.rs", SOURCE);
        scratch.write("app.py", "def test_one():\n    assert True\n");
        for (sources, files, test_fns) in [
            (
                crate::sources::Sources::RUST,
                "1 .rs files scanned",
                "#[test] functions",
            ),
            (
                crate::sources::Sources::PYTHON,
                "1 .py files scanned",
                "def test_ functions",
            ),
            (
                crate::sources::Sources::parse("rust,python").unwrap(),
                "2 .rs .py files scanned",
                "test functions",
            ),
        ] {
            let options = Options {
                sources,
                ..Options::default()
            };
            let report = scratch.scan(&options);
            let cards = summary_cards(Some(&report), &options);
            let hint = |key: &str| {
                cards
                    .iter()
                    .find(|card| card.key == key)
                    .unwrap()
                    .hint
                    .clone()
            };
            assert_eq!(hint("files"), files, "{sources:?}");
            assert_eq!(hint("test_fns"), test_fns, "{sources:?}");
        }
    }

    #[test]
    fn a_saved_setting_round_trips_through_the_language_slugs() {
        // A setting is a comma separated list of slugs, and one written by an
        // older build is a bare index of the three choices it offered.
        for (saved, selection) in [
            ("rust", crate::sources::Sources::RUST),
            ("python", crate::sources::Sources::PYTHON),
            (
                "rust,python",
                crate::sources::Sources::parse("rust,python").unwrap(),
            ),
            ("go", crate::sources::Sources::parse("go").unwrap()),
            ("0", crate::sources::Sources::RUST),
            ("1", crate::sources::Sources::PYTHON),
            ("2", crate::sources::Sources::parse("rust,python").unwrap()),
        ] {
            assert_eq!(sources_of(saved), selection, "{saved}");
            assert_eq!(sources_slugs(selection), selection.slug());
        }
        // An empty or nonsense value reads as the Rust default rather than as
        // a scan of nothing.
        assert_eq!(sources_of(""), crate::sources::Sources::RUST);
        assert_eq!(sources_of("cobol"), crate::sources::Sources::RUST);
        assert_eq!(sources_slugs(crate::sources::Sources::RUST), "rust");
        assert_eq!(sources_slugs(crate::sources::Sources::ALL), "all");
    }

    #[test]
    fn a_command_line_names_the_languages_that_were_read() {
        let scratch = Scratch::new("cli_command_langs");
        scratch.write("src/lib.rs", SOURCE);
        scratch.write("cmd/main.go", "package main\n");
        let cwd = scratch.path.clone();
        let rust = scratch.scan(&Options::default());
        assert!(!cli_command(&rust, &cwd).contains("--sources"));
        let mixed = scratch.scan(&Options {
            sources: crate::sources::Sources::parse("rust,go").unwrap(),
            ..Options::default()
        });
        let command = cli_command(&mixed, &cwd);
        assert!(command.contains("--sources rust,go"), "{}", command);
        let go = scratch.scan(&Options {
            sources: crate::sources::Sources::parse("go").unwrap(),
            ..Options::default()
        });
        assert!(cli_command(&go, &cwd).contains("--sources go"));
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
        let sorted = table_hint(View::Functions, Some(&report), 0, 0, 4, true);
        assert!(sorted.contains("sorted by Depth ▼"), "{sorted}");
    }

    #[test]
    fn a_header_item_names_the_column_under_it() {
        // The label column is item zero, so the counters start at one.
        assert_eq!(View::Files.header_item_column(0), Some(-1));
        assert_eq!(View::Files.header_item_column(1), Some(0));
        assert_eq!(View::Files.header_item_column(7), Some(6));
        // The number column leads the tables that number their rows, so the
        // label moves to item one and the counters to item two.
        assert_eq!(View::AuthorTotals.header_item_column(1), Some(-1));
        assert_eq!(View::AuthorTotals.header_item_column(2), Some(0));
        // A number that counts the rows ranks nothing, so its header sorts no.
        assert_eq!(View::AuthorTotals.header_item_column(0), None);
    }

    #[test]
    fn only_the_author_tally_numbers_its_rows() {
        assert!(View::AuthorTotals.numbers_rows());
        for view in View::ALL {
            assert_eq!(view.numbers_rows(), view == View::AuthorTotals, "{view:?}");
        }
    }

    #[test]
    fn every_tab_has_a_natural_sort_column() {
        assert_eq!(default_sort(View::Functions), (2, true));
        assert_eq!(default_sort(View::Smells), (6, true));
        assert_eq!(default_sort(View::Files), (3, true));
        assert_eq!(default_sort(View::Modules), (3, true));
        // The authors tab reads by file, not by counter.
        assert_eq!(default_sort(View::Authors), (-1, false));
        assert_eq!(default_sort(View::AuthorTotals), (1, true));
    }

    /// The text columns share the room the counters leave: the one whose text
    /// is widest takes what the others do not need.
    #[test]
    fn the_widest_text_column_takes_the_room_the_others_leave() {
        assert_eq!(flex_widths(&[420, 150], 900), vec![750, 150]);
        // The room follows the widest column, not the first one.
        assert_eq!(flex_widths(&[140, 460], 900), vec![140, 760]);
        // A single column fills the row on its own.
        assert_eq!(flex_widths(&[420], 900), vec![900]);
        assert!(flex_widths(&[], 900).is_empty());
    }

    #[test]
    fn text_columns_give_way_together_when_the_window_is_too_narrow() {
        // Both give way in proportion to what they hold, so the paths keep more
        // room than the short names beside them.
        let widths = flex_widths(&[420, 150], 400);
        assert_eq!(widths, vec![294, 105]);
        assert!(widths.iter().sum::<i32>() <= 400);
        // Nothing shrinks past the floor that keeps a cell readable.
        assert_eq!(flex_widths(&[140, 90], 120), vec![96, 96]);
    }

    /// The author tally filters on the name in its first column, so its cue
    /// must not promise a path filter.
    #[test]
    fn the_filter_cue_names_what_the_view_filters_on() {
        i18n::set_lang(i18n::Lang::En);
        assert_eq!(View::AuthorTotals.filter_cue(), "Filter authors…");
        i18n::set_lang(i18n::Lang::Zh);
        assert_eq!(View::AuthorTotals.filter_cue(), "按作者筛选…");
        for view in View::ALL {
            if view != View::AuthorTotals {
                assert_eq!(view.filter_cue(), i18n::t().filter_cue, "{view:?}");
            }
        }
    }

    #[test]
    fn view_indices_round_trip_with_the_tab_strip() {
        for (index, view) in View::ALL.iter().enumerate() {
            assert_eq!(View::from_index(index), *view);
            assert_eq!(view.index(), index);
        }
        assert_eq!(View::from_index(View::ALL.len()), View::Files);
        assert!(View::from_index(4) == View::Smells);
        assert_eq!(View::Smells.tab_title(), i18n::t().tab_smells);
        assert_eq!(View::Authors.tab_title(), i18n::t().tab_author_files);
        // An index an older build wrote still names the same table.
        assert_eq!(View::from_index(2), View::Authors);
        assert_eq!(View::from_index(5), View::Excluded);
    }

    #[test]
    fn the_panes_own_their_strips() {
        assert_eq!(View::Authors.group(), Group::Authors);
        assert_eq!(View::AuthorTotals.group(), Group::Authors);
        assert_eq!(View::Files.group(), Group::Code);
        assert!(View::strip(Group::Overview).is_empty());
        assert_eq!(View::strip(Group::Code), View::CODE_STRIP);
        assert_eq!(View::strip(Group::Authors), View::AUTHOR_STRIP);
        // The overview pane is the one with nothing to switch to.
        assert_eq!(Group::from_index(0), Group::Overview);
        assert_eq!(Group::Overview.index(), 0);
        assert_eq!(Group::Authors.index(), 1);
        assert_eq!(Group::ALL.len(), 4);
        // The panes that came later keep their index, so a settings file an
        // older build wrote still opens on the pane it named.
        assert_eq!(Group::from_index(Group::ALL.len()), Group::Code);
        assert!(View::strip(Group::Settings).is_empty());
        assert_eq!(Group::Settings.index(), 3);
        assert_eq!(Group::from_index(3), Group::Settings);
        // The settings screen's own second-level grouping.
        assert_eq!(SettingsPage::ALL.len(), 3);
        assert_eq!(SettingsPage::from_index(0), SettingsPage::Languages);
        assert_eq!(SettingsPage::Languages.index(), 0);
        assert_eq!(SettingsPage::from_index(1), SettingsPage::Filters);
        assert_eq!(SettingsPage::Filters.index(), 1);
        assert_eq!(SettingsPage::from_index(2), SettingsPage::Report);
        assert_eq!(SettingsPage::Report.index(), 2);
        assert_eq!(SettingsPage::from_index(9), SettingsPage::Languages);
        for group in Group::ALL {
            for (index, view) in View::strip(group).iter().enumerate() {
                assert_eq!(View::from_strip(group, index), *view);
                assert_eq!(view.strip_index(), index);
            }
        }
        assert_eq!(View::AuthorTotals.strip_index(), 0);
        assert_eq!(View::Authors.strip_index(), 1);
        assert!(!View::AuthorTotals.opens_files());
        assert!(View::Files.opens_files());
    }

    #[test]
    fn the_authors_tab_lists_every_author_of_every_file() {
        i18n::set_lang(i18n::Lang::En);
        let scratch = Scratch::new("authors");
        scratch.write("src/lib.rs", SOURCE);
        let options = Options::default();
        let mut report = scratch.scan(&options);
        let path = scratch.path.join("src/lib.rs");
        let ada = crate::authors::Lines {
            blank: 2,
            comment: 1,
            doc: 0,
            code: 5,
            test: 0,
        };
        let bob = crate::authors::Lines {
            blank: 0,
            comment: 0,
            doc: 0,
            code: 3,
            test: 4,
        };
        report.authors = Some(crate::authors::Authors {
            available: true,
            untracked: 0,
            files: vec![crate::authors::FileShare {
                path: path.clone(),
                authors: vec![
                    crate::authors::Share {
                        name: "Ada".to_string(),
                        email: "ada@example.com".to_string(),
                        lines: ada,
                    },
                    crate::authors::Share {
                        name: "Bob".to_string(),
                        email: "bob@example.com".to_string(),
                        lines: bob,
                    },
                ],
            }],
            rows: vec![
                crate::authors::AuthorRow {
                    name: "Ada".to_string(),
                    email: "ada@example.com".to_string(),
                    files: 1,
                    lines: ada,
                },
                crate::authors::AuthorRow {
                    name: "Bob".to_string(),
                    email: "bob@example.com".to_string(),
                    files: 1,
                    lines: bob,
                },
            ],
        });
        let columns = view_columns(View::Authors, &options, false);
        assert_eq!(
            columns.iter().map(|column| column.key).collect::<Vec<_>>(),
            ["author", "lines", "code", "comment", "blank", "test"]
        );
        assert!(is_text_column("author"));
        let rows = build_rows(&report, View::Authors, &columns, &scratch.path);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].label, "src/lib.rs");
        assert_eq!(rows[0].cells[0].text(), "Ada");
        assert_eq!(rows[0].cells[1].number(), 8);
        assert_eq!(rows[0].cells[2].number(), 5);
        assert_eq!(rows[0].cells[3].number(), 1);
        assert_eq!(rows[0].cells[4].number(), 2);
        assert_eq!(rows[0].cells[5].number(), 0);
        assert_eq!(rows[1].cells[1].number(), 3);
        assert_eq!(rows[1].cells[5].number(), 4);
        let hint = table_hint(View::Authors, Some(&report), 2, 2, -1, false);
        assert!(hint.contains("2 author(s) across 1 blamed files"), "{hint}");
        assert!(hint.contains("sorted by File ▲"), "{hint}");

        // The tally pane lists one row per author instead of one per file, and
        // folds the same numbers the file view splits apart.
        let totals = view_columns(View::AuthorTotals, &options, false);
        assert_eq!(
            totals.iter().map(|column| column.key).collect::<Vec<_>>(),
            ["files", "lines", "code", "comment", "blank", "test"]
        );
        assert_eq!(column_title("files"), i18n::t().col_files);
        assert_eq!(column_title(NUMBER_KEY), i18n::t().col_index);
        let rows = build_rows(&report, View::AuthorTotals, &totals, &scratch.path);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].label, "Ada");
        assert_eq!(rows[0].cells[0].number(), 1);
        assert_eq!(rows[0].cells[1].number(), 8);
        assert_eq!(rows[0].cells[2].number(), 5);
        assert_eq!(rows[0].cells[5].number(), 0);
        assert_eq!(rows[1].label, "Bob");
        assert_eq!(rows[1].cells[1].number(), 3);
        assert_eq!(rows[1].cells[5].number(), 4);
        assert!(rows.iter().all(|row| row.target.is_none()));

        // Without author data the tab says so instead of drawing nothing.
        report.authors = None;
        let rows = build_rows(&report, View::Authors, &columns, &scratch.path);
        assert!(rows.is_empty());
        assert_eq!(
            table_hint(View::Authors, Some(&report), 0, 0, -1, false),
            i18n::t().hint_no_authors
        );
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
        assert_eq!(rows[0].cells[0].number(), 1);
        assert_eq!(rows[0].cells[6].number(), 1);
        assert_eq!(column_title("smells"), i18n::t().col_smells);
        let hint = table_hint(View::Smells, Some(&report), 1, 1, 6, true);
        assert!(hint.contains("1 smells"), "{hint}");
        assert!(hint.contains("sorted by Total ▼"), "{hint}");
    }

    /// The smell headers used to read the same in both languages, so a Chinese
    /// window still showed English column names. They carry a measure word now,
    /// and the wider text has to fit the column, so the width follows the
    /// language too.
    #[test]
    fn smell_headers_translate_and_widen_for_chinese() {
        let keys = [
            "unwrap",
            "expect",
            "panic",
            "todo",
            "unimplemented",
            "allow",
        ];
        let smell = |key: &str| {
            view_columns(View::Smells, &Options::default(), false)
                .into_iter()
                .find(|column| column.key == key)
                .unwrap_or_else(|| panic!("no {key} column"))
        };
        i18n::set_lang(i18n::Lang::En);
        let english: Vec<(&str, i32)> = keys
            .iter()
            .map(|key| (column_title(key), smell(key).pixels()))
            .collect();
        i18n::set_lang(i18n::Lang::Zh);
        for (index, key) in keys.iter().enumerate() {
            let title = column_title(key);
            assert_ne!(title, english[index].0, "{key} is still English");
            assert!(title.contains(english[index].0), "{title} lost the token");
            assert_ne!(
                smell(key).pixels(),
                smell(key).width,
                "{key} keeps the English width in Chinese",
            );
            assert!(
                smell(key).pixels() > english[index].1,
                "{key} did not widen for the Chinese header",
            );
        }
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
            rows[0].cells.len(),
            columns.len() - 1,
            "the delta column is drawn from Row::delta, not from a cell"
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
