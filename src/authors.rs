//! Who wrote the lines: `git blame` attribution per author and per file.
//!
//! `--authors` answers the question a reviewer asks next: not how many lines
//! the tree holds, but who holds them. Every business file is blamed, and each
//! blamed line is classified the way the rest of the report classifies it, so
//! an author's code/comment/blank split adds up to the same totals the summary
//! prints. Each file is blamed in the repository it lives in, so one tree
//! reports the same authors from wherever the process was started.
//!
//! Nothing here runs unless the flag is given: one `git blame` per file costs
//! far more than the scan itself.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::git;
use crate::options::Options;
use crate::scan::{self, Entry, Progress, Stage};
use crate::sources::Lang;
use crate::{lexer, pylexer};

/// The per-line classes, in the shape the report counts them.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Lines {
    pub blank: u32,
    pub comment: u32,
    pub doc: u32,
    pub code: u32,
    /// Lines inside test items, always counted on their own so the author
    /// tables can show test code as a column. With `--include-tests` they are
    /// also part of the class counters above, so there they read as a share of
    /// the totals rather than as a class beside them.
    pub test: u32,
}

impl Lines {
    /// The business lines: what the summary calls `lines`.
    pub fn business(&self) -> u32 {
        self.blank + self.comment + self.code
    }

    /// The weight a share or a row is ranked by: every line the blame covered.
    /// With `include_tests` the class counters already carry the test lines, so
    /// this only adds the ones the split left out.
    pub fn physical(&self, include_tests: bool) -> u32 {
        if include_tests {
            self.business()
        } else {
            self.business() + self.test
        }
    }

    fn add(&mut self, other: Lines) {
        self.blank += other.blank;
        self.comment += other.comment;
        self.doc += other.doc;
        self.code += other.code;
        self.test += other.test;
    }
}

/// One author's share of a single file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Share {
    pub name: String,
    pub email: String,
    pub lines: Lines,
}

/// One author's share of the whole scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorRow {
    pub name: String,
    pub email: String,
    pub files: usize,
    pub lines: Lines,
}

/// A file and the authors whose lines still live in it.
#[derive(Debug, Clone)]
pub struct FileShare {
    pub path: PathBuf,
    pub authors: Vec<Share>,
}

/// Everything `--authors` collected.
#[derive(Debug, Default)]
pub struct Authors {
    /// False when no scanned file lives in a git repository.
    pub available: bool,
    /// Files git could not blame: untracked, or outside the repository, and
    /// only counted once the scan has a repository to blame against.
    pub untracked: usize,
    /// Files that were blamed, in scan order.
    pub files: Vec<FileShare>,
    /// Every author, largest first.
    pub rows: Vec<AuthorRow>,
}

impl Authors {
    /// Blame every entry and fold the result per author and per file.
    ///
    /// The repository is resolved from each file, so the report credits the
    /// same authors whether it ran inside the tree or somewhere else.
    pub fn collect(
        entries: &[&Entry],
        options: &Options,
        progress: Option<&Progress<'_>>,
    ) -> Authors {
        let tops = repo_tops(entries);
        let blamed = blame_all(&tops, entries, options, progress);
        let available = blamed.iter().any(|result| result.repo);
        let mut files: Vec<FileShare> = Vec::new();
        let mut untracked = 0usize;
        for (entry, result) in entries.iter().zip(blamed) {
            match result.shares {
                Some(authors) => files.push(FileShare {
                    path: entry.path().to_path_buf(),
                    authors,
                }),
                None if available => untracked += 1,
                None => {}
            }
        }
        let rows = fold(&files, options.include_tests);
        Authors {
            available,
            untracked,
            files,
            rows,
        }
    }
    /// The rows a listing shows, after `--top` truncation.
    pub fn shown(&self, top: usize) -> &[AuthorRow] {
        if top == 0 || top >= self.rows.len() {
            &self.rows
        } else {
            &self.rows[..top]
        }
    }

    /// The shares of one file, for the per-file detail block.
    pub fn file(&self, path: &Path) -> Option<&FileShare> {
        self.files.iter().find(|file| file.path == path)
    }
}

/// What blaming one file produced.
struct Blamed {
    /// Whether the file lives inside a repository, tracked or not.
    repo: bool,
    /// The line shares, once git could blame the file.
    shares: Option<Vec<Share>>,
}

impl Blamed {
    /// A file that no repository claimed.
    fn plain() -> Blamed {
        Blamed {
            repo: false,
            shares: None,
        }
    }
}

/// The repository every entry lives in, asked once per directory so a tree
/// costs one `rev-parse` per folder instead of one per file.
fn repo_tops(entries: &[&Entry]) -> Vec<Option<PathBuf>> {
    let mut roots: Vec<PathBuf> = Vec::new();
    let mut asked: HashMap<PathBuf, Option<PathBuf>> = HashMap::new();
    entries
        .iter()
        .map(|entry| {
            if let Some(root) = roots
                .iter()
                .find(|root| entry.path().starts_with(root.as_path()))
            {
                return Some(root.clone());
            }
            let dir = entry.path().parent()?;
            if let Some(known) = asked.get(dir) {
                return known.clone();
            }
            let found = scan::run_git(dir, &["rev-parse", "--show-toplevel"])
                .map(|text| PathBuf::from(text.trim()))
                .filter(|root| root.is_dir() && entry.path().starts_with(root));
            if let Some(root) = &found {
                roots.push(root.clone());
            }
            asked.insert(dir.to_path_buf(), found.clone());
            found
        })
        .collect()
}

/// Report one blamed file, named from the scan root the way the tables name
/// files rather than from the drive the process happens to run on.
fn report_blame(callback: &Progress<'_>, entry: &Entry, done: usize, total: usize) {
    let shown = scan::relative(entry.path(), entry.root());
    callback(Stage::Blame, done, total, Path::new(&shown));
}

/// Blame every entry in parallel, keeping one slot per entry. Every finished
/// file is reported to `progress`, because this stage outlasts the read that
/// filled the bar before it.
fn blame_all(
    tops: &[Option<PathBuf>],
    entries: &[&Entry],
    options: &Options,
    progress: Option<&Progress<'_>>,
) -> Vec<Blamed> {
    let total = entries.len();
    let blame = |index: usize| -> Blamed {
        match &tops[index] {
            Some(top) => blame_file(top, entries[index], options),
            None => Blamed::plain(),
        }
    };
    let mut results: Vec<Blamed> = entries.iter().map(|_| Blamed::plain()).collect();
    if total == 0 {
        return results;
    }
    let workers = options.worker_count().max(1).min(total);
    if workers <= 1 || total < 4 {
        for (index, slot) in results.iter_mut().enumerate() {
            *slot = blame(index);
            if let Some(callback) = progress {
                report_blame(callback, entries[index], index + 1, total);
            }
        }
        return results;
    }
    let slots: Vec<std::sync::Mutex<Blamed>> = (0..total)
        .map(|_| std::sync::Mutex::new(Blamed::plain()))
        .collect();
    let cursor = std::sync::atomic::AtomicUsize::new(0);
    let done = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let index = cursor.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if index >= total {
                    break;
                }
                let result = blame(index);
                if let Ok(mut slot) = slots[index].lock() {
                    *slot = result;
                }
                let finished = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                if let Some(callback) = progress {
                    report_blame(callback, entries[index], finished, total);
                }
            });
        }
    });
    slots
        .into_iter()
        .map(|slot| slot.into_inner().unwrap_or_else(|_| Blamed::plain()))
        .collect()
}

/// Blame one file and split its lines across the authors that wrote them.
fn blame_file(top: &Path, entry: &Entry, options: &Options) -> Blamed {
    let Some(relative) = git::repo_relative(entry.path(), top) else {
        return Blamed::plain();
    };
    let Some(blame) = scan::run_git(top, &["blame", "--incremental", "--", &relative]) else {
        // Inside the repository, but git does not track the file.
        return Blamed {
            repo: true,
            shares: None,
        };
    };
    let authors: Vec<(String, String)> = parse_blame(&blame);
    let classes = classify(entry.path(), entry.lang());
    let mut shares: Vec<Share> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for (position, (name, email)) in authors.iter().enumerate() {
        let Some(class) = classes.get(position) else {
            break;
        };
        let lines = line_counters(*class, options.include_tests);
        let key = identity(name, email);
        let slot = match index.get(&key) {
            Some(slot) => *slot,
            None => {
                shares.push(Share {
                    name: name.clone(),
                    email: email.clone(),
                    lines: Lines::default(),
                });
                index.insert(key, shares.len() - 1);
                shares.len() - 1
            }
        };
        shares[slot].lines.add(lines);
    }
    sort_shares(&mut shares, options.include_tests);
    Blamed {
        repo: true,
        shares: Some(shares),
    }
}

/// One classified line of a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Line {
    kind: Kind,
    is_test: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Blank,
    Comment,
    Doc,
    Code,
}

/// Re-read a file and classify every line the way the scan does.
fn classify(path: &Path, lang: Lang) -> Vec<Line> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let analysis = match lang {
        Lang::Rust => lexer::analyze(&text),
        Lang::Python => pylexer::analyze(&text),
    };
    let mut out = Vec::with_capacity(analysis.total);
    for index in 0..analysis.total {
        let kind = if analysis.code[index] {
            Kind::Code
        } else if analysis.doc[index] {
            Kind::Doc
        } else if analysis.comment[index] {
            Kind::Comment
        } else {
            Kind::Blank
        };
        out.push(Line {
            kind,
            is_test: analysis.is_test[index],
        });
    }
    out
}

/// The counters one blamed line adds to its author's share.
///
/// A line inside a test item is always counted as a test line, so the author
/// tables can show test code as its own column. With `include_tests` it also
/// lands in the class counters, the way the file summary counts it.
fn line_counters(class: Line, include_tests: bool) -> Lines {
    let mut lines = Lines::default();
    if class.is_test {
        lines.test = 1;
    }
    if !class.is_test || include_tests {
        match class.kind {
            Kind::Blank => lines.blank = 1,
            Kind::Comment => lines.comment = 1,
            Kind::Doc => {
                lines.comment = 1;
                lines.doc = 1;
            }
            Kind::Code => lines.code = 1,
        }
    }
    lines
}

/// Parse `git blame --incremental` into one pair per blamed line.
///
/// The header of every run of lines is `<sha> <orig> <final> <count>`, and the
/// author block that follows is printed only the first time a commit shows up,
/// so names are collected per commit and replayed in the order runs appeared.
pub fn parse_blame(text: &str) -> Vec<(String, String)> {
    let mut runs: Vec<(String, usize)> = Vec::new();
    let mut names: HashMap<String, (String, String)> = HashMap::new();
    let mut current = String::new();
    for raw in text.lines() {
        let line = raw.trim_end_matches('\r');
        if let Some((sha, count)) = header(line) {
            runs.push((sha.clone(), count));
            current = sha;
            continue;
        }
        if let Some(rest) = line.strip_prefix("author-mail ") {
            names.entry(current.clone()).or_default().1 = trim_mail(rest);
            continue;
        }
        if let Some(rest) = line.strip_prefix("author ") {
            names.entry(current.clone()).or_default().0 = rest.trim().to_string();
        }
    }
    let mut out = Vec::new();
    for (sha, count) in runs {
        let entry = names.get(&sha).cloned().unwrap_or_default();
        for _ in 0..count {
            out.push(entry.clone());
        }
    }
    out
}

/// The `<sha> <orig> <final> <count>` header, when the line is one.
fn header(line: &str) -> Option<(String, usize)> {
    let mut fields = line.split(' ');
    let sha = fields.next()?;
    fields.next()?;
    fields.next()?;
    let count = fields.next()?;
    if fields.next().is_some() {
        return None;
    }
    if sha.len() < 7 || !sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let count = count.parse::<usize>().ok()?;
    Some((sha.to_string(), count))
}

/// `<foo@bar>` becomes `foo@bar`.
fn trim_mail(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .to_string()
}

/// The key that tells two authors apart: the address when git has one.
fn identity(name: &str, email: &str) -> String {
    if email.is_empty() {
        name.to_lowercase()
    } else {
        email.to_lowercase()
    }
}

fn sort_shares(shares: &mut [Share], include_tests: bool) {
    shares.sort_by(|left, right| {
        right
            .lines
            .physical(include_tests)
            .cmp(&left.lines.physical(include_tests))
            .then_with(|| left.name.cmp(&right.name))
    });
}

/// Fold the per-file shares into one row per author, largest first.
///
/// A file counts for an author once, however many of its lines are theirs, so
/// the tally adds `files` per share rather than per line. Blame already folds
/// the identical spellings of one address together, so one share is one author.
fn fold(files: &[FileShare], include_tests: bool) -> Vec<AuthorRow> {
    let mut rows: Vec<AuthorRow> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for file in files {
        for share in &file.authors {
            let key = identity(&share.name, &share.email);
            let slot = match index.get(&key) {
                Some(slot) => *slot,
                None => {
                    rows.push(AuthorRow {
                        name: share.name.clone(),
                        email: share.email.clone(),
                        files: 0,
                        lines: Lines::default(),
                    });
                    index.insert(key, rows.len() - 1);
                    rows.len() - 1
                }
            };
            rows[slot].lines.add(share.lines);
            rows[slot].files += 1;
        }
    }
    rows.sort_by(|left, right| {
        right
            .lines
            .physical(include_tests)
            .cmp(&left.lines.physical(include_tests))
            .then_with(|| left.name.cmp(&right.name))
    });
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA_A: &str = "1111111111111111111111111111111111111111";
    const SHA_B: &str = "2222222222222222222222222222222222222222";

    fn block(name: &str, mail: &str, sha: &str, start: usize, count: usize) -> String {
        format!(
            "{sha} {start} {start} {count}\nauthor {name}\nauthor-mail <{mail}>\nauthor-time 1\nauthor-tz +0000\ncommitter {name}\ncommitter-mail <{mail}>\ncommitter-time 1\ncommitter-tz +0000\nsummary x\nfilename f.rs\n"
        )
    }

    #[test]
    fn the_same_commits_metadata_is_replayed_for_every_run() {
        let mut text = block("Ada", "ada@x", SHA_A, 1, 2);
        text.push_str(SHA_A);
        text.push_str(" 3 3 1\nauthor Ada\nauthor-mail <ada@x>\n");
        let parsed = parse_blame(&text);
        assert_eq!(parsed.len(), 3);
        assert!(parsed
            .iter()
            .all(|(name, mail)| name == "Ada" && mail == "ada@x"));
    }

    #[test]
    fn each_commit_keeps_its_own_name() {
        let mut text = block("Ada", "ada@x", SHA_A, 1, 1);
        text.push_str(&block("Bob", "bob@x", SHA_B, 2, 1));
        let parsed = parse_blame(&text);
        assert_eq!(parsed[0].0, "Ada");
        assert_eq!(parsed[1].0, "Bob");
    }

    #[test]
    fn content_that_looks_like_a_header_is_not_one() {
        assert!(header("author Muu").is_none());
        assert!(header("not-a-sha 1 1 1").is_none());
        assert!(header("1111111 1 1").is_none());
        assert!(header(&format!("{SHA_A} 1 1 2")).is_some());
    }

    #[test]
    fn mail_brackets_are_stripped() {
        assert_eq!(trim_mail(" <a@b> "), "a@b");
        assert_eq!(trim_mail("<a@b>"), "a@b");
    }

    #[test]
    fn two_spellings_of_one_address_are_one_author() {
        assert_eq!(identity("Ada", "ADA@x"), identity("Ada Lovelace", "ada@x"));
        assert_eq!(identity("Ada", ""), "ada");
        assert_eq!(identity("Ada", "ada@x"), "ada@x");
    }

    #[test]
    fn folding_counts_a_file_once_per_author() {
        let ada = |code: u32, blank: u32| Share {
            name: "Ada".into(),
            email: "ada@x".into(),
            lines: Lines {
                code,
                blank,
                ..Lines::default()
            },
        };
        let files = vec![
            FileShare {
                path: PathBuf::from("a.rs"),
                authors: vec![
                    ada(3, 0),
                    Share {
                        name: "Bob".into(),
                        email: "bob@x".into(),
                        lines: Lines {
                            code: 2,
                            ..Lines::default()
                        },
                    },
                ],
            },
            FileShare {
                path: PathBuf::from("b.rs"),
                authors: vec![ada(5, 2)],
            },
        ];
        let rows = fold(&files, false);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "Ada");
        assert_eq!(rows[0].files, 2);
        assert_eq!(rows[0].lines.code, 8);
        assert_eq!(rows[0].lines.blank, 2);
        assert_eq!(rows[0].lines.business(), 10);
        assert_eq!(rows[1].name, "Bob");
        assert_eq!(rows[1].files, 1);
    }

    #[test]
    fn a_test_line_is_counted_on_its_own_and_folded_when_asked() {
        let code = Line {
            kind: Kind::Code,
            is_test: true,
        };
        let kept_out = line_counters(code, false);
        assert_eq!(kept_out.test, 1);
        assert_eq!(kept_out.business(), 0);
        let folded = line_counters(code, true);
        assert_eq!(folded.test, 1);
        assert_eq!(folded.code, 1);

        let plain = line_counters(
            Line {
                kind: Kind::Comment,
                is_test: false,
            },
            true,
        );
        assert_eq!(plain.test, 0);
        assert_eq!(plain.comment, 1);
    }

    #[test]
    fn the_ranking_weight_counts_a_test_line_once() {
        let lines = Lines {
            code: 3,
            test: 2,
            ..Lines::default()
        };
        assert_eq!(lines.physical(false), 5);
        assert_eq!(lines.physical(true), 3);
    }
}
