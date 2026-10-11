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

pub const PANEL_BG: u32 = rgb(0xF3F5F9);
pub const CARD_BG: u32 = rgb(0xFFFFFF);
pub const GRID_BG: u32 = rgb(0xFBFCFE);
pub const BORDER: u32 = rgb(0xE2E6EE);
pub const TEXT: u32 = rgb(0x1F2430);
pub const MUTED: u32 = rgb(0x6B7280);
pub const ACCENT: u32 = rgb(0x2563EB);
pub const ACCENT_DARK: u32 = rgb(0x1D4ED8);
pub const TRACK_BG: u32 = rgb(0xEEF1F6);
pub const BAR_TRACK: u32 = rgb(0xF1F4F9);

pub const COLOR_CODE: u32 = rgb(0x2563EB);
pub const COLOR_DOC: u32 = rgb(0x8B5CF6);
pub const COLOR_COMMENT: u32 = rgb(0x22C55E);
pub const COLOR_BLANK: u32 = rgb(0xCBD5E1);
pub const COLOR_TEST: u32 = rgb(0xF59E0B);
pub const COLOR_COMPLEXITY: u32 = rgb(0x7C3AED);
pub const COLOR_UNSAFE: u32 = rgb(0xDC2626);
pub const COLOR_TEST_FNS: u32 = rgb(0x0891B2);
pub const COLOR_ULOC: u32 = rgb(0x16A34A);

pub const VEIL_BG: u32 = rgb(0x101828);
pub const VEIL_CARD: u32 = rgb(0x1B2437);
pub const VEIL_BORDER: u32 = rgb(0x2E3A52);
pub const VEIL_TEXT: u32 = rgb(0xE8EDF8);
pub const VEIL_MUTED: u32 = rgb(0x9AA9C6);
pub const VEIL_TRACK: u32 = rgb(0x243352);
pub const VEIL_ACCENT: u32 = rgb(0x60A5FA);

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
            base: make_font(ui, scale(95, dpi), 400),
            bold: make_font(ui, scale(95, dpi), FW_SEMIBOLD),
            mono: make_font("Cascadia Mono", scale(90, dpi), 400),
            mono_small: make_font("Cascadia Mono", scale(82, dpi), 400),
            brand: make_font(ui, scale(145, dpi), FW_SEMIBOLD),
            title: make_font(ui, scale(150, dpi), FW_SEMIBOLD),
            value: make_font(ui, scale(150, dpi), FW_SEMIBOLD),
            label: make_font(ui, scale(88, dpi), 400),
            label_bold: make_font(ui, scale(80, dpi), FW_SEMIBOLD),
            small: make_font(ui, scale(92, dpi), 400),
            small_bold: make_font(ui, scale(88, dpi), FW_SEMIBOLD),
            hint: make_font(ui, scale(80, dpi), 400),
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
