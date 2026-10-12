//! Application state plus the actions the window procedures drive.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use crate::baseline;
use crate::gui::sys::*;
use crate::gui::{i18n, model, paint, scan, theme, watch, window};
use crate::history;
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
pub const ID_SOURCES: i32 = 1025;
pub const ID_AUTHORS: i32 = 1026;

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
/// Export the window itself as a PNG image.
pub const ID_EXPORT_PNG: i32 = 1206;
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
pub const FORMATS: [(&str, Format); 5] = [
    ("Text", Format::Text),
    ("JSON", Format::Json),
    ("CSV", Format::Csv),
    ("Markdown", Format::Markdown),
    ("SVG", Format::Svg),
];

/// What a save still has to wait for before it can write its file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resume {
    /// Nothing is missing: write from what the app holds.
    Now,
    /// A worker is reading what the report still needs.
    Wait,
    /// There is no report yet, so the tree has to be read first.
    Scan,
}

/// The stages a job runs, in the order the mask draws them as segments: one for
/// a plain scan, one for a save that only has to read the authors, and both for
/// a save that has to read the tree before it can blame it.
pub const STAGES_SCAN: &[crate::scan::Stage] = &[crate::scan::Stage::Files];
pub const STAGES_BLAME: &[crate::scan::Stage] = &[crate::scan::Stage::Blame];
pub const STAGES_SCAN_BLAME: &[crate::scan::Stage] =
    &[crate::scan::Stage::Files, crate::scan::Stage::Blame];

/// How many bars the module chart draws at most.
pub const CHART_ROWS: usize = 12;
/// KPI cards per row in the strip.
pub const CARD_COLUMNS: i32 = 5;
/// Function rows the GUI keeps; the table filters and sorts them in place.
pub const GUI_TOP_FUNCTIONS: usize = 200;
/// The owner drawn check boxes, in the order the sidebar lays them out.
pub const CHECK_BOXES: [i32; 8] = [
    ID_GITIGNORE,
    ID_DEFAULT_IGNORES,
    ID_TESTS,
    ID_COMPLEXITY,
    ID_ULOC,
    ID_EXCLUDE_GENERATED,
    ID_COCOMO,
    ID_AUTHORS,
];

/// The slot an owner drawn check box fills in the state check table.
fn check_slot(id: i32) -> Option<usize> {
    CHECK_BOXES.iter().position(|candidate| *candidate == id)
}

/// A framed input: `frame` paints the rounded surface and `control` sits
/// inset above it, so the field keeps the native behaviour of whatever
/// widget it wraps (an edit, a list, a selector).
#[derive(Clone, Copy)]
pub struct Field {
    pub frame: HWND,
    pub control: HWND,
}

pub struct AppState {
    pub theme: theme::Theme,
    pub settings: model::Settings,
    pub options: Options,
    pub roots: Vec<String>,
    pub report: Option<Report>,
    /// Business files of the previous scan, so the next one can show what
    /// changed since.
    pub previous: Option<baseline::Snapshot>,
    /// Scans of this root set, newest last, for the history card.
    pub history: history::History,
    /// The sparkline card's numbers, empty until two scans agree on a tree.
    pub spark: Option<model::Spark>,
    pub rows: Vec<model::Row>,
    pub visible: Vec<usize>,
    pub columns: Vec<model::Column>,
    pub cards: Vec<model::Card>,
    pub segments: Vec<(&'static str, u32, u32)>,
    pub modules: Vec<(String, u32)>,
    pub view: model::View,
    /// The pane whose content the right side shows.
    pub group: model::Group,
    /// The table each pane was left on, so switching back lands where it was.
    pub group_views: [model::View; model::Group::ALL.len()],
    /// Whether the charts pane, rather than a table, is the one on show. None
    /// until the first layout, which has to place the controls whatever the
    /// pane is: they are all created visible.
    pub overview_shown: Option<bool>,
    pub sort_column: i32,
    pub sort_descending: bool,
    pub scanning: bool,
    pub progress_done: usize,
    pub progress_total: usize,
    pub progress_path: String,
    /// The stage the numbers above belong to; `None` before the first report.
    pub progress_stage: Option<crate::scan::Stage>,
    pub started: Option<Instant>,
    pub elapsed: f64,
    pub receiver: Option<Receiver<scan::Msg>>,
    /// Whether the author pass is running on its own thread, after the scan
    /// that asked for it has already been rendered.
    pub authors_running: bool,
    /// True while the mask is up for a save that is still reading, so the
    /// overlay says it is exporting rather than scanning.
    pub exporting: bool,
    /// The save waiting for the worker, as an index into `FORMATS`, held
    /// until the report it needs has been read.
    pub pending_export: Option<usize>,
    /// The stages the job behind the mask runs, in order, so the bar can show
    /// how much of it is behind and how much is still to come.
    pub plan: &'static [crate::scan::Stage],
    /// Watches the roots so the report refreshes when files change.
    pub watcher: Option<watch::Watcher>,
    /// The roots the watcher was started for, so it restarts only when the
    /// list actually changes.
    pub watched: Vec<String>,
    pub controls: Vec<(i32, HWND)>,
    /// The one pixel image list that sets the report row height.
    pub row_images: HIMAGELIST,
    pub status: String,
    pub table_title: String,
    pub table_hint: String,
    /// Text the shell paints itself, with the rectangle it belongs in.
    pub labels: Vec<paint::Label>,
    pub header_rect: RECT,
    pub sidebar_rect: RECT,
    pub table_rect: RECT,
    pub spark_rect: RECT,
    pub table_title_rect: RECT,
    pub table_hint_rect: RECT,
    pub tab_rects: [RECT; model::View::MAX_STRIP],
    /// The pane captions above the content.
    pub group_rects: [RECT; model::Group::ALL.len()],
    /// Every framed input, in the order the sidebar lays them out.
    pub fields: Vec<Field>,
    /// The control the pointer rests on; owner drawn buttons read it to
    /// light up, since they get no hover state of their own.
    pub hot: i32,
    /// The owner drawn check boxes. A box keeps no state of its own, so the
    /// app carries the flag its painter reads back.
    pub checks: [bool; CHECK_BOXES.len()],
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
            authors: settings.authors,
            exclude: model::parse_list(&settings.exclude),
            ignore_dirs: configured_ignore_dirs(&settings),
            gitignore: settings.gitignore,
            sources: model::sources_at(settings.sources),
            format: format_at(settings.format),
            ..Options::default()
        };
        let theme = theme::Theme::new(hwnd);
        // One history file per root set, so switching projects does not splice
        // two series together.
        let history_path = model::history_path(&roots);
        let history = history::History::load(&history_path).unwrap_or_else(|_| history::History {
            path: history_path,
            samples: Vec::new(),
        });
        // A table belongs to a pane, so the saved tab is only honoured when it
        // sits in the pane that is about to be shown.
        let saved = model::View::from_index(settings.view);
        let group = model::Group::from_index(settings.group);
        let mut group_views = [model::View::Files; model::Group::ALL.len()];
        group_views[model::Group::Authors.index()] = model::View::Authors;
        group_views[saved.group().index()] = saved;
        let view = if group == model::Group::Overview {
            saved
        } else {
            group_views[group.index()]
        };
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
            history,
            spark: None,
            rows: Vec::new(),
            visible: Vec::new(),
            columns: Vec::new(),
            cards: Vec::new(),
            segments: Vec::new(),
            modules: Vec::new(),
            view,
            group,
            group_views,
            overview_shown: None,
            sort_column,
            sort_descending,
            scanning: false,
            progress_done: 0,
            progress_total: 0,
            progress_path: String::new(),
            progress_stage: None,
            started: None,
            elapsed: 0.0,
            receiver: None,
            authors_running: false,
            exporting: false,
            pending_export: None,
            plan: STAGES_SCAN,
            watcher: None,
            watched: Vec::new(),
            controls: Vec::new(),
            row_images: 0,
            status: i18n::t().status_ready.to_string(),
            table_title,
            table_hint: String::new(),
            labels: Vec::new(),
            header_rect: RECT::default(),
            sidebar_rect: RECT::default(),
            table_rect: RECT::default(),
            spark_rect: RECT::default(),
            table_title_rect: RECT::default(),
            table_hint_rect: RECT::default(),
            tab_rects: [RECT::default(); model::View::MAX_STRIP],
            group_rects: [RECT::default(); model::Group::ALL.len()],
            fields: Vec::new(),
            hot: 0,
            checks: [false; CHECK_BOXES.len()],
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

    /// Pair a rounded frame with the control it wraps.
    pub fn remember_field(&mut self, frame: HWND, control: HWND) {
        self.fields.push(Field { frame, control });
    }

    /// The frame that wraps a control, when it has one.
    pub fn field_for(&self, control: HWND) -> HWND {
        self.fields
            .iter()
            .find(|field| field.control == control)
            .map(|field| field.frame)
            .unwrap_or(std::ptr::null_mut())
    }

    /// The control a frame wraps, so a frame can paint itself in the right
    /// focus state.
    pub fn frame_control(&self, frame: HWND) -> HWND {
        self.fields
            .iter()
            .find(|field| field.frame == frame)
            .map(|field| field.control)
            .unwrap_or(std::ptr::null_mut())
    }

    /// Whether one owner drawn check box is on.
    pub fn check(&self, id: i32) -> bool {
        check_slot(id)
            .map(|slot| self.checks[slot])
            .unwrap_or(false)
    }

    /// Set the flag one owner drawn check box draws from.
    pub fn set_check(&mut self, id: i32, value: bool) {
        if let Some(slot) = check_slot(id) {
            self.checks[slot] = value;
        }
    }

    /// Read every scope widget back into the scan options. The check boxes
    /// draw themselves, so their flags come from the app rather than from the
    /// control.
    pub fn sync_options(&mut self) {
        self.options.include_tests = self.check(ID_TESTS);
        self.options.complexity = self.check(ID_COMPLEXITY);
        self.options.uloc = self.check(ID_ULOC);
        self.options.exclude_generated = self.check(ID_EXCLUDE_GENERATED);
        self.options.cocomo = self.check(ID_COCOMO);
        self.options.authors = self.check(ID_AUTHORS);
        self.options.gitignore = self.check(ID_GITIGNORE);
        self.options.sources = model::sources_at(window::combo_index(self.control(ID_SOURCES)));
        let defaults = self.check(ID_DEFAULT_IGNORES);
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
        // A different root set keeps its own history, so reload it here.
        let history_path = model::history_path(&self.roots);
        if history_path != self.history.path {
            self.history =
                history::History::load(&history_path).unwrap_or_else(|_| history::History {
                    path: history_path,
                    samples: Vec::new(),
                });
        }
        self.settings.roots = self.roots.join(";");
        self.settings.gitignore = self.options.gitignore;
        self.settings.default_ignores = self.check(ID_DEFAULT_IGNORES);
        self.settings.include_tests = self.options.include_tests;
        self.settings.complexity = self.options.complexity;
        self.settings.uloc = self.options.uloc;
        self.settings.exclude_generated = self.options.exclude_generated;
        self.settings.cocomo = self.options.cocomo;
        self.settings.authors = self.options.authors;
        self.settings.exclude = window::get_text(self.control(ID_EXCLUDE));
        self.settings.extra_dirs = window::get_text(self.control(ID_EXTRA_DIRS));
        self.settings.view = self.view.index();
        self.settings.group = self.group.index();
        self.settings.sort_column = self.sort_column;
        self.settings.sort_descending = self.sort_descending;
        self.settings.sources = model::sources_index(self.options.sources);
        self.scanning = true;
        self.progress_done = 0;
        self.progress_total = 0;
        self.progress_path = String::new();
        self.progress_stage = None;
        self.started = Some(Instant::now());
        self.status = i18n::t().status_scanning.to_string();
        self.authors_running = false;
        // A plain scan runs one stage; a save that queued one runs its blame
        // behind it, and the mask says so from the start.
        self.plan = if self.pending_export.is_some() && self.options.authors {
            STAGES_SCAN_BLAME
        } else {
            STAGES_SCAN
        };
        // The author pass is not part of the scan. It is asked for by the pane
        // that shows it, so no other pane waits on a `git blame`.
        let mut options = self.options.clone();
        options.authors = false;
        self.receiver = Some(scan::start(roots, options, cwd));
        window::set_busy(self, true);
        window::show_veil(self, false);
    }

    /// Whether the report in hand could still gain its author tables: the flag
    /// is on, and nothing has blamed the files yet.
    fn wants_authors(&self) -> bool {
        self.options.authors
            && self
                .report
                .as_ref()
                .is_some_and(|report| report.authors.is_none())
    }

    /// Ask for the author pass when the pane in front is the one it fills.
    ///
    /// The pane is what makes the data worth collecting: one `git blame` per
    /// file costs more than the whole scan, so the overview and the code
    /// listings never pay for it.
    pub fn maybe_start_authors(&mut self, hwnd: HWND) {
        if matches!(self.view, model::View::Authors | model::View::AuthorTotals) {
            self.start_authors(hwnd);
        }
    }

    /// Run whatever a save still needs, and say what the caller has to wait for.
    ///
    /// A save is the one action that may not leave the author tables out, and
    /// the one place the whole job is asked for in one go: with no report yet
    /// it reads the tree first, then blames it, and the bar in the mask covers
    /// both stages. A format that carries no blame writes from what is in
    /// hand.
    pub fn begin_export(&mut self, hwnd: HWND, index: usize) -> Resume {
        if !format_at(index).carries_authors() {
            return Resume::Now;
        }
        let blame = self.options.authors;
        if self.scanning || self.authors_running {
            // Something is reading already: queue behind it, and put the mask
            // up now that somebody is waiting for a file.
            self.pending_export = Some(index);
            self.plan = match (self.scanning, blame) {
                (true, true) => STAGES_SCAN_BLAME,
                (true, false) => STAGES_SCAN,
                (false, _) => STAGES_BLAME,
            };
            self.mask_for_export();
            return Resume::Wait;
        }
        if self.report.is_none() {
            // Nothing has been read yet, so the save reads it: the scan takes
            // the first half of the bar, the blame the rest.
            self.pending_export = Some(index);
            self.exporting = true;
            self.plan = if blame {
                STAGES_SCAN_BLAME
            } else {
                STAGES_SCAN
            };
            return Resume::Scan;
        }
        if !self.wants_authors() {
            return Resume::Now;
        }
        self.pending_export = Some(index);
        self.plan = STAGES_BLAME;
        self.start_authors(hwnd);
        if !self.authors_running {
            // Nothing to blame: the report is as complete as it gets.
            self.pending_export = None;
            return Resume::Now;
        }
        self.mask_for_export();
        Resume::Wait
    }

    /// Drop a save that can no longer run - a scan that had nothing to read.
    pub fn cancel_export(&mut self) {
        self.pending_export = None;
        self.exporting = false;
    }

    /// Put the mask up over a save that is still reading, so the overlay names
    /// the export rather than a scan nobody asked for.
    fn mask_for_export(&mut self) {
        self.exporting = true;
        window::set_busy(self, true);
        window::show_veil(self, true);
    }

    /// Take the mask down and hand the window back to the user.
    fn unmask(&mut self) {
        self.exporting = false;
        window::set_busy(self, false);
        window::show_veil(self, false);
    }

    /// Blame the business files off the UI thread, unless that is already
    /// running, already done, or switched off. Whoever asks for the pass owns
    /// the mask: one behind a pane leaves the window in the user's hands, one a
    /// save is waiting on wears the overlay.
    fn start_authors(&mut self, hwnd: HWND) {
        if self.scanning || self.authors_running || !self.options.authors {
            return;
        }
        let Some(report) = &self.report else {
            return;
        };
        if report.authors.is_some() {
            return;
        }
        let targets = report.blame_targets();
        let options = report.options.clone();
        if targets.is_empty() {
            // Nothing to blame is still an answer, and the pane can say so.
            if let Some(report) = &mut self.report {
                report.options.authors = true;
                report.authors = Some(crate::authors::Authors::default());
            }
            self.rebuild();
            window::repaint(self, hwnd);
            return;
        }
        self.progress_stage = Some(crate::scan::Stage::Blame);
        self.progress_done = 0;
        self.progress_total = targets.len();
        self.progress_path.clear();
        self.authors_running = true;
        self.refresh_hint();
        self.receiver = Some(scan::start_authors(targets, options));
        window::start_timer(hwnd, TIMER_PROGRESS, 60);
    }

    /// The caption the author pass moves while it runs. It carries the only
    /// sign of the pass, because the window stays in the user's hands.
    fn refresh_hint(&mut self) {
        self.table_hint = i18n::fill(
            i18n::t().hint_authors_reading,
            &[
                ("done", self.progress_done.to_string()),
                ("total", self.progress_total.to_string()),
            ],
        );
    }

    /// Drain worker messages. Returns the save that may run now, if one was
    /// waiting: its dialog pumps messages of its own, so it cannot run while
    /// the state is held.
    pub fn poll(&mut self, hwnd: HWND) -> Option<usize> {
        let mut finished = false;
        if let Some(receiver) = &self.receiver {
            loop {
                match receiver.try_recv() {
                    Ok(scan::Msg::Progress {
                        stage,
                        done,
                        total,
                        path,
                    }) => {
                        self.progress_stage = Some(stage);
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
                        self.record_history(&report);
                        self.report = Some(report);
                        finished = true;
                        break;
                    }
                    Ok(scan::Msg::Authors(authors)) => {
                        if let Some(report) = &mut self.report {
                            // The report holds what `--authors` asks for now, so
                            // an export taken from it carries the author tables.
                            report.options.authors = true;
                            report.authors = Some(*authors);
                        }
                        self.authors_running = false;
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
            // A scan, and a pass a save is waiting on, mask the window, so the
            // bar is what moves. The pane's own pass leaves the window usable:
            // only the caption above its table moves.
            if self.authors_running && !self.exporting {
                self.refresh_hint();
                window::invalidate_hint(self, hwnd);
            } else {
                window::invalidate(self.control(ID_VEIL));
            }
            return None;
        }
        if let Some(started) = self.started.take() {
            self.elapsed = started.elapsed().as_secs_f64();
        }
        self.receiver = None;
        window::stop_timer(hwnd, TIMER_PROGRESS);
        window::stop_timer(hwnd, TIMER_VEIL);
        // Only a scan gated the toolbar and masked the window; the author pass
        // ran behind a window the user kept.
        let scanned = self.scanning;
        self.scanning = false;
        self.authors_running = false;
        // A save that is still waiting keeps the mask up for the pass behind
        // the scan it just waited for.
        let waiting = self.pending_export.is_some() && self.wants_authors();
        if (scanned || self.exporting) && !waiting {
            self.unmask();
        }
        self.rebuild();
        window::layout_state(hwnd, self);
        window::reconfigure_list(self);
        window::refresh_status(self);
        window::repaint(self, hwnd);
        if scanned {
            self.watch_roots(hwnd);
            // A scan that ran with the author pane in front is what asks for
            // the blame behind it.
            self.maybe_start_authors(hwnd);
        }
        if self.pending_export.is_some() && self.wants_authors() {
            // The save is what asked for this pass, so the mask stays up for
            // it: the plan the bar is drawn from was set when it went up.
            self.start_authors(hwnd);
            if self.authors_running {
                self.mask_for_export();
                return None;
            }
            // Nothing to blame after all, so nothing is left to wait for.
            self.unmask();
        }
        self.pending_export.take()
    }

    /// Recompute columns, rows, cards, charts and the table hint.
    pub fn rebuild(&mut self) {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        // The trend belongs to a scan: with no report the strip stays hidden
        // and the table keeps the space.
        self.spark = if self.report.is_some() {
            model::history_trend(&self.history)
        } else {
            None
        };
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
        if self.authors_running {
            self.refresh_hint();
        }
    }

    /// Add this scan to the history, unless nothing moved since the last one;
    /// a watched tree then grows a point per real change, not per tick.
    fn record_history(&mut self, report: &Report) {
        let sample = history::Sample::of(report, history::now());
        if self.history.latest().map(history::Sample::counters) == Some(sample.counters()) {
            return;
        }
        self.history.record(sample);
        let _ = self.history.save();
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
