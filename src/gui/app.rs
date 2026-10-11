//! Application state plus the actions the window procedures drive.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use crate::baseline;
use crate::gui::sys::*;
use crate::gui::{i18n, model, paint, scan, theme, watch, window};
use crate::options::{Format, Options};
use crate::report::Report;

pub const ID_ROOTS: i32 = 1001;
pub const ID_ADD_FOLDER: i32 = 1002;
pub const ID_ADD_PATH: i32 = 1003;
pub const ID_REMOVE: i32 = 1004;
pub const ID_GITIGNORE: i32 = 1005;
pub const ID_DEFAULT_IGNORES: i32 = 1006;
pub const ID_TESTS: i32 = 1007;
pub const ID_COMPLEXITY: i32 = 1008;
pub const ID_ULOC: i32 = 1009;
pub const ID_EXCLUDE: i32 = 1010;
pub const ID_EXTRA_DIRS: i32 = 1011;
pub const ID_SCAN: i32 = 1012;
pub const ID_EXPORT: i32 = 1013;
pub const ID_FILTER: i32 = 1014;
pub const ID_TOP: i32 = 1015;
pub const ID_LIST: i32 = 1016;
pub const ID_STATUS: i32 = 1017;
pub const ID_CARDS: i32 = 1018;
pub const ID_BAR: i32 = 1019;
pub const ID_MODULES: i32 = 1020;
pub const ID_VEIL: i32 = 1021;
pub const ID_LANG: i32 = 1022;
pub const ID_EXCLUDE_GENERATED: i32 = 1023;
pub const ID_COCOMO: i32 = 1024;

/// Commands reachable from the keyboard, dispatched through an accelerator table.
pub const ID_ACCEL_SCAN: i32 = 1100;
pub const ID_ACCEL_FILTER: i32 = 1101;
pub const ID_ACCEL_ESCAPE: i32 = 1102;
pub const ID_ACCEL_ADD_FOLDER: i32 = 1103;
pub const ID_ACCEL_EXPORT: i32 = 1104;
pub const ID_ACCEL_NEXT_VIEW: i32 = 1105;
pub const ID_ACCEL_OPEN: i32 = 1106;
pub const ID_ACCEL_CTX_MENU: i32 = 1107;

/// Export menu commands; the format is `index` into `app::FORMATS`.
pub const ID_EXPORT_MENU: i32 = 1200;
pub const ID_COPY_COMMAND: i32 = 1205;
/// Row context menu commands.
pub const ID_CTX_OPEN: i32 = 1210;
pub const ID_CTX_REVEAL: i32 = 1211;
pub const ID_CTX_COPY_PATH: i32 = 1212;

/// Polls the worker while a scan runs.
pub const TIMER_PROGRESS: usize = 1;
/// Delays the scan mask so a fast scan never flashes it.
pub const TIMER_VEIL: usize = 2;
/// Debounces a rescan after an option changed.
pub const TIMER_RESCAN: usize = 3;

/// Row limits offered by the Top selector; 0 means every row.
pub const LIMITS: [usize; 4] = [100, 300, 1000, 0];
pub const FORMATS: [(&str, Format); 4] = [
    ("Text", Format::Text),
    ("JSON", Format::Json),
    ("CSV", Format::Csv),
    ("Markdown", Format::Markdown),
];

/// How many bars the module chart draws at most.
pub const CHART_ROWS: usize = 12;
/// KPI cards per row in the strip.
pub const CARD_COLUMNS: i32 = 5;
/// Function rows the GUI keeps; the table filters and sorts them in place.
pub const GUI_TOP_FUNCTIONS: usize = 200;

pub struct AppState {
    pub theme: theme::Theme,
    pub settings: model::Settings,
    pub options: Options,
    pub roots: Vec<String>,
    pub report: Option<Report>,
    /// Business files of the previous scan, so the next one can show what
    /// changed since.
    pub previous: Option<baseline::Snapshot>,
    pub rows: Vec<model::Row>,
    pub visible: Vec<usize>,
    pub columns: Vec<model::Column>,
    pub cards: Vec<model::Card>,
    pub segments: Vec<(&'static str, u32, u32)>,
    pub modules: Vec<(String, u32)>,
    pub view: model::View,
    pub sort_column: i32,
    pub sort_descending: bool,
    pub scanning: bool,
    pub progress_done: usize,
    pub progress_total: usize,
    pub progress_path: String,
    pub started: Option<Instant>,
    pub elapsed: f64,
    pub receiver: Option<Receiver<scan::Msg>>,
    /// Watches the roots so the report refreshes when files change.
    pub watcher: Option<watch::Watcher>,
    /// The roots the watcher was started for, so it restarts only when the
    /// list actually changes.
    pub watched: Vec<String>,
    pub controls: Vec<(i32, HWND)>,
    pub card_brush: HBRUSH,
    pub status: String,
    pub table_title: String,
    pub table_hint: String,
    /// Text the shell paints itself, with the rectangle it belongs in.
    pub labels: Vec<paint::Label>,
    pub header_rect: RECT,
    pub sidebar_rect: RECT,
    pub table_rect: RECT,
    pub table_title_rect: RECT,
    pub table_hint_rect: RECT,
    pub tab_rects: [RECT; 5],
}

impl AppState {
    pub fn new(hwnd: HWND) -> AppState {
        let mut settings = model::load_settings();
        let roots: Vec<String> = if settings.roots.trim().is_empty() {
            vec![default_root()]
        } else {
            model::parse_paths(&settings.roots)
        };
        settings.roots = roots.join(";");
        i18n::set_lang(i18n::Lang::from_index(settings.lang));
        let options = Options {
            include_tests: settings.include_tests,
            complexity: settings.complexity,
            uloc: settings.uloc,
            functions: true,
            top_functions: GUI_TOP_FUNCTIONS,
            // The smells tab reads counters the lexer always produces, so the
            // GUI keeps the report's smell data on for every scan and export.
            smells: true,
            exclude_generated: settings.exclude_generated,
            cocomo: settings.cocomo,
            exclude: model::parse_list(&settings.exclude),
            ignore_dirs: configured_ignore_dirs(&settings),
            gitignore: settings.gitignore,
            format: format_at(settings.format),
            ..Options::default()
        };
        let theme = theme::Theme::new(hwnd);
        let card_brush = unsafe { CreateSolidBrush(theme::CARD_BG) };
        let view = model::View::from_index(settings.view);
        // The saved column only means something on the file and module tabs;
        // the others always come up in their natural order.
        let (sort_column, sort_descending) = match view {
            model::View::Files | model::View::Modules => {
                (settings.sort_column, settings.sort_descending)
            }
            _ => model::default_sort(view),
        };
        let table_title = view.table_title().to_string();
        AppState {
            theme,
            settings,
            options,
            roots,
            report: None,
            previous: None,
            rows: Vec::new(),
            visible: Vec::new(),
            columns: Vec::new(),
            cards: Vec::new(),
            segments: Vec::new(),
            modules: Vec::new(),
            view,
            sort_column,
            sort_descending,
            scanning: false,
            progress_done: 0,
            progress_total: 0,
            progress_path: String::new(),
            started: None,
            elapsed: 0.0,
            receiver: None,
            watcher: None,
            watched: Vec::new(),
            controls: Vec::new(),
            card_brush,
            status: i18n::t().status_ready.to_string(),
            table_title,
            table_hint: String::new(),
            labels: Vec::new(),
            header_rect: RECT::default(),
            sidebar_rect: RECT::default(),
            table_rect: RECT::default(),
            table_title_rect: RECT::default(),
            table_hint_rect: RECT::default(),
            tab_rects: [RECT::default(); 5],
        }
    }

    pub fn control(&self, id: i32) -> HWND {
        self.controls
            .iter()
            .find(|(key, _)| *key == id)
            .map(|(_, hwnd)| *hwnd)
            .unwrap_or(std::ptr::null_mut())
    }

    pub fn remember(&mut self, id: i32, hwnd: HWND) {
        self.controls.push((id, hwnd));
    }

    /// Read every scope widget back into the scan options.
    pub fn sync_options(&mut self) {
        self.options.include_tests = window::checked(self.control(ID_TESTS));
        self.options.complexity = window::checked(self.control(ID_COMPLEXITY));
        self.options.uloc = window::checked(self.control(ID_ULOC));
        self.options.exclude_generated = window::checked(self.control(ID_EXCLUDE_GENERATED));
        self.options.cocomo = window::checked(self.control(ID_COCOMO));
        self.options.gitignore = window::checked(self.control(ID_GITIGNORE));
        let defaults = window::checked(self.control(ID_DEFAULT_IGNORES));
        let extra = model::parse_list(&window::get_text(self.control(ID_EXTRA_DIRS)));
        let mut dirs: BTreeSet<String> = if defaults {
            crate::scan::default_ignore_dirs()
        } else {
            BTreeSet::new()
        };
        for name in extra {
            dirs.insert(name);
        }
        self.options.ignore_dirs = dirs;
        self.options.exclude = model::parse_list(&window::get_text(self.control(ID_EXCLUDE)));
    }

    /// Roots currently listed in the sidebar.
    pub fn roots_paths(&self) -> Vec<PathBuf> {
        self.roots.iter().map(PathBuf::from).collect()
    }

    /// Follow the roots, restarting the watch only when the list changes.
    pub fn watch_roots(&mut self, hwnd: HWND) {
        if self.watcher.is_some() && self.watched == self.roots {
            return;
        }
        let roots = self.roots_paths();
        self.watched = self.roots.clone();
        self.watcher = Some(watch::start(hwnd, &roots));
    }

    /// Add a folder typed, browsed or pasted by the user; returns the label added.
    pub fn add_root(&mut self, raw: &str) -> Option<String> {
        let trimmed = raw.trim().trim_matches('"').trim();
        if trimmed.is_empty() {
            return None;
        }
        let candidate = PathBuf::from(trimmed);
        let absolute = if candidate.is_absolute() {
            candidate
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(candidate)
        };
        let text = absolute.to_string_lossy().into_owned();
        if text.is_empty() || self.roots.iter().any(|seen| seen == &text) {
            return None;
        }
        self.roots.push(text.clone());
        Some(text)
    }

    pub fn remove_root(&mut self, index: usize) {
        if index < self.roots.len() {
            self.roots.remove(index);
        }
    }

    pub fn begin_scan(&mut self) {
        if self.scanning {
            return;
        }
        self.sync_options();
        let roots = self.roots_paths();
        let missing: Vec<String> = self
            .roots
            .iter()
            .filter(|root| !PathBuf::from(root).is_dir())
            .cloned()
            .collect();
        if roots.is_empty() {
            self.status = i18n::t().status_no_root.to_string();
            window::set_status_text(self.control(ID_STATUS), &self.status);
            return;
        }
        if !missing.is_empty() {
            self.status = i18n::fill(
                i18n::t().status_not_folder,
                &[("paths", missing.join(", "))],
            );
            window::set_status_text(self.control(ID_STATUS), &self.status);
            return;
        }
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        self.settings.roots = self.roots.join(";");
        self.settings.gitignore = self.options.gitignore;
        self.settings.default_ignores = window::checked(self.control(ID_DEFAULT_IGNORES));
        self.settings.include_tests = self.options.include_tests;
        self.settings.complexity = self.options.complexity;
        self.settings.uloc = self.options.uloc;
        self.settings.exclude_generated = self.options.exclude_generated;
        self.settings.cocomo = self.options.cocomo;
        self.settings.exclude = window::get_text(self.control(ID_EXCLUDE));
        self.settings.extra_dirs = window::get_text(self.control(ID_EXTRA_DIRS));
        self.settings.view = self.view.index();
        self.settings.sort_column = self.sort_column;
        self.settings.sort_descending = self.sort_descending;
        self.scanning = true;
        self.progress_done = 0;
        self.progress_total = 0;
        self.progress_path = String::new();
        self.started = Some(Instant::now());
        self.status = i18n::t().status_scanning.to_string();
        let options = self.options.clone();
        self.receiver = Some(scan::start(roots, options, cwd));
        window::set_busy(self, true);
        window::show_veil(self, false);
    }

    /// Drain worker messages; returns true when the overlay needs a repaint.
    pub fn poll(&mut self, hwnd: HWND) {
        let mut finished = false;
        if let Some(receiver) = &self.receiver {
            loop {
                match receiver.try_recv() {
                    Ok(scan::Msg::Progress { done, total, path }) => {
                        self.progress_done = done;
                        self.progress_total = total;
                        self.progress_path = path;
                    }
                    Ok(scan::Msg::Done(report)) => {
                        let mut report = *report;
                        // Compare with the previous scan of this window,
                        // then remember this one for the next scan.
                        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                        report.diff = self
                            .previous
                            .as_ref()
                            .map(|snapshot| baseline::Diff::of(&report, snapshot, &cwd));
                        self.previous = Some(baseline::Snapshot::of(&report, &cwd));
                        self.report = Some(report);
                        finished = true;
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        finished = true;
                        break;
                    }
                }
            }
        }
        if !finished {
            window::invalidate(self.control(ID_VEIL));
            return;
        }
        if let Some(started) = self.started.take() {
            self.elapsed = started.elapsed().as_secs_f64();
        }
        self.receiver = None;
        self.scanning = false;
        window::stop_timer(hwnd, TIMER_PROGRESS);
        window::stop_timer(hwnd, TIMER_VEIL);
        window::set_busy(self, false);
        window::show_veil(self, false);
        self.rebuild();
        window::reconfigure_list(self);
        window::refresh_status(self);
        window::repaint(self, hwnd);
        self.watch_roots(hwnd);
    }

    /// Recompute columns, rows, cards, charts and the table hint.
    pub fn rebuild(&mut self) {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let has_delta = self
            .report
            .as_ref()
            .map(|report| report.diff.is_some())
            .unwrap_or(false);
        self.columns = model::view_columns(self.view, &self.options, has_delta);
        self.cards = model::summary_cards(self.report.as_ref(), &self.options);
        match &self.report {
            Some(report) => {
                self.segments = model::composition(report);
                self.modules = model::top_modules(report, &cwd, CHART_ROWS);
                self.rows = model::build_rows(report, self.view, &self.columns, &cwd);
            }
            None => {
                self.segments = Vec::new();
                self.modules = Vec::new();
                self.rows = Vec::new();
            }
        }
        if self.view != model::View::Excluded {
            if self.sort_column >= self.columns.len() as i32 {
                let (column, descending) = model::default_sort(self.view);
                self.sort_column = column;
                self.sort_descending = descending;
            }
            model::sort_rows(&mut self.rows, self.sort_column, self.sort_descending);
        }
        let query = window::get_text(self.control(ID_FILTER));
        self.visible = model::filter_rows(&self.rows, &query);
        let limit = LIMITS
            .get(window::combo_index(self.control(ID_TOP)))
            .copied()
            .unwrap_or(0);
        if limit > 0 && self.view != model::View::Excluded && self.visible.len() > limit {
            self.visible.truncate(limit);
        }
        self.table_title = self.view.table_title().to_string();
        self.table_hint = model::table_hint(
            self.view,
            self.report.as_ref(),
            self.visible.len(),
            self.rows.len(),
            self.sort_column,
            self.sort_descending,
        );
    }

    /// Switch the interface language and remember the choice.
    pub fn set_lang(&mut self, lang: i18n::Lang) {
        i18n::set_lang(lang);
        self.settings.lang = lang.index();
    }
}

/// Ignored directory names from the persisted scope switches.
fn configured_ignore_dirs(settings: &model::Settings) -> BTreeSet<String> {
    let mut dirs = if settings.default_ignores {
        crate::scan::default_ignore_dirs()
    } else {
        BTreeSet::new()
    };
    for name in model::parse_list(&settings.extra_dirs) {
        dirs.insert(name);
    }
    dirs
}

/// Current directory, skipping the `target/` build tree so a binary launched
/// straight from `cargo run` still points at the project.
fn default_root() -> String {
    let Ok(cwd) = std::env::current_dir() else {
        return String::new();
    };
    let parent_is_target = cwd
        .parent()
        .and_then(|parent| parent.file_name())
        .map(|name| name.eq_ignore_ascii_case("target"))
        .unwrap_or(false);
    let picked = if parent_is_target {
        cwd.parent()
            .and_then(|parent| parent.parent())
            .unwrap_or(&cwd)
    } else {
        cwd.as_path()
    };
    picked.to_string_lossy().into_owned()
}

pub fn format_at(index: usize) -> Format {
    FORMATS
        .get(index)
        .map(|(_, format)| *format)
        .unwrap_or(Format::Text)
}
