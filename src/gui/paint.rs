//! GDI helpers plus the custom painted surfaces.
//!
//! Everything the shell draws by hand lives here: the header band, the
//! sidebar frame, the KPI strip, the two chart cards, the tab strip and the
//! scan overlay. Widgets themselves stay real Win32 controls.

use crate::gui::sys::*;
use crate::gui::{i18n, model, theme};

pub const DT_LEFT: u32 = 0;
pub const DT_CENTER: u32 = 1;
pub const DT_RIGHT: u32 = 2;
pub const DT_VCENTER: u32 = 4;
pub const DT_SINGLELINE: u32 = 0x20;
pub const DT_NOPREFIX: u32 = 0x800;
pub const DT_END_ELLIPSIS: u32 = 0x8000;

const TRANSPARENT: i32 = 1;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
    RECT {
        left,
        top,
        right,
        bottom,
    }
}

/// Opaque fill of a rectangle.
pub fn fill(hdc: HDC, area: &RECT, color: u32) {
    unsafe {
        let brush: HBRUSH = CreateSolidBrush(color);
        FillRect(hdc, area, brush);
        DeleteObject(brush as HGDIOBJ);
    }
}

/// A rounded region matching an area, never thicker than half of it so a
/// "pill" request still lands inside the shape.
pub fn region(area: &RECT, radius: i32) -> HRGN {
    let width = area.right - area.left;
    let height = area.bottom - area.top;
    let radius = radius.max(0).min(height / 2).min(width / 2);
    unsafe {
        CreateRoundRectRgn(
            area.left,
            area.top,
            area.right + 1,
            area.bottom + 1,
            radius * 2,
            radius * 2,
        )
    }
}

/// Run a painter with the clip replaced by a rounded region, so no fill can
/// spill past a corner.
fn clipped(hdc: HDC, area: &RECT, radius: i32, paint: impl FnOnce()) {
    let clip = region(area, radius);
    unsafe { SelectClipRgn(hdc, clip) };
    paint();
    unsafe {
        SelectClipRgn(hdc, std::ptr::null_mut());
        DeleteObject(clip as HGDIOBJ);
    }
}

/// Rounded rectangle with an optional 1px border, corners included.
pub fn card(hdc: HDC, area: &RECT, radius: i32, fill_color: u32, border: Option<u32>) {
    if area.right <= area.left || area.bottom <= area.top {
        return;
    }
    clipped(hdc, area, radius, || match border {
        // The border is the outer shape; the fill covers all but its last pixel.
        Some(color) => {
            fill(hdc, area, color);
            let inner = rect(area.left + 1, area.top + 1, area.right - 1, area.bottom - 1);
            if inner.right > inner.left && inner.bottom > inner.top {
                fill(hdc, &inner, fill_color);
            }
        }
        None => fill(hdc, area, fill_color),
    });
}

/// A card with a one pixel lip under it: the cheapest way to read as lifted.
pub fn raised(hdc: HDC, area: &RECT, radius: i32, fill_color: u32, border: Option<u32>) {
    let below = rect(area.left, area.top + 1, area.right, area.bottom + 1);
    card(hdc, &below, radius, theme::SHADOW, None);
    card(hdc, area, radius, fill_color, border);
}

/// The rounded surface behind a text field, a scan-root list or a selector:
/// white inside a hairline that turns accent while the control holds the
/// focus. The frame window paints it, the control sits inset above it.
pub fn field(hdc: HDC, area: &RECT, radius: i32, focused: bool) {
    let border = if focused {
        theme::ACCENT
    } else {
        theme::FIELD_BORDER
    };
    card(hdc, area, radius, theme::FIELD_BG, Some(border));
}

/// A one pixel hairline.
fn rule(hdc: HDC, area: &RECT, color: u32) {
    fill(hdc, area, color);
}

/// Draw single or multi line text clipped to `area`.
pub fn text(hdc: HDC, value: &str, area: &mut RECT, color: u32, font: HFONT, format: u32) {
    if value.is_empty() {
        return;
    }
    let buffer = wide(value);
    unsafe {
        let old = SelectObject(hdc, font as HGDIOBJ);
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, color);
        DrawTextW(hdc, buffer.as_ptr(), -1, area, format | DT_NOPREFIX);
        SelectObject(hdc, old);
    }
}

/// Width of a string in the given font.
pub fn measure(hdc: HDC, value: &str, font: HFONT) -> i32 {
    let buffer = wide(value);
    let mut size = SIZE { cx: 0, cy: 0 };
    unsafe {
        let old = SelectObject(hdc, font as HGDIOBJ);
        GetTextExtentPoint32W(hdc, buffer.as_ptr(), buffer.len() as i32 - 1, &mut size);
        SelectObject(hdc, old);
    }
    size.cx
}

/// An upper case caption in the muted section style.
fn caption(hdc: HDC, value: &str, area: &mut RECT, theme: &theme::Theme) {
    let upper = value.to_uppercase();
    text(
        hdc,
        &upper,
        area,
        theme::MUTED,
        theme.fonts.label_bold,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER,
    );
}

/// A sidebar section caption, with the rest of its row ruled off.
fn section_caption(hdc: HDC, value: &str, area: &RECT, theme: &theme::Theme) {
    let upper = value.to_uppercase();
    let width = measure(hdc, &upper, theme.fonts.label_bold);
    let mut label = *area;
    label.right = (label.left + width + theme.px(8)).min(area.right);
    text(
        hdc,
        &upper,
        &mut label,
        theme::MUTED,
        theme.fonts.label_bold,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER,
    );
    let start = area.left + width + theme.px(10);
    let middle = (area.top + area.bottom) / 2;
    if start + theme.px(8) < area.right {
        fill(
            hdc,
            &rect(start, middle, area.right, middle + 1),
            theme::HAIRLINE,
        );
    }
}

// ---------------------------------------------------------------------------
// KPI strip
// ---------------------------------------------------------------------------

/// Paint the KPI cards as a `columns` wide grid.
pub fn cards(
    hdc: HDC,
    area: &RECT,
    cards: &[model::Card],
    columns: i32,
    gap: i32,
    padding: i32,
    theme: &theme::Theme,
) {
    let count = cards.len() as i32;
    if count == 0 || columns <= 0 {
        return;
    }
    let rows = (count + columns - 1) / columns;
    let available = (area.right - area.left) - padding * 2 - gap * (columns - 1);
    let width = available / columns;
    if width < theme.px(80) || rows == 0 {
        return;
    }
    let height = ((area.bottom - area.top) - padding * 2 - gap * (rows - 1)) / rows;
    if height < theme.px(48) {
        return;
    }
    let fonts = &theme.fonts;
    for (index, item) in cards.iter().enumerate() {
        let index = index as i32;
        let column = index % columns;
        let row = index / columns;
        let left = area.left + padding + column * (width + gap);
        let top = area.top + padding + row * (height + gap);
        let bounds = rect(left, top, left + width, top + height);
        raised(
            hdc,
            &bounds,
            theme.px(theme::R_CARD),
            theme::CARD_BG,
            Some(theme::BORDER),
        );
        let stripe = rect(
            bounds.left + 1,
            bounds.top + 1,
            bounds.left + 1 + theme.px(4),
            bounds.bottom - 1,
        );
        card(hdc, &stripe, theme.px(2), item.tone(), None);
        let inner = theme.px(14);
        let mut label = rect(
            left + inner,
            top + theme.px(8),
            bounds.right - theme.px(8),
            top + theme.px(24),
        );
        caption(hdc, item.label, &mut label, theme);
        let mut value = rect(
            left + inner,
            top + theme.px(24),
            bounds.right - theme.px(8),
            top + theme.px(53),
        );
        text(
            hdc,
            &item.value,
            &mut value,
            item.tone(),
            fonts.value,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
        );
        let mut hint = rect(
            left + inner,
            bounds.bottom - theme.px(19),
            bounds.right - theme.px(8),
            bounds.bottom - theme.px(5),
        );
        text(
            hdc,
            &item.hint,
            &mut hint,
            theme::MUTED,
            fonts.hint,
            DT_LEFT | DT_SINGLELINE | DT_END_ELLIPSIS,
        );
    }
}

// ---------------------------------------------------------------------------
// Chart cards
// ---------------------------------------------------------------------------

/// A white card with a title in its top left corner and a hairline under it.
fn chart_card(hdc: HDC, area: &RECT, title: &str, theme: &theme::Theme) -> RECT {
    raised(hdc, area, theme.px(10), theme::CARD_BG, Some(theme::BORDER));
    let mut head = rect(
        area.left + theme.px(14),
        area.top + theme.px(8),
        area.right - theme.px(14),
        area.top + theme.px(30),
    );
    text(
        hdc,
        title,
        &mut head,
        theme::TEXT,
        theme.fonts.small_bold,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER,
    );
    fill(
        hdc,
        &rect(
            area.left + theme.px(14),
            area.top + theme.px(34),
            area.right - theme.px(14),
            area.top + theme.px(35),
        ),
        theme::HAIRLINE,
    );
    rect(
        area.left + theme.px(14),
        area.top + theme.px(46),
        area.right - theme.px(14),
        area.bottom - theme.px(12),
    )
}

/// The line composition card: a stacked bar plus a two column legend.
pub fn composition(
    hdc: HDC,
    area: &RECT,
    title: &str,
    segments: &[(&'static str, u32, u32)],
    theme: &theme::Theme,
    regions: &mut Vec<(RECT, String)>,
) {
    let content = chart_card(hdc, area, title, theme);
    if content.right - content.left < theme.px(60) || content.bottom - content.top < theme.px(30) {
        return;
    }
    let total: u64 = segments.iter().map(|segment| segment.1 as u64).sum();
    let bar_height = theme.px(26).min(content.bottom - content.top);
    let track = rect(
        content.left,
        content.top,
        content.right,
        content.top + bar_height,
    );
    if total == 0 {
        let mut note = rect(
            content.left,
            content.top,
            content.right,
            content.top + theme.px(20),
        );
        text(
            hdc,
            i18n::t().chart_empty_composition,
            &mut note,
            theme::MUTED,
            theme.fonts.hint,
            DT_LEFT | DT_SINGLELINE,
        );
        return;
    }
    let width = track.right - track.left;
    let visible = segments.iter().filter(|segment| segment.1 > 0).count();
    clipped(hdc, &track, theme.px(theme::R_CARD), || {
        let mut cursor = track.left;
        let mut drawn = 0usize;
        let mut seams: Vec<i32> = Vec::with_capacity(visible);
        for (_, value, color) in segments {
            if *value == 0 {
                continue;
            }
            let span = (*value as u64)
                .saturating_mul(width as u64)
                .checked_div(total)
                .unwrap_or(0) as i32;
            let right = (cursor + span).min(track.right);
            if right > cursor {
                fill(hdc, &rect(cursor, track.top, right, track.bottom), *color);
                drawn += 1;
                if drawn < visible {
                    seams.push(right);
                }
            }
            cursor = right;
        }
        // A card coloured seam between neighbours reads the bar as segments
        // without needing a second row of gaps.
        for seam in seams {
            fill(
                hdc,
                &rect(seam - 1, track.top, seam + 1, track.bottom),
                theme::CARD_BG,
            );
        }
    });
    let legend_top = track.bottom + theme.px(10);
    let row_height = theme.px(23);
    let column_width = width / 2;
    // A gutter between the two columns keeps the left column's percentage from
    // touching the right column's swatch.
    let gutter = theme.px(14);
    for (index, (name, value, color)) in segments.iter().enumerate() {
        let column = index as i32 % 2;
        let row = index as i32 / 2;
        let left = content.left + column * (column_width + gutter);
        let right = left + column_width - gutter;
        let top = legend_top + row * row_height;
        if top + theme.px(16) > content.bottom {
            break;
        }
        let swatch = rect(
            left,
            top + theme.px(3),
            left + theme.px(9),
            top + theme.px(12),
        );
        card(hdc, &swatch, theme.px(2), *color, None);
        let mut name_bounds = rect(
            left + theme.px(15),
            top - theme.px(1),
            right - theme.px(4),
            top + theme.px(17),
        );
        text(
            hdc,
            name,
            &mut name_bounds,
            theme::MUTED,
            theme.fonts.hint,
            DT_LEFT | DT_SINGLELINE | DT_END_ELLIPSIS,
        );
        let value_text = format!(
            "{}   {}",
            model::human_int(*value as u64),
            model::human_share(*value as u64, total)
        );
        let mut value_bounds = rect(left, top - theme.px(1), right, top + theme.px(17));
        text(
            hdc,
            &value_text,
            &mut value_bounds,
            theme::TEXT,
            theme.fonts.small_bold,
            DT_RIGHT | DT_SINGLELINE,
        );
        regions.push((
            rect(left, top, right, top + row_height),
            i18n::fill(
                i18n::t().hover_segment,
                &[
                    ("name", (*name).to_string()),
                    ("value", model::human_int(*value as u64)),
                    ("share", model::human_share(*value as u64, total)),
                ],
            ),
        ));
    }
}

/// The module chart card: one horizontal bar per module, biggest first.
pub fn modules(
    hdc: HDC,
    area: &RECT,
    title: &str,
    rows: &[(String, u32)],
    theme: &theme::Theme,
    regions: &mut Vec<(RECT, String)>,
) {
    let content = chart_card(hdc, area, title, theme);
    if content.right - content.left < theme.px(80) || content.bottom - content.top < theme.px(20) {
        return;
    }
    if rows.is_empty() {
        let mut note = rect(
            content.left,
            content.top,
            content.right,
            content.top + theme.px(20),
        );
        text(
            hdc,
            i18n::t().chart_empty_modules,
            &mut note,
            theme::MUTED,
            theme.fonts.hint,
            DT_LEFT | DT_SINGLELINE,
        );
        return;
    }
    let row_height = theme.px(23);
    let capacity = ((content.bottom - content.top) / row_height).max(1) as usize;
    let rows = &rows[..rows.len().min(capacity)];
    let peak = rows.iter().map(|row| row.1).max().unwrap_or(1).max(1);
    let width = content.right - content.left;
    let mut label_width = rows
        .iter()
        .map(|row| measure(hdc, &row.0, theme.fonts.hint))
        .max()
        .unwrap_or(0);
    label_width = label_width.min((width as f64 * 0.42) as i32);
    let value_width = rows
        .iter()
        .map(|row| measure(hdc, &model::human_int(row.1 as u64), theme.fonts.small_bold))
        .max()
        .unwrap_or(0);
    let track_left = content.left + label_width + theme.px(8);
    let track_right = (content.right - value_width - theme.px(10)).max(track_left + theme.px(24));
    for (index, (label, value)) in rows.iter().enumerate() {
        let top = content.top + index as i32 * row_height;
        let middle = top + row_height / 2;
        let mut name = rect(
            content.left,
            top,
            content.left + label_width,
            top + row_height,
        );
        text(
            hdc,
            label,
            &mut name,
            theme::TEXT,
            theme.fonts.hint,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
        );
        card(
            hdc,
            &rect(
                track_left,
                middle - theme.px(6),
                track_right,
                middle + theme.px(6),
            ),
            theme.px(theme::R_MARK),
            theme::BAR_TRACK,
            None,
        );
        let span = ((track_right - track_left) as f64 * (*value as f64 / peak as f64)) as i32;
        if span > 0 {
            card(
                hdc,
                &rect(
                    track_left,
                    middle - theme.px(6),
                    (track_left + span).min(track_right),
                    middle + theme.px(6),
                ),
                theme.px(theme::R_MARK),
                theme::ACCENT,
                None,
            );
        }
        let mut amount = rect(
            track_right + theme.px(4),
            top,
            content.right,
            top + row_height,
        );
        text(
            hdc,
            &model::human_int(*value as u64),
            &mut amount,
            theme::MUTED,
            theme.fonts.small_bold,
            DT_RIGHT | DT_SINGLELINE | DT_VCENTER,
        );
        regions.push((
            rect(content.left, top, content.right, top + row_height),
            i18n::fill(
                i18n::t().hover_module,
                &[
                    ("name", label.clone()),
                    ("value", model::human_int(*value as u64)),
                ],
            ),
        ));
    }
}

// ---------------------------------------------------------------------------
// Hover tooltip
// ---------------------------------------------------------------------------

/// The chart tooltip: a caption anchored at a point in the chart client area.
#[derive(Clone)]
pub struct Hover {
    pub text: String,
    pub x: i32,
    pub y: i32,
}

/// Draw the tooltip card next to the cursor, kept inside the chart.
pub fn tooltip(hdc: HDC, area: &RECT, hover: &Hover, theme: &theme::Theme) {
    let pad = theme.px(9);
    let width = measure(hdc, &hover.text, theme.fonts.hint) + pad * 2;
    let height = theme.px(23);
    let mut left = hover.x + theme.px(14);
    let mut top = hover.y + theme.px(18);
    if left + width > area.right - theme.px(4) {
        left = (hover.x - width - theme.px(12)).max(area.left + theme.px(4));
    }
    if top + height > area.bottom - theme.px(4) {
        top = (hover.y - height - theme.px(12)).max(area.top + theme.px(4));
    }
    let bounds = rect(left, top, left + width, top + height);
    card(
        hdc,
        &bounds,
        theme.px(theme::R_CARD),
        theme::TIP_BG,
        Some(theme::TIP_BORDER),
    );
    let mut caption = bounds;
    text(
        hdc,
        &hover.text,
        &mut caption,
        theme::TIP_TEXT,
        theme.fonts.hint,
        DT_CENTER | DT_SINGLELINE | DT_VCENTER,
    );
}

// ---------------------------------------------------------------------------
// Shell chrome
// ---------------------------------------------------------------------------

/// Text painted by the shell, with the role that decides its font.
#[derive(Clone, Copy)]
pub enum Ink {
    /// An upper case sidebar caption.
    Section,
    /// A muted note below a field.
    Note,
    /// A small right aligned caption in front of a field.
    CaptionEnd,
    /// The title of the table card.
    Title,
    /// The hint below the table.
    Hint,
}

/// One piece of shell text and where it belongs.
#[derive(Clone)]
pub struct Label {
    pub area: RECT,
    pub text: String,
    pub ink: Ink,
}

/// The scan history card's data: one bar per recorded scan.
pub struct Spark<'a> {
    pub area: RECT,
    pub title: &'a str,
    pub latest: &'a str,
    pub delta: &'a str,
    pub tone: u32,
    pub values: &'a [u32],
}

// ---------------------------------------------------------------------------
// Scan history
// ---------------------------------------------------------------------------

/// The history card: the newest code count with its change on the left, one
/// bar per recorded scan on the right.
fn spark_card(hdc: HDC, spark: &Spark, theme: &theme::Theme) {
    let area = spark.area;
    if area.right - area.left < theme.px(140) || area.bottom - area.top < theme.px(40) {
        return;
    }
    raised(
        hdc,
        &area,
        theme.px(theme::R_CARD),
        theme::CARD_BG,
        Some(theme::BORDER),
    );
    let pad = theme.px(14);
    let column = theme.px(150).min((area.right - area.left) / 2);
    let right = area.left + pad + column;
    let mut title = rect(
        area.left + pad,
        area.top + theme.px(7),
        right,
        area.top + theme.px(23),
    );
    text(
        hdc,
        spark.title,
        &mut title,
        theme::MUTED,
        theme.fonts.label,
        DT_LEFT | DT_SINGLELINE,
    );
    let mut value = rect(
        area.left + pad,
        area.top + theme.px(23),
        right,
        area.top + theme.px(53),
    );
    text(
        hdc,
        spark.latest,
        &mut value,
        spark.tone,
        theme.fonts.value,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER,
    );
    let mut change = rect(
        area.left + pad,
        area.bottom - theme.px(20),
        right,
        area.bottom - theme.px(6),
    );
    text(
        hdc,
        spark.delta,
        &mut change,
        theme::MUTED,
        theme.fonts.hint,
        DT_LEFT | DT_SINGLELINE,
    );
    let plot = rect(
        right + theme.px(10),
        area.top + theme.px(12),
        area.right - pad,
        area.bottom - theme.px(12),
    );
    bars(hdc, &plot, spark.values, spark.tone, theme);
}

/// One bar per value, scaled to the box; a series that never moves draws
/// even stubs so the chart still reads as a series.
fn bars(hdc: HDC, area: &RECT, values: &[u32], tone: u32, theme: &theme::Theme) {
    let width = area.right - area.left;
    let height = area.bottom - area.top;
    if values.is_empty() || width < theme.px(24) || height < theme.px(16) {
        return;
    }
    fill(
        hdc,
        &rect(area.left, area.bottom - 1, area.right, area.bottom),
        theme::HAIRLINE,
    );
    let min = *values.iter().min().unwrap();
    let max = *values.iter().max().unwrap();
    let span = max - min;
    let floor = theme.px(2);
    // A short series stays a readable column chart instead of stretching into
    // a wall; a long one packs the bars together.
    let slot = (width / values.len() as i32)
        .min(theme.px(26))
        .max(theme.px(2));
    let gap = theme.px(3).min(slot / 4);
    let bar = (slot - gap).max(theme.px(2));
    let usable = height - theme.px(4);
    // The newest scan sits on the right edge, so the series reads like the
    // rest of the card: latest first.
    let start = area.right - slot * values.len() as i32;
    for (index, value) in values.iter().enumerate() {
        let scaled = if span == 0 {
            usable / 2
        } else {
            usable * (value - min) as i32 / span as i32
        };
        let left = start.max(area.left) + index as i32 * slot;
        card(
            hdc,
            &rect(
                left,
                area.bottom - scaled.max(floor),
                left + bar,
                area.bottom,
            ),
            theme.px(theme::R_MARK),
            tone,
            None,
        );
    }
}

/// Everything the main window paints behind its child controls.
pub struct Shell<'a> {
    pub brand: &'a str,
    pub version: &'a str,
    pub subtitle: &'a str,
    pub summary: &'a str,
    pub header: RECT,
    pub sidebar: RECT,
    pub labels: &'a [Label],
    pub spark: Option<Spark<'a>>,
    /// The table card, absent while the overview pane is the one showing.
    pub table: Option<RECT>,
    /// The pane captions above the content, left to right.
    pub panes: &'a [(RECT, &'a str, bool)],
    /// The tabs of the table in front. Empty when there is no table.
    pub tabs: &'a [(RECT, &'a str, bool)],
}

/// The pane strip: plain captions on a hairline, the active one underlined.
///
/// It is deliberately flatter than the tablet strip inside a pane, so the two
/// levels cannot be mistaken for each other.
fn pane_strip(hdc: HDC, panes: &[(RECT, &str, bool)], theme: &theme::Theme) {
    let Some((first, rest)) = panes.split_first() else {
        return;
    };
    let right = rest
        .last()
        .map(|(area, _, _)| area.right)
        .unwrap_or(first.0.right);
    let line_y = first.0.bottom - 1;
    rule(
        hdc,
        &rect(first.0.left, line_y, right, line_y + 1),
        theme::HAIRLINE,
    );
    for (area, title, active) in panes {
        let color = if *active { theme::TEXT } else { theme::MUTED };
        let font = if *active {
            theme.fonts.small_bold
        } else {
            theme.fonts.small
        };
        let mut label = rect(area.left, area.top, area.right, area.bottom - theme.px(5));
        text(
            hdc,
            title,
            &mut label,
            color,
            font,
            DT_CENTER | DT_SINGLELINE | DT_VCENTER,
        );
        if *active {
            // An underline as wide as the caption rather than the whole tab,
            // so a long caption does not read as a rule across the window.
            let width = measure(hdc, title, font) + theme.px(14);
            let left = (area.left + area.right - width) / 2;
            let top = line_y - theme.px(2);
            rule(
                hdc,
                &rect(left, top, left + width, top + theme.px(2).max(2)),
                theme::ACCENT,
            );
        }
    }
}

pub fn shell(hdc: HDC, area: &RECT, spec: &Shell, theme: &theme::Theme) {
    fill(hdc, area, theme::PANEL_BG);

    // Header band.
    fill(hdc, &spec.header, theme::CARD_BG);
    rule(
        hdc,
        &rect(
            spec.header.left,
            spec.header.bottom - 1,
            spec.header.right,
            spec.header.bottom,
        ),
        theme::BORDER,
    );
    let pad = theme.px(16);
    let brand_width = measure(hdc, spec.brand, theme.fonts.brand);
    let mut brand = rect(
        spec.header.left + pad,
        spec.header.top,
        spec.header.left + pad + brand_width,
        spec.header.bottom,
    );
    text(
        hdc,
        spec.brand,
        &mut brand,
        theme::ACCENT,
        theme.fonts.brand,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER,
    );
    let mut cursor = spec.header.left + pad + brand_width + theme.px(10);
    let version_width = measure(hdc, spec.version, theme.fonts.hint);
    let chip = rect(
        cursor - theme.px(7),
        spec.header.top + theme.px(15),
        cursor + version_width + theme.px(7),
        spec.header.bottom - theme.px(15),
    );
    card(hdc, &chip, theme.px(theme::R_MARK), theme::TRACK_BG, None);
    let mut version = rect(cursor, chip.top, cursor + version_width, chip.bottom);
    text(
        hdc,
        spec.version,
        &mut version,
        theme::MUTED,
        theme.fonts.hint,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER,
    );
    cursor += version_width + theme.px(14);
    let mut subtitle = rect(
        cursor,
        spec.header.top,
        spec.header.right - pad,
        spec.header.bottom,
    );
    text(
        hdc,
        spec.subtitle,
        &mut subtitle,
        theme::MUTED,
        theme.fonts.label,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
    );
    let mut summary = rect(
        spec.header.left + pad,
        spec.header.top,
        spec.header.right - pad,
        spec.header.bottom,
    );
    text(
        hdc,
        spec.summary,
        &mut summary,
        theme::TEXT,
        theme.fonts.mono_small,
        DT_RIGHT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
    );

    // Sidebar frame.
    fill(hdc, &spec.sidebar, theme::CARD_BG);
    rule(
        hdc,
        &rect(
            spec.sidebar.right - 1,
            spec.sidebar.top,
            spec.sidebar.right,
            spec.sidebar.bottom,
        ),
        theme::BORDER,
    );

    // The pane captions, then the pane in front: a table card is only drawn
    // when a table pane is the one showing.
    pane_strip(hdc, spec.panes, theme);
    if let Some(table) = &spec.table {
        card(
            hdc,
            table,
            theme.px(theme::R_CARD),
            theme::CARD_BG,
            Some(theme::BORDER),
        );
    }

    if let Some(spark) = &spec.spark {
        spark_card(hdc, spark, theme);
    }

    for label in spec.labels {
        let mut bounds = label.area;
        match label.ink {
            Ink::Section => section_caption(hdc, &label.text, &bounds, theme),
            Ink::Note => text(
                hdc,
                &label.text,
                &mut bounds,
                theme::MUTED,
                theme.fonts.hint,
                DT_LEFT | DT_SINGLELINE,
            ),
            Ink::CaptionEnd => text(
                hdc,
                &label.text,
                &mut bounds,
                theme::MUTED,
                theme.fonts.label,
                DT_RIGHT | DT_SINGLELINE | DT_VCENTER,
            ),
            Ink::Title => text(
                hdc,
                &label.text,
                &mut bounds,
                theme::TEXT,
                theme.fonts.small_bold,
                DT_LEFT | DT_SINGLELINE,
            ),
            Ink::Hint => text(
                hdc,
                &label.text,
                &mut bounds,
                theme::MUTED,
                theme.fonts.hint,
                DT_LEFT | DT_SINGLELINE | DT_END_ELLIPSIS,
            ),
        }
    }

    // Tab strip: the active tab is a pill, the strip sits on a hairline.
    if let (Some(first), Some(table)) = (spec.tabs.first(), &spec.table) {
        let line_y = first.0.bottom - 1;
        rule(
            hdc,
            &rect(
                table.left + theme.px(12),
                line_y,
                table.right - theme.px(12),
                line_y + 1,
            ),
            theme::HAIRLINE,
        );
    }
    for (bounds, title, active) in spec.tabs {
        let pill = rect(
            bounds.left + theme.px(2),
            bounds.top + theme.px(2),
            bounds.right - theme.px(2),
            bounds.bottom - theme.px(7),
        );
        if *active {
            card(
                hdc,
                &pill,
                theme.px(theme::R_CONTROL),
                theme::ACCENT_SOFT,
                None,
            );
        }
        let color = if *active { theme::ACCENT } else { theme::MUTED };
        let font = if *active {
            theme.fonts.small_bold
        } else {
            theme.fonts.small
        };
        let mut label = pill;
        text(
            hdc,
            title,
            &mut label,
            color,
            font,
            DT_CENTER | DT_SINGLELINE | DT_VCENTER,
        );
    }
}

// ---------------------------------------------------------------------------
// Owner drawn buttons and the scan overlay
// ---------------------------------------------------------------------------

/// Which look an owner drawn button uses.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonStyle {
    /// Filled with the accent colour, white label: the primary action.
    Accent,
    /// White with a hairline border: the secondary actions.
    Ghost,
}

/// Paint an owner drawn button from its `WM_DRAWITEM` payload. `hot` is the
/// pointer resting on it, which owner drawn buttons do not report by
/// themselves.
pub fn button(
    hdc: HDC,
    item: &DRAWITEMSTRUCT,
    style: ButtonStyle,
    hot: bool,
    theme: &theme::Theme,
) {
    let bounds = item.rcItem;
    let pressed = item.itemState & ODS_SELECTED != 0;
    let disabled = item.itemState & ODS_DISABLED != 0;
    let (background, border, ink) = match style {
        ButtonStyle::Accent => {
            let fill = if disabled {
                theme::TRACK_BG
            } else if pressed {
                theme::ACCENT_DARK
            } else if hot {
                theme::ACCENT_HOVER
            } else {
                theme::ACCENT
            };
            (
                fill,
                None,
                if disabled {
                    theme::MUTED
                } else {
                    theme::CARD_BG
                },
            )
        }
        // A secondary action is a filled chip rather than an outlined box:
        // no border to read as a Window 95 button.
        ButtonStyle::Ghost => {
            let fill = if disabled {
                theme::PANEL_BG
            } else if pressed {
                theme::CHIP_PRESS
            } else if hot {
                theme::CHIP_HOVER
            } else {
                theme::CHIP_BG
            };
            (fill, None, theme::TEXT)
        }
    };
    card(hdc, &bounds, theme.px(theme::R_CONTROL), background, border);
    let mut label = bounds;
    let mut text_buffer = vec![0u16; 256];
    let length = unsafe {
        GetWindowTextW(
            item.hwndItem,
            text_buffer.as_mut_ptr(),
            text_buffer.len() as i32,
        )
    };
    let label_text = String::from_utf16_lossy(&text_buffer[..length.max(0) as usize]);
    text(
        hdc,
        label_text.trim(),
        &mut label,
        ink,
        theme.fonts.base,
        DT_CENTER | DT_SINGLELINE | DT_VCENTER,
    );
}

/// Paint one row of the owner drawn scan-root list: the selected row gets a
/// washed accent pill, every row a little breathing room.
pub fn list_item(hdc: HDC, item: &DRAWITEMSTRUCT, theme: &theme::Theme) {
    let bounds = item.rcItem;
    fill(hdc, &bounds, theme::FIELD_BG);
    let index = item.itemID as i32;
    if index < 0 || item.hwndItem.is_null() {
        return;
    }
    let mut buffer = vec![0u16; 1024];
    let length = unsafe {
        SendMessageW(
            item.hwndItem,
            LB_GETTEXT,
            index as usize,
            buffer.as_mut_ptr() as isize,
        )
    };
    if length <= 0 {
        return;
    }
    let value = String::from_utf16_lossy(&buffer[..length as usize]);
    let selected = item.itemState & ODS_SELECTED != 0;
    let pad = theme.px(5);
    let pill = rect(
        bounds.left + pad,
        bounds.top + theme.px(2),
        bounds.right - pad,
        bounds.bottom - theme.px(2),
    );
    if selected {
        card(
            hdc,
            &pill,
            theme.px(theme::R_MARK),
            theme::ACCENT_SOFT,
            None,
        );
    }
    let colour = if selected {
        theme::ACCENT_DARK
    } else {
        theme::TEXT
    };
    let mut area = rect(
        pill.left + theme.px(8),
        bounds.top,
        pill.right - theme.px(6),
        bounds.bottom,
    );
    text(
        hdc,
        &value,
        &mut area,
        colour,
        theme.fonts.base,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
    );
}

/// The text of a control the shell draws by hand, so a painter can render
/// its own label without reaching into the window helpers.
fn window_text(hwnd: HWND) -> String {
    if hwnd.is_null() {
        return String::new();
    }
    let length = unsafe { GetWindowTextLengthW(hwnd) };
    if length <= 0 {
        return String::new();
    }
    let mut buffer = vec![0u16; length as usize + 1];
    let read = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    if read <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buffer[..read as usize])
}

/// The check glyph: one polyline of a white pen inside the box.
fn tick(hdc: HDC, box_area: &RECT, theme: &theme::Theme) {
    let width = box_area.right - box_area.left;
    let at = |x: f64, y: f64| POINT {
        x: box_area.left + (width as f64 * x / 15.0) as i32,
        y: box_area.top + (width as f64 * y / 15.0) as i32,
    };
    let points = [at(4.0, 8.2), at(6.6, 10.7), at(11.0, 4.6)];
    let pen = unsafe { CreatePen(PS_SOLID, theme.px(2).max(1), theme::FIELD_BG) };
    if pen.is_null() {
        return;
    }
    let previous = unsafe { SelectObject(hdc, pen as HGDIOBJ) };
    unsafe { Polyline(hdc, points.as_ptr(), points.len() as i32) };
    unsafe {
        SelectObject(hdc, previous);
        DeleteObject(pen as HGDIOBJ);
    }
}

/// Paint one owner drawn check box: a small rounded box that fills with the
/// accent when it is on, with the label beside it. The box has no toggle and
/// no check state of its own, so the caller passes the flag to draw.
pub fn checkbox(hdc: HDC, item: &DRAWITEMSTRUCT, on: bool, theme: &theme::Theme) {
    let bounds = item.rcItem;
    fill(hdc, &bounds, theme::CARD_BG);
    if item.hwndItem.is_null() {
        return;
    }
    let disabled = item.itemState & ODS_DISABLED != 0;
    let focused = item.itemState & ODS_FOCUS != 0;
    let side = theme.px(15);
    let top = bounds.top + (bounds.bottom - bounds.top - side) / 2;
    let area = rect(bounds.left, top, bounds.left + side, top + side);
    let mark = theme.px(theme::R_MARK);
    if on {
        let color = if disabled {
            theme::FIELD_BORDER
        } else if focused {
            theme::ACCENT_DARK
        } else {
            theme::ACCENT
        };
        card(hdc, &area, mark, color, None);
        tick(hdc, &area, theme);
    } else {
        let border = if focused {
            theme::ACCENT
        } else {
            theme::FIELD_BORDER
        };
        card(hdc, &area, mark, theme::FIELD_BG, Some(border));
    }
    let mut label = rect(
        area.right + theme.px(8),
        bounds.top,
        bounds.right,
        bounds.bottom,
    );
    let color = if disabled { theme::MUTED } else { theme::TEXT };
    text(
        hdc,
        &window_text(item.hwndItem),
        &mut label,
        color,
        theme.fonts.base,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
    );
}

/// The text of one entry of an owner drawn selector.
fn combo_text(hwnd: HWND, index: i32) -> String {
    let length = unsafe { SendMessageW(hwnd, CB_GETLBTEXTLEN, index as usize, 0) };
    if length <= 0 {
        return String::new();
    }
    let mut buffer = vec![0u16; length as usize + 1];
    let read = unsafe {
        SendMessageW(
            hwnd,
            CB_GETLBTEXT,
            index as usize,
            buffer.as_mut_ptr() as isize,
        )
    };
    if read <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buffer[..read as usize])
}

/// Paint one row of an owner drawn selector, or its closed field. The row the
/// keyboard or the pointer rests on gets the washed accent the scan-root list
/// uses, rather than the solid system highlight.
pub fn combo_item(hdc: HDC, item: &DRAWITEMSTRUCT, theme: &theme::Theme) {
    let bounds = item.rcItem;
    fill(hdc, &bounds, theme::FIELD_BG);
    if item.hwndItem.is_null() {
        return;
    }
    let index = item.itemID as i32;
    // A negative index is the closed field, which shows the current choice.
    let slot = if index < 0 {
        (unsafe { SendMessageW(item.hwndItem, CB_GETCURSEL, 0, 0) }) as i32
    } else {
        index
    };
    if slot < 0 {
        return;
    }
    let value = combo_text(item.hwndItem, slot);
    if index < 0 {
        let mut area = rect(
            bounds.left + theme.px(4),
            bounds.top,
            (bounds.right - theme.px(4)).max(bounds.left),
            bounds.bottom,
        );
        text(
            hdc,
            &value,
            &mut area,
            theme::TEXT,
            theme.fonts.base,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
        );
        return;
    }
    let selected = item.itemState & ODS_SELECTED != 0;
    let pill = rect(
        bounds.left + theme.px(2),
        bounds.top + theme.px(1),
        bounds.right - theme.px(2),
        bounds.bottom - theme.px(1),
    );
    if selected {
        card(
            hdc,
            &pill,
            theme.px(theme::R_MARK),
            theme::ACCENT_SOFT,
            None,
        );
    }
    let color = if selected {
        theme::ACCENT_DARK
    } else {
        theme::TEXT
    };
    let mut area = rect(
        pill.left + theme.px(8),
        bounds.top,
        pill.right - theme.px(6),
        bounds.bottom,
    );
    text(
        hdc,
        &value,
        &mut area,
        color,
        theme.fonts.base,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
    );
}

/// What the mask says while it is up: the caption over the bar, the stage the
/// counters belong to, and the file that stage is reading.
#[derive(Clone, Copy)]
pub struct Progress<'a> {
    pub title: &'a str,
    pub stage: Option<crate::scan::Stage>,
    pub done: usize,
    pub total: usize,
    pub path: &'a str,
}

/// Paint the scan overlay: scrim, card, progress track and caption. The stage
/// names the work the numbers belong to, so a bar that has just filled up
/// reads as "next: blaming" rather than as a scan that is somehow stuck. The
/// caption over it comes from the caller: a save that has to read the authors
/// before it can write says so.
pub fn overlay(hdc: HDC, area: &RECT, progress: &Progress<'_>, theme: &theme::Theme) {
    let Progress {
        title,
        stage,
        done,
        total,
        path,
    } = *progress;
    let width = area.right - area.left;
    let height = area.bottom - area.top;
    if width <= 0 || height <= 0 {
        return;
    }
    fill(hdc, area, theme::VEIL_BG);
    let card_width = theme.px(560).min(width - theme.px(40));
    let card_height = theme.px(172);
    let left = area.left + (width - card_width) / 2;
    let top = area.top + (height - card_height) / 2;
    let bounds = rect(left, top, left + card_width, top + card_height);
    // A soft ring under the card lifts it off the scrim without a real shadow.
    card(
        hdc,
        &rect(
            bounds.left - theme.px(4),
            bounds.top,
            bounds.right + theme.px(4),
            bounds.bottom + theme.px(6),
        ),
        theme.px(theme::R_CARD + 2),
        theme::SHADOW,
        None,
    );
    card(
        hdc,
        &bounds,
        theme.px(theme::R_CARD),
        theme::VEIL_CARD,
        Some(theme::VEIL_BORDER),
    );
    let padding = theme.px(26);
    let mut heading = rect(
        left + padding,
        top + theme.px(18),
        bounds.right - padding,
        top + theme.px(46),
    );
    text(
        hdc,
        title,
        &mut heading,
        theme::VEIL_TEXT,
        theme.fonts.title,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER,
    );
    let share = if total > 0 {
        format!("{:.0}%", done as f64 / total as f64 * 100.0)
    } else {
        "0%".to_string()
    };
    let mut counter = rect(
        left + padding,
        top + theme.px(18),
        bounds.right - padding,
        top + theme.px(46),
    );
    text(
        hdc,
        &share,
        &mut counter,
        theme::VEIL_ACCENT,
        theme.fonts.title,
        DT_RIGHT | DT_SINGLELINE | DT_VCENTER,
    );
    let track_top = top + theme.px(62);
    let track = rect(
        left + padding,
        track_top,
        bounds.right - padding,
        track_top + theme.px(14),
    );
    card(
        hdc,
        &track,
        theme.px(theme::R_MARK),
        theme::VEIL_TRACK,
        None,
    );
    let track_width = (track.right - track.left).max(1) as u64;
    let span = (done as u64)
        .saturating_mul(track_width)
        .checked_div(total as u64)
        .unwrap_or(0) as i32;
    if span > 0 {
        let filled = rect(
            track.left,
            track.top,
            (track.left + span).min(track.right),
            track.bottom,
        );
        card(
            hdc,
            &filled,
            theme.px(theme::R_MARK),
            theme::VEIL_ACCENT,
            None,
        );
    }
    let mut caption = rect(
        left + padding,
        track.bottom + theme.px(12),
        bounds.right - padding,
        bounds.bottom - theme.px(18),
    );
    let counter = if total > 0 {
        i18n::fill(
            i18n::t().overlay_files,
            &[("done", done.to_string()), ("total", total.to_string())],
        )
    } else {
        i18n::t().overlay_discovering.to_string()
    };
    let counter = match stage {
        Some(crate::scan::Stage::Blame) => {
            format!("{} · {counter}", i18n::t().overlay_stage_blame)
        }
        Some(crate::scan::Stage::Files) => {
            format!("{} · {counter}", i18n::t().overlay_stage_files)
        }
        None => counter,
    };
    let caption_text = if path.is_empty() {
        counter
    } else {
        format!("{counter} · {path}")
    };
    text(
        hdc,
        &caption_text,
        &mut caption,
        theme::VEIL_MUTED,
        theme.fonts.base,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
    );
}
