//! Baseline snapshots: save a scan, compare a later scan with it.
//!
//! The file is deliberately plain: a header and one tab separated row per
//! business file, sorted by path, so it reads well in a diff and can be
//! committed beside the code it describes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::report::Report;
use crate::scan;
use crate::scan::Entry;

/// First line of a snapshot file.
pub const HEADER: &str = "# rust-loc snapshot v1";

/// How a business file is named in a snapshot: relative to its scan root,
/// prefixed with the root when the run has more than one. Two runs from
/// different directories then agree on the same key.
pub fn key(entry: &Entry, roots: &[PathBuf], cwd: &Path) -> String {
    let relative = scan::relative(entry.path(), entry.root());
    if roots.len() <= 1 {
        return relative;
    }
    format!("{}/{}", scan::relative(entry.root(), cwd), relative)
}

/// The counters a snapshot remembers for one file.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub lines: u32,
    pub code: u32,
    pub complexity: u32,
    pub smells: u32,
}

impl Row {
    fn add(&mut self, other: Row) {
        self.lines += other.lines;
        self.code += other.code;
        self.complexity += other.complexity;
        self.smells += other.smells;
    }
}

/// A saved scan: every business file and the counters a diff needs.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Where the snapshot lives, echoed in the report.
    pub path: PathBuf,
    pub files: BTreeMap<String, Row>,
}

impl Snapshot {
    /// Reduce a report to the snapshot a later run compares against.
    pub fn of(report: &Report, cwd: &Path) -> Snapshot {
        let mut files = BTreeMap::new();
        for entry in report.business_entries() {
            files.insert(
                key(entry, &report.roots, cwd),
                Row {
                    lines: entry.stats.lines,
                    code: entry.stats.code,
                    complexity: entry.stats.complexity,
                    smells: entry.stats.smells.total(),
                },
            );
        }
        Snapshot {
            path: PathBuf::new(),
            files,
        }
    }

    pub fn totals(&self) -> Row {
        let mut total = Row::default();
        for row in self.files.values() {
            total.add(*row);
        }
        total
    }

    /// Text form: the header, a column line, one row per file.
    pub fn render(&self) -> String {
        let mut out = String::from(HEADER);
        out.push('\n');
        out.push_str("# file\tlines\tcode\tcomplexity\tsmells\n");
        for (path, row) in &self.files {
            out.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\n",
                escape(path),
                row.lines,
                row.code,
                row.complexity,
                row.smells
            ));
        }
        out
    }

    /// Write the snapshot, creating the parent directory when needed.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("cannot create {}: {}", parent.display(), error))?;
            }
        }
        std::fs::write(path, self.render())
            .map_err(|error| format!("cannot write {}: {}", path.display(), error))
    }

    pub fn load(path: &Path) -> Result<Snapshot, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("cannot read {}: {}", path.display(), error))?;
        parse(&text, path)
    }
}

fn escape(path: &str) -> String {
    path.replace('\t', "\\t").replace('\n', "\\n")
}

fn unescape(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut chars = path.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// Parse a snapshot written by `render`; every complaint names the line.
pub fn parse(text: &str, path: &Path) -> Result<Snapshot, String> {
    let text = text.trim_start_matches('\u{feff}');
    let mut files: BTreeMap<String, Row> = BTreeMap::new();
    let mut header_seen = false;
    for (index, raw) in text.lines().enumerate() {
        let number = index + 1;
        let line = raw.trim_end_matches('\r');
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            if !header_seen && !line.is_empty() {
                if line != HEADER {
                    return Err(broken(path, number, "not a rust-loc snapshot"));
                }
                header_seen = true;
            }
            continue;
        }
        if !header_seen {
            return Err(broken(
                path,
                number,
                "missing the # rust-loc snapshot v1 header",
            ));
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() != 5 {
            return Err(broken(
                path,
                number,
                "expected 5 tab separated fields: file, lines, code, complexity, smells",
            ));
        }
        let mut numbers = [0u32; 4];
        for (slot, field) in numbers.iter_mut().zip(fields.iter().skip(1)) {
            *slot = field
                .trim()
                .parse::<u32>()
                .map_err(|_| broken(path, number, &format!("not a number: {}", field)))?;
        }
        files.insert(
            unescape(fields[0]),
            Row {
                lines: numbers[0],
                code: numbers[1],
                complexity: numbers[2],
                smells: numbers[3],
            },
        );
    }
    if !header_seen {
        return Err(broken(path, 1, "not a rust-loc snapshot"));
    }
    Ok(Snapshot {
        path: path.to_path_buf(),
        files,
    })
}

fn broken(path: &Path, line: usize, message: &str) -> String {
    format!("{}:{}: {}", path.display(), line, message)
}

/// One file in a comparison; a missing side means added or deleted.
#[derive(Debug, Clone)]
pub struct FileDelta {
    pub path: String,
    pub old: Option<Row>,
    pub new: Option<Row>,
}

impl FileDelta {
    pub fn added(&self) -> bool {
        self.old.is_none()
    }

    pub fn deleted(&self) -> bool {
        self.new.is_none()
    }

    pub fn code_delta(&self) -> i64 {
        let old = self.old.map(|row| row.code).unwrap_or(0) as i64;
        let new = self.new.map(|row| row.code).unwrap_or(0) as i64;
        new - old
    }
}

/// A scan compared with a snapshot.
#[derive(Debug)]
pub struct Diff {
    /// The snapshot this comparison used.
    pub path: PathBuf,
    pub old: Row,
    pub new: Row,
    /// Every file that changed, ranked by the size of the code change.
    pub files: Vec<FileDelta>,
    code_deltas: BTreeMap<String, i64>,
}

impl Diff {
    /// Compare the business files of `report` with `snapshot`.
    pub fn of(report: &Report, snapshot: &Snapshot, cwd: &Path) -> Diff {
        let mut current: BTreeMap<String, Row> = BTreeMap::new();
        for entry in report.business_entries() {
            current.insert(
                key(entry, &report.roots, cwd),
                Row {
                    lines: entry.stats.lines,
                    code: entry.stats.code,
                    complexity: entry.stats.complexity,
                    smells: entry.stats.smells.total(),
                },
            );
        }

        let mut code_deltas: BTreeMap<String, i64> = BTreeMap::new();
        for (path, row) in &current {
            let before = snapshot.files.get(path).map(|row| row.code).unwrap_or(0) as i64;
            code_deltas.insert(path.clone(), row.code as i64 - before);
        }
        for (path, row) in &snapshot.files {
            code_deltas
                .entry(path.clone())
                .or_insert(-(row.code as i64));
        }

        let mut files: Vec<FileDelta> = Vec::new();
        for (path, row) in &snapshot.files {
            match current.remove(path) {
                Some(new) => files.push(FileDelta {
                    path: path.clone(),
                    old: Some(*row),
                    new: Some(new),
                }),
                None => files.push(FileDelta {
                    path: path.clone(),
                    old: Some(*row),
                    new: None,
                }),
            }
        }
        for (path, new) in current {
            files.push(FileDelta {
                path,
                old: None,
                new: Some(new),
            });
        }
        files.retain(|delta| delta.code_delta() != 0 || delta.added() || delta.deleted());
        files.sort_by(|left, right| {
            right
                .code_delta()
                .abs()
                .cmp(&left.code_delta().abs())
                .then_with(|| left.path.cmp(&right.path))
        });

        Diff {
            path: snapshot.path.clone(),
            old: snapshot.totals(),
            new: Row {
                lines: report.summary.lines,
                code: report.summary.code,
                complexity: report.summary.complexity,
                smells: report.summary.smells.total(),
            },
            files,
            code_deltas,
        }
    }

    /// Code change for one reported path, when the file moved.
    pub fn code_delta(&self, path: &str) -> Option<i64> {
        self.code_deltas.get(path).copied()
    }

    pub fn added(&self) -> usize {
        self.files.iter().filter(|delta| delta.added()).count()
    }

    pub fn deleted(&self) -> usize {
        self.files.iter().filter(|delta| delta.deleted()).count()
    }

    pub fn changed(&self) -> usize {
        self.files
            .iter()
            .filter(|delta| !delta.added() && !delta.deleted())
            .count()
    }

    /// The counters the report block shows, as (label, before, after).
    pub fn counters(&self) -> [(&'static str, u32, u32); 4] {
        [
            ("Lines", self.old.lines, self.new.lines),
            ("Code", self.old.code, self.new.code),
            (
                "Complexity (est.)",
                self.old.complexity,
                self.new.complexity,
            ),
            ("Smells", self.old.smells, self.new.smells),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Snapshot {
        let mut files = BTreeMap::new();
        files.insert(
            "src/a.rs".to_string(),
            Row {
                lines: 5,
                code: 4,
                complexity: 1,
                smells: 2,
            },
        );
        files.insert(
            "src/b.rs".to_string(),
            Row {
                lines: 3,
                code: 2,
                complexity: 0,
                smells: 0,
            },
        );
        Snapshot {
            path: PathBuf::from("base.txt"),
            files,
        }
    }

    #[test]
    fn render_round_trips_through_parse() {
        let snapshot = sample();
        let parsed = parse(&snapshot.render(), Path::new("base.txt")).unwrap();
        assert_eq!(parsed.files, snapshot.files);
        assert_eq!(parsed.totals(), snapshot.totals());
        assert_eq!(parsed.totals().code, 6);
        assert_eq!(parsed.totals().smells, 2);
    }

    #[test]
    fn a_snapshot_is_sorted_by_path() {
        let text = sample().render();
        let paths: Vec<&str> = text
            .lines()
            .filter(|line| !line.starts_with('#'))
            .map(|line| line.split('\t').next().unwrap())
            .collect();
        assert_eq!(paths, ["src/a.rs", "src/b.rs"]);
    }

    #[test]
    fn a_file_without_the_header_is_rejected() {
        let error = parse("src/a.rs\t1\t2\t3\t4\n", Path::new("base.txt")).unwrap_err();
        assert!(error.contains("base.txt:1"), "{}", error);
        assert!(
            error.contains("missing the # rust-loc snapshot v1 header"),
            "{}",
            error
        );
    }

    #[test]
    fn a_broken_row_names_the_line_and_the_problem() {
        let short = format!("{}\nsrc/a.rs\t1\t2\n", HEADER);
        let error = parse(&short, Path::new("base.txt")).unwrap_err();
        assert!(error.contains("base.txt:2"), "{}", error);
        assert!(error.contains("5 tab separated fields"), "{}", error);

        let text = format!("{}\nsrc/a.rs\tnope\t2\t3\t4\n", HEADER);
        let error = parse(&text, Path::new("base.txt")).unwrap_err();
        assert!(error.contains("not a number: nope"), "{}", error);
    }

    #[test]
    fn blank_lines_and_comments_are_ignored() {
        let text = format!("{}\n\n# a comment\nsrc/a.rs\t1\t2\t3\t4\n\n", HEADER);
        let parsed = parse(&text, Path::new("base.txt")).unwrap();
        assert_eq!(parsed.files.len(), 1);
        assert_eq!(parsed.files["src/a.rs"].code, 2);
    }
}
