//! README badges.
//!
//! `--badge` prints one small SVG in the flat shield style: a grey caption and
//! a coloured figure, sized to the text. Nothing is fetched and nothing is
//! hosted, so the number comes from the same scan as every other format:
//!
//! ```text
//! rust-loc . --badge > docs/lines.svg
//! rust-loc . --badge-metric smells --badge-label smell > docs/smells.svg
//! ```

use std::fmt::Write;

use crate::report::{self, Report};

const ACCENT: &str = "#2f6fed";
const GREEN: &str = "#22a06b";
const AMBER: &str = "#d98324";
const MUTED: &str = "#5b6570";

/// The figure a badge shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Metric {
    Code,
    Lines,
    Files,
    Comment,
    Blank,
    Tests,
    Complexity,
    Unsafe,
    Smells,
    Uloc,
    Index,
}

impl Metric {
    /// Accept the names the rest of the switches already use.
    pub fn parse(name: &str) -> Option<Metric> {
        Some(match name {
            "code" | "loc" | "sloc" => Metric::Code,
            "lines" | "line" => Metric::Lines,
            "files" | "file" => Metric::Files,
            "comment" | "comments" | "doc" => Metric::Comment,
            "blank" | "blanks" => Metric::Blank,
            "test" | "tests" => Metric::Tests,
            "complexity" | "cc" => Metric::Complexity,
            "unsafe" => Metric::Unsafe,
            "smells" | "smell" => Metric::Smells,
            "uloc" | "unique" => Metric::Uloc,
            "mi" | "maintainability" | "index" => Metric::Index,
            _ => return None,
        })
    }

    /// The caption, unless `--badge-label` replaces it.
    pub fn label(self) -> &'static str {
        match self {
            Metric::Code => "code lines",
            Metric::Lines => "lines",
            Metric::Files => "files",
            Metric::Comment => "comment lines",
            Metric::Blank => "blank lines",
            Metric::Tests => "test lines",
            Metric::Complexity => "complexity",
            Metric::Unsafe => "unsafe",
            Metric::Smells => "smells",
            Metric::Uloc => "unique lines",
            Metric::Index => "maintainability",
        }
    }

    /// The figure, already grouped the way the report writes numbers.
    pub fn value(self, report: &Report) -> String {
        let summary = &report.summary;
        match self {
            Metric::Code => count(u64::from(summary.code)),
            Metric::Lines => count(u64::from(summary.lines)),
            Metric::Files => count(report.business.len() as u64),
            Metric::Comment => count(u64::from(summary.comment)),
            Metric::Blank => count(u64::from(summary.blank)),
            Metric::Tests => count(u64::from(summary.test)),
            Metric::Complexity => count(u64::from(summary.complexity)),
            Metric::Unsafe => count(u64::from(summary.unsafe_kw)),
            Metric::Smells => count(u64::from(summary.smells.total())),
            Metric::Uloc => count(report.uloc_total.unwrap_or(0) as u64),
            Metric::Index => match &report.metrics {
                Some(row) => format!("{:.1} {}", row.index, row.band()),
                None => "--".to_string(),
            },
        }
    }

    /// The colour of the figure block; the maintainability score reads its own
    /// band so a low score looks different at a glance.
    pub fn colour(self, report: &Report) -> &'static str {
        match self {
            Metric::Comment | Metric::Tests => GREEN,
            Metric::Blank => MUTED,
            Metric::Complexity | Metric::Unsafe | Metric::Smells => AMBER,
            Metric::Index => match report.metrics.as_ref().map(|row| row.index) {
                Some(index) if index >= 80.0 => GREEN,
                Some(index) if index >= 40.0 => ACCENT,
                Some(_) => AMBER,
                None => MUTED,
            },
            Metric::Code | Metric::Lines | Metric::Files | Metric::Uloc => ACCENT,
        }
    }
}

/// One flat badge, sized to its caption and figure.
pub fn render(report: &Report, metric: Metric, label: Option<&str>) -> String {
    let caption = label.unwrap_or_else(|| metric.label());
    let caption = escape(caption);
    let value = escape(&metric.value(report));
    let left = width(&caption);
    let right = width(&value);
    let total = left + right;
    let fill = metric.colour(report);
    let mut out = String::with_capacity(400);
    let _ = write!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{total}\" height=\"20\" role=\"img\" aria-label=\"{caption}: {value}\"><title>{caption}: {value}</title>"
    );
    let _ = write!(
        out,
        "<rect width=\"{total}\" height=\"20\" rx=\"3\" fill=\"#555555\"/>"
    );
    let _ = write!(
        out,
        "<rect x=\"{left}\" width=\"{right}\" height=\"20\" rx=\"3\" fill=\"{fill}\"/>"
    );
    // Square off the rounded corners where the two blocks meet.
    let _ = write!(
        out,
        "<rect x=\"{left}\" width=\"4\" height=\"20\" fill=\"{fill}\"/>"
    );
    out.push_str("<g fill=\"#ffffff\" text-anchor=\"middle\" font-family=\"Consolas, monospace\" font-size=\"11\">");
    let _ = write!(
        out,
        "<text x=\"{:.1}\" y=\"14\">{caption}</text>",
        left as f64 / 2.0
    );
    let _ = write!(
        out,
        "<text x=\"{:.1}\" y=\"14\">{value}</text>",
        left as f64 + right as f64 / 2.0
    );
    out.push_str("</g></svg>");
    out
}

/// A block wide enough for its text at 11pixel monospace.
fn width(text: &str) -> i32 {
    text.chars().count() as i32 * 7 + 12
}

fn count(value: u64) -> String {
    report::group(value)
}

/// XML text escaping; both the caption and the path can hold markup.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::Options;
    use std::path::PathBuf;

    const SOURCE: &str = "// a comment\n\n/// Doc.\nfn main() {\n    let value = 1;\n}\n";

    /// Scan a throwaway tree twice so the report has a number to show.
    fn report() -> (Report, PathBuf) {
        let root = std::env::temp_dir().join(format!("rustloc-badge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("main.rs"), SOURCE).unwrap();
        let options = Options::default();
        let report = report::scan(std::slice::from_ref(&root), &options, &root, None);
        (report, root)
    }

    #[test]
    fn the_names_the_other_switches_use_all_parse() {
        for name in [
            "code",
            "lines",
            "files",
            "comment",
            "blank",
            "tests",
            "complexity",
            "unsafe",
            "smells",
            "uloc",
            "mi",
        ] {
            assert!(Metric::parse(name).is_some(), "{name} did not parse");
        }
        assert_eq!(Metric::parse("sloc"), Some(Metric::Code));
        assert_eq!(Metric::parse("nope"), None);
    }

    #[test]
    fn a_badge_is_one_self_contained_svg() {
        let (report, root) = report();
        let svg = render(&report, Metric::Code, None);
        let _ = std::fs::remove_dir_all(&root);
        assert!(
            svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""),
            "{svg}"
        );
        assert!(svg.ends_with("</svg>"));
        assert!(svg.contains("code lines"));
        assert!(svg.contains(&report::group(u64::from(report.summary.code))));
        assert!(!svg.contains("<html"));
    }

    #[test]
    fn a_longer_figure_widens_the_badge() {
        let (report, root) = report();
        let short = render(&report, Metric::Files, Some("f"));
        let long = render(&report, Metric::Files, Some("a much longer caption"));
        let _ = std::fs::remove_dir_all(&root);
        assert!(attribute(&long, "width") > attribute(&short, "width"));
    }

    #[test]
    fn a_label_is_escaped_and_can_be_replaced() {
        let (report, root) = report();
        let svg = render(&report, Metric::Blank, Some("a <b> & c"));
        let _ = std::fs::remove_dir_all(&root);
        assert!(svg.contains("a &lt;b&gt; &amp; c"));
        assert!(!svg.contains("a <b> & c"));
    }

    /// The `width` attribute of the root element.
    fn attribute(svg: &str, name: &str) -> i32 {
        let start = svg.find(&format!("{name}=\"")).unwrap() + name.len() + 2;
        let end = svg[start..].find('"').unwrap() + start;
        svg[start..end].parse().unwrap()
    }
}
