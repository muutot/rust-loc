//! A self-contained HTML report.
//!
//! `--format html` renders the same tables the text and Markdown reports
//! draw into one standalone page: the stylesheet is inline, nothing is
//! fetched, nothing runs. The file mails and archives like a text report but
//! reads like the web page.

use std::fmt::Write as _;
use std::path::Path;

use crate::options::Options;
use crate::report::Report;

const MAX_FILE_ROWS: usize = 50;
const MAX_FUNCTION_ROWS: usize = 20;
const MAX_HOTSPOT_ROWS: usize = 20;
const MAX_COUPLING_ROWS: usize = 20;

/// Render the report as one self-contained page.
pub fn render(report: &Report, options: &Options, cwd: &Path) -> String {
    let mut out = String::with_capacity(96 * 1024);
    let _ = write!(
        out,
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <title>rust-loc report</title>\n<style>\n{STYLE}\n</style>\n</head>\n<body>\n"
    );
    let _ = writeln!(
        out,
        "<h1>rust-loc report <span class=\"muted\">v{}</span></h1>",
        crate::VERSION
    );
    let _ = writeln!(
        out,
        "<p class=\"muted\">roots: {}</p>",
        report
            .roots
            .iter()
            .map(|root| esc(&root.to_string_lossy()))
            .collect::<Vec<_>>()
            .join(", ")
    );

    let summary = &report.summary;
    let mut rows: Vec<(String, String)> = vec![
        ("Lines".into(), num(u64::from(summary.lines))),
        ("Code".into(), num(u64::from(summary.code))),
        ("Comment".into(), num(u64::from(summary.comment))),
        ("Doc".into(), num(u64::from(summary.doc))),
        ("Blank".into(), num(u64::from(summary.blank))),
        ("Test".into(), num(u64::from(summary.test))),
        ("Files".into(), num(report.business_entries().len() as u64)),
        (
            "Functions".into(),
            num(report.function_count as u64),
        ),
        ("Complexity".into(), num(u64::from(summary.complexity))),
        ("Cognitive".into(), num(u64::from(summary.cognitive))),
        (
            "Smells".into(),
            num(u64::from(summary.smells.total())),
        ),
    ];
    if let Some(uloc) = summary.uloc {
        rows.push(("ULOC".into(), num(uloc as u64)));
    }
    if let Some(metrics) = &report.metrics {
        rows.push((
            "Maintainability".into(),
            format!("{:.1} ({})", metrics.index, metrics.band()),
        ));
    }
    let table: Vec<Vec<String>> = rows
        .into_iter()
        .map(|(name, value)| vec![name, value])
        .collect();
    let _ = writeln!(out, "<h2>Summary</h2>{}", table_html(&["Metric", "Value"], &table, &[false, true]));

    if options.languages && !report.languages.is_empty() {
        let table: Vec<Vec<String>> = report
            .languages
            .iter()
            .map(|row| {
                vec![
                    row.name.to_string(),
                    num(u64::from(row.files)),
                    num(u64::from(row.code)),
                    num(u64::from(row.comment)),
                    num(u64::from(row.blank)),
                ]
            })
            .collect();
        let _ = writeln!(
            out,
            "<h2>Languages</h2>{}",
            table_html(&["Language", "Files", "Code", "Comment", "Blank"], &table, &[false, true, true, true, true])
        );
    }

    let mut files: Vec<_> = report.business_entries();
    files.sort_by(|a, b| {
        b.stats
            .code
            .cmp(&a.stats.code)
            .then(a.path().cmp(b.path()))
    });
    let table: Vec<Vec<String>> = files
        .iter()
        .take(MAX_FILE_ROWS)
        .map(|entry| {
            vec![
                esc(&crate::scan::relative(entry.path(), cwd)),
                num(u64::from(entry.stats.code)),
                num(u64::from(entry.stats.comment)),
                num(u64::from(entry.stats.blank)),
                num(u64::from(entry.stats.complexity)),
            ]
        })
        .collect();
    let _ = writeln!(
        out,
        "<h2>Files <span class=\"muted\">(top {}, by code)</span></h2>{}",
        MAX_FILE_ROWS.min(files.len()),
        table_html(&["Path", "Code", "Comment", "Blank", "Cplx"], &table, &[false, true, true, true, true])
    );

    if options.functions && !report.functions.is_empty() {
        let table: Vec<Vec<String>> = report
            .functions
            .iter()
            .take(MAX_FUNCTION_ROWS)
            .map(|row| {
                vec![
                    esc(&row.name),
                    format!("{}:{}", esc(&row.path), row.line),
                    num(u64::from(row.code)),
                    num(u64::from(row.complexity)),
                    num(u64::from(row.params)),
                    num(u64::from(row.depth)),
                ]
            })
            .collect();
        let _ = writeln!(
            out,
            "<h2>Worst functions <span class=\"muted\">(by complexity)</span></h2>{}",
            table_html(
                &["Function", "Location", "Code", "Cplx", "Args", "Depth"],
                &table,
                &[false, false, true, true, true, true]
            )
        );
    }

    if options.hotspots && !report.hotspots.is_empty() {
        let table: Vec<Vec<String>> = report
            .hotspots
            .iter()
            .take(MAX_HOTSPOT_ROWS)
            .map(|row| {
                vec![
                    esc(&row.path),
                    num(row.code as u64),
                    num(u64::from(row.complexity)),
                    num(row.commits),
                    num(row.churn),
                    num(row.score),
                ]
            })
            .collect();
        let _ = writeln!(
            out,
            "<h2>Git hotspots</h2>{}",
            table_html(
                &["File", "Code", "Cplx", "Commits", "Churn", "Score"],
                &table,
                &[false, true, true, true, true, true]
            )
        );
    }

    if let Some(coupling) = &report.coupling {
        let table: Vec<Vec<String>> = coupling
            .top(MAX_COUPLING_ROWS)
            .iter()
            .map(|pair| {
                vec![
                    format!("{} &harr; {}", esc(&pair.a), esc(&pair.b)),
                    num(pair.commits),
                ]
            })
            .collect();
        let _ = writeln!(
            out,
            "<h2>Change coupling <span class=\"muted\">({} commit(s) examined)</span></h2>{}",
            num(coupling.commits as u64),
            table_html(&["File pair", "Commits"], &table, &[false, true])
        );
    }

    if let Some(diff) = &report.vdiff {
        let (added, removed) = diff.totals();
        let _ = write!(
            out,
            "<h2>Version diff {}..{}</h2>{}",
            esc(&diff.rev_a),
            esc(&diff.rev_b),
            table_html(
                &["Side", "Code", "Comment", "Blank"],
                &[
                    vec!["added".into(), num(added.code as u64), num(added.comment as u64), num(added.blank as u64)],
                    vec!["removed".into(), num(removed.code as u64), num(removed.comment as u64), num(removed.blank as u64)],
                ],
                &[false, true, true, true]
            )
        );
        let mut movers: Vec<&crate::vdiff::Row> = diff.rows.iter().collect();
        movers.sort_by_key(|row| std::cmp::Reverse(row.added.code + row.removed.code));
        let table: Vec<Vec<String>> = movers
            .iter()
            .take(10)
            .map(|row| {
                vec![
                    esc(&row.path),
                    format!("+{}", num(row.added.code as u64)),
                    format!("-{}", num(row.removed.code as u64)),
                ]
            })
            .collect();
        let _ = writeln!(
            out,
            "<h3>Largest code changes</h3>{}",
            table_html(&["File", "Added", "Removed"], &table, &[false, true, true])
        );
    }

    if let Some(authors) = &report.authors {
        if authors.available {
            let table: Vec<Vec<String>> = authors
                .rows
                .iter()
                .take(20)
                .map(|row| {
                    vec![
                        esc(&row.name),
                        num(row.files as u64),
                        num(row.lines.business() as u64),
                        num(row.lines.code as u64),
                        num(row.lines.comment as u64),
                        num(row.lines.blank as u64),
                    ]
                })
                .collect();
            let _ = writeln!(
                out,
                "<h2>Authors</h2>{}",
                table_html(
                    &["Author", "Files", "Lines", "Code", "Comment", "Blank"],
                    &table,
                    &[false, true, true, true, true, true]
                )
            );
        }
    }
    if let Some(activity) = &report.activity {
        if !activity.months.is_empty() && !activity.rows.is_empty() {
            let head: Vec<String> = std::iter::once("Author".to_string())
                .chain(activity.months.iter().map(|month| month[2..].to_string()))
                .collect();
            let head: Vec<&str> = head.iter().map(|item| item.as_str()).collect();
            let table: Vec<Vec<String>> = activity
                .rows
                .iter()
                .map(|row| {
                    std::iter::once(esc(&row.name))
                        .chain(row.commits.iter().map(|count| num(u64::from(*count))))
                        .collect()
                })
                .collect();
            let numeric = std::iter::once(false)
                .chain(std::iter::repeat(true).take(activity.months.len()))
                .collect::<Vec<_>>();
            let _ = writeln!(
                out,
                "<h2>Author activity</h2>{}",
                table_html(&head, &table, &numeric)
            );
        }
    }

    if let Some(dupes) = &report.duplicates {
        let _ = writeln!(
            out,
            "<h2>Duplicated code</h2><p>{} copy(ies) of {}+ lines; {} of {} normalised lines duplicated.</p>",
            num(dupes.clusters.len() as u64),
            num(dupes.window as u64),
            num(dupes.duplicated_lines as u64),
            num(dupes.code_lines as u64)
        );
    }

    if let Some(cocomo) = &report.cocomo {
        let table: Vec<Vec<String>> = vec![
            vec!["Effort".into(), format!("{:.1} person-months", cocomo.estimate.effort)],
            vec!["Schedule".into(), format!("{:.1} months", cocomo.estimate.schedule)],
            vec!["People".into(), format!("{:.1}", cocomo.estimate.people)],
            vec!["Cost".into(), format!("${}", num(cocomo.estimate.cost as u64))],
        ];
        let _ = writeln!(
            out,
            "<h2>Effort estimate <span class=\"muted\">({} project)</span></h2>{}",
            cocomo.project,
            table_html(&["", "Estimate"], &table, &[false, true])
        );
    }

    let _ = writeln!(
        out,
        "<p class=\"muted\">Generated by rust-loc v{}. Scan of business code only; \
         test-only files are excluded unless the run asked for them.</p>\n</body>\n</html>\n",
        crate::VERSION
    );
    out
}

/// One table: header row, body rows, and which columns align right. Headers
/// accept anything string-like, so call sites can pass literals or a Vec.
fn table_html<H: AsRef<str>>(headers: &[H], rows: &[Vec<String>], numeric: &[bool]) -> String {
    let mut out = String::from("<table>\n<thead><tr>");
    for head in headers {
        let _ = write!(out, "<th>{}</th>", head.as_ref());
    }
    out.push_str("</tr></thead>\n<tbody>\n");
    for row in rows {
        out.push_str("<tr>");
        for (index, cell) in row.iter().enumerate() {
            if numeric.get(index).copied().unwrap_or(false) {
                out.push_str("<td class=\"num\">");
            } else {
                out.push_str("<td>");
            }
            out.push_str(cell);
            out.push_str("</td>");
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</tbody>\n</table>\n");
    out
}

/// `&` `<` `>` `"` escapes for text that came from the scan.
fn esc(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            other => out.push(other),
        }
    }
    out
}

/// Thousands separators, the way the text report groups its counters.
fn num(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, byte) in digits.bytes().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(byte as char);
    }
    out
}

const STYLE: &str = "\
body { font-family: \"Cascadia Mono\", Consolas, \"JetBrains Mono\", monospace; \
font-size: 13px; line-height: 1.5; color: #1f2430; background: #fafafa; \
max-width: 1100px; margin: 24px auto; padding: 0 16px; }
h1 { font-size: 20px; border-bottom: 2px solid #1f2430; padding-bottom: 6px; }
h2 { font-size: 16px; margin-top: 28px; }
h3 { font-size: 14px; }
table { border-collapse: collapse; margin: 8px 0 16px; }
th, td { border: 1px solid #d7dae0; padding: 3px 10px; text-align: left; }
th { background: #eef0f4; font-weight: 600; }
td.num, th.num { text-align: right; font-variant-numeric: tabular-nums; }
tbody tr:nth-child(even) { background: #f2f4f7; }
.muted { color: #6b7280; font-weight: 400; font-size: 12px; }
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_group_and_text_escapes() {
        assert_eq!(num(1234567), "1,234,567");
        assert_eq!(num(0), "0");
        assert_eq!(esc("a<b>&\"c\""), "a&lt;b&gt;&amp;&quot;c&quot;");
    }

    #[test]
    fn the_page_is_self_contained_and_carries_the_summary() {
        let dir = std::env::temp_dir().join(format!("rust-loc-html-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lib.rs"), "fn main() {}\n").unwrap();
        let options = Options::default();
        let report = crate::report::scan(&[dir.clone()], &options, &dir, None);
        let page = render(&report, &options, &dir);
        std::fs::remove_dir_all(&dir).ok();

        assert!(page.starts_with("<!DOCTYPE html>"));
        assert!(!page.contains("<script"), "no scripts, ever");
        assert!(page.contains("Summary"));
        assert!(page.contains("<table>"));
        assert!(page.contains("rust-loc report"));
    }
}
