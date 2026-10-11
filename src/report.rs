//! The scan pipeline plus the text, JSON, CSV and Markdown renderers.

use std::path::{Path, PathBuf};

use crate::cocomo;
use crate::options::{Format, Options};
use crate::scan::{self, Entry, FileFacts, Progress, Totals};

pub struct Excluded {
    pub files: usize,
    pub file_lines: u32,
    pub item_lines: u32,
    pub paths: Vec<(String, u32)>,
    pub generated_files: usize,
    pub generated_lines: u32,
}

/// One function in the ranked list, flattened across every scanned file.
pub struct FunctionRow {
    pub path: String,
    pub name: String,
    pub line: u32,
    pub code: u32,
    pub complexity: u32,
    pub params: u32,
    pub depth: u32,
}

/// The COCOMO estimate for the business scope.
pub struct CocomoRow {
    pub project: &'static str,
    pub params: cocomo::Params,
    pub estimate: cocomo::Estimate,
}

pub struct Report {
    pub roots: Vec<PathBuf>,
    pub options: Options,
    pub entries: Vec<Entry>,
    pub business: Vec<usize>,
    pub excluded_entries: Vec<usize>,
    pub summary: Totals,
    pub uloc_total: Option<usize>,
    pub all_lines: u32,
    pub excluded: Excluded,
    /// Business functions ranked by complexity; empty unless requested.
    pub functions: Vec<FunctionRow>,
    /// Every business function found before the ranking was truncated.
    pub function_count: usize,
    /// Cost estimate for the business code lines, when requested.
    pub cocomo: Option<CocomoRow>,
}

impl Report {
    pub fn business_entries(&self) -> Vec<&Entry> {
        self.business
            .iter()
            .map(|index| &self.entries[*index])
            .collect()
    }

    /// Re-render this report in another format without scanning again.
    pub fn render_as(&self, format: Format, cwd: &Path) -> String {
        let mut options = self.options.clone();
        options.format = format;
        render_with(self, &options, cwd)
    }
}

/// Analyse every root and reduce it to the numbers all renderers share.
pub fn scan(
    roots: &[PathBuf],
    options: &Options,
    cwd: &Path,
    progress: Option<&Progress<'_>>,
) -> Report {
    let mut discovered: Vec<(PathBuf, Vec<PathBuf>)> = Vec::with_capacity(roots.len());
    let mut total = 0usize;
    for root in roots {
        let files = scan::apply_gitignore(root, scan::discover(root, options), options.gitignore);
        total += files.len();
        discovered.push((root.clone(), files));
    }

    let mut facts: Vec<FileFacts> = Vec::new();
    let mut done = 0usize;
    for (root, files) in &discovered {
        let offset = done;
        done += files.len();
        let forward: Option<Box<Progress<'_>>> = match progress {
            Some(callback) => Some(Box::new(move |index, _total, path| {
                callback(offset + index, total, path)
            })),
            None => None,
        };
        facts.extend(scan::collect_files(
            root,
            files,
            options.read_options(),
            options.worker_count(),
            forward.as_deref(),
        ));
    }

    let siblings = scan::resolve_siblings(&facts);
    let entries: Vec<Entry> = facts
        .into_iter()
        .map(|facts| {
            let stats = scan::summarize(&facts, &siblings, options.include_tests);
            Entry { facts, stats }
        })
        .collect();

    let drop_generated =
        |entry: &Entry| options.exclude_generated && entry.stats.generated.is_some();
    let business: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| {
            (options.include_tests || !entry.stats.forced_test) && !drop_generated(entry)
        })
        .map(|(index, _)| index)
        .collect();
    let excluded_entries: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.stats.forced_test)
        .map(|(index, _)| index)
        .collect();

    let business_refs: Vec<&Entry> = business.iter().map(|index| &entries[*index]).collect();
    let excluded_refs: Vec<&Entry> = excluded_entries
        .iter()
        .map(|index| &entries[*index])
        .collect();
    let summary = scan::totals(&business_refs);
    let uloc_total = if options.uloc {
        scan::union_uloc(&business_refs)
    } else {
        None
    };
    let all_lines = entries.iter().map(|entry| entry.stats.physical).sum();
    let generated_refs: Vec<&Entry> = if options.exclude_generated {
        entries
            .iter()
            .filter(|entry| entry.stats.generated.is_some() && !entry.stats.forced_test)
            .collect()
    } else {
        Vec::new()
    };
    let generated_files = generated_refs.len();
    let generated_lines: u32 = generated_refs
        .iter()
        .map(|entry| entry.stats.physical)
        .sum();
    let mut paths: Vec<(String, u32)> = excluded_refs
        .iter()
        .map(|entry| (scan::relative(entry.path(), cwd), entry.stats.physical))
        .collect();
    paths.sort();
    let excluded = Excluded {
        files: excluded_refs.len(),
        file_lines: excluded_refs.iter().map(|entry| entry.stats.physical).sum(),
        item_lines: business_refs.iter().map(|entry| entry.stats.test).sum(),
        paths,
        generated_files,
        generated_lines,
    };

    let mut function_count = 0usize;
    let mut functions: Vec<FunctionRow> = Vec::new();
    if options.functions {
        for entry in &business_refs {
            for function in &entry.facts.functions {
                if function.test && !options.include_tests {
                    continue;
                }
                function_count += 1;
                functions.push(FunctionRow {
                    path: scan::relative(entry.path(), cwd),
                    name: function.name.clone(),
                    line: function.line,
                    code: function.code,
                    complexity: function.complexity,
                    params: function.params,
                    depth: function.depth,
                });
            }
        }
        functions.sort_by(|left, right| {
            right
                .complexity
                .cmp(&left.complexity)
                .then(right.code.cmp(&left.code))
                .then(left.path.cmp(&right.path))
                .then(left.line.cmp(&right.line))
        });
        if options.top_functions > 0 {
            functions.truncate(options.top_functions);
        }
    }

    let cocomo_row = if options.cocomo {
        Some(CocomoRow {
            project: options.cocomo_project.name(),
            params: options.cocomo_params,
            estimate: cocomo::estimate(summary.code, options.cocomo_params),
        })
    } else {
        None
    };

    Report {
        roots: roots.to_vec(),
        options: options.clone(),
        entries,
        business,
        excluded_entries,
        summary,
        uloc_total,
        all_lines,
        excluded,
        functions,
        function_count,
        cocomo: cocomo_row,
    }
}

/// 1234567 -> 1,234,567, matching Python string formatting.
pub fn group<T: Into<u64>>(value: T) -> String {
    let digits = value.into().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    let bytes = digits.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if index > 0 && (bytes.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(*byte as char);
    }
    out
}

/// One-decimal percentage of a base.
pub fn share(value: usize, base: usize) -> String {
    if base == 0 {
        return "0.0%".to_string();
    }
    format!("{:.1}%", value as f64 / base as f64 * 100.0)
}

fn pad(text: &str, width: usize) -> String {
    format!("{:<width$}", text, width = width)
}

/// A file path for a listing, tagged when generated code is only marked.
fn file_label(entry: &Entry, options: &Options, cwd: &Path) -> String {
    let mut label = scan::relative(entry.path(), cwd);
    if options.mark_generated {
        label.push_str(entry.stats.generated.tag());
    }
    label
}

fn number(value: u32, width: usize) -> String {
    format!("{:>width$}", group(value), width = width)
}

/// Whole dollars, grouped the way every other counter is.
fn money(value: f64) -> String {
    let dollars = group(value.max(0.0).round() as u64);
    let mut out = String::with_capacity(dollars.len() + 1);
    out.push('$');
    out.push_str(&dollars);
    out
}

fn table_row(label: &str, values: &[u32], width: usize) -> String {
    let mut row = pad(label, width);
    for value in values {
        row.push_str(&number(*value, 10));
    }
    row
}

/// Render a report in the requested format.
pub fn render(report: &Report, cwd: &Path) -> String {
    render_with(report, &report.options, cwd)
}

/// Render an existing report with a different set of options, so a caller can
/// re-export without scanning again.
pub fn render_with(report: &Report, options: &Options, cwd: &Path) -> String {
    match options.format {
        Format::Text => render_text(report, options, cwd),
        Format::Json => render_json(report, options, cwd),
        Format::Csv => render_csv(report, options, cwd),
        Format::Markdown => render_markdown(report, options, cwd),
    }
}

fn render_text(report: &Report, options: &Options, cwd: &Path) -> String {
    let business = report.business_entries();
    let columns = options.column_keys();
    let titles = options.column_titles();
    let mut out: Vec<String> = Vec::new();

    if options.by_file {
        let mut ordered = scan::sort_entries(business.clone(), &options.sort);
        if options.top > 0 {
            ordered.truncate(options.top);
        }
        let labels: Vec<String> = ordered
            .iter()
            .map(|entry| file_label(entry, options, cwd))
            .collect();
        let width = label_width(&labels, "File");
        let mut head = pad("File", width);
        for title in &titles {
            head.push_str(&format!("{:>10}", title));
        }
        out.push(head);
        out.push("-".repeat(width + 10 * titles.len()));
        for (label, entry) in labels.iter().zip(ordered.iter()) {
            let values: Vec<u32> = columns.iter().map(|key| entry.column(key)).collect();
            out.push(table_row(label, &values, width));
        }
        out.push(String::new());
    }

    if options.by_module {
        let groups = scan::module_groups(&business, cwd);
        let labels: Vec<String> = groups.iter().map(|(label, _, _)| label.clone()).collect();
        let width = label_width(&labels, "Module");
        let mut head = pad("Module", width);
        for title in &titles {
            head.push_str(&format!("{:>10}", title));
        }
        out.push(head);
        out.push("-".repeat(width + 10 * titles.len()));
        for (label, stats, _) in groups.iter() {
            let values: Vec<u32> = columns.iter().map(|key| stats.column(key)).collect();
            out.push(table_row(label, &values, width));
        }
        out.push(String::new());
    }

    out.push(
        if options.include_tests {
            "Rust source lines"
        } else {
            "Rust business code (tests excluded)"
        }
        .to_string(),
    );
    let roots: Vec<String> = report
        .roots
        .iter()
        .map(|root| scan::relative(root, cwd))
        .collect();
    out.push("Roots: ".to_string() + &roots.join(", "));
    out.push(format!(
        "Scope: {}, {} ignored directory names",
        if options.gitignore {
            "gitignore-aware"
        } else {
            "no gitignore filtering"
        },
        options.ignore_dirs.len()
    ));
    out.push(String::new());

    let summary = &report.summary;
    let lines = summary.lines as usize;
    out.push(pad("Files", 22) + &number(report.business.len() as u32, 12));
    out.push(pad("Lines", 22) + &number(summary.lines, 12));
    out.push(
        pad("  Blank", 22)
            + &number(summary.blank, 12)
            + "   "
            + &share(summary.blank as usize, lines),
    );
    let mut comment_line = pad("  Comment", 22)
        + &number(summary.comment, 12)
        + "   "
        + &share(summary.comment as usize, lines);
    if summary.doc > 0 {
        comment_line.push_str(&format!("   (doc {})", group(summary.doc)));
    }
    out.push(comment_line);
    out.push(
        pad("  Code", 22)
            + &number(summary.code, 12)
            + "   "
            + &share(summary.code as usize, lines),
    );
    out.push(String::new());
    out.push("Rust metrics".to_string());
    if options.complexity {
        out.push(pad("  Complexity (est.)", 22) + &number(summary.complexity, 12));
    }
    out.push(pad("  Unsafe", 22) + &number(summary.unsafe_kw, 12));
    out.push(pad("  Test functions", 22) + &number(summary.test_fns, 12));
    if options.uloc {
        let uloc = report.uloc_total.unwrap_or(0);
        out.push(
            pad("  ULOC", 22)
                + &number(uloc as u32, 12)
                + "   "
                + &share(uloc, lines)
                + " of lines (DRYness)",
        );
    }
    out.push(String::new());

    if let Some(row) = &report.cocomo {
        render_cocomo_text(row, &mut out);
        out.push(String::new());
    }

    if options.include_tests {
        out.push("Test code included (--include-tests)".to_string());
    } else {
        out.push("Excluded test code".to_string());
        out.push(format!(
            "  Test-only files          {:>6} file(s), {} lines",
            group(report.excluded.files as u32),
            group(report.excluded.file_lines)
        ));
        out.push(format!(
            "  #[cfg(test)] items       {:>6}          {} lines",
            "",
            group(report.excluded.item_lines)
        ));
    }
    out.push(String::new());
    out.push(pad("All Rust lines", 22) + &number(report.all_lines, 12));
    if options.exclude_generated {
        out.push(String::new());
        out.push("Excluded generated code".to_string());
        out.push(format!(
            "  Generated files          {:>6} file(s), {} lines",
            group(report.excluded.generated_files as u32),
            group(report.excluded.generated_lines)
        ));
    }
    if options.show_excluded && !report.excluded.paths.is_empty() {
        out.push(String::new());
        out.push("Excluded test-only files".to_string());
        for (path, count) in &report.excluded.paths {
            out.push(format!("  {} ({} lines)", path, group(*count)));
        }
    }

    if options.functions {
        out.push(String::new());
        render_functions_text(report, &mut out);
    }
    out.join("\n")
}

fn label_width(labels: &[String], title: &str) -> usize {
    let mut width = 44usize.max(title.chars().count());
    for label in labels {
        width = width.max(label.chars().count());
    }
    width
}

/// Cut a label to width characters, marking the loss with a trailing "..".
fn clip(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    if width <= 2 {
        return text.chars().take(width).collect();
    }
    let mut out: String = text.chars().take(width - 2).collect();
    out.push_str("..");
    out
}

fn column_width(values: &[String], title: &str, minimum: usize, maximum: usize) -> usize {
    let mut width = minimum.max(title.chars().count());
    for value in values {
        width = width.max(value.chars().count());
    }
    width.min(maximum)
}

/// The worst functions by complexity, one row per function.
fn render_functions_text(report: &Report, out: &mut Vec<String>) {
    let rows = &report.functions;
    out.push("Functions".to_string());
    let mut scope = format!(
        "  {} function(s) in scope, ranked by complexity",
        group(report.function_count as u32)
    );
    if rows.len() < report.function_count {
        scope.push_str(&format!(", showing {}", group(rows.len() as u32)));
    }
    out.push(scope);
    if rows.is_empty() {
        return;
    }
    out.push(String::new());
    let names: Vec<String> = rows.iter().map(|row| row.name.clone()).collect();
    let locations: Vec<String> = rows
        .iter()
        .map(|row| format!("{}:{}", row.path, row.line))
        .collect();
    let name_width = column_width(&names, "Function", 20, 40);
    let location_width = column_width(&locations, "Location", 24, 64);
    let mut head = pad("Function", name_width);
    head.push_str(&pad("Location", location_width));
    head.push_str(&format!(
        "{:>8}{:>8}{:>7}{:>7}",
        "Code", "Cplx", "Args", "Depth"
    ));
    out.push(head);
    out.push("-".repeat(name_width + location_width + 30));
    for (row, name) in rows.iter().zip(names.iter()) {
        let mut line = pad(&clip(name, name_width), name_width);
        line.push_str(&pad(
            &clip(&format!("{}:{}", row.path, row.line), location_width),
            location_width,
        ));
        line.push_str(&format!(
            "{:>8}{:>8}{:>7}{:>7}",
            group(row.code),
            group(row.complexity),
            row.params,
            row.depth
        ));
        out.push(line);
    }
}

fn render_markdown(report: &Report, options: &Options, cwd: &Path) -> String {
    let business = report.business_entries();
    let columns = options.column_keys();
    let titles = options.column_titles();
    let title = if options.include_tests {
        "Rust source lines"
    } else {
        "Rust business code (tests excluded)"
    };
    let mut out: Vec<String> = vec![format!("### {}", title), String::new()];
    let roots: Vec<String> = report
        .roots
        .iter()
        .map(|root| format!("`{}`", scan::relative(root, cwd)))
        .collect();
    out.push("Roots: ".to_string() + &roots.join(", "));
    out.push(String::new());
    out.push("| Metric | Value |".to_string());
    out.push("| --- | ---: |".to_string());
    let summary = &report.summary;
    let mut rows: Vec<(&str, u32)> = vec![
        ("Files", report.business.len() as u32),
        ("Lines", summary.lines),
        ("Blank", summary.blank),
        ("Comment", summary.comment),
        ("Code", summary.code),
    ];
    if options.complexity {
        rows.push(("Complexity (est.)", summary.complexity));
    }
    rows.push(("Unsafe", summary.unsafe_kw));
    rows.push(("Test functions", summary.test_fns));
    if options.uloc {
        rows.push(("ULOC", report.uloc_total.unwrap_or(0) as u32));
    }
    for (name, value) in rows {
        out.push(format!("| {} | {} |", name, group(value)));
    }
    out.push(String::new());
    if let Some(row) = &report.cocomo {
        out.push(format!("### COCOMO estimate ({})", row.project));
        out.push(String::new());
        out.push("| Metric | Value |".to_string());
        out.push("| --- | ---: |".to_string());
        out.push(format!(
            "| Cost to develop | {} |",
            money(row.estimate.cost)
        ));
        out.push(format!(
            "| Schedule effort | {:.2} months |",
            row.estimate.schedule
        ));
        out.push(format!("| People required | {:.2} |", row.estimate.people));
        out.push(String::new());
    }
    if !options.include_tests {
        out.push(format!(
            "Excluded tests: {} test-only files ({} lines), {} lines inside `#[cfg(test)]` items. All Rust lines: {}.",
            group(report.excluded.files as u32),
            group(report.excluded.file_lines),
            group(report.excluded.item_lines),
            group(report.all_lines)
        ));
    }
    out.push(String::new());
    if options.by_file {
        let mut ordered = scan::sort_entries(business.clone(), &options.sort);
        if options.top > 0 {
            ordered.truncate(options.top);
        }
        out.push(format!("| File | {} |", titles.join(" | ")));
        out.push(format!(
            "| --- | {} |",
            vec!["---:"; titles.len()].join(" | ")
        ));
        for entry in ordered {
            let values: Vec<String> = columns.iter().map(|key| group(entry.column(key))).collect();
            out.push(format!(
                "| `{}` | {} |",
                file_label(entry, options, cwd),
                values.join(" | ")
            ));
        }
        out.push(String::new());
    }
    if options.by_module {
        out.push(format!("| Module | {} |", titles.join(" | ")));
        out.push(format!(
            "| --- | {} |",
            vec!["---:"; titles.len()].join(" | ")
        ));
        for (label, stats, _) in scan::module_groups(&business, cwd) {
            let values: Vec<String> = columns.iter().map(|key| group(stats.column(key))).collect();
            out.push(format!("| `{}` | {} |", label, values.join(" | ")));
        }
        out.push(String::new());
    }
    if options.functions {
        out.push("### Worst functions by complexity".to_string());
        out.push(String::new());
        out.push(format!(
            "{} function(s) in scope, showing {}",
            group(report.function_count as u32),
            group(report.functions.len() as u32)
        ));
        out.push(String::new());
        out.push("| Function | Location | Code | Cplx | Args | Depth |".to_string());
        out.push("| --- | --- | ---: | ---: | ---: | ---: |".to_string());
        for row in &report.functions {
            out.push(format!(
                "| `{}` | `{}:{}` | {} | {} | {} | {} |",
                row.name, row.path, row.line, row.code, row.complexity, row.params, row.depth
            ));
        }
        out.push(String::new());
    }
    out.join("\n")
}

fn csv_field(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') || value.contains('\r') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn render_csv(report: &Report, options: &Options, cwd: &Path) -> String {
    let business = report.business_entries();
    let mut columns: Vec<&str> = vec!["lines", "blank", "comment", "doc", "code", "test"];
    if options.complexity {
        columns.push("complexity");
    }
    columns.push("unsafe");
    columns.push("test_fns");
    if options.uloc {
        columns.push("uloc");
    }
    let mut out: Vec<String> = Vec::new();
    let mut header: Vec<String> = vec!["path".to_string()];
    header.extend(columns.iter().map(|key| key.to_string()));
    out.push(header.join(","));
    if options.by_file {
        let mut ordered = scan::sort_entries(business.clone(), &options.sort);
        if options.top > 0 {
            ordered.truncate(options.top);
        }
        for entry in ordered {
            let mut row: Vec<String> = vec![csv_field(&scan::relative(entry.path(), cwd))];
            row.extend(columns.iter().map(|key| entry.column(key).to_string()));
            out.push(row.join(","));
        }
    } else {
        let stats = scan::totals(&business);
        let uloc = scan::union_uloc(&business).unwrap_or(0);
        let mut row: Vec<String> = vec!["TOTAL".to_string()];
        row.extend(columns.iter().map(|key| {
            if *key == "uloc" {
                uloc.to_string()
            } else {
                stats.column(key).to_string()
            }
        }));
        out.push(row.join(","));
    }
    out.join("\n") + "\n"
}

fn json_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            ch if (ch as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out
}

fn json_string(value: &str) -> String {
    format!("\"{}\"", json_escape(value))
}

fn json_number(value: f64) -> String {
    format!("{:.1}", (value * 10.0).round() / 10.0)
}

fn optional_number<T: std::fmt::Display>(value: Option<T>) -> String {
    match value {
        Some(number) => number.to_string(),
        None => "null".to_string(),
    }
}

/// Render a JSON array the way Python's `json.dumps(..., indent=2)` does:
/// inline when empty, one element per line otherwise.
fn json_array(items: &[String], indent: usize) -> String {
    if items.is_empty() {
        return "[]".to_string();
    }
    let item_pad = " ".repeat(indent + 2);
    let body: Vec<String> = items
        .iter()
        .map(|item| format!("{}{}", item_pad, item))
        .collect();
    format!("[\n{}\n{}]", body.join(",\n"), " ".repeat(indent))
}

fn render_json(report: &Report, options: &Options, cwd: &Path) -> String {
    let roots: Vec<String> = report
        .roots
        .iter()
        .map(|root| json_string(&scan::relative(root, cwd)))
        .collect();
    let ignore_dirs: Vec<String> = options
        .ignore_dirs
        .iter()
        .map(|name| json_string(name))
        .collect();
    let exclude: Vec<String> = options
        .exclude
        .iter()
        .map(|item| json_string(item))
        .collect();
    let summary = &report.summary;
    let dryness = if options.uloc && summary.lines > 0 {
        Some(json_number(
            report.uloc_total.unwrap_or(0) as f64 / summary.lines as f64 * 100.0,
        ))
    } else {
        None
    };

    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(&format!("  \"roots\": {},\n", json_array(&roots, 2)));
    out.push_str("  \"scope\": {\n");
    out.push_str(&format!("    \"gitignore\": {},\n", options.gitignore));
    out.push_str(&format!(
        "    \"ignore_dirs\": {},\n",
        json_array(&ignore_dirs, 4)
    ));
    out.push_str(&format!("    \"exclude\": {},\n", json_array(&exclude, 4)));
    out.push_str(&format!(
        "    \"include_tests\": {}\n",
        options.include_tests
    ));
    out.push_str("  },\n");
    out.push_str("  \"summary\": {\n");
    out.push_str(&format!("    \"files\": {},\n", report.business.len()));
    out.push_str(&format!("    \"lines\": {},\n", summary.lines));
    out.push_str(&format!("    \"blank\": {},\n", summary.blank));
    out.push_str(&format!("    \"comment\": {},\n", summary.comment));
    out.push_str(&format!("    \"doc\": {},\n", summary.doc));
    out.push_str(&format!("    \"code\": {},\n", summary.code));
    out.push_str(&format!("    \"test\": {},\n", summary.test));
    out.push_str(&format!("    \"physical\": {},\n", summary.physical));
    out.push_str(&format!("    \"complexity\": {},\n", summary.complexity));
    out.push_str(&format!("    \"unsafe\": {},\n", summary.unsafe_kw));
    out.push_str(&format!("    \"test_fns\": {},\n", summary.test_fns));
    out.push_str(&format!(
        "    \"uloc\": {},\n",
        optional_number(report.uloc_total)
    ));
    out.push_str(&format!(
        "    \"dryness_percent\": {}\n",
        match dryness {
            Some(value) => value,
            None => "null".to_string(),
        }
    ));
    out.push_str("  },\n");
    if let Some(row) = &report.cocomo {
        out.push_str("  \"cocomo\": {\n");
        out.push_str(&format!("    \"project\": {},\n", json_string(row.project)));
        out.push_str(&format!("    \"a\": {},\n", json_number(row.params.a)));
        out.push_str(&format!("    \"b\": {},\n", json_number(row.params.b)));
        out.push_str(&format!("    \"c\": {},\n", json_number(row.params.c)));
        out.push_str(&format!("    \"d\": {},\n", json_number(row.params.d)));
        out.push_str(&format!(
            "    \"effort_person_months\": {},\n",
            json_number(row.estimate.effort)
        ));
        out.push_str(&format!(
            "    \"schedule_months\": {},\n",
            json_number(row.estimate.schedule)
        ));
        out.push_str(&format!(
            "    \"people\": {},\n",
            json_number(row.estimate.people)
        ));
        out.push_str(&format!(
            "    \"cost_usd\": {}\n",
            json_number(row.estimate.cost)
        ));
        out.push_str("  },\n");
    }
    if options.include_tests {
        out.push_str("  \"excluded\": null,\n");
    } else {
        out.push_str(&format!(
            "  \"excluded\": {{\n    \"files\": {},\n    \"file_lines\": {},\n    \"item_lines\": {},\n    \"paths\": null\n  }},\n",
            report.excluded.files, report.excluded.file_lines, report.excluded.item_lines
        ));
    }
    if options.exclude_generated {
        out.push_str(&format!(
            "  \"generated_excluded\": {{\"files\": {}, \"lines\": {}}},\n",
            report.excluded.generated_files, report.excluded.generated_lines
        ));
    }
    out.push_str(&format!("  \"all_rust_lines\": {},\n", report.all_lines));
    if options.functions {
        out.push_str(&format!(
            "  \"functions_total\": {},\n",
            report.function_count
        ));
        let mut items: Vec<String> = Vec::with_capacity(report.functions.len());
        for row in &report.functions {
            items.push(format!(
                "{{\"name\": {}, \"path\": {}, \"line\": {}, \"code\": {}, \"complexity\": {}, \"params\": {}, \"depth\": {}}}",
                json_string(&row.name),
                json_string(&row.path),
                row.line,
                row.code,
                row.complexity,
                row.params,
                row.depth
            ));
        }
        out.push_str(&format!("  \"functions\": {},\n", json_array(&items, 2)));
    }
    let mut rows: Vec<String> = Vec::with_capacity(report.entries.len());
    for entry in &report.entries {
        let stats = &entry.stats;
        let mut row = String::new();
        row.push_str("    {\n");
        row.push_str(&format!(
            "      \"path\": {},\n",
            json_string(&scan::relative(entry.path(), cwd))
        ));
        row.push_str(&format!("      \"test_only\": {},\n", stats.forced_test));
        if options.mark_generated {
            row.push_str(&format!(
                "      \"generated\": {},\n",
                json_string(stats.generated.label())
            ));
        }
        row.push_str(&format!("      \"lines\": {},\n", stats.lines));
        row.push_str(&format!("      \"blank\": {},\n", stats.blank));
        row.push_str(&format!("      \"comment\": {},\n", stats.comment));
        row.push_str(&format!("      \"doc\": {},\n", stats.doc));
        row.push_str(&format!("      \"code\": {},\n", stats.code));
        row.push_str(&format!("      \"test\": {},\n", stats.test));
        row.push_str(&format!("      \"physical\": {},\n", stats.physical));
        row.push_str(&format!("      \"complexity\": {},\n", stats.complexity));
        row.push_str(&format!("      \"unsafe\": {},\n", stats.unsafe_kw));
        row.push_str(&format!("      \"test_fns\": {},\n", stats.test_fns));
        row.push_str(&format!(
            "      \"uloc\": {}\n",
            optional_number(stats.uloc)
        ));
        row.push_str("    }");
        rows.push(row);
    }
    if rows.is_empty() {
        out.push_str("  \"files\": []\n}");
    } else {
        out.push_str("  \"files\": [\n");
        out.push_str(&rows.join(",\n"));
        out.push_str("\n  ]\n}");
    }
    out
}

/// The COCOMO estimate block, in the shape `scc` prints it.
fn render_cocomo_text(row: &CocomoRow, out: &mut Vec<String>) {
    let estimate = &row.estimate;
    out.push(format!("COCOMO estimate ({})", row.project));
    out.push(pad("  Cost to develop", 22) + &format!("{:>12}", money(estimate.cost)));
    out.push(
        pad("  Schedule effort", 22)
            + &format!("{:>12} months", format!("{:.2}", estimate.schedule)),
    );
    out.push(pad("  People required", 22) + &format!("{:>12}", format!("{:.2}", estimate.people)));
}
