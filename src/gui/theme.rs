//! Palette, fonts and metrics shared by every painter.
//!
//! The colours are the same literals the Python reference front end uses
//! (`python/src/rust_loc/ui_model.py`), so both shells look identical.

use crate::gui::i18n::{self, Lang};
use crate::gui::sys::*;

/// Pack a `0xRRGGBB` literal into the `COLORREF` layout GDI expects.
pub const fn rgb(value: u32) -> u32 {
    ((value & 0xFF) << 16) | (value & 0xFF00) | ((value >> 16) & 0xFF)
}

pub const PANEL_BG: u32 = rgb(0xF1F3F9);
pub const CARD_BG: u32 = rgb(0xFFFFFF);
pub const BORDER: u32 = rgb(0xE3E8F0);
/// A hairline between table rows.
pub const HAIRLINE: u32 = rgb(0xEEF1F7);
/// The one pixel lip under a raised card.
pub const SHADOW: u32 = rgb(0xE7EBF3);
pub const TEXT: u32 = rgb(0x111827);
pub const MUTED: u32 = rgb(0x667085);
pub const ACCENT: u32 = rgb(0x2563EB);
pub const ACCENT_DARK: u32 = rgb(0x1D4ED8);
/// Washed accent for the active tab and the selected row.
pub const ACCENT_SOFT: u32 = rgb(0xEDF3FF);
pub const TRACK_BG: u32 = rgb(0xEDF1F7);
pub const BAR_TRACK: u32 = rgb(0xF0F3F9);
/// A text field, list or selector: white inside a rounded hairline.
pub const FIELD_BG: u32 = rgb(0xFFFFFF);
/// A touch darker than a card border, so an empty field still reads as one.
pub const FIELD_BORDER: u32 = rgb(0xD7DEEA);
/// The resting fill of a secondary button; it darkens under the pointer.
pub const CHIP_BG: u32 = rgb(0xEEF1F8);
pub const CHIP_HOVER: u32 = rgb(0xE3E8F3);
pub const CHIP_PRESS: u32 = rgb(0xD6DEED);
/// A lighter blue under the pointer, darker while pressed.
pub const ACCENT_HOVER: u32 = rgb(0x3B78F0);

/// Corner radii, in logical pixels. The chrome stays close to square on
/// purpose - a three to five pixel arc reads as a modern surface, while
/// anything rounder reads as a bubble.
pub const R_CARD: i32 = 5;
pub const R_CONTROL: i32 = 4;
pub const R_MARK: i32 = 3;

pub const COLOR_CODE: u32 = rgb(0x2563EB);
pub const COLOR_DOC: u32 = rgb(0x8B5CF6);
pub const COLOR_COMMENT: u32 = rgb(0x22C55E);
pub const COLOR_BLANK: u32 = rgb(0xCBD5E1);
pub const COLOR_TEST: u32 = rgb(0xF59E0B);
pub const COLOR_COMPLEXITY: u32 = rgb(0x7C3AED);
pub const COLOR_UNSAFE: u32 = rgb(0xDC2626);
pub const COLOR_TEST_FNS: u32 = rgb(0x0891B2);
pub const COLOR_ULOC: u32 = rgb(0x16A34A);

/// The dark popover a chart tooltip is painted on.
pub const TIP_BG: u32 = rgb(0x101828);
pub const TIP_BORDER: u32 = rgb(0x2E3A52);
pub const TIP_TEXT: u32 = rgb(0xE8EDF8);

/// The scan overlay: a light scrim that stays inside the app palette so the
/// progress card reads as part of the window instead of a second theme.
pub const VEIL_BG: u32 = rgb(0xF2F5FA);
pub const VEIL_CARD: u32 = rgb(0xFFFFFF);
pub const VEIL_BORDER: u32 = rgb(0xE3E8F0);
pub const VEIL_TEXT: u32 = rgb(0x111827);
pub const VEIL_MUTED: u32 = rgb(0x667085);
pub const VEIL_TRACK: u32 = rgb(0xEDF1F7);
pub const VEIL_ACCENT: u32 = rgb(0x2563EB);
/// The stage in hand, paler than the ones already done.
pub const VEIL_ACCENT_SOFT: u32 = rgb(0x9DBAF6);

const FW_SEMIBOLD: i32 = 600;
const DEFAULT_CHARSET: u8 = 1;
const CLEARTYPE_QUALITY: u8 = 5;

/// One font per role; GDI handles are released when the theme is replaced.
#[derive(Default)]
pub struct Fonts {
    /// Widgets, buttons and entries.
    pub base: HFONT,
    /// Table headers and emphasised body text.
    pub bold: HFONT,
    /// The report table's column header.
    pub header: HFONT,
    /// The report table.
    pub mono: HFONT,
    /// The header summary on the right of the title bar.
    pub mono_small: HFONT,
    /// The "rust-loc" wordmark.
    pub brand: HFONT,
    /// The overlay headline.
    pub title: HFONT,
    /// A KPI card value.
    pub value: HFONT,
    /// Generic small text.
    pub label: HFONT,
    /// Uppercase section and card captions.
    pub label_bold: HFONT,
    /// Card and table titles.
    pub small: HFONT,
    /// Chart legends and numeric chart labels.
    pub small_bold: HFONT,
    /// Captions, hints and inline notes.
    pub hint: HFONT,
}

impl Drop for Fonts {
    fn drop(&mut self) {
        for handle in [
            self.base,
            self.bold,
            self.header,
            self.mono,
            self.mono_small,
            self.brand,
            self.title,
            self.value,
            self.label,
            self.label_bold,
            self.small,
            self.small_bold,
            self.hint,
        ] {
            if !handle.is_null() {
                unsafe { DeleteObject(handle as HGDIOBJ) };
            }
        }
    }
}

pub struct Theme {
    pub dpi: u32,
    pub fonts: Fonts,
}

impl Theme {
    pub fn new(hwnd: HWND) -> Theme {
        let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
        // Segoe UI has no CJK coverage; the Chinese interface uses the system
        // UI face for Chinese so the glyphs sit on the Latin baseline cleanly.
        let ui = match i18n::lang() {
            Lang::Zh => "Microsoft YaHei UI",
            Lang::En => "Segoe UI",
        };
        let fonts = Fonts {
            base: make_font(ui, scale(100, dpi), 400),
            bold: make_font(ui, scale(100, dpi), FW_SEMIBOLD),
            header: make_font(ui, scale(96, dpi), FW_SEMIBOLD),
            mono: make_font("Cascadia Mono", scale(94, dpi), 400),
            mono_small: make_font("Cascadia Mono", scale(86, dpi), 400),
            brand: make_font(ui, scale(155, dpi), FW_SEMIBOLD),
            title: make_font(ui, scale(155, dpi), FW_SEMIBOLD),
            value: make_font(ui, scale(170, dpi), FW_SEMIBOLD),
            label: make_font(ui, scale(92, dpi), 400),
            label_bold: make_font(ui, scale(82, dpi), FW_SEMIBOLD),
            small: make_font(ui, scale(97, dpi), 400),
            small_bold: make_font(ui, scale(93, dpi), FW_SEMIBOLD),
            hint: make_font(ui, scale(87, dpi), 400),
        };
        Theme { dpi, fonts }
    }

    /// Scale a 96 DPI design pixel value.
    pub fn px(&self, value: i32) -> i32 {
        (value * self.dpi as i32 + 48) / 96
    }
}

/// Point size in tenths to a negative character height for the given DPI.
fn scale(tenths: i32, dpi: u32) -> i32 {
    (tenths * dpi as i32 + 360) / 720
}

fn make_font(face: &str, height: i32, weight: i32) -> HFONT {
    let mut info: LOGFONTW = unsafe { std::mem::zeroed() };
    info.lfHeight = -height.max(1);
    info.lfWeight = weight;
    info.lfCharSet = DEFAULT_CHARSET;
    info.lfQuality = CLEARTYPE_QUALITY;
    info.lfPitchAndFamily = 0;
    for (slot, code) in info
        .lfFaceName
        .iter_mut()
        .zip(face.encode_utf16().chain(std::iter::once(0)))
    {
        *slot = code;
    }
    unsafe { CreateFontIndirectW(&info) }
}
