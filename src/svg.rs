//! SVG report card.
//!
//! `--format svg` and the export menu write the scan as a standalone vector
//! document: no HTML and no browser, just shapes and text any viewer or a
//! README can show.

use std::fmt::Write;
use std::path::Path;

use crate::options::Options;
use crate::report::{self, Report};
use crate::scan;
use crate::VERSION;

const WIDTH: i32 = 1000;
const PAD: i32 = 32;
const ROW: i32 = 22;

const INK: &str = "#1b1f24";
const MUTED: &str = "#5b6570";
const ACCENT: &str = "#2f6fed";
const GREEN: &str = "#22a06b";
const AMBER: &str = "#d98324";
const LINE: &str = "#e3e7ec";
const PANEL: &str = "#f7f9fc";

/// Draw the whole report as one SVG document.
pub fn render(report: &Report, options: &Options, cwd: &Path) -> String {
    let summary = &report.summary;
    let mut canvas = Canvas::new();
    let mut y = PAD + 6;

    let heading = crate::report::scope_title(options.sources, options.include_tests);
    canvas.text(
        PAD,
        y,
        24,
        INK,
        "700",
        "start",
        &format!("rust-loc {VERSION}"),
    );
    canvas.text(WIDTH - PAD, y, 14, MUTED, "400", "end", heading);
    y += 24;
    let roots: Vec<String> = report
        .roots
        .iter()
        .map(|root| scan::relative(root, cwd))
        .collect();
    canvas.text(
        PAD,
        y,
        12,
        MUTED,
        "400",
        "start",
        &format!("Roots: {}", roots.join(", ")),
    );
    y += 26;

    let mut cards: Vec<(String, String, &str)> = vec![
        ("Files".into(), count(report.business.len() as u64), ACCENT),
        ("Lines".into(), count(summary.lines.into()), INK),
        ("Blank".into(), count(summary.blank.into()), MUTED),
        ("Comment".into(), count(summary.comment.into()), GREEN),
        ("Code".into(), count(summary.code.into()), ACCENT),
    ];
    if options.complexity {
        cards.push(("Complexity".into(), count(summary.complexity.into()), AMBER));
    }
    cards.push(("Unsafe".into(), count(summary.unsafe_kw.into()), AMBER));
    cards.push(("Test fns".into(), count(summary.test_fns.into()), MUTED));
    if options.uloc {
        cards.push((
            "ULOC".into(),
            count(report.uloc_total.unwrap_or(0) as u64),
            GREEN,
        ));
    }
    if options.smells {
        cards.push(("Smells".into(), count(summary.smells.total().into()), AMBER));
    }
    draw_cards(&mut canvas, &mut y, &cards);

    let lines = summary.lines.max(1);
    let bar = WIDTH - 2 * PAD;
    let code = (bar as i64 * i64::from(summary.code) / i64::from(lines)) as i32;
    let comment = (bar as i64 * i64::from(summary.comment) / i64::from(lines)) as i32;
    canvas.rect(PAD, y, bar, 18, 6, LINE);
    if code > 0 {
        canvas.rect(PAD, y, code, 18, 6, ACCENT);
    }
    if comment > 0 {
        canvas.rect(PAD + code, y, comment, 18, 6, GREEN);
    }
    y += 30;
    let legend = [
        ("Code", summary.code, ACCENT, PAD),
        ("Comment", summary.comment, GREEN, PAD + 220),
        ("Blank", summary.blank, MUTED, PAD + 440),
    ];
    for (label, value, colour, x) in legend {
        canvas.rect(x, y - 9, 10, 10, 2, colour);
        canvas.text(
            x + 16,
            y,
            12,
            MUTED,
            "400",
            "start",
            &format!(
                "{label} {} ({})",
                report::group(u64::from(value)),
                share(value, lines)
            ),
        );
    }
    y += 34;

    let limit = if options.top > 0 { options.top } else { 20 };
    let ordered = scan::sort_entries(report.business_entries(), "code");
    let shown = ordered.len().min(limit);
    canvas.text(PAD, y, 15, INK, "700", "start", "Top files by code");
    y += 22;
    canvas.text(PAD, y, 12, MUTED, "700", "start", "File");
    canvas.text(WIDTH - PAD - 300, y, 12, MUTED, "700", "end", "Lines");
    canvas.text(WIDTH - PAD - 200, y, 12, MUTED, "700", "end", "Comment");
    canvas.text(WIDTH - PAD - 100, y, 12, MUTED, "700", "end", "Test");
    canvas.text(WIDTH - PAD, y, 12, MUTED, "700", "end", "Code");
    y += 6;
    for (index, entry) in ordered.iter().take(shown).enumerate() {
        let top = y + index as i32 * ROW;
        if index % 2 == 1 {
            canvas.rect(PAD, top, WIDTH - 2 * PAD, ROW, 4, PANEL);
        }
        let baseline = top + 16;
        canvas.text(
            PAD,
            baseline,
            12,
            INK,
            "400",
            "start",
            &clip(&scan::relative(entry.path(), cwd), 62),
        );
        canvas.text(
            WIDTH - PAD - 300,
            baseline,
            12,
            MUTED,
            "400",
            "end",
            &count(entry.stats.lines.into()),
        );
        canvas.text(
            WIDTH - PAD - 200,
            baseline,
            12,
            GREEN,
            "400",
            "end",
            &count(entry.stats.comment.into()),
        );
        canvas.text(
            WIDTH - PAD - 100,
            baseline,
            12,
            AMBER,
            "400",
            "end",
            &count(entry.stats.test.into()),
        );
        canvas.text(
            WIDTH - PAD,
            baseline,
            12,
            INK,
            "700",
            "end",
            &count(entry.stats.code.into()),
        );
    }
    y += shown as i32 * ROW + 10;
    if shown < ordered.len() {
        canvas.text(
            PAD,
            y,
            12,
            MUTED,
            "400",
            "start",
            &format!("showing {shown} of {} files", ordered.len()),
        );
        y += 22;
    }

    if options.languages && !report.languages.is_empty() {
        let total: u32 = report.languages.iter().map(|row| row.code).sum();
        y += 10;
        canvas.text(PAD, y, 15, INK, "700", "start", "Languages");
        y += 22;
        canvas.text(PAD, y, 12, MUTED, "700", "start", "Language");
        canvas.text(WIDTH - PAD - 300, y, 12, MUTED, "700", "end", "Files");
        canvas.text(WIDTH - PAD - 200, y, 12, MUTED, "700", "end", "Lines");
        canvas.text(WIDTH - PAD - 100, y, 12, MUTED, "700", "end", "Code");
        canvas.text(WIDTH - PAD, y, 12, MUTED, "700", "end", "Share");
        y += 6;
        for (index, row) in report.languages.iter().enumerate() {
            let top = y + index as i32 * ROW;
            if index % 2 == 1 {
                canvas.rect(PAD, top, WIDTH - 2 * PAD, ROW, 4, PANEL);
            }
            let baseline = top + 16;
            canvas.text(PAD, baseline, 12, INK, "400", "start", row.name);
            canvas.text(
                WIDTH - PAD - 300,
                baseline,
                12,
                MUTED,
                "400",
                "end",
                &count(row.files.into()),
            );
            canvas.text(
                WIDTH - PAD - 200,
                baseline,
                12,
                MUTED,
                "400",
                "end",
                &count(row.lines.into()),
            );
            canvas.text(
                WIDTH - PAD - 100,
                baseline,
                12,
                INK,
                "700",
                "end",
                &count(row.code.into()),
            );
            canvas.text(
                WIDTH - PAD,
                baseline,
                12,
                ACCENT,
                "400",
                "end",
                &share(row.code, total),
            );
        }
        y += report.languages.len() as i32 * ROW + 10;
    }

    y += 8;
    canvas.line(PAD, y, WIDTH - PAD, y, LINE);
    y += 20;
    canvas.text(
        PAD,
        y,
        12,
        MUTED,
        "400",
        "start",
        &format!(
            "{} file(s), {} code lines - rust-loc {VERSION}",
            report::group(report.business.len() as u64),
            report::group(u64::from(summary.code))
        ),
    );
    y += PAD;

    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{WIDTH}\" height=\"{y}\" viewBox=\"0 0 {WIDTH} {y}\"><rect width=\"{WIDTH}\" height=\"{y}\" fill=\"#ffffff\"/>{}</svg>",
        canvas.out
    )
}

fn draw_cards(canvas: &mut Canvas, y: &mut i32, cards: &[(String, String, &str)]) {
    let columns = 5usize;
    let gap = 12;
    let width = (WIDTH - 2 * PAD - gap * (columns as i32 - 1)) / columns as i32;
    for (index, (label, value, colour)) in cards.iter().enumerate() {
        let x = PAD + (index % columns) as i32 * (width + gap);
        let top = *y + (index / columns) as i32 * 74;
        canvas.rect(x, top, width, 64, 10, PANEL);
        canvas.text(x + 14, top + 40, 22, colour, "700", "start", value);
        canvas.text(x + 14, top + 56, 12, MUTED, "400", "start", label);
    }
    *y += cards.len().div_ceil(columns) as i32 * 74 + 4;
}

/// A percentage, the way the text report writes it.
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

/// XML text escaping; the report holds paths, so `&` and `<` can show up.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(character),
        }
    }
    out
}

struct Canvas {
    out: String,
}

impl Canvas {
    fn new() -> Canvas {
        Canvas { out: String::new() }
    }

    fn rect(&mut self, x: i32, y: i32, width: i32, height: i32, radius: i32, fill: &str) {
        let _ = write!(
            self.out,
            "<rect x=\"{x}\" y=\"{y}\" width=\"{width}\" height=\"{height}\" rx=\"{radius}\" fill=\"{fill}\"/>"
        );
    }

    fn line(&mut self, x1: i32, y1: i32, x2: i32, y2: i32, stroke: &str) {
        let _ = write!(
            self.out,
            "<line x1=\"{x1}\" y1=\"{y1}\" x2=\"{x2}\" y2=\"{y2}\" stroke=\"{stroke}\"/>"
        );
    }

    /// A drawing primitive: position, size, colour, weight and anchor all
    /// matter for the report, so the argument list stays wide on purpose.
    #[allow(clippy::too_many_arguments)]
    fn text(
        &mut self,
        x: i32,
        y: i32,
        size: i32,
        fill: &str,
        weight: &str,
        anchor: &str,
        content: &str,
    ) {
        let _ = write!(
            self.out,
            "<text x=\"{x}\" y=\"{y}\" font-family=\"Consolas, monospace\" font-size=\"{size}\" font-weight=\"{weight}\" text-anchor=\"{anchor}\" fill=\"{fill}\">{}</text>",
            escape(content)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_covers_markup_and_quotes() {
        assert_eq!(escape("a & b < c > d"), "a &amp; b &lt; c &gt; d");
        assert_eq!(escape(r#"a "b" c"#), "a &quot;b&quot; c");
        assert_eq!(escape("it's"), "it&#39;s");
    }

    #[test]
    fn share_reads_as_a_percentage() {
        assert_eq!(share(1, 4), "25.0%");
        assert_eq!(share(0, 0), "0.0%");
    }

    #[test]
    fn clip_marks_the_loss() {
        assert_eq!(clip("abcdef", 6), "abcdef");
        assert_eq!(clip("abcdefg", 6), "abcd..");
    }
}
