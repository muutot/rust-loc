//! File discovery, parallel analysis and the aggregates every renderer shares.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use crate::generated::{self, Generated};
use crate::lexer::{self, Analysis, FunctionInfo, Metrics};
use crate::options::Options;

pub const DEFAULT_IGNORE_DIRS: [&str; 15] = [
    ".git",
    ".hg",
    ".svn",
    "target",
    "node_modules",
    ".svelte-kit",
    "build",
    "dist",
    "vendor",
    ".venv",
    "venv",
    "__pycache__",
    ".mypy_cache",
    ".pytest_cache",
    ".tox",
];

const TEST_DIR_NAMES: [&str; 3] = ["tests", "benches", "test"];

/// Progress callback: `(done, total, path)`. A UI can show a progress bar
/// without duplicating the walking logic.
pub type Progress<'a> = dyn Fn(usize, usize, &Path) + Sync + 'a;

/// Canonical, non-verbatim path.
pub fn resolve_path(path: &Path) -> PathBuf {
    let joined = match path.canonicalize() {
        Ok(resolved) => resolved,
        Err(_) => match std::path::absolute(path) {
            Ok(absolute) => absolute,
            Err(_) => path.to_path_buf(),
        },
    };
    strip_verbatim(joined)
}

fn strip_verbatim(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        PathBuf::from(rest.to_string())
    } else {
        path
    }
}

/// Forward-slashed path text, matching Python pathlib as_posix().
pub fn posix(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Path relative to cwd when possible, else the absolute path.
pub fn relative(path: &Path, cwd: &Path) -> String {
    match path.strip_prefix(cwd) {
        Ok(rest) if !rest.as_os_str().is_empty() => posix(rest),
        Ok(_) => ".".to_string(),
        Err(_) => posix(path),
    }
}

/// Top level component below the root, or (root) for files at the root.
pub fn group_key(path: &Path, root: &Path) -> String {
    let mut parts = match path.strip_prefix(root) {
        Ok(rest) => rest.components(),
        Err(_) => path.components(),
    };
    match (parts.next(), parts.next()) {
        (Some(first), Some(_)) => first.as_os_str().to_string_lossy().into_owned(),
        _ => "(root)".to_string(),
    }
}

/// fnmatch-style glob over the whole string, case-insensitive on Windows.
fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.to_lowercase().chars().collect();
    let text: Vec<char> = text.to_lowercase().chars().collect();
    glob_here(&pattern, &text)
}

fn glob_here(pattern: &[char], text: &[char]) -> bool {
    let mut p = 0usize;
    let mut t = 0usize;
    let mut star: Option<usize> = None;
    let mut mark = 0usize;
    while t < text.len() {
        if p < pattern.len() {
            match pattern[p] {
                '*' => {
                    star = Some(p);
                    mark = t;
                    p += 1;
                    continue;
                }
                '?' => {
                    p += 1;
                    t += 1;
                    continue;
                }
                '[' => {
                    if let Some((matched, next)) = match_class(&pattern[p..], text[t]) {
                        if matched {
                            p += next;
                            t += 1;
                            continue;
                        }
                    } else if text[t] == '[' {
                        p += 1;
                        t += 1;
                        continue;
                    }
                }
                other if other == text[t] => {
                    p += 1;
                    t += 1;
                    continue;
                }
                _ => {}
            }
        }
        match star {
            Some(index) => {
                p = index + 1;
                mark += 1;
                t = mark;
            }
            None => return false,
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

/// Match a [seq] class, returning whether it matched and its length.
fn match_class(pattern: &[char], ch: char) -> Option<(bool, usize)> {
    let mut index = 1usize;
    let negated = pattern.get(index) == Some(&'!') || pattern.get(index) == Some(&'^');
    if negated {
        index += 1;
    }
    let mut matched = false;
    let mut first = true;
    while index < pattern.len() {
        let current = pattern[index];
        if current == ']' && !first {
            return Some((matched != negated, index + 1));
        }
        first = false;
        if index + 2 < pattern.len() && pattern[index + 1] == '-' && pattern[index + 2] != ']' {
            let low = current;
            let high = pattern[index + 2];
            if low <= ch && ch <= high {
                matched = true;
            }
            index += 3;
            continue;
        }
        if current == ch {
            matched = true;
        }
        index += 1;
    }
    None
}

/// True when the path matches one of the user exclusion globs.
pub fn matches_exclude(path: &Path, root: &Path, patterns: &[String]) -> bool {
    if patterns.is_empty() {
        return false;
    }
    let rel = match path.strip_prefix(root) {
        Ok(rest) => posix(rest),
        Err(_) => posix(path),
    };
    let absolute = posix(path);
    for pattern in patterns {
        let trimmed = pattern.trim_end_matches('/');
        if glob_match(pattern, &rel) || glob_match(pattern, &absolute) {
            return true;
        }
        if rel == trimmed || glob_match(&format!("{}/*", trimmed), &rel) {
            return true;
        }
    }
    false
}

/// Collect every .rs file below root that survives the ignore rules.
pub fn discover(root: &Path, options: &Options) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let read = match std::fs::read_dir(&dir) {
            Ok(read) => read,
            Err(_) => continue,
        };
        for entry in read.flatten() {
            let path = entry.path();
            let skipped = path.components().any(|part| {
                options
                    .ignore_dirs
                    .contains(part.as_os_str().to_string_lossy().as_ref())
            });
            if skipped {
                continue;
            }
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => continue,
            };
            if file_type.is_dir() {
                stack.push(path);
            } else if file_type.is_file() {
                let is_rust = path
                    .extension()
                    .map(|extension| extension.eq_ignore_ascii_case("rs"))
                    .unwrap_or(false);
                if is_rust && !matches_exclude(&path, root, &options.exclude) {
                    found.push(path);
                }
            }
        }
    }
    // Mirror Python's `sorted(root.rglob("*.rs"))`: `Path` orders by path
    // components (case-insensitively on Windows), not by the joined string.
    // The difference shows up for `a/backup.rs` vs `a/backup/scale.rs`,
    // where the directory sorts first.
    found.sort_by_key(|path| component_key(path));
    found
}

/// Lowercased path components, the key Python's `PurePath` comparison uses.
fn component_key(path: &Path) -> Vec<String> {
    path.components()
        .map(|part| part.as_os_str().to_string_lossy().to_lowercase())
        .collect()
}

/// Run git to honour .gitignore rules; degrade to a no-op on any failure.
pub fn apply_gitignore(root: &Path, files: Vec<PathBuf>, enabled: bool) -> Vec<PathBuf> {
    if !enabled || files.is_empty() {
        return files;
    }
    let top = match run_git(root, &["rev-parse", "--show-toplevel"]) {
        Some(top) => PathBuf::from(top.trim()),
        None => return files,
    };
    let listing = match run_git(
        &top,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
            "*.rs",
        ],
    ) {
        Some(listing) => listing,
        None => return files,
    };
    let kept: HashSet<PathBuf> = listing
        .split('\0')
        .filter(|name| !name.is_empty())
        .map(|name| resolve_path(&top.join(name)))
        .collect();
    files
        .into_iter()
        .filter(|path| kept.contains(&resolve_path(path)))
        .collect()
}

/// Run git hidden (no console window flash) and return stdout on success.
fn run_git(dir: &Path, args: &[&str]) -> Option<String> {
    let mut command = std::process::Command::new("git");
    command.arg("-C").arg(dir).args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Per-line classes for a file; test counts lines inside test-only items.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub code: u32,
    pub comment: u32,
    pub doc: u32,
    pub blank: u32,
    pub test: u32,
}

impl Counts {
    fn sub(self, other: Counts) -> Counts {
        Counts {
            code: self.code - other.code,
            comment: self.comment - other.comment,
            doc: self.doc - other.doc,
            blank: self.blank - other.blank,
            test: other.test,
        }
    }
}

/// Everything one analysed file contributes, with the source text dropped.
#[derive(Debug)]
pub struct FileFacts {
    pub path: PathBuf,
    pub root: PathBuf,
    pub name_test: bool,
    pub crate_test: bool,
    pub sibling_targets: Vec<String>,
    pub physical: u32,
    pub all: Counts,
    pub test_only: Counts,
    pub metrics: Metrics,
    pub metrics_all: Metrics,
    pub unique: Option<HashSet<Box<str>>>,
    pub functions: Vec<FunctionInfo>,
    pub generated: Generated,
}

/// What one file read should compute beyond the line classes.
#[derive(Debug, Clone, Copy)]
pub struct ReadOptions {
    pub uloc: bool,
    pub functions: bool,
    pub include_tests: bool,
    /// 0 disables the minified-line heuristic.
    pub min_gen_line_length: usize,
}

impl Default for ReadOptions {
    fn default() -> Self {
        ReadOptions {
            uloc: false,
            functions: false,
            include_tests: false,
            min_gen_line_length: generated::DEFAULT_MIN_LINE_LENGTH,
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Stats {
    pub lines: u32,
    pub blank: u32,
    pub comment: u32,
    pub doc: u32,
    pub code: u32,
    pub test: u32,
    pub physical: u32,
    pub complexity: u32,
    pub unsafe_kw: u32,
    pub test_fns: u32,
    pub uloc: Option<u32>,
    pub forced_test: bool,
    pub generated: Generated,
}

#[derive(Debug)]
pub struct Entry {
    pub facts: FileFacts,
    pub stats: Stats,
}

impl Entry {
    pub fn path(&self) -> &Path {
        &self.facts.path
    }

    pub fn root(&self) -> &Path {
        &self.facts.root
    }

    /// Value for a sort/table column key, mirroring the Python stats dict.
    pub fn column(&self, key: &str) -> u32 {
        match key {
            "lines" => self.stats.lines,
            "blank" => self.stats.blank,
            "comment" => self.stats.comment,
            "doc" => self.stats.doc,
            "code" => self.stats.code,
            "test" => self.stats.test,
            "physical" => self.stats.physical,
            "complexity" => self.stats.complexity,
            "unsafe" => self.stats.unsafe_kw,
            "test_fns" => self.stats.test_fns,
            "uloc" => self.stats.uloc.unwrap_or(0),
            _ => 0,
        }
    }
}

/// Test-only path heuristics: test directories and tests.rs style names.
pub fn is_test_file(path: &Path) -> bool {
    for part in path.components() {
        let name = part.as_os_str().to_string_lossy();
        if TEST_DIR_NAMES.contains(&name.as_ref()) {
            return true;
        }
    }
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = stem.to_lowercase();
    stem == "test" || stem == "tests" || stem.ends_with("_test") || stem.ends_with("_tests")
}

/// Read and analyse one file, reducing it to facts.
fn read_facts(path: &Path, root: &Path, read: ReadOptions) -> Option<FileFacts> {
    let bytes = std::fs::read(path).ok()?;
    let bytes = match bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        Some(rest) => rest,
        None => bytes.as_slice(),
    };
    let text = String::from_utf8_lossy(bytes);
    let text = lexer::normalize_newlines(&text);
    let analysis: Analysis = lexer::analyze(&text);
    let generated_kind = generated::detect(&text, &analysis, read.min_gen_line_length);

    let mut all = Counts::default();
    let mut test_only = Counts::default();
    let mut unique: HashSet<Box<str>> = HashSet::new();
    for (index, raw) in text.lines().enumerate() {
        if index >= analysis.total {
            break;
        }
        let is_test = analysis.is_test[index];
        let class = if analysis.code[index] {
            "code"
        } else if analysis.comment[index] {
            "comment"
        } else {
            "blank"
        };
        if class == "code" {
            all.code += 1;
        } else if class == "comment" {
            all.comment += 1;
            if analysis.doc[index] {
                all.doc += 1;
            }
        } else {
            all.blank += 1;
        }
        if is_test {
            test_only.test += 1;
            if class == "code" {
                test_only.code += 1;
            } else if class == "comment" {
                test_only.comment += 1;
                if analysis.doc[index] {
                    test_only.doc += 1;
                }
            } else {
                test_only.blank += 1;
            }
        }
        if read.uloc {
            let stripped = raw.trim();
            if !stripped.is_empty() && (read.include_tests || !is_test) {
                unique.insert(stripped.into());
            }
        }
    }

    let relative_path = path.strip_prefix(root).unwrap_or(path);
    Some(FileFacts {
        path: resolve_path(path),
        root: root.to_path_buf(),
        name_test: is_test_file(relative_path),
        crate_test: analysis.crate_test,
        sibling_targets: analysis.sibling_targets,
        physical: analysis.total as u32,
        all,
        test_only,
        metrics: analysis.metrics,
        metrics_all: analysis.metrics_all,
        unique: if read.uloc { Some(unique) } else { None },
        functions: if read.functions {
            analysis.functions
        } else {
            Vec::new()
        },
        generated: generated_kind,
    })
}

/// Analyse every discovered file, in parallel when it pays off.
pub fn collect_files(
    root: &Path,
    files: &[PathBuf],
    read: ReadOptions,
    jobs: usize,
    progress: Option<&Progress<'_>>,
) -> Vec<FileFacts> {
    let total = files.len();
    if total == 0 {
        return Vec::new();
    }
    let worker_count = jobs.max(1).min(total);
    if worker_count <= 1 || total < 8 {
        let mut facts = Vec::with_capacity(total);
        for (index, path) in files.iter().enumerate() {
            if let Some(item) = read_facts(path, root, read) {
                facts.push(item);
            }
            if let Some(callback) = progress {
                callback(index + 1, total, path);
            }
        }
        return facts;
    }

    let cursor = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let slots: Vec<Mutex<Option<FileFacts>>> = (0..total).map(|_| Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..worker_count {
            scope.spawn(|| loop {
                let index = cursor.fetch_add(1, Ordering::Relaxed);
                if index >= total {
                    break;
                }
                let path = &files[index];
                if let Some(facts) = read_facts(path, root, read) {
                    if let Ok(mut slot) = slots[index].lock() {
                        *slot = Some(facts);
                    }
                }
                let finished = done.fetch_add(1, Ordering::Relaxed) + 1;
                if let Some(callback) = progress {
                    callback(finished, total, path);
                }
            });
        }
    });

    slots
        .into_iter()
        .filter_map(|slot| slot.into_inner().ok().flatten())
        .collect()
}

/// Reduce one analysed file to the business-code counters.
pub fn summarize(facts: &FileFacts, siblings: &HashSet<PathBuf>, include_tests: bool) -> Stats {
    let forced_test = facts.name_test || siblings.contains(&facts.path) || facts.crate_test;
    let counts = if include_tests {
        facts.all
    } else if forced_test {
        Counts {
            test: facts.physical,
            ..Counts::default()
        }
    } else {
        facts.all.sub(facts.test_only)
    };
    let metrics = if include_tests {
        facts.metrics_all
    } else if forced_test {
        Metrics::default()
    } else {
        facts.metrics
    };
    Stats {
        lines: counts.blank + counts.comment + counts.code,
        blank: counts.blank,
        comment: counts.comment,
        doc: counts.doc,
        code: counts.code,
        test: counts.test,
        physical: facts.physical,
        complexity: metrics.complexity,
        unsafe_kw: metrics.unsafe_kw,
        test_fns: metrics.test_fns,
        // A test-only file contributes no business lines, so its ULOC set is
        // empty in the Python implementation even though the raw lines were
        // read; zeroing it here keeps the per-file numbers identical.
        uloc: match facts.unique.as_ref() {
            None => None,
            Some(_) if forced_test && !include_tests => Some(0),
            Some(set) => Some(set.len() as u32),
        },
        forced_test,
        generated: facts.generated,
    }
}

/// Files pulled in by cfg(test) mod declarations, which are test-only too.
pub fn resolve_siblings(facts: &[FileFacts]) -> HashSet<PathBuf> {
    let mut siblings = HashSet::new();
    for item in facts {
        let base = match item.path.parent() {
            Some(base) => base,
            None => continue,
        };
        for target in &item.sibling_targets {
            let candidate = if target.ends_with(".rs") {
                base.join(target)
            } else {
                base.join(format!("{}.rs", target))
            };
            siblings.insert(resolve_path(&candidate));
        }
    }
    siblings
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Totals {
    pub lines: u32,
    pub blank: u32,
    pub comment: u32,
    pub doc: u32,
    pub code: u32,
    pub test: u32,
    pub physical: u32,
    pub complexity: u32,
    pub unsafe_kw: u32,
    pub test_fns: u32,
    /// Distinct-line count for a module group; `None` mirrors how the Python
    /// `totals()` dict simply omits ULOC.
    pub uloc: Option<usize>,
}

impl Totals {
    pub fn column(&self, key: &str) -> u32 {
        match key {
            "lines" => self.lines,
            "blank" => self.blank,
            "comment" => self.comment,
            "doc" => self.doc,
            "code" => self.code,
            "test" => self.test,
            "physical" => self.physical,
            "complexity" => self.complexity,
            "unsafe" => self.unsafe_kw,
            "test_fns" => self.test_fns,
            "uloc" => self.uloc.unwrap_or(0) as u32,
            _ => 0,
        }
    }
}

pub fn totals(entries: &[&Entry]) -> Totals {
    let mut total = Totals::default();
    for entry in entries {
        let stats = &entry.stats;
        total.lines += stats.lines;
        total.blank += stats.blank;
        total.comment += stats.comment;
        total.doc += stats.doc;
        total.code += stats.code;
        total.test += stats.test;
        total.physical += stats.physical;
        total.complexity += stats.complexity;
        total.unsafe_kw += stats.unsafe_kw;
        total.test_fns += stats.test_fns;
    }
    total
}

/// Distinct business lines across every file that kept a ULOC set.
pub fn union_uloc(entries: &[&Entry]) -> Option<usize> {
    let mut merged: HashSet<&str> = HashSet::new();
    let mut seen = false;
    for entry in entries {
        if let Some(set) = entry.facts.unique.as_ref() {
            seen = true;
            for line in set.iter() {
                merged.insert(line);
            }
        }
    }
    if seen {
        Some(merged.len())
    } else {
        None
    }
}

pub const SORT_KEYS: [&str; 10] = [
    "code",
    "lines",
    "blank",
    "comment",
    "test",
    "complexity",
    "unsafe",
    "test_fns",
    "uloc",
    "path",
];

/// Order entries the way the Python implementation does (stable, desc).
pub fn sort_entries<'a>(mut entries: Vec<&'a Entry>, key: &str) -> Vec<&'a Entry> {
    match key {
        "path" => entries.sort_by_key(|entry| posix(entry.path())),
        "uloc" => entries.sort_by_key(|entry| std::cmp::Reverse(entry.column("uloc"))),
        other => entries.sort_by_key(|entry| std::cmp::Reverse(entry.column(other))),
    }
    entries
}

/// Group business entries by (root, first path component), biggest first.
pub fn module_groups<'a>(
    entries: &[&'a Entry],
    cwd: &Path,
) -> Vec<(String, Totals, Vec<&'a Entry>)> {
    let mut order: Vec<String> = Vec::new();
    let mut buckets: std::collections::HashMap<String, Vec<&'a Entry>> =
        std::collections::HashMap::new();
    for entry in entries {
        let label = format!(
            "{}/{}",
            relative(entry.root(), cwd),
            group_key(entry.path(), entry.root())
        );
        if !buckets.contains_key(&label) {
            order.push(label.clone());
        }
        buckets.entry(label).or_default().push(entry);
    }
    let mut rows: Vec<(String, Totals, Vec<&'a Entry>)> = order
        .into_iter()
        .map(|label| {
            let items = buckets.remove(&label).unwrap_or_default();
            let mut stats = totals(&items);
            stats.uloc = union_uloc(&items);
            (label, stats, items)
        })
        .collect();
    rows.sort_by_key(|row| std::cmp::Reverse(row.1.code));
    rows
}

/// Ignored directory names for a scan, honouring the default list.
pub fn default_ignore_dirs() -> BTreeSet<String> {
    DEFAULT_IGNORE_DIRS
        .iter()
        .map(|name| name.to_string())
        .collect()
}
