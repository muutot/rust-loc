//! Window classes, layout and message dispatch.
//!
//! The shell is a two column layout: a fixed sidebar built from real Win32
//! controls and a panel whose chrome (header band, chart cards, tab strip,
//! KPI grid) is painted by hand. Nothing here reaches outside the system DLLs.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use crate::gui::sys::*;
use crate::gui::{app, dialog, i18n, model, paint, theme};

pub const CLASS_MAIN: &str = "RustLocMainWindow";
pub const CLASS_CARDS: &str = "RustLocCards";
pub const CLASS_BAR: &str = "RustLocBar";
pub const CLASS_MODULES: &str = "RustLocModules";
pub const CLASS_VEIL: &str = "RustLocVeil";

thread_local! {
    static STATE: RefCell<Option<Box<app::AppState>>> = const { RefCell::new(None) };
}

/// Run `callback` with the live application state.
///
/// Re-entrant calls (a control notifying us while we populate the table) are
/// dropped instead of panicking on the borrow.
pub fn with_state<R>(callback: impl FnOnce(&mut app::AppState) -> R) -> Option<R> {
    STATE.with(|cell| {
        let mut borrow = cell.try_borrow_mut().ok()?;
        borrow.as_mut().map(|state| callback(state))
    })
}

pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn send(hwnd: HWND, message: u32, wparam: usize, lparam: isize) -> isize {
    if hwnd.is_null() {
        return 0;
    }
    unsafe { SendMessageW(hwnd, message, wparam, lparam) }
}

pub fn get_text(hwnd: HWND) -> String {
    if hwnd.is_null() {
        return String::new();
    }
    unsafe {
        let length = GetWindowTextLengthW(hwnd);
        if length <= 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; length as usize + 1];
        let read = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
        String::from_utf16_lossy(&buffer[..read.max(0) as usize])
    }
}

pub fn set_text(hwnd: HWND, text: &str) {
    if hwnd.is_null() {
        return;
    }
    let buffer = wide(text);
    unsafe { SetWindowTextW(hwnd, buffer.as_ptr()) };
}

pub fn checked(hwnd: HWND) -> bool {
    send(hwnd, BM_GETCHECK, 0, 0) as usize == BST_CHECKED as usize
}

pub fn set_checked(hwnd: HWND, value: bool) {
    send(hwnd, BM_SETCHECK, if value { 1 } else { 0 }, 0);
}

pub fn combo_index(hwnd: HWND) -> usize {
    let index = send(hwnd, CB_GETCURSEL, 0, 0);
    if index < 0 {
        0
    } else {
        index as usize
    }
}

pub fn combo_selection(hwnd: HWND) -> usize {
    combo_index(hwnd)
}

pub fn combo_choose(hwnd: HWND, index: usize) {
    send(hwnd, CB_SETCURSEL, index, 0);
}

pub fn invalidate(hwnd: HWND) {
    if !hwnd.is_null() {
        unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
    }
}

pub fn start_timer(hwnd: HWND, id: usize, milliseconds: u32) {
    unsafe { SetTimer(hwnd, id, milliseconds, None) };
}

pub fn stop_timer(hwnd: HWND, id: usize) {
    unsafe { KillTimer(hwnd, id) };
}

fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
    RECT {
        left,
        top,
        right: right.max(left),
        bottom: bottom.max(top),
    }
}

/// Double buffered paint helper for the custom child windows.
fn buffered(hwnd: HWND, painter: impl FnOnce(HDC, &RECT)) {
    unsafe {
        let mut state = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut state);
        let mut client = RECT::default();
        GetClientRect(hwnd, &mut client);
        let width = client.right.max(1);
        let height = client.bottom.max(1);
        let memory = CreateCompatibleDC(hdc);
        let bitmap = CreateCompatibleBitmap(hdc, width, height);
        let previous = SelectObject(memory, bitmap as HGDIOBJ);
        painter(memory, &client);
        BitBlt(hdc, 0, 0, width, height, memory, 0, 0, SRCCOPY);
        SelectObject(memory, previous);
        DeleteObject(bitmap as HGDIOBJ);
        DeleteDC(memory);
        EndPaint(hwnd, &state);
    }
}

unsafe extern "system" fn cards_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            buffered(hwnd, |hdc, client| {
                with_state(|state| {
                    paint::fill(hdc, client, theme::PANEL_BG);
                    let gap = state.theme.px(8);
                    let padding = state.theme.px(2);
                    let cards = std::mem::take(&mut state.cards);
                    paint::cards(
                        hdc,
                        client,
                        &cards,
                        app::CARD_COLUMNS,
                        gap,
                        padding,
                        &state.theme,
                    );
                    state.cards = cards;
                });
            });
            0
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

/// Which custom painted chart a message belongs to.
#[derive(Clone, Copy)]
enum Chart {
    Composition,
    Modules,
}

/// Arm the leave notification so a tooltip disappears when the pointer exits.
fn track_leave(hwnd: HWND) {
    let mut track = TRACKMOUSEEVENT {
        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_LEAVE,
        hwndTrack: hwnd,
        dwHoverTime: 0,
    };
    unsafe { TrackMouseEvent(&mut track) };
}

/// Repaint the chart under the pointer so its tooltip follows the cursor.
fn hover_repaint(hwnd: HWND) {
    track_leave(hwnd);
    invalidate(hwnd);
}

/// Paint one chart card and its hover tooltip.
fn paint_chart(hwnd: HWND, chart: Chart) {
    buffered(hwnd, |hdc, client| {
        with_state(|state| {
            paint::fill(hdc, client, theme::PANEL_BG);
            let mut regions: Vec<(RECT, String)> = Vec::new();
            match chart {
                Chart::Composition => {
                    let segments = std::mem::take(&mut state.segments);
                    paint::composition(
                        hdc,
                        client,
                        i18n::t().chart_composition,
                        &segments,
                        &state.theme,
                        &mut regions,
                    );
                    state.segments = segments;
                }
                Chart::Modules => {
                    let modules = std::mem::take(&mut state.modules);
                    paint::modules(
                        hdc,
                        client,
                        i18n::t().chart_modules,
                        &modules,
                        &state.theme,
                        &mut regions,
                    );
                    state.modules = modules;
                }
            }
            // The tooltip is hit tested against the live cursor position, so it
            // cannot go stale when the pointer moves between two repaints.
            let mut point = POINT { x: 0, y: 0 };
            if unsafe { GetCursorPos(&mut point) } != 0 {
                unsafe { ScreenToClient(hwnd, &mut point) };
                let hit = regions.iter().find(|(area, _)| {
                    point.x >= area.left
                        && point.x < area.right
                        && point.y >= area.top
                        && point.y < area.bottom
                });
                if let Some((_, text)) = hit {
                    let hover = paint::Hover {
                        text: text.clone(),
                        x: point.x,
                        y: point.y,
                    };
                    paint::tooltip(hdc, client, &hover, &state.theme);
                }
            }
        });
    });
}

/// Shared window procedure for both charts.
fn chart_message(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    chart: Chart,
) -> LRESULT {
    match message {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            paint_chart(hwnd, chart);
            0
        }
        WM_MOUSEMOVE => {
            hover_repaint(hwnd);
            0
        }
        WM_MOUSELEAVE => {
            invalidate(hwnd);
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

unsafe extern "system" fn bar_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    chart_message(hwnd, message, wparam, lparam, Chart::Composition)
}

unsafe extern "system" fn modules_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    chart_message(hwnd, message, wparam, lparam, Chart::Modules)
}

unsafe extern "system" fn veil_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            buffered(hwnd, |hdc, client| {
                with_state(|state| {
                    let done = state.progress_done;
                    let total = state.progress_total;
                    let path = state.progress_path.clone();
                    paint::overlay(hdc, client, done, total, &path, &state.theme);
                });
            });
            0
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

struct Metrics {
    margin: i32,
    padding: i32,
    gap: i32,
    header: i32,
    status: i32,
    sidebar: i32,
    section: i32,
    control: i32,
    check: i32,
    button: i32,
    primary: i32,
    chart: i32,
    card: i32,
    tab: i32,
}

impl Metrics {
    fn new(px: impl Fn(i32) -> i32) -> Metrics {
        Metrics {
            margin: px(10),
            padding: px(14),
            gap: px(8),
            header: px(44),
            status: px(26),
            sidebar: px(264),
            section: px(18),
            control: px(24),
            check: px(22),
            button: px(26),
            primary: px(30),
            chart: px(150),
            card: px(64),
            tab: px(30),
        }
    }
}

fn label(area: RECT, text: &str, ink: paint::Ink) -> paint::Label {
    paint::Label {
        area,
        text: text.to_string(),
        ink,
    }
}

fn layout(hwnd: HWND) {
    let mut client = RECT::default();
    unsafe { GetClientRect(hwnd, &mut client) };
    let width = client.right;
    let height = client.bottom;
    with_state(|state| {
        let px = |value: i32| state.theme.px(value);
        let metrics = Metrics::new(px);
        let header = rect(0, 0, width, metrics.header);
        let sidebar = rect(0, metrics.header, metrics.sidebar, height - metrics.status);
        let panel_left = metrics.sidebar + metrics.margin;
        let panel_right = width - metrics.margin;
        let panel_top = metrics.header + metrics.margin;
        let panel_bottom = (height - metrics.status - metrics.margin).max(panel_top);

        let chart_width = ((panel_right - panel_left - metrics.gap) / 2).max(px(120));
        let bar = rect(
            panel_left,
            panel_top,
            panel_left + chart_width,
            panel_top + metrics.chart,
        );
        let modules = rect(
            panel_left + chart_width + metrics.gap,
            panel_top,
            panel_right,
            panel_top + metrics.chart,
        );
        let cards_top = panel_top + metrics.chart + metrics.gap;
        // The strip grows a row at a time: the COCOMO cards add a third one.
        let card_rows =
            ((state.cards.len() as i32 + app::CARD_COLUMNS - 1) / app::CARD_COLUMNS).max(2);
        let cards = rect(
            panel_left,
            cards_top,
            panel_right,
            cards_top + metrics.card * card_rows + metrics.gap * (card_rows - 1),
        );
        let table_top = cards.bottom + metrics.gap;
        let table = rect(panel_left, table_top, panel_right, panel_bottom);

        unsafe {
            MoveWindow(
                state.control(app::ID_BAR),
                bar.left,
                bar.top,
                bar.right - bar.left,
                bar.bottom - bar.top,
                1,
            );
            MoveWindow(
                state.control(app::ID_MODULES),
                modules.left,
                modules.top,
                modules.right - modules.left,
                modules.bottom - modules.top,
                1,
            );
            MoveWindow(
                state.control(app::ID_CARDS),
                cards.left,
                cards.top,
                cards.right - cards.left,
                cards.bottom - cards.top,
                1,
            );
        }

        let mut labels: Vec<paint::Label> = Vec::new();

        // ---- sidebar ----------------------------------------------------
        let pad = metrics.padding;
        let left = pad;
        let inner = (metrics.sidebar - pad * 2 - 1).max(px(80));
        let mut y = metrics.header + px(12);
        labels.push(label(
            rect(left, y, left + inner, y + metrics.section),
            i18n::t().section_roots,
            paint::Ink::Section,
        ));
        y += metrics.section;
        let roots_height = px(104).min((height - metrics.status - px(372)).max(px(72)));
        unsafe {
            MoveWindow(
                state.control(app::ID_ROOTS),
                left,
                y,
                inner,
                roots_height,
                1,
            )
        };
        y += roots_height + px(6);
        let small = (inner - px(4) * 2) / 3;
        for (index, id) in [app::ID_ADD_FOLDER, app::ID_ADD_PATH, app::ID_REMOVE]
            .iter()
            .enumerate()
        {
            unsafe {
                MoveWindow(
                    state.control(*id),
                    left + index as i32 * (small + px(4)),
                    y,
                    small,
                    metrics.button,
                    1,
                )
            };
        }
        y += metrics.button + px(4);

        labels.push(label(
            rect(left, y, left + inner, y + metrics.section),
            i18n::t().section_scope,
            paint::Ink::Section,
        ));
        y += metrics.section;
        for id in [
            app::ID_GITIGNORE,
            app::ID_DEFAULT_IGNORES,
            app::ID_TESTS,
            app::ID_COMPLEXITY,
            app::ID_ULOC,
        ] {
            unsafe { MoveWindow(state.control(id), left, y, inner, metrics.check, 1) };
            y += metrics.check;
        }
        y += px(2);

        labels.push(label(
            rect(left, y, left + inner, y + metrics.section),
            i18n::t().section_report,
            paint::Ink::Section,
        ));
        y += metrics.section;
        for id in [app::ID_EXCLUDE_GENERATED, app::ID_COCOMO] {
            unsafe { MoveWindow(state.control(id), left, y, inner, metrics.check, 1) };
            y += metrics.check;
        }
        y += px(2);

        labels.push(label(
            rect(left, y, left + inner, y + metrics.section),
            i18n::t().section_exclude,
            paint::Ink::Section,
        ));
        y += metrics.section;
        unsafe {
            MoveWindow(
                state.control(app::ID_EXCLUDE),
                left,
                y,
                inner,
                metrics.control,
                1,
            )
        };
        y += metrics.control + px(3);
        labels.push(label(
            rect(left, y, left + inner, y + px(14)),
            i18n::t().hint_exclude,
            paint::Ink::Note,
        ));
        y += px(14) + px(10);

        labels.push(label(
            rect(left, y, left + inner, y + metrics.section),
            i18n::t().section_extra_dirs,
            paint::Ink::Section,
        ));
        y += metrics.section;
        unsafe {
            MoveWindow(
                state.control(app::ID_EXTRA_DIRS),
                left,
                y,
                inner,
                metrics.control,
                1,
            )
        };
        y += metrics.control + px(3);
        labels.push(label(
            rect(left, y, left + inner, y + px(14)),
            i18n::t().hint_extra_dirs,
            paint::Ink::Note,
        ));

        y += px(14) + px(12);
        labels.push(label(
            rect(left, y, left + inner, y + metrics.section),
            i18n::t().section_language,
            paint::Ink::Section,
        ));
        y += metrics.section;
        unsafe { MoveWindow(state.control(app::ID_LANG), left, y, inner, px(200), 1) };

        let sidebar_bottom = sidebar.bottom - px(12);
        unsafe {
            MoveWindow(
                state.control(app::ID_EXPORT),
                left,
                sidebar_bottom - metrics.button,
                inner,
                metrics.button,
                1,
            );
            MoveWindow(
                state.control(app::ID_SCAN),
                left,
                sidebar_bottom - metrics.button - px(6) - metrics.primary,
                inner,
                metrics.primary,
                1,
            );
        }

        // ---- table card -------------------------------------------------
        let combo_width = px(84);
        let combo_left = table.right - px(12) - combo_width;
        let field_top = table.top + px(9);
        unsafe {
            MoveWindow(
                state.control(app::ID_TOP),
                combo_left,
                field_top,
                combo_width,
                px(200),
                1,
            )
        };
        let filter_right = combo_left - px(8);
        let filter_left = filter_right - px(190);
        unsafe {
            MoveWindow(
                state.control(app::ID_FILTER),
                filter_left,
                field_top,
                px(190),
                metrics.control,
                1,
            )
        };
        labels.push(label(
            rect(
                filter_left - px(46),
                field_top,
                filter_left - px(6),
                field_top + metrics.control,
            ),
            i18n::t().filter_label,
            paint::Ink::CaptionEnd,
        ));
        labels.push(label(
            rect(
                combo_left - px(40),
                field_top,
                combo_left - px(6),
                field_top + metrics.control,
            ),
            i18n::t().top_label,
            paint::Ink::CaptionEnd,
        ));

        let mut tab_rects = [RECT::default(); 4];
        let mut tab_x = table.left + px(12);
        for (index, (_view, tab_width)) in [
            (model::View::Files, px(60)),
            (model::View::Modules, px(78)),
            (model::View::Functions, px(84)),
            (model::View::Excluded, px(112)),
        ]
        .iter()
        .enumerate()
        {
            tab_rects[index] = rect(
                tab_x,
                table.top + px(32),
                tab_x + tab_width,
                table.top + px(32) + metrics.tab,
            );
            tab_x += tab_width;
        }
        let list_top = table.top + px(32) + metrics.tab + px(6);
        let list_bottom = table.bottom - px(26);
        let list_width = (table.right - table.left - px(20)).max(px(120));
        unsafe {
            MoveWindow(
                state.control(app::ID_LIST),
                table.left + px(10),
                list_top,
                list_width,
                (list_bottom - list_top).max(px(60)),
                1,
            )
        };
        state.header_rect = header;
        state.sidebar_rect = sidebar;
        state.table_rect = table;
        state.tab_rects = tab_rects;
        state.table_title_rect = rect(
            table.left + px(12),
            table.top + px(9),
            table.left + px(320),
            table.top + px(29),
        );
        state.table_hint_rect = rect(
            table.left + px(12),
            table.bottom - px(24),
            table.right - px(12),
            table.bottom - px(8),
        );
        state.labels = labels;

        let status = state.control(app::ID_STATUS);
        unsafe { MoveWindow(status, 0, height - metrics.status, width, metrics.status, 1) };
        send(status, WM_SIZE, 0, 0);
        let parts = [width * 62 / 100, -1i32];
        send(status, SB_SETPARTS, 2, parts.as_ptr() as isize);
        let veil = state.control(app::ID_VEIL);
        unsafe { MoveWindow(veil, 0, 0, width, height, 1) };
        unsafe {
            SetWindowPos(
                veil,
                HWND_TOP,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
        };
        reconfigure_list(state);
    });
}

// ---------------------------------------------------------------------------
// Table and status bar
// ---------------------------------------------------------------------------

fn set_columns(state: &app::AppState) {
    let list = state.control(app::ID_LIST);
    if list.is_null() {
        return;
    }
    let mut index = 0i32;
    while send(list, LVM_DELETECOLUMN, 0, 0) != 0 && index < 64 {
        index += 1;
    }
    let arrow = if state.sort_descending {
        " ▼"
    } else {
        " ▲"
    };
    let mut client = RECT::default();
    unsafe { GetClientRect(list, &mut client) };
    let numeric: i32 = state
        .columns
        .iter()
        .map(|column| state.theme.px(column.width))
        .sum();
    let label_width = (client.right - state.theme.px(18) - numeric).max(state.theme.px(170));
    let mut specs: Vec<(String, bool, i32)> =
        vec![(state.view.label_title().to_string(), false, label_width)];
    for (position, column) in state.columns.iter().enumerate() {
        let active = position as i32 == state.sort_column;
        let title = model::column_title(column.key);
        let title = if active {
            format!("{}{}", title, arrow)
        } else {
            title.to_string()
        };
        specs.push((title, true, state.theme.px(column.width)));
    }
    for (position, (title, numeric, width)) in specs.iter().enumerate() {
        let mut buffer = wide(title);
        let mut spec: LVCOLUMNW = unsafe { std::mem::zeroed() };
        spec.mask = LVCF_FMT | LVCF_WIDTH | LVCF_TEXT;
        spec.fmt = if *numeric { LVCFMT_RIGHT } else { LVCFMT_LEFT };
        spec.cx = *width;
        spec.pszText = buffer.as_mut_ptr();
        send(
            list,
            LVM_INSERTCOLUMNW,
            position,
            &spec as *const LVCOLUMNW as isize,
        );
    }
}

/// Rebuild the header and every row of the table.
pub fn reconfigure_list(state: &mut app::AppState) {
    set_columns(state);
    refresh_list(state);
}

/// Replace every row in the table. Only visible rows are inserted.
pub fn refresh_list(state: &mut app::AppState) {
    let list = state.control(app::ID_LIST);
    if list.is_null() {
        return;
    }
    send(list, WM_SETREDRAW, 0, 0);
    send(list, LVM_DELETEALLITEMS, 0, 0);
    for (position, index) in state.visible.iter().enumerate() {
        let Some(row) = state.rows.get(*index) else {
            continue;
        };
        let mut label = wide(&row.label);
        let mut item: LVITEMW = unsafe { std::mem::zeroed() };
        item.mask = LVIF_TEXT;
        item.iItem = position as i32;
        item.iSubItem = 0;
        item.pszText = label.as_mut_ptr();
        send(list, LVM_INSERTITEMW, 0, &item as *const LVITEMW as isize);
        for (offset, value) in row.values.iter().enumerate() {
            let mut text = wide(&model::human_int(*value as u64));
            let mut cell: LVITEMW = unsafe { std::mem::zeroed() };
            cell.iSubItem = offset as i32 + 1;
            cell.pszText = text.as_mut_ptr();
            send(
                list,
                LVM_SETITEMTEXTW,
                position,
                &cell as *const LVITEMW as isize,
            );
        }
    }
    send(list, WM_SETREDRAW, 1, 0);
    invalidate(list);
}

pub fn refresh_status(state: &mut app::AppState) {
    let status = state.control(app::ID_STATUS);
    if status.is_null() {
        return;
    }
    let (summary, scope) = match &state.report {
        Some(report) => (
            model::scan_status(report, state.elapsed),
            model::scope_line(&report.options, report.roots.len()),
        ),
        None => (state.status.clone(), String::new()),
    };
    let summary_buffer = wide(&summary);
    send(status, SB_SETTEXTW, 0, summary_buffer.as_ptr() as isize);
    let scope_buffer = wide(&scope);
    send(status, SB_SETTEXTW, 1, scope_buffer.as_ptr() as isize);
}

/// Gate the toolbar while a scan runs.
pub fn set_busy(state: &app::AppState, busy: bool) {
    for id in [
        app::ID_ROOTS,
        app::ID_ADD_FOLDER,
        app::ID_ADD_PATH,
        app::ID_REMOVE,
        app::ID_GITIGNORE,
        app::ID_DEFAULT_IGNORES,
        app::ID_TESTS,
        app::ID_COMPLEXITY,
        app::ID_ULOC,
        app::ID_EXCLUDE_GENERATED,
        app::ID_COCOMO,
        app::ID_EXCLUDE,
        app::ID_EXTRA_DIRS,
        app::ID_SCAN,
        app::ID_EXPORT,
        app::ID_LANG,
    ] {
        let control = state.control(id);
        if !control.is_null() {
            unsafe { EnableWindow(control, if busy { 0 } else { 1 }) };
        }
    }
}

/// Show or hide the scan mask.
pub fn show_veil(state: &app::AppState, visible: bool) {
    let veil = state.control(app::ID_VEIL);
    if veil.is_null() {
        return;
    }
    unsafe { ShowWindow(veil, if visible { SW_SHOW } else { SW_HIDE }) };
    if visible {
        unsafe {
            SetWindowPos(
                veil,
                HWND_TOP,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
        };
        invalidate(veil);
    }
}

/// Repaint the shell and every custom painted child.
pub fn repaint(state: &app::AppState, hwnd: HWND) {
    invalidate(hwnd);
    for id in [app::ID_CARDS, app::ID_BAR, app::ID_MODULES, app::ID_LIST] {
        invalidate(state.control(id));
    }
}

/// Replace the headline status text without touching the scope part.
pub fn set_status_text(status: HWND, text: &str) {
    if status.is_null() {
        return;
    }
    let buffer = wide(text);
    send(status, SB_SETTEXTW, 0, buffer.as_ptr() as isize);
}

// ---------------------------------------------------------------------------
// Control creation
// ---------------------------------------------------------------------------

fn create_child(
    parent: HWND,
    class: &str,
    text: &str,
    style: u32,
    ex_style: u32,
    id: i32,
    height: i32,
) -> HWND {
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let class_name = wide(class);
        let label = wide(text);
        CreateWindowExW(
            ex_style,
            class_name.as_ptr(),
            label.as_ptr(),
            style,
            0,
            0,
            10,
            height,
            parent,
            id as isize as HMENU,
            instance,
            std::ptr::null(),
        )
    }
}

fn create_controls(parent: HWND) -> Vec<(i32, HWND)> {
    let mut controls = Vec::new();
    let edit = WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL as u32;
    let button = WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32;
    let check = WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX as u32;
    let combo = WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL | CBS_DROPDOWNLIST as u32;
    let mut add = |id: i32, hwnd: HWND| {
        if !hwnd.is_null() {
            controls.push((id, hwnd));
        }
    };
    add(
        app::ID_ROOTS,
        create_child(
            parent,
            "LISTBOX",
            "",
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | WS_VSCROLL
                | LBS_NOTIFY as u32
                | LBS_NOINTEGRALHEIGHT as u32
                | WS_BORDER,
            0,
            app::ID_ROOTS,
            104,
        ),
    );
    add(
        app::ID_ADD_FOLDER,
        create_child(
            parent,
            "BUTTON",
            i18n::t().add_folder,
            button,
            0,
            app::ID_ADD_FOLDER,
            26,
        ),
    );
    add(
        app::ID_ADD_PATH,
        create_child(
            parent,
            "BUTTON",
            i18n::t().paste_path,
            button,
            0,
            app::ID_ADD_PATH,
            26,
        ),
    );
    add(
        app::ID_REMOVE,
        create_child(
            parent,
            "BUTTON",
            i18n::t().remove,
            button,
            0,
            app::ID_REMOVE,
            26,
        ),
    );
    add(
        app::ID_GITIGNORE,
        create_child(
            parent,
            "BUTTON",
            i18n::t().opt_gitignore,
            check,
            0,
            app::ID_GITIGNORE,
            22,
        ),
    );
    add(
        app::ID_DEFAULT_IGNORES,
        create_child(
            parent,
            "BUTTON",
            i18n::t().opt_default_ignores,
            check,
            0,
            app::ID_DEFAULT_IGNORES,
            22,
        ),
    );
    add(
        app::ID_TESTS,
        create_child(
            parent,
            "BUTTON",
            i18n::t().opt_include_tests,
            check,
            0,
            app::ID_TESTS,
            22,
        ),
    );
    add(
        app::ID_COMPLEXITY,
        create_child(
            parent,
            "BUTTON",
            i18n::t().opt_complexity,
            check,
            0,
            app::ID_COMPLEXITY,
            22,
        ),
    );
    add(
        app::ID_ULOC,
        create_child(
            parent,
            "BUTTON",
            i18n::t().opt_uloc,
            check,
            0,
            app::ID_ULOC,
            22,
        ),
    );
    add(
        app::ID_EXCLUDE_GENERATED,
        create_child(
            parent,
            "BUTTON",
            i18n::t().opt_exclude_generated,
            check,
            0,
            app::ID_EXCLUDE_GENERATED,
            22,
        ),
    );
    add(
        app::ID_COCOMO,
        create_child(
            parent,
            "BUTTON",
            i18n::t().opt_cocomo,
            check,
            0,
            app::ID_COCOMO,
            22,
        ),
    );
    add(
        app::ID_EXCLUDE,
        create_child(parent, "EDIT", "", edit | WS_BORDER, 0, app::ID_EXCLUDE, 24),
    );
    add(
        app::ID_EXTRA_DIRS,
        create_child(
            parent,
            "EDIT",
            "",
            edit | WS_BORDER,
            0,
            app::ID_EXTRA_DIRS,
            24,
        ),
    );
    add(
        app::ID_SCAN,
        create_child(
            parent,
            "BUTTON",
            i18n::t().button_scan,
            button,
            0,
            app::ID_SCAN,
            30,
        ),
    );
    add(
        app::ID_EXPORT,
        create_child(
            parent,
            "BUTTON",
            i18n::t().button_export,
            button,
            0,
            app::ID_EXPORT,
            26,
        ),
    );
    add(
        app::ID_LANG,
        create_child(parent, "COMBOBOX", "", combo, 0, app::ID_LANG, 200),
    );
    add(
        app::ID_FILTER,
        create_child(parent, "EDIT", "", edit | WS_BORDER, 0, app::ID_FILTER, 24),
    );
    add(
        app::ID_TOP,
        create_child(parent, "COMBOBOX", "", combo, 0, app::ID_TOP, 200),
    );
    add(
        app::ID_LIST,
        create_child(
            parent,
            "SysListView32",
            "",
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | WS_VSCROLL
                | LVS_REPORT
                | LVS_SINGLESEL
                | LVS_SHOWSELALWAYS,
            0,
            app::ID_LIST,
            400,
        ),
    );
    add(
        app::ID_STATUS,
        create_child(
            parent,
            "msctls_statusbar32",
            "",
            WS_CHILD | WS_VISIBLE | SBARS_SIZEGRIP,
            0,
            app::ID_STATUS,
            26,
        ),
    );
    add(
        app::ID_CARDS,
        create_child(
            parent,
            CLASS_CARDS,
            "",
            WS_CHILD | WS_VISIBLE,
            0,
            app::ID_CARDS,
            64,
        ),
    );
    add(
        app::ID_BAR,
        create_child(
            parent,
            CLASS_BAR,
            "",
            WS_CHILD | WS_VISIBLE,
            0,
            app::ID_BAR,
            150,
        ),
    );
    add(
        app::ID_MODULES,
        create_child(
            parent,
            CLASS_MODULES,
            "",
            WS_CHILD | WS_VISIBLE,
            0,
            app::ID_MODULES,
            150,
        ),
    );
    add(
        app::ID_VEIL,
        create_child(parent, CLASS_VEIL, "", WS_CHILD, 0, app::ID_VEIL, 100),
    );
    controls
}

fn apply_fonts(state: &app::AppState) {
    let base = state.theme.fonts.base as usize;
    let mono = state.theme.fonts.mono as usize;
    let bold = state.theme.fonts.bold as usize;
    let small = state.theme.fonts.label as usize;
    for (id, hwnd) in &state.controls {
        let font = match *id {
            app::ID_LIST => mono,
            app::ID_ROOTS => small,
            _ => base,
        };
        send(*hwnd, WM_SETFONT, font, 1);
    }
    let header = send(state.control(app::ID_LIST), LVM_GETHEADER, 0, 0) as HWND;
    if !header.is_null() {
        send(header, WM_SETFONT, bold, 1);
    }
}

fn combo_add_all(hwnd: HWND, items: &[&str]) {
    for item in items {
        let buffer = wide(item);
        send(hwnd, CB_ADDSTRING, 0, buffer.as_ptr() as isize);
    }
}

fn set_cue(hwnd: HWND, text: &str) {
    let cue = wide(text);
    send(hwnd, EM_SETCUEBANNER, 1, cue.as_ptr() as isize);
}

/// Add a folder picked from the shell tree to the scan roots.
fn browse_folder(hwnd: HWND) {
    let Some(folder) = dialog::pick_folder(hwnd) else {
        return;
    };
    let added = with_state(|state| state.add_root(&folder)).flatten();
    with_state(refresh_roots);
    if added.is_none() {
        let message = i18n::t().status_folder_listed;
        report_status(hwnd, message);
    }
    schedule_rescan(hwnd);
}

/// Add every folder found on the clipboard to the scan roots.
fn paste_roots(hwnd: HWND) {
    let pasted = dialog::clipboard_roots();
    let mut added = 0;
    with_state(|state| {
        for root in &pasted {
            if state.add_root(root).is_some() {
                added += 1;
            }
        }
    });
    with_state(refresh_roots);
    let message = if added > 0 {
        i18n::t().added_roots(added)
    } else {
        i18n::t().status_no_clipboard_folder.to_string()
    };
    report_status(hwnd, &message);
    if added > 0 {
        schedule_rescan(hwnd);
    }
}

/// Step through the four report tabs from the keyboard.
fn cycle_view(hwnd: HWND) {
    with_state(|state| {
        let next = model::View::from_index((state.view.index() + 1) % 4);
        set_view(state, next);
    });
    invalidate(hwnd);
}

/// Switch tabs and give the new one its natural sort before anything is drawn.
fn set_view(state: &mut app::AppState, view: model::View) {
    if state.view == view {
        return;
    }
    state.view = view;
    let (column, descending) = model::default_sort(view);
    state.sort_column = column;
    state.sort_descending = descending;
    state.rebuild();
    reconfigure_list(state);
}

/// Absolute path of the row selected in the table, if any.
fn selected_row_path() -> Option<PathBuf> {
    with_state(|state| {
        let index = send(
            state.control(app::ID_LIST),
            LVM_GETNEXTITEM,
            -1isize as usize,
            LVNI_SELECTED as isize,
        );
        if index < 0 {
            return None;
        }
        let row = state.rows.get(*state.visible.get(index as usize)?)?;
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let label = row
            .target
            .as_deref()
            .unwrap_or(&row.label)
            .trim_start_matches("./")
            .trim()
            .to_string();
        Some(if label.is_empty() || label == "." {
            cwd
        } else {
            cwd.join(label)
        })
    })
    .flatten()
}

/// Hand a file or folder to the shell: open it, or reveal it in Explorer.
fn shell_open(path: &Path, reveal: bool) {
    let file = if reveal {
        wide("explorer.exe")
    } else {
        wide(&path.to_string_lossy())
    };
    let verb = wide("open");
    let parameters = if reveal {
        wide(&format!("/select,\"{}\"", path.display()))
    } else {
        wide("")
    };
    unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            parameters.as_ptr(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

fn open_selected(hwnd: HWND, reveal: bool) {
    match selected_row_path() {
        Some(path) => shell_open(&path, reveal),
        None => {
            let message = i18n::t().status_no_row;
            report_status(hwnd, message);
        }
    }
}

fn copy_selected_path(hwnd: HWND) {
    let Some(path) = selected_row_path() else {
        let message = i18n::t().status_no_row;
        report_status(hwnd, message);
        return;
    };
    let text = path.to_string_lossy().into_owned();
    let message = if dialog::copy_text(&text) {
        i18n::t().status_copied_path
    } else {
        i18n::t().status_clipboard_failed
    };
    report_status(hwnd, message);
}

/// Anchor the popup at the cursor, or at the table when it came from the
/// keyboard (Shift+F10 and the Menu key report -1,-1 instead of a point).
fn context_menu_anchor(keyboard: bool) -> POINT {
    let mut point = POINT { x: 0, y: 0 };
    if !keyboard {
        unsafe { GetCursorPos(&mut point) };
        return point;
    }
    let (list, index) = with_state(|state| {
        let list = state.control(app::ID_LIST);
        let index = send(
            list,
            LVM_GETNEXTITEM,
            -1isize as usize,
            LVNI_SELECTED as isize,
        );
        (list, index)
    })
    .unwrap_or((std::ptr::null_mut(), -1));
    let mut bounds = RECT {
        left: LVIR_BOUNDS as i32,
        top: 0,
        right: 0,
        bottom: 0,
    };
    let on_row = index >= 0
        && send(
            list,
            LVM_GETITEMRECT,
            index as usize,
            &mut bounds as *mut RECT as isize,
        ) != 0;
    if on_row {
        point = POINT {
            x: bounds.left,
            y: bounds.bottom,
        };
        unsafe { ClientToScreen(list, &mut point) };
    } else {
        let mut frame = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        if !list.is_null() && unsafe { GetWindowRect(list, &mut frame) } != 0 {
            point = POINT {
                x: frame.left + 12,
                y: frame.top + 12,
            };
        }
    }
    point
}

fn select_row(list: HWND, index: i32) {
    if list.is_null() || index < 0 {
        return;
    }
    let mut item: LVITEMW = unsafe { std::mem::zeroed() };
    item.state = LVIS_SELECTED | LVIS_FOCUSED;
    item.stateMask = LVIS_SELECTED | LVIS_FOCUSED;
    send(
        list,
        LVM_SETITEMSTATE,
        index as usize,
        &item as *const LVITEMW as isize,
    );
}

/// Row actions for the table: open, reveal, copy path.
fn show_context_menu(hwnd: HWND, keyboard: bool) {
    let menu = unsafe { CreatePopupMenu() };
    if menu.is_null() {
        return;
    }
    let flags = if selected_row_path().is_some() {
        MF_STRING
    } else {
        MF_STRING | MF_GRAYED
    };
    for (id, text) in [
        (app::ID_CTX_OPEN, i18n::t().ctx_open),
        (app::ID_CTX_REVEAL, i18n::t().ctx_reveal),
        (app::ID_CTX_COPY_PATH, i18n::t().ctx_copy_path),
    ] {
        let label = wide(text);
        unsafe { AppendMenuW(menu, flags, id as usize, label.as_ptr()) };
    }
    let point = context_menu_anchor(keyboard);
    unsafe { SetForegroundWindow(hwnd) };
    let chosen = unsafe {
        TrackPopupMenu(
            menu,
            TPM_RETURNCMD,
            point.x,
            point.y,
            0,
            hwnd,
            std::ptr::null(),
        )
    };
    unsafe { DestroyMenu(menu) };
    match chosen as i32 {
        app::ID_CTX_OPEN => open_selected(hwnd, false),
        app::ID_CTX_REVEAL => open_selected(hwnd, true),
        app::ID_CTX_COPY_PATH => copy_selected_path(hwnd),
        _ => {}
    }
}

/// Fill the Top selector; only the row limit label is translatable.
fn fill_top_combo(state: &app::AppState) {
    let all = i18n::t().top_all;
    combo_add_all(state.control(app::ID_TOP), &["100", "300", "1000", all]);
}

/// The language selector always offers each language in its own script.
fn fill_language_combo(state: &app::AppState) {
    let combo = state.control(app::ID_LANG);
    for lang in i18n::LANGS {
        combo_add_all(combo, &[lang.native()]);
    }
}

fn sync_window_title(hwnd: HWND) {
    set_text(hwnd, i18n::t().window_title);
}

/// Re-label every widget after a language switch and repaint the shell.
fn apply_language(hwnd: HWND) {
    with_state(|state| {
        set_text(state.control(app::ID_ADD_FOLDER), i18n::t().add_folder);
        set_text(state.control(app::ID_ADD_PATH), i18n::t().paste_path);
        set_text(state.control(app::ID_REMOVE), i18n::t().remove);
        set_text(state.control(app::ID_GITIGNORE), i18n::t().opt_gitignore);
        set_text(
            state.control(app::ID_DEFAULT_IGNORES),
            i18n::t().opt_default_ignores,
        );
        set_text(state.control(app::ID_TESTS), i18n::t().opt_include_tests);
        set_text(state.control(app::ID_COMPLEXITY), i18n::t().opt_complexity);
        set_text(state.control(app::ID_ULOC), i18n::t().opt_uloc);
        set_text(
            state.control(app::ID_EXCLUDE_GENERATED),
            i18n::t().opt_exclude_generated,
        );
        set_text(state.control(app::ID_COCOMO), i18n::t().opt_cocomo);
        set_text(state.control(app::ID_SCAN), i18n::t().button_scan);
        set_text(state.control(app::ID_EXPORT), i18n::t().button_export);
        set_cue(state.control(app::ID_FILTER), i18n::t().filter_cue);
        let top = state.control(app::ID_TOP);
        let selected = combo_index(top).min(3);
        send(top, CB_RESETCONTENT, 0, 0);
        fill_top_combo(state);
        combo_choose(top, selected);
        // The UI face changes with the language, so the fonts are rebuilt.
        state.theme = theme::Theme::new(hwnd);
        apply_fonts(state);
        if state.report.is_none() {
            state.status = i18n::t().status_ready.to_string();
        }
        state.rebuild();
        reconfigure_list(state);
        refresh_status(state);
    });
    sync_window_title(hwnd);
    layout(hwnd);
    with_state(|state| repaint(state, hwnd));
}

// ---------------------------------------------------------------------------
// Main window procedure
// ---------------------------------------------------------------------------

fn on_create(hwnd: HWND) {
    STATE.with(|cell| {
        if let Ok(mut borrow) = cell.try_borrow_mut() {
            *borrow = Some(Box::new(app::AppState::new(hwnd)));
        }
    });
    let controls = create_controls(hwnd);
    with_state(|state| {
        for (id, control) in controls {
            state.remember(id, control);
        }
        apply_fonts(state);
        state.sync_options();
        let settings = state.settings.clone();
        set_checked(state.control(app::ID_GITIGNORE), settings.gitignore);
        set_checked(
            state.control(app::ID_DEFAULT_IGNORES),
            settings.default_ignores,
        );
        set_checked(state.control(app::ID_TESTS), settings.include_tests);
        set_checked(state.control(app::ID_COMPLEXITY), settings.complexity);
        set_checked(state.control(app::ID_ULOC), settings.uloc);
        set_checked(
            state.control(app::ID_EXCLUDE_GENERATED),
            settings.exclude_generated,
        );
        set_checked(state.control(app::ID_COCOMO), settings.cocomo);
        set_text(state.control(app::ID_EXCLUDE), &settings.exclude);
        set_text(state.control(app::ID_EXTRA_DIRS), &settings.extra_dirs);
        set_text(state.control(app::ID_FILTER), &settings.filter);
        set_cue(state.control(app::ID_FILTER), i18n::t().filter_cue);
        fill_top_combo(state);
        combo_choose(state.control(app::ID_TOP), settings.top.min(3));
        fill_language_combo(state);
        combo_choose(state.control(app::ID_LANG), settings.lang.min(1));
        refresh_roots(state);
        let list = state.control(app::ID_LIST);
        let extended = LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER | LVS_EX_LABELTIP;
        send(
            list,
            LVM_SETEXTENDEDLISTVIEWSTYLE,
            extended as usize,
            extended as isize,
        );
        send(list, LVM_SETBKCOLOR, 0, theme::CARD_BG as isize);
        send(list, LVM_SETTEXTBKCOLOR, 0, theme::CARD_BG as isize);
        send(list, LVM_SETTEXTCOLOR, 0, theme::TEXT as isize);
        state.rebuild();
        refresh_status(state);
    });
    sync_window_title(hwnd);
    layout(hwnd);
}

/// Redraw the sidebar list of scan roots.
fn refresh_roots(state: &mut app::AppState) {
    let list = state.control(app::ID_ROOTS);
    if list.is_null() {
        return;
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    send(list, LB_RESETCONTENT, 0, 0);
    for root in &state.roots {
        // A root that *is* the working directory has no relative form, so the
        // absolute path is the only readable label.
        let relative = model::display_path(std::path::Path::new(root), &cwd);
        let label = if relative == "." || relative.is_empty() {
            root.clone()
        } else {
            relative
        };
        let text = wide(&label);
        send(list, LB_ADDSTRING, 0, text.as_ptr() as isize);
    }
}

/// Zebra striping for the report table.
unsafe fn custom_draw(lparam: LPARAM) -> LRESULT {
    let info = lparam as *const NMLVCUSTOMDRAW;
    if info.is_null() {
        return CDRF_DODEFAULT as LRESULT;
    }
    match (*info).nmcd.dwDrawStage {
        CDDS_PREPAINT => CDRF_NOTIFYITEMDRAW as LRESULT,
        CDDS_ITEMPREPAINT => {
            let row = (*info).nmcd.dwItemSpec;
            let color = if row % 2 == 0 {
                theme::CARD_BG
            } else {
                theme::GRID_BG
            };
            (*(info as *mut NMLVCUSTOMDRAW)).clrTextBk = color;
            CDRF_DODEFAULT as LRESULT
        }
        _ => CDRF_DODEFAULT as LRESULT,
    }
}

/// A click on the painted tab strip.
fn tab_hit(state: &app::AppState, x: i32, y: i32) -> Option<model::View> {
    for (index, bounds) in state.tab_rects.iter().enumerate() {
        if x >= bounds.left && x < bounds.right && y >= bounds.top && y < bounds.bottom {
            return Some(model::View::from_index(index));
        }
    }
    None
}

fn schedule_rescan(hwnd: HWND) {
    let pending = with_state(|state| state.report.is_some() && !state.scanning).unwrap_or(false);
    if !pending {
        return;
    }
    stop_timer(hwnd, app::TIMER_RESCAN);
    start_timer(hwnd, app::TIMER_RESCAN, 600);
}

fn start_scan(hwnd: HWND) {
    with_state(|state| {
        state.begin_scan();
        if state.scanning {
            stop_timer(hwnd, app::TIMER_VEIL);
            start_timer(hwnd, app::TIMER_VEIL, 250);
            start_timer(hwnd, app::TIMER_PROGRESS, 60);
        }
    });
}

unsafe extern "system" fn main_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_CREATE => {
            on_create(hwnd);
            0
        }
        WM_SIZE => {
            layout(hwnd);
            0
        }
        WM_GETMINMAXINFO => {
            let info = lparam as *mut MINMAXINFO;
            if !info.is_null() {
                let scale = with_state(|state| state.theme.px(1)).unwrap_or(1);
                (*info).ptMinTrackSize.x = 1180 * scale;
                (*info).ptMinTrackSize.y = 744 * scale;
            }
            0
        }
        WM_COMMAND => {
            let id = (wparam & 0xFFFF) as i32;
            let code = ((wparam >> 16) & 0xFFFF) as u32;
            match id {
                app::ID_SCAN => {
                    if code == BN_CLICKED {
                        start_scan(hwnd);
                    }
                }
                app::ID_ACCEL_SCAN => start_scan(hwnd),
                app::ID_ACCEL_FILTER => {
                    with_state(|state| unsafe {
                        SetFocus(state.control(app::ID_FILTER));
                    });
                }
                app::ID_ACCEL_ESCAPE => {
                    with_state(|state| {
                        let filter = state.control(app::ID_FILTER);
                        if !get_text(filter).is_empty() {
                            set_text(filter, "");
                            state.rebuild();
                            refresh_list(state);
                            invalidate(state.control(app::ID_LIST));
                        }
                    });
                }
                app::ID_ACCEL_ADD_FOLDER => browse_folder(hwnd),
                app::ID_ACCEL_EXPORT => show_export_menu(hwnd),
                app::ID_ACCEL_NEXT_VIEW => cycle_view(hwnd),
                app::ID_ACCEL_OPEN => open_selected(hwnd, false),
                app::ID_ACCEL_CTX_MENU => {
                    let on_table =
                        with_state(|state| unsafe { GetFocus() == state.control(app::ID_LIST) })
                            .unwrap_or(false);
                    if on_table {
                        show_context_menu(hwnd, true);
                    }
                }
                app::ID_ADD_FOLDER => {
                    if code == BN_CLICKED {
                        browse_folder(hwnd);
                    }
                }
                app::ID_ADD_PATH => {
                    if code == BN_CLICKED {
                        paste_roots(hwnd);
                    }
                }
                app::ID_REMOVE => {
                    if code == BN_CLICKED {
                        with_state(|state| {
                            let index = send(state.control(app::ID_ROOTS), LB_GETCURSEL, 0, 0);
                            if index >= 0 {
                                state.remove_root(index as usize);
                            }
                        });
                        with_state(refresh_roots);
                        schedule_rescan(hwnd);
                    }
                }
                app::ID_GITIGNORE
                | app::ID_DEFAULT_IGNORES
                | app::ID_TESTS
                | app::ID_COMPLEXITY
                | app::ID_ULOC
                | app::ID_EXCLUDE_GENERATED
                | app::ID_COCOMO => {
                    if code == BN_CLICKED {
                        with_state(|state| {
                            state.sync_options();
                            state.rebuild();
                            reconfigure_list(state);
                            refresh_status(state);
                            repaint(state, hwnd);
                        });
                        layout(hwnd);
                        schedule_rescan(hwnd);
                    }
                }
                app::ID_EXCLUDE | app::ID_EXTRA_DIRS => {
                    if code == EN_CHANGE {
                        with_state(|state| {
                            state.sync_options();
                            invalidate(hwnd);
                        });
                        schedule_rescan(hwnd);
                    }
                }
                app::ID_TOP => {
                    if code == CBN_SELCHANGE {
                        with_state(|state| {
                            state.rebuild();
                            refresh_list(state);
                            invalidate(hwnd);
                        });
                    }
                }
                app::ID_LANG => {
                    if code == CBN_SELCHANGE {
                        let lang = with_state(|state| {
                            i18n::Lang::from_index(combo_index(state.control(app::ID_LANG)))
                        })
                        .unwrap_or(i18n::Lang::Zh);
                        with_state(|state| state.set_lang(lang));
                        apply_language(hwnd);
                    }
                }
                app::ID_FILTER => {
                    if code == EN_CHANGE {
                        with_state(|state| {
                            state.rebuild();
                            refresh_list(state);
                            invalidate(hwnd);
                        });
                    }
                }
                app::ID_EXPORT => {
                    if code == BN_CLICKED {
                        show_export_menu(hwnd);
                    }
                }
                other => {
                    if (app::ID_EXPORT_MENU..app::ID_EXPORT_MENU + 4).contains(&other) {
                        export(hwnd, (other - app::ID_EXPORT_MENU) as usize);
                    } else if other == app::ID_COPY_COMMAND {
                        copy_command(hwnd);
                    } else if other == app::ID_CTX_OPEN {
                        open_selected(hwnd, false);
                    } else if other == app::ID_CTX_REVEAL {
                        open_selected(hwnd, true);
                    } else if other == app::ID_CTX_COPY_PATH {
                        copy_selected_path(hwnd);
                    }
                }
            }
            0
        }
        WM_DRAWITEM => {
            let item = lparam as *const DRAWITEMSTRUCT;
            if item.is_null() {
                return 0;
            }
            with_state(|state| {
                let style = match (*item).CtlID as i32 {
                    app::ID_SCAN => Some(paint::ButtonStyle::Accent),
                    app::ID_EXPORT | app::ID_ADD_FOLDER | app::ID_ADD_PATH | app::ID_REMOVE => {
                        Some(paint::ButtonStyle::Ghost)
                    }
                    _ => None,
                };
                match style {
                    Some(style) => {
                        paint::button((*item).hDC, &*item, style, &state.theme);
                        1
                    }
                    None => 0,
                }
            })
            .unwrap_or(0)
        }
        WM_NOTIFY => {
            let header = lparam as *const NMHDR;
            if header.is_null() {
                return 0;
            }
            let code = (*header).code;
            if code == NM_CUSTOMDRAW && (*header).idFrom == app::ID_LIST as usize {
                return custom_draw(lparam);
            }
            if code == NM_DBLCLK && (*header).idFrom == app::ID_LIST as usize {
                open_selected(hwnd, false);
                return 0;
            }
            if code == NM_RCLICK && (*header).idFrom == app::ID_LIST as usize {
                let info = lparam as *const NMLISTVIEW;
                select_row((*header).hwndFrom, (*info).iItem);
                show_context_menu(hwnd, false);
                return 0;
            }
            if code == HDN_ITEMCLICKW && (*header).idFrom == app::ID_LIST as usize {
                let info = lparam as *const NMLISTVIEW;
                let sub = (*info).iSubItem;
                with_state(|state| {
                    let column = if sub <= 0 { -1 } else { sub - 1 };
                    if column == state.sort_column {
                        state.sort_descending = !state.sort_descending;
                    } else {
                        state.sort_column = column;
                        state.sort_descending = column >= 0;
                    }
                    state.rebuild();
                    reconfigure_list(state);
                    invalidate(hwnd);
                });
            }
            0
        }
        WM_CONTEXTMENU => {
            let source = wparam as HWND;
            let keyboard = (lparam as u32) == u32::MAX;
            let from_table =
                with_state(|state| source == state.control(app::ID_LIST) || source == hwnd)
                    .unwrap_or(false);
            if from_table {
                show_context_menu(hwnd, keyboard);
            }
            0
        }
        WM_LBUTTONDOWN => {
            let x = (lparam as u32 & 0xFFFF) as i16 as i32;
            let y = ((lparam as u32 >> 16) & 0xFFFF) as i16 as i32;
            let view = with_state(|state| tab_hit(state, x, y)).flatten();
            if let Some(view) = view {
                with_state(|state| set_view(state, view));
                invalidate(hwnd);
            }
            0
        }
        WM_SETCURSOR => {
            let hit = (lparam & 0xFFFF) as u32;
            let over_tab = with_state(|state| {
                if state.tab_rects[0].right == 0 {
                    return false;
                }
                let mut point = POINT { x: 0, y: 0 };
                unsafe {
                    if GetCursorPos(&mut point) == 0 {
                        return false;
                    }
                    ScreenToClient(hwnd, &mut point);
                }
                tab_hit(state, point.x, point.y).is_some()
            })
            .unwrap_or(false);
            if hit == HTCLIENT && over_tab {
                unsafe { SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_HAND)) };
                return 1;
            }
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        WM_TIMER => {
            match wparam {
                app::TIMER_PROGRESS => {
                    with_state(|state| state.poll(hwnd));
                    layout(hwnd);
                }
                app::TIMER_VEIL => {
                    stop_timer(hwnd, app::TIMER_VEIL);
                    with_state(|state| {
                        if state.scanning {
                            show_veil(state, true);
                        }
                    });
                }
                app::TIMER_RESCAN => {
                    stop_timer(hwnd, app::TIMER_RESCAN);
                    start_scan(hwnd);
                }
                _ => {}
            }
            0
        }
        WM_DPICHANGED => {
            with_state(|state| {
                state.theme = theme::Theme::new(hwnd);
                apply_fonts(state);
            });
            layout(hwnd);
            invalidate(hwnd);
            0
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            unsafe {
                let mut state_struct = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut state_struct);
                let mut client = RECT::default();
                GetClientRect(hwnd, &mut client);
                with_state(|state| {
                    let mut labels = state.labels.clone();
                    let title_rect = state.table_title_rect;
                    labels.push(paint::Label {
                        area: title_rect,
                        text: state.table_title.clone(),
                        ink: paint::Ink::Title,
                    });
                    let hint_rect = state.table_hint_rect;
                    labels.push(paint::Label {
                        area: hint_rect,
                        text: state.table_hint.clone(),
                        ink: paint::Ink::Hint,
                    });
                    let tabs: Vec<(RECT, &str, bool)> = [
                        model::View::Files,
                        model::View::Modules,
                        model::View::Functions,
                        model::View::Excluded,
                    ]
                    .iter()
                    .enumerate()
                    .map(|(index, view)| {
                        (
                            state.tab_rects[index],
                            view.tab_title(),
                            *view == state.view,
                        )
                    })
                    .collect();
                    let summary = model::header_summary(state.report.as_ref());
                    let version = format!("v{}", crate::VERSION);
                    let spec = paint::Shell {
                        brand: "rust-loc",
                        version: &version,
                        subtitle: i18n::t().subtitle,
                        summary: &summary,
                        header: state.header_rect,
                        sidebar: state.sidebar_rect,
                        labels: &labels,
                        table: state.table_rect,
                        tabs: &tabs,
                    };
                    paint::shell(hdc, &client, &spec, &state.theme);
                });
                EndPaint(hwnd, &state_struct);
            }
            0
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
            let hdc = wparam as HDC;
            SetBkMode(hdc, 1);
            SetTextColor(hdc, theme::TEXT);
            with_state(|state| state.card_brush as isize).unwrap_or(0)
        }
        WM_CTLCOLOREDIT => {
            let hdc = wparam as HDC;
            SetBkColor(hdc, theme::CARD_BG);
            SetTextColor(hdc, theme::TEXT);
            with_state(|state| state.card_brush as isize).unwrap_or(0)
        }
        WM_CTLCOLORLISTBOX => {
            let hdc = wparam as HDC;
            SetBkColor(hdc, theme::GRID_BG);
            SetTextColor(hdc, theme::TEXT);
            with_state(|state| state.card_brush as isize).unwrap_or(0)
        }
        WM_CLOSE => {
            persist(hwnd);
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            persist(hwnd);
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

// ---------------------------------------------------------------------------
// Settings, export and entry point
// ---------------------------------------------------------------------------

fn persist(hwnd: HWND) {
    let mut frame = RECT::default();
    let mut client = RECT::default();
    let placed = unsafe { GetWindowRect(hwnd, &mut frame) } != 0
        && unsafe { GetClientRect(hwnd, &mut client) } != 0;
    with_state(|state| {
        let mut settings = state.settings.clone();
        settings.roots = state.roots.join(";");
        settings.gitignore = checked(state.control(app::ID_GITIGNORE));
        settings.default_ignores = checked(state.control(app::ID_DEFAULT_IGNORES));
        settings.include_tests = checked(state.control(app::ID_TESTS));
        settings.complexity = checked(state.control(app::ID_COMPLEXITY));
        settings.uloc = checked(state.control(app::ID_ULOC));
        settings.exclude_generated = checked(state.control(app::ID_EXCLUDE_GENERATED));
        settings.cocomo = checked(state.control(app::ID_COCOMO));
        settings.exclude = get_text(state.control(app::ID_EXCLUDE));
        settings.extra_dirs = get_text(state.control(app::ID_EXTRA_DIRS));
        settings.view = state.view.index();
        settings.top = combo_selection(state.control(app::ID_TOP));
        settings.filter = get_text(state.control(app::ID_FILTER));
        settings.sort_column = state.sort_column;
        settings.sort_descending = state.sort_descending;
        if placed {
            settings.window = Some((
                frame.left,
                frame.top,
                client.right - client.left,
                client.bottom - client.top,
            ));
        }
        model::save_settings(&settings);
        state.settings = settings;
    });
}

fn default_export_name(format: crate::options::Format) -> &'static str {
    match format {
        crate::options::Format::Json => "rust-loc.json",
        crate::options::Format::Csv => "rust-loc.csv",
        crate::options::Format::Markdown => "rust-loc.md",
        crate::options::Format::Text => "rust-loc.txt",
    }
}

fn show_export_menu(hwnd: HWND) {
    let menu = unsafe { CreatePopupMenu() };
    if menu.is_null() {
        return;
    }
    for (index, (label, _)) in app::FORMATS.iter().enumerate() {
        let text = wide(&i18n::fill(
            i18n::t().menu_export_as,
            &[("format", (*label).to_string())],
        ));
        unsafe {
            AppendMenuW(
                menu,
                MF_STRING,
                (app::ID_EXPORT_MENU as usize) + index,
                text.as_ptr(),
            )
        };
    }
    unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null()) };
    let copy = wide(i18n::t().menu_copy_command);
    unsafe {
        AppendMenuW(
            menu,
            MF_STRING,
            app::ID_COPY_COMMAND as usize,
            copy.as_ptr(),
        )
    };
    let anchor = with_state(|state| {
        let button = state.control(app::ID_EXPORT);
        let mut frame = RECT::default();
        unsafe { GetWindowRect(button, &mut frame) };
        (frame.left, frame.bottom)
    })
    .unwrap_or((0, 0));
    unsafe { SetForegroundWindow(hwnd) };
    let chosen = unsafe {
        TrackPopupMenu(
            menu,
            TPM_RETURNCMD,
            anchor.0,
            anchor.1,
            0,
            hwnd,
            std::ptr::null(),
        )
    };
    unsafe { DestroyMenu(menu) };
    if chosen != 0 {
        let command = chosen as i32;
        if (app::ID_EXPORT_MENU..app::ID_EXPORT_MENU + 4).contains(&command) {
            export(hwnd, (command - app::ID_EXPORT_MENU) as usize);
        } else if command == app::ID_COPY_COMMAND {
            copy_command(hwnd);
        }
    }
}

fn report_status(hwnd: HWND, message: &str) {
    with_state(|state| {
        state.status = message.to_string();
        let status = state.control(app::ID_STATUS);
        set_status_text(status, message);
    });
    let _ = hwnd;
}

fn copy_command(hwnd: HWND) {
    let command = with_state(|state| {
        let cwd = std::env::current_dir().unwrap_or_default();
        state
            .report
            .as_ref()
            .map(|report| model::cli_command(report, &cwd))
    })
    .flatten();
    let Some(command) = command else {
        let message = i18n::t().status_nothing_to_copy;
        report_status(hwnd, message);
        return;
    };
    let message = if dialog::copy_text(&command) {
        i18n::t().status_copied_command
    } else {
        i18n::t().status_clipboard_failed
    };
    report_status(hwnd, message);
}

fn export(hwnd: HWND, index: usize) {
    let payload = with_state(|state| {
        let format = app::format_at(index);
        let cwd = std::env::current_dir().unwrap_or_default();
        let text = state
            .report
            .as_ref()
            .map(|report| report.render_as(format, &cwd));
        state.options.format = format;
        state.settings.format = index;
        (text, default_export_name(format))
    });
    let Some((payload, name)) = payload else {
        return;
    };
    let Some(text) = payload else {
        let message = i18n::t().status_nothing_to_export;
        report_status(hwnd, message);
        return;
    };
    if let Some(path) = dialog::save_file(hwnd, name) {
        let outcome = std::fs::write(&path, text);
        let message = match outcome {
            Ok(()) => i18n::fill(i18n::t().status_exported, &[("path", path.clone())]),
            Err(error) => i18n::fill(
                i18n::t().status_export_failed,
                &[("error", error.to_string())],
            ),
        };
        report_status(hwnd, &message);
    }
}

fn geometry() -> (i32, i32, i32, i32) {
    let settings = model::load_settings();
    let mut frame = RECT {
        left: 0,
        top: 0,
        right: 1440,
        bottom: 900,
    };
    if let Some((x, y, width, height)) = settings.window {
        if width >= 1000 && height >= 640 {
            frame.left = x;
            frame.top = y;
            frame.right = x + width;
            frame.bottom = y + height;
        }
    } else {
        let screen_width = unsafe { GetSystemMetrics(SM_CXSCREEN) };
        let screen_height = unsafe { GetSystemMetrics(SM_CYSCREEN) };
        let width = (screen_width * 74 / 100).min(1680);
        let height = (screen_height * 84 / 100).min(1040);
        frame.left = (screen_width - width) / 2;
        frame.top = (screen_height - height) / 2;
        frame.right = frame.left + width;
        frame.bottom = frame.top + height;
    }
    let style = WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN;
    let mut client = frame;
    unsafe { AdjustWindowRectEx(&mut client, style, 0, 0) };
    (
        frame.left,
        frame.top,
        client.right - client.left,
        client.bottom - client.top,
    )
}

fn register_class(name: &str, proc: WNDPROC, instance: HINSTANCE) -> bool {
    let class = wide(name);
    let mut description: WNDCLASSW = unsafe { std::mem::zeroed() };
    description.style = CS_HREDRAW | CS_VREDRAW;
    description.lpfnWndProc = proc;
    description.hInstance = instance;
    description.hCursor = unsafe { LoadCursorW(std::ptr::null_mut(), IDC_ARROW) };
    description.lpszClassName = class.as_ptr();
    let atom = unsafe { RegisterClassW(&description) };
    atom != 0
}

/// Create the window, run the message loop and return the process exit code.
pub fn run() -> i32 {
    // The language is decided before the window exists so its title and every
    // control label are created in the right language.
    i18n::set_lang(i18n::Lang::from_index(model::load_settings().lang));
    unsafe {
        let controls = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_STANDARD_CLASSES | ICC_LISTVIEW_CLASSES | ICC_BAR_CLASSES,
        };
        InitCommonControlsEx(&controls);
        let instance = GetModuleHandleW(std::ptr::null());
        if !register_class(CLASS_MAIN, Some(main_proc), instance) {
            return 1;
        }
        register_class(CLASS_CARDS, Some(cards_proc), instance);
        register_class(CLASS_BAR, Some(bar_proc), instance);
        register_class(CLASS_MODULES, Some(modules_proc), instance);
        register_class(CLASS_VEIL, Some(veil_proc), instance);
        let keys = [
            ACCEL {
                fVirt: FVIRTKEY,
                key: VK_F5,
                cmd: app::ID_ACCEL_SCAN as u16,
            },
            ACCEL {
                fVirt: FVIRTKEY | FCONTROL,
                key: 'F' as u16,
                cmd: app::ID_ACCEL_FILTER as u16,
            },
            ACCEL {
                fVirt: FVIRTKEY,
                key: VK_ESCAPE,
                cmd: app::ID_ACCEL_ESCAPE as u16,
            },
            ACCEL {
                fVirt: FVIRTKEY | FCONTROL,
                key: 'O' as u16,
                cmd: app::ID_ACCEL_ADD_FOLDER as u16,
            },
            ACCEL {
                fVirt: FVIRTKEY | FCONTROL,
                key: 'E' as u16,
                cmd: app::ID_ACCEL_EXPORT as u16,
            },
            ACCEL {
                fVirt: FVIRTKEY | FCONTROL,
                key: VK_TAB,
                cmd: app::ID_ACCEL_NEXT_VIEW as u16,
            },
            ACCEL {
                fVirt: FVIRTKEY,
                key: VK_RETURN,
                cmd: app::ID_ACCEL_OPEN as u16,
            },
            ACCEL {
                fVirt: FVIRTKEY,
                key: VK_APPS,
                cmd: app::ID_ACCEL_CTX_MENU as u16,
            },
            ACCEL {
                fVirt: FVIRTKEY | FSHIFT,
                key: VK_F10,
                cmd: app::ID_ACCEL_CTX_MENU as u16,
            },
        ];
        let accel = CreateAcceleratorTableW(keys.as_ptr(), keys.len() as i32);
        let (x, y, width, height) = geometry();
        let class = wide(CLASS_MAIN);
        let title = wide(i18n::t().window_title);
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            x,
            y,
            width,
            height,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return 1;
        }
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
            if TranslateAcceleratorW(hwnd, accel, &message) == 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        0
    }
}
