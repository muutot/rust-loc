//! The report card, drawn straight into a bitmap for the PNG export.
//!
//! The layout mirrors `svg::render`, so the vector export and the bitmap show
//! the same report. Everything here is GDI: a 32-bit DIB, four fonts and
//! `DrawTextW`, with the pixels handed to `png`. No window is needed, which is
//! what lets the layout be measured and painted in the same pass.

use std::path::Path;

use crate::gui::sys::*;
use crate::options::Options;
use crate::png;
use crate::report::{self, Report};
use crate::scan;
use crate::VERSION;

const WIDTH: i32 = 1000;
const PAD: i32 = 32;
const ROW: i32 = 24;

fn rgb(red: u32, green: u32, blue: u32) -> u32 {
    (blue << 16) | (green << 8) | red
}

/// `COLORREF` values, the same palette the SVG uses.
fn ink() -> u32 {
    rgb(0x1b, 0x1f, 0x24)
}
fn muted() -> u32 {
    rgb(0x5b, 0x65, 0x70)
}
fn accent() -> u32 {
    rgb(0x2f, 0x6f, 0xed)
}
fn green() -> u32 {
    rgb(0x22, 0xa0, 0x6b)
}
fn amber() -> u32 {
    rgb(0xd9, 0x83, 0x24)
}
fn panel() -> u32 {
    rgb(0xf7, 0xf9, 0xfc)
}
fn line() -> u32 {
    rgb(0xe3, 0xe7, 0xec)
}

/// Which font and colour a run of text uses.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    Title,
    Value,
    Body,
    Small,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Anchor {
    Left,
    Right,
}

/// Where a card is drawn. Measuring ignores the calls and only walks the
/// layout, so the bitmap can be sized before a single pixel is painted.
trait Painter {
    fn rect(&mut self, x: i32, y: i32, width: i32, height: i32, radius: i32, colour: u32);
    fn text(&mut self, x: i32, y: i32, style: Style, colour: u32, anchor: Anchor, content: &str);
}

struct Measure;

impl Painter for Measure {
    fn rect(&mut self, _x: i32, _y: i32, _width: i32, _height: i32, _radius: i32, _colour: u32) {}
    fn text(
        &mut self,
        _x: i32,
        _y: i32,
        _style: Style,
        _colour: u32,
        _anchor: Anchor,
        _content: &str,
    ) {
    }
}

/// Render the report as PNG bytes, or `None` when GDI refuses a bitmap.
pub fn encode(report: &Report, options: &Options, cwd: &Path) -> Option<Vec<u8>> {
    let height = layout(&mut Measure, report, options, cwd).max(1);
    let target = Target::new(WIDTH, height)?;
    let mut gdi = Gdi::new(target.hdc);
    layout(&mut gdi, report, options, cwd);
    gdi.release();
    Some(target.bytes(WIDTH, height))
}

/// Paint the card, returning the bottom edge. Called twice: once with
/// `Measure` to size the bitmap, once with GDI to fill it.
fn layout(painter: &mut dyn Painter, report: &Report, options: &Options, cwd: &Path) -> i32 {
    let summary = &report.summary;
    let mut y = PAD;
    let heading = crate::report::scope_title(options.sources, options.include_tests);
    painter.text(
        PAD,
        y,
        Style::Title,
        ink(),
        Anchor::Left,
        &format!("rust-loc {VERSION}"),
    );
    painter.text(
        WIDTH - PAD,
        y + 10,
        Style::Body,
        muted(),
        Anchor::Right,
        &heading,
    );
    y += 40;
    let roots: Vec<String> = report
        .roots
        .iter()
        .map(|root| scan::relative(root, cwd))
        .collect();
    painter.text(
        PAD,
        y,
        Style::Small,
        muted(),
        Anchor::Left,
        &format!("Roots: {}", roots.join(", ")),
    );
    y += 30;

    let mut cards: Vec<(String, String, u32)> = vec![
        (
            "Files".into(),
            count(report.business.len() as u64),
            accent(),
        ),
        ("Lines".into(), count(summary.lines.into()), ink()),
        ("Blank".into(), count(summary.blank.into()), muted()),
        ("Comment".into(), count(summary.comment.into()), green()),
        ("Code".into(), count(summary.code.into()), accent()),
    ];
    if options.complexity {
        cards.push((
            "Complexity".into(),
            count(summary.complexity.into()),
            amber(),
        ));
    }
    cards.push(("Unsafe".into(), count(summary.unsafe_kw.into()), amber()));
    cards.push(("Test fns".into(), count(summary.test_fns.into()), muted()));
    if options.uloc {
        cards.push((
            "ULOC".into(),
            count(report.uloc_total.unwrap_or(0) as u64),
            green(),
        ));
    }
    if options.smells {
        cards.push((
            "Smells".into(),
            count(summary.smells.total().into()),
            amber(),
        ));
    }
    y = cards_at(painter, y, &cards);

    let lines = summary.lines.max(1);
    let bar = WIDTH - 2 * PAD;
    let code = (bar as i64 * i64::from(summary.code) / i64::from(lines)) as i32;
    let comment = (bar as i64 * i64::from(summary.comment) / i64::from(lines)) as i32;
    painter.rect(PAD, y, bar, 18, 6, line());
    if code > 0 {
        painter.rect(PAD, y, code, 18, 6, accent());
    }
    if comment > 0 {
        painter.rect(PAD + code, y, comment, 18, 6, green());
    }
    y += 30;
    let legend = [
        ("Code", summary.code, accent(), PAD),
        ("Comment", summary.comment, green(), PAD + 280),
        ("Blank", summary.blank, muted(), PAD + 560),
    ];
    for (label, value, colour, x) in legend {
        painter.rect(x, y + 3, 11, 11, 2, colour);
        painter.text(
            x + 18,
            y,
            Style::Small,
            muted(),
            Anchor::Left,
            &format!(
                "{label} {} ({})",
                report::group(u64::from(value)),
                share(value, lines)
            ),
        );
    }
    y += 40;

    let limit = if options.top > 0 { options.top } else { 20 };
    let ordered = scan::sort_entries(report.business_entries(), "code");
    let shown = ordered.len().min(limit);
    painter.text(
        PAD,
        y,
        Style::Value,
        ink(),
        Anchor::Left,
        "Top files by code",
    );
    y += 30;
    y = table_header(painter, y, &["File", "Lines", "Comment", "Test", "Code"]);
    for (index, entry) in ordered.iter().take(shown).enumerate() {
        if index % 2 == 1 {
            painter.rect(PAD, y - 3, WIDTH - 2 * PAD, ROW, 4, panel());
        }
        let path = scan::relative(entry.path(), cwd);
        let cells = [
            clip(&path, 60),
            count(entry.stats.lines.into()),
            count(entry.stats.comment.into()),
            count(entry.stats.test.into()),
            count(entry.stats.code.into()),
        ];
        y = table_row(painter, y, &cells);
    }
    y += 14;

    if options.languages && !report.languages.is_empty() {
        let total: u32 = report.languages.iter().map(|row| row.code).sum();
        painter.text(PAD, y, Style::Value, ink(), Anchor::Left, "Languages");
        y += 30;
        y = table_header(painter, y, &["Language", "Files", "Lines", "Code", "Share"]);
        for (index, row) in report.languages.iter().enumerate() {
            if index % 2 == 1 {
                painter.rect(PAD, y - 3, WIDTH - 2 * PAD, ROW, 4, panel());
            }
            let cells = [
                row.name.to_string(),
                count(row.files.into()),
                count(row.lines.into()),
                count(row.code.into()),
                share(row.code, total),
            ];
            y = table_row(painter, y, &cells);
        }
        y += 14;
    }

    y += 6;
    painter.rect(PAD, y, WIDTH - 2 * PAD, 1, 0, line());
    y += 16;
    painter.text(
        PAD,
        y,
        Style::Small,
        muted(),
        Anchor::Left,
        &format!(
            "{} file(s), {} code lines - rust-loc {VERSION}",
            report::group(report.business.len() as u64),
            report::group(u64::from(summary.code))
        ),
    );
    y += 20 + PAD;
    y
}

fn cards_at(painter: &mut dyn Painter, y: i32, cards: &[(String, String, u32)]) -> i32 {
    let columns = 5usize;
    let gap = 12;
    let width = (WIDTH - 2 * PAD - gap * (columns as i32 - 1)) / columns as i32;
    for (index, (label, value, colour)) in cards.iter().enumerate() {
        let x = PAD + (index % columns) as i32 * (width + gap);
        let top = y + (index / columns) as i32 * 80;
        painter.rect(x, top, width, 64, 10, panel());
        painter.text(x + 14, top + 12, Style::Value, *colour, Anchor::Left, value);
        painter.text(x + 14, top + 42, Style::Small, muted(), Anchor::Left, label);
    }
    y + cards.len().div_ceil(columns) as i32 * 80 + 6
}

/// Column x positions, shared by the header and the rows below it.
const COLUMNS: [i32; 5] = [0, 300, 200, 100, 0];

fn table_header(painter: &mut dyn Painter, y: i32, labels: &[&str]) -> i32 {
    for (index, label) in labels.iter().enumerate() {
        let (x, anchor) = column(index);
        painter.text(x, y, Style::Small, muted(), anchor, label);
    }
    y + ROW - 4
}

fn table_row(painter: &mut dyn Painter, y: i32, cells: &[String]) -> i32 {
    for (index, cell) in cells.iter().enumerate() {
        let (x, anchor) = column(index);
        let colour = if index == 0 || index + 1 == cells.len() {
            ink()
        } else {
            muted()
        };
        painter.text(x, y, Style::Small, colour, anchor, cell);
    }
    y + ROW
}

fn column(index: usize) -> (i32, Anchor) {
    match index {
        0 => (PAD, Anchor::Left),
        _ => (WIDTH - PAD - COLUMNS[index], Anchor::Right),
    }
}

fn share(value: u32, total: u32) -> String {
    if total == 0 {
        return "0.0%".to_string();
    }
    format!("{:.1}%", f64::from(value) / f64::from(total) * 100.0)
}

fn count(value: u64) -> String {
    report::group(value)
}

fn clip(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let mut out: String = text.chars().take(width.saturating_sub(2)).collect();
    out.push_str("..");
    out
}

const TRANSPARENT: i32 = 1;
const DEFAULT_CHARSET: u8 = 1;
const CLEARTYPE_QUALITY: u8 = 5;

const DT_LEFT: u32 = 0;
const DT_VCENTER: u32 = 4;
const DT_SINGLELINE: u32 = 0x20;
const DT_NOPREFIX: u32 = 0x800;

impl Style {
    /// The pixel size a run is drawn at.
    fn size(self) -> i32 {
        match self {
            Style::Title => 22,
            Style::Value => 15,
            Style::Body => 13,
            Style::Small => 12,
        }
    }

    /// The line box a run sits in: the glyphs plus a little leading.
    fn line(self) -> i32 {
        self.size() + 7
    }

    fn weight(self) -> i32 {
        match self {
            Style::Title | Style::Value => 600,
            Style::Body | Style::Small => 400,
        }
    }

    /// The index of this style's font in `Gdi::fonts`.
    fn slot(self) -> usize {
        match self {
            Style::Title => 0,
            Style::Value => 1,
            Style::Body => 2,
            Style::Small => 3,
        }
    }
}

/// A top-down 32-bit DIB and the DC that paints into it. Dropping the target
/// puts the stock bitmap back and frees both handles.
struct Target {
    hdc: HDC,
    bits: *mut std::ffi::c_void,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
}

impl Target {
    fn new(width: i32, height: i32) -> Option<Target> {
        if width <= 0 || height <= 0 {
            return None;
        }
        unsafe {
            let hdc = CreateCompatibleDC(std::ptr::null_mut());
            if hdc.is_null() {
                return None;
            }
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = width;
            // A negative height asks for top-down rows, the order PNG wants.
            info.bmiHeader.biHeight = -height;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB;
            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let bitmap = CreateDIBSection(
                hdc,
                &info,
                DIB_RGB_COLORS,
                &mut bits,
                std::ptr::null_mut(),
                0,
            );
            if bitmap.is_null() || bits.is_null() {
                DeleteDC(hdc);
                return None;
            }
            let previous = SelectObject(hdc, bitmap as HGDIOBJ);
            let brush: HBRUSH = CreateSolidBrush(rgb(0xff, 0xff, 0xff));
            let area = RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            };
            FillRect(hdc, &area, brush);
            DeleteObject(brush as HGDIOBJ);
            Some(Target {
                hdc,
                bits,
                bitmap,
                previous,
            })
        }
    }

    /// Read the DIB back as RGBA and encode it as a PNG.
    fn bytes(&self, width: i32, height: i32) -> Vec<u8> {
        let count = width as usize * height as usize;
        let source = unsafe { std::slice::from_raw_parts(self.bits as *const u8, count * 4) };
        let mut rgba = Vec::with_capacity(count * 4);
        // GDI leaves the alpha byte at zero; every pixel here is opaque.
        for pixel in source.chunks_exact(4) {
            rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
        }
        png::encode(width as u32, height as u32, &rgba)
    }
}

impl Drop for Target {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.hdc, self.previous);
            DeleteObject(self.bitmap as HGDIOBJ);
            DeleteDC(self.hdc);
        }
    }
}

/// The GDI painter: one font per style, `DrawTextW` for every run.
struct Gdi {
    hdc: HDC,
    fonts: [HFONT; 4],
    previous: HGDIOBJ,
}

impl Gdi {
    fn new(hdc: HDC) -> Gdi {
        let fonts = [
            make_font(Style::Title),
            make_font(Style::Value),
            make_font(Style::Body),
            make_font(Style::Small),
        ];
        unsafe { SetBkMode(hdc, TRANSPARENT) };
        Gdi {
            hdc,
            fonts,
            previous: std::ptr::null_mut(),
        }
    }

    /// Choose the font for a style, remembering the stock object the first
    /// time so it can be selected back before the DC goes away.
    fn select(&mut self, style: Style) {
        let font = self.fonts[style.slot()];
        let previous = unsafe { SelectObject(self.hdc, font as HGDIOBJ) };
        if self.previous.is_null() {
            self.previous = previous;
        }
    }

    /// Put the stock object back so the fonts can be freed.
    fn release(&mut self) {
        if !self.previous.is_null() {
            unsafe { SelectObject(self.hdc, self.previous) };
            self.previous = std::ptr::null_mut();
        }
    }
}

impl Drop for Gdi {
    fn drop(&mut self) {
        self.release();
        for font in self.fonts {
            if !font.is_null() {
                unsafe { DeleteObject(font as HGDIOBJ) };
            }
        }
    }
}

impl Painter for Gdi {
    fn rect(&mut self, x: i32, y: i32, width: i32, height: i32, radius: i32, colour: u32) {
        if width <= 0 || height <= 0 {
            return;
        }
        unsafe {
            let brush: HBRUSH = CreateSolidBrush(colour);
            let pen = CreatePen(PS_SOLID, 1, colour);
            let old_brush = SelectObject(self.hdc, brush as HGDIOBJ);
            let old_pen = SelectObject(self.hdc, pen as HGDIOBJ);
            if radius > 0 {
                RoundRect(
                    self.hdc,
                    x,
                    y,
                    x + width,
                    y + height,
                    radius * 2,
                    radius * 2,
                );
            } else {
                let area = RECT {
                    left: x,
                    top: y,
                    right: x + width,
                    bottom: y + height,
                };
                FillRect(self.hdc, &area, brush);
            }
            SelectObject(self.hdc, old_brush);
            SelectObject(self.hdc, old_pen);
            DeleteObject(brush as HGDIOBJ);
            DeleteObject(pen as HGDIOBJ);
        }
    }

    fn text(&mut self, x: i32, y: i32, style: Style, colour: u32, anchor: Anchor, content: &str) {
        if content.is_empty() {
            return;
        }
        self.select(style);
        let wide: Vec<u16> = content.encode_utf16().chain(std::iter::once(0)).collect();
        let width = unsafe {
            let mut size = SIZE { cx: 0, cy: 0 };
            GetTextExtentPoint32W(self.hdc, wide.as_ptr(), wide.len() as i32 - 1, &mut size);
            size.cx
        };
        let left = match anchor {
            Anchor::Left => x,
            Anchor::Right => x - width,
        };
        let mut area = RECT {
            left,
            top: y,
            right: left + width.max(1) + 2,
            bottom: y + style.line(),
        };
        unsafe {
            SetTextColor(self.hdc, colour);
            DrawTextW(
                self.hdc,
                wide.as_ptr(),
                -1,
                &mut area,
                DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
            );
        }
    }
}

/// A font for one style, in the monospace face the SVG report names.
fn make_font(style: Style) -> HFONT {
    let mut info: LOGFONTW = unsafe { std::mem::zeroed() };
    info.lfHeight = -style.size();
    info.lfWeight = style.weight();
    info.lfCharSet = DEFAULT_CHARSET;
    info.lfQuality = CLEARTYPE_QUALITY;
    for (slot, code) in info
        .lfFaceName
        .iter_mut()
        .zip("Consolas".encode_utf16().chain(std::iter::once(0)))
    {
        *slot = code;
    }
    unsafe { CreateFontIndirectW(&info) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Scan a throwaway tree so the card has real numbers to draw.
    fn report_from(name: &str, source: &str) -> (Report, Options, PathBuf) {
        let root = std::env::temp_dir().join(format!("rustloc-card-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("main.rs"), source).unwrap();
        let options = Options::default();
        let report = report::scan(std::slice::from_ref(&root), &options, &root, None);
        (report, options, root)
    }

    const SOURCE: &str =
        "fn main() {\n    let value = 1;\n    println!(\"{value}\");\n}\n\n/// Doc.\nfn helper() -> i32 {\n    2\n}\n";

    #[test]
    fn the_card_is_a_png_of_the_reported_size() {
        let (report, options, root) = report_from("size", SOURCE);
        let bytes = encode(&report, &options, &root).expect("encode");
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(&bytes[..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        assert_eq!(&bytes[12..16], b"IHDR");
        assert_eq!(
            u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
            WIDTH as u32
        );
        let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
        assert!(height > 300, "the card has a body, got {height}");
    }

    #[test]
    fn the_card_paints_more_than_the_background() {
        let (report, options, root) = report_from("paint", SOURCE);
        let bytes = encode(&report, &options, &root).expect("encode");
        let _ = std::fs::remove_dir_all(&root);
        let (width, height, rgba) = crate::png::decode_rgba(&bytes);
        assert_eq!(width, WIDTH as u32);
        assert!(height > 300, "the card has a body, got {height}");
        let painted = rgba
            .chunks_exact(4)
            .filter(|pixel| pixel[..3] != [255, 255, 255])
            .count();
        assert!(painted > 10_000, "expected a painted card, got {painted}");
    }

    /// Regenerate the PNG the README shows. Run it by hand after a layout change:
    /// `cargo test --lib write_the_documented_card -- --ignored`.
    #[test]
    #[ignore = "writes docs/report.png by hand"]
    fn write_the_documented_card() {
        let cwd = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let root = cwd.to_path_buf();
        let options = Options {
            languages: true,
            ..Options::default()
        };
        let report = report::scan(std::slice::from_ref(&root), &options, cwd, None);
        let bytes = encode(&report, &options, cwd).expect("encode");
        std::fs::write(cwd.join("docs/report.png"), bytes).unwrap();
    }
}
