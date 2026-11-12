//! What two git revisions changed, classified the way a scan classifies.
//!
//! `--diff <A> <B>` walks the unified diff between the two revisions and
//! feeds every changed line through the reader of the file's language, so an
//! added line counts as code, comment or blank exactly as the same line would
//! count in a scan. Unknown languages and binary files contribute nothing but
//! a skipped count.
//!
//! ```text
//! rust-loc . --diff v1.2..HEAD
//! rust-loc . --diff HEAD~7 HEAD
//! ```

use std::collections::BTreeMap;
use std::path::Path;

use crate::scan;
use crate::sources::Lang;

/// The class counts one side of a diff accumulates.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Lines {
    pub code: u32,
    pub comment: u32,
    pub blank: u32,
}

impl Lines {
    pub fn total(self) -> u32 {
        self.code + self.comment + self.blank
    }

    /// Classify a batch of diff lines with the reader of their language.
    /// Lines the flags say nothing about are blank, which is exactly how a
    /// scan reads an empty line.
    fn classify(lang: Lang, text: &str) -> Lines {
        let analysis = lang.analyze(text);
        let mut lines = Lines::default();
        for index in 0..analysis.total {
            match (analysis.code[index], analysis.comment[index]) {
                (true, _) => lines.code += 1,
                (false, true) => lines.comment += 1,
                (false, false) => lines.blank += 1,
            }
        }
        lines
    }

    fn add(&mut self, other: Lines) {
        self.code += other.code;
        self.comment += other.comment;
        self.blank += other.blank;
    }
}

/// One file's share of the diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub path: String,
    pub lang: Lang,
    pub added: Lines,
    pub removed: Lines,
}

/// What one revision pair changed.
#[derive(Debug, Clone)]
pub struct VersionDiff {
    pub rev_a: String,
    pub rev_b: String,
    /// Per-file rows, sorted by path.
    pub rows: Vec<Row>,
    /// Files the diff carried but no reader claims (binaries, unknown types).
    pub skipped: usize,
}

impl VersionDiff {
    /// The class totals across every file, added side and removed side.
    pub fn totals(&self) -> (Lines, Lines) {
        let mut added = Lines::default();
        let mut removed = Lines::default();
        for row in &self.rows {
            added.add(row.added);
            removed.add(row.removed);
        }
        (added, removed)
    }

    /// Run `git diff` between the revisions and classify what it changed.
    /// `None` when the directory holds no repository or git refuses the pair.
    pub fn collect(dir: &Path, rev_a: &str, rev_b: &str) -> Option<VersionDiff> {
        let text = scan::run_git(
            dir,
            &[
                "diff",
                "--no-color",
                "--no-renames",
                "-U0",
                rev_a,
                rev_b,
            ],
        )?;
        let diff = parse(&text, rev_a, rev_b);
        Some(diff)
    }
}

/// One side of a file section: the path and the raw lines gathered for it.
#[derive(Default)]
struct Side {
    path: Option<String>,
    lang: Option<Lang>,
    text: String,
}

/// Walk a unified diff. Header lines are only headers before the first hunk
/// of their file section, so a removed line that reads `--- x` stays content.
fn parse(text: &str, rev_a: &str, rev_b: &str) -> VersionDiff {
    let mut added: BTreeMap<String, (Lang, String)> = BTreeMap::new();
    let mut removed: BTreeMap<String, (Lang, String)> = BTreeMap::new();
    let mut source = Side::default();
    let mut dest = Side::default();
    let mut in_hunk = false;
    let mut section = false;
    let mut skipped = 0usize;

    for line in text.lines() {
        if line.starts_with("diff --git ") {
            flush_section(&dest, &source, section, &mut added, &mut removed, &mut skipped);
            source = Side::default();
            dest = Side::default();
            in_hunk = false;
            section = true;
            continue;
        }
        if !in_hunk {
            if let Some(rest) = line.strip_prefix("--- ") {
                source.path = diff_path(rest, "a/");
                source.lang = source.path.as_deref().and_then(|path| lang_of(path));
                continue;
            }
            if let Some(rest) = line.strip_prefix("+++ ") {
                dest.path = diff_path(rest, "b/");
                dest.lang = dest.path.as_deref().and_then(|path| lang_of(path));
                continue;
            }
            if line.starts_with("@@") {
                in_hunk = true;
            }
            continue;
        }
        // Hunk body: the first byte picks the side, `\` is the "no newline"
        // note, and context lines carry nothing to classify.
        match line.as_bytes().first() {
            Some(b'+') => {
                if dest.lang.is_some() {
                    dest.text.push_str(&line[1..]);
                    dest.text.push('\n');
                }
            }
            Some(b'-') => {
                if source.lang.is_some() {
                    source.text.push_str(&line[1..]);
                    source.text.push('\n');
                }
            }
            _ => {}
        }
    }
    flush_section(&dest, &source, section, &mut added, &mut removed, &mut skipped);

    let mut paths: Vec<String> = added.keys().chain(removed.keys()).cloned().collect();
    paths.sort();
    paths.dedup();
    let rows = paths
        .into_iter()
        .filter_map(|path| {
            let (lang, _) = added.get(&path).or_else(|| removed.get(&path))?;
            Some(Row {
                path: path.clone(),
                lang: *lang,
                added: added
                    .get(&path)
                    .map(|(_, text)| Lines::classify(*lang, text))
                    .unwrap_or_default(),
                removed: removed
                    .get(&path)
                    .map(|(_, text)| Lines::classify(*lang, text))
                    .unwrap_or_default(),
            })
        })
        .collect();
    VersionDiff {
        rev_a: rev_a.to_string(),
        rev_b: rev_b.to_string(),
        rows,
        skipped,
    }
}

/// Fold one file section into the buffers: the gathered text joins the path's
/// side, and a section no reader claims counts as skipped.
fn flush_section(
    dest: &Side,
    source: &Side,
    section: bool,
    added: &mut BTreeMap<String, (Lang, String)>,
    removed: &mut BTreeMap<String, (Lang, String)>,
    skipped: &mut usize,
) {
    if !section {
        return;
    }
    if let (Some(path), Some(lang)) = (&dest.path, dest.lang) {
        if !dest.text.is_empty() {
            added.insert(path.clone(), (lang, dest.text.clone()));
        }
    }
    if let (Some(path), Some(lang)) = (&source.path, source.lang) {
        if !source.text.is_empty() {
            removed.insert(path.clone(), (lang, source.text.clone()));
        }
    }
    if dest.lang.is_none() && source.lang.is_none() {
        *skipped += 1;
    }
}

/// The path a `---`/`+++` header names: the side's `a/`/`b/` prefix stripped,
/// or `/dev/null` when the side adds or deletes the whole file.
fn diff_path(header: &str, prefix: &str) -> Option<String> {
    let header = header.trim_end_matches('\t').trim_end();
    if header == "/dev/null" {
        return None;
    }
    let path = header.strip_prefix(prefix).unwrap_or(header);
    if path.is_empty() {
        None
    } else {
        Some(path.to_string())
    }
}

/// The reader a diff path would be scanned with.
fn lang_of(path: &str) -> Option<Lang> {
    Lang::of(Path::new(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUST_DIFF: &str = "\
diff --git a/src/main.rs b/src/main.rs
index 1111111..2222222 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,4 @@
 fn main() {
+    // a note
+    let value = 1;
     let keep = 2;
-// a deleted comment line
 }

diff --git a/notes.txt b/notes.txt
index 3333333..4444444 100644
--- a/notes.txt
+++ b/notes.txt
@@ -1 +1 @@
-old text
+new text

diff --git a/logo.png b/logo.png
index 5555555..6666666 100644
Binary files a/logo.png and b/logo.png differ
";

    #[test]
    fn added_lines_are_classified_like_a_scan() {
        let diff = parse(RUST_DIFF, "v1", "v2");
        assert_eq!(diff.rev_a, "v1");
        assert_eq!(diff.rev_b, "v2");
        assert_eq!(diff.skipped, 2, "the binary and the text file are skipped");
        let row = diff
            .rows
            .iter()
            .find(|row| row.path == "src/main.rs")
            .expect("the rust file is a row");
        assert_eq!(row.lang, Lang::Rust);
        assert_eq!(row.added.code, 1, "let value");
        assert_eq!(row.added.comment, 1, "the note");
        assert_eq!(row.added.blank, 0);
        assert_eq!(
            row.removed,
            Lines {
                code: 0,
                comment: 1,
                blank: 0
            },
            "only the deleted comment line was removed"
        );
    }

    #[test]
    fn a_removed_comment_line_is_classified_on_the_removed_side() {
        let diff = parse(RUST_DIFF, "v1", "v2");
        let row = diff
            .rows
            .iter()
            .find(|row| row.path == "src/main.rs")
            .expect("the rust file is a row");
        assert_eq!(row.removed.comment, 1, "the deleted comment line");
    }

    #[test]
    fn unknown_languages_are_skipped_not_counted() {
        let diff = parse(RUST_DIFF, "v1", "v2");
        assert!(diff.rows.iter().all(|row| row.path != "notes.txt"));
    }

    #[test]
    fn totals_add_up_across_the_rows() {
        let diff = parse(RUST_DIFF, "v1", "v2");
        let (added, _removed) = diff.totals();
        assert_eq!(added.total(), 2);
    }
}
