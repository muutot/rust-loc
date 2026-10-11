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
            theme.px(10),
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
    clipped(hdc, &track, bar_height / 2, || {
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
            theme.px(6),
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
                theme.px(6),
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
        theme.px(6),
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
        theme.px(10),
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
            theme.px(3),
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
    pub table: RECT,
    pub tabs: &'a [(RECT, &'a str, bool)],
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
    card(hdc, &chip, theme.px(9), theme::TRACK_BG, None);
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

    // Table card.
    card(
        hdc,
        &spec.table,
        theme.px(10),
        theme::CARD_BG,
        Some(theme::BORDER),
    );

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
    if let Some(first) = spec.tabs.first() {
        let line_y = first.0.bottom - 1;
        rule(
            hdc,
            &rect(
                spec.table.left + theme.px(12),
                line_y,
                spec.table.right - theme.px(12),
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
                (pill.bottom - pill.top) / 2,
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

/// Paint an owner drawn button from its `WM_DRAWITEM` payload.
pub fn button(hdc: HDC, item: &DRAWITEMSTRUCT, style: ButtonStyle, theme: &theme::Theme) {
    let bounds = item.rcItem;
    let pressed = item.itemState & ODS_SELECTED != 0;
    let disabled = item.itemState & ODS_DISABLED != 0;
    let (background, border, ink) = match style {
        ButtonStyle::Accent => {
            let fill = if disabled {
                theme::TRACK_BG
            } else if pressed {
                theme::ACCENT_DARK
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
        ButtonStyle::Ghost => {
            let fill = if disabled {
                theme::PANEL_BG
            } else if pressed {
                theme::TRACK_BG
            } else {
                theme::CARD_BG
            };
            (fill, Some(theme::BORDER), theme::TEXT)
        }
    };
    card(hdc, &bounds, theme.px(8), background, border);
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

/// Paint the scan overlay: scrim, card, progress track and caption.
pub fn overlay(hdc: HDC, area: &RECT, done: usize, total: usize, path: &str, theme: &theme::Theme) {
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
        theme.px(18),
        theme::SHADOW,
        None,
    );
    card(
        hdc,
        &bounds,
        theme.px(14),
        theme::VEIL_CARD,
        Some(theme::VEIL_BORDER),
    );
    let padding = theme.px(26);
    let mut title = rect(
        left + padding,
        top + theme.px(18),
        bounds.right - padding,
        top + theme.px(46),
    );
    text(
        hdc,
        i18n::t().overlay_title,
        &mut title,
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
    card(hdc, &track, theme.px(7), theme::VEIL_TRACK, None);
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
        card(hdc, &filled, theme.px(7), theme::VEIL_ACCENT, None);
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
