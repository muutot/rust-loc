//! File discovery, parallel analysis and the aggregates every renderer shares.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use crate::generated::{self, Generated};
use crate::languages::{self, LineCounts};
use crate::lexer::{self, Analysis, FunctionInfo, Halstead, Metrics, Smells};
use crate::options::Options;
use crate::sources::{Lang, Sources};

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

/// Which part of a scan a progress report belongs to. They run one after the
/// other, so a bar that only knows the file count sits at 100% while the
/// slowest stage - blaming every business file - is still ahead of it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    /// Reading and analysing the discovered files.
    Files,
    /// Blaming the business files for the author tables.
    Blame,
}

/// Progress callback: `(stage, done, total, path)`, where `total` counts the
/// work of that one stage. A UI can show a progress bar without duplicating the
/// walking logic.
pub type Progress<'a> = dyn Fn(Stage, usize, usize, &Path) + Sync + 'a;

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

/// Collect every source file below root that the scan's language selection
/// reads and the ignore rules keep.
pub fn discover(root: &Path, options: &Options) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    walk(root, options, &mut found, &|path| {
        options.sources.accepts(path)
            && !excluded_extension(path, &options.exclude_ext)
            && within_size_cap(path, options.max_file_size_mb)
    });
    sort_paths(&mut found);
    found
}

/// Whether a path carries an extension the scan was told to skip.
///
/// An entry is matched against the end of the file name, so a plain extension
/// (`lock`) and a dotted suffix (`min.js`) both work: the first skips
/// `Cargo.lock`, the second skips `app.min.js`. The dot and the case are
/// optional on either side.
pub fn excluded_extension(path: &Path, skipped: &BTreeSet<String>) -> bool {
    if skipped.is_empty() {
        return false;
    }
    let Some(name) = path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
    else {
        return false;
    };
    if name.is_empty() {
        return false;
    }
    skipped.iter().any(|entry| {
        let entry = entry.trim().trim_start_matches('.').to_ascii_lowercase();
        if entry.is_empty() {
            return false;
        }
        name == entry || name.ends_with(&format!(".{entry}"))
    })
}
/// Whether a file is larger than the cap allows, in megabytes. A cap of zero
/// reads every file, which is the behaviour a scan always had.
pub fn within_size_cap(path: &Path, megabytes: u64) -> bool {
    if megabytes == 0 {
        return true;
    }
    let Ok(meta) = std::fs::metadata(path) else {
        return true;
    };
    let cap = megabytes.saturating_mul(1024 * 1024);
    meta.len() <= cap
}

/// Collect every file the language table knows, for `--languages`.
pub fn discover_languages(root: &Path, options: &Options) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    walk(root, options, &mut found, &|path| {
        languages::detect(path).is_some()
    });
    sort_paths(&mut found);
    found
}

/// Depth-first walk that keeps the files `keep` accepts.
fn walk(root: &Path, options: &Options, found: &mut Vec<PathBuf>, keep: &dyn Fn(&Path) -> bool) {
    // Ignore rules apply to the components below the scan root only: a root
    // that happens to sit inside a directory named "build" or "target" must
    // still be scanned instead of being silently skipped as a whole.
    let base = root.components().count();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let read = match std::fs::read_dir(&dir) {
            Ok(read) => read,
            Err(_) => continue,
        };
        for entry in read.flatten() {
            let path = entry.path();
            let skipped = path.components().skip(base).any(|part| {
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
            } else if file_type.is_file()
                && keep(&path)
                && !matches_exclude(&path, root, &options.exclude)
            {
                found.push(path);
            }
        }
    }
}

/// Mirror Python's `sorted(root.rglob("*.rs"))`: `Path` orders by path
/// components (case-insensitively on Windows), not by the joined string.
/// The difference shows up for `a/backup.rs` vs `a/backup/scale.rs`,
/// where the directory sorts first.
fn sort_paths(found: &mut [PathBuf]) {
    found.sort_by_key(|path| component_key(path));
}

/// Lowercased path components, the key Python's `PurePath` comparison uses.
fn component_key(path: &Path) -> Vec<String> {
    path.components()
        .map(|part| part.as_os_str().to_string_lossy().to_lowercase())
        .collect()
}

/// Run git to honour .gitignore rules; degrade to a no-op on any failure.
pub fn apply_gitignore(
    root: &Path,
    files: Vec<PathBuf>,
    enabled: bool,
    sources: Sources,
) -> Vec<PathBuf> {
    // The listing only holds the files the walk kept, so the pathspec only has
    // to name the languages this run reads.
    gitignore_filter(root, files, enabled, &sources.pathspecs())
}

/// The same filter for the language walk, which is not Rust-only.
pub fn apply_gitignore_all(root: &Path, files: Vec<PathBuf>, enabled: bool) -> Vec<PathBuf> {
    gitignore_filter(root, files, enabled, &[])
}

fn gitignore_filter(
    root: &Path,
    files: Vec<PathBuf>,
    enabled: bool,
    patterns: &[String],
) -> Vec<PathBuf> {
    if !enabled || files.is_empty() {
        return files;
    }
    let top = match run_git(root, &["rev-parse", "--show-toplevel"]) {
        Some(top) => PathBuf::from(top.trim()),
        None => return files,
    };
    let mut args = vec![
        "ls-files",
        "-z",
        "--cached",
        "--others",
        "--exclude-standard",
        "--",
    ];
    args.extend(patterns.iter().map(String::as_str));
    let listing = match run_git(&top, &args) {
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

/// Files and line classes for one language.
#[derive(Debug, Default, Clone, Copy)]
pub struct Tally {
    pub files: u32,
    pub counts: LineCounts,
}

impl Tally {
    fn merge(&mut self, other: Tally) {
        self.files += other.files;
        self.counts.lines += other.counts.lines;
        self.counts.blank += other.counts.blank;
        self.counts.comment += other.counts.comment;
        self.counts.code += other.counts.code;
    }
}

/// Count every known file and fold the result per language, largest first.
pub fn language_totals(files: &[PathBuf], workers: usize) -> Vec<(&'static str, Tally)> {
    let mut merged: HashMap<&'static str, Tally> = HashMap::new();
    let chunk = chunk_size(files.len(), workers);
    std::thread::scope(|scope| {
        let handles: Vec<_> = files
            .chunks(chunk)
            .map(|slice| scope.spawn(move || count_languages(slice)))
            .collect();
        for handle in handles {
            let Ok(partial) = handle.join() else {
                continue;
            };
            for (name, tally) in partial {
                merged.entry(name).or_default().merge(tally);
            }
        }
    });
    let mut rows: Vec<(&'static str, Tally)> = merged.into_iter().collect();
    rows.sort_by(|left, right| {
        right
            .1
            .counts
            .code
            .cmp(&left.1.counts.code)
            .then(left.0.cmp(right.0))
    });
    rows
}

/// How many files one thread of the language pass takes.
fn chunk_size(total: usize, workers: usize) -> usize {
    if total == 0 {
        return 1;
    }
    total.div_ceil(workers.max(1)).max(1)
}

/// Line classes for one slice of files, keyed by language name.
fn count_languages(files: &[PathBuf]) -> Vec<(&'static str, Tally)> {
    let mut partial: HashMap<&'static str, Tally> = HashMap::new();
    for path in files {
        let Some(language) = languages::detect(path) else {
            continue;
        };
        let Some(text) = read_text(path) else {
            continue;
        };
        let counts = if language.name == "Rust" {
            rust_counts(&text)
        } else {
            languages::count(&text, language)
        };
        partial
            .entry(language.name)
            .or_default()
            .merge(Tally { files: 1, counts });
    }
    partial.into_iter().collect()
}

/// What the Rust lexer sees in a whole file, test code included: the language
/// overview describes the repository, not just its business scope.
fn rust_counts(text: &str) -> LineCounts {
    let analysis = lexer::analyze(text);
    let mut counts = LineCounts {
        lines: analysis.total as u32,
        ..LineCounts::default()
    };
    for index in 0..analysis.total {
        if analysis.code[index] {
            counts.code += 1;
        } else if analysis.comment[index] {
            counts.comment += 1;
        } else {
            counts.blank += 1;
        }
    }
    counts
}

/// Read a file as lossy UTF-8, without a BOM and with normalised newlines.
pub(crate) fn read_text(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let bytes = match bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        Some(rest) => rest,
        None => bytes.as_slice(),
    };
    let text = String::from_utf8_lossy(bytes);
    Some(lexer::normalize_newlines(&text).into_owned())
}

/// Run git hidden (no console window flash) and return stdout on success.
pub(crate) fn run_git(dir: &Path, args: &[&str]) -> Option<String> {
    let output = git_command(git_program())
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The same call as [`run_git`], exposed for the `probe` example: it prints
/// what git answered instead of swallowing the failure.
pub fn run_git_probe(dir: &Path) -> Option<String> {
    run_git(dir, &["rev-parse", "--show-toplevel"])
}

/// A git command that never shows a console window of its own.
fn git_command(program: &OsStr) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command
}

/// The program every git call runs.
///
/// Windows puts Git for Windows' `cmd\git.exe` on `PATH`: a launcher that
/// starts the real `mingw64\bin\git.exe` behind it. The author pass starts
/// one git per file, so that second spawn is worth a good part of the pass, and
/// it buys nothing - ask git where its own parts live once, and run the binary
/// that sits next to them. Anything that is not that layout, and every other
/// platform, keeps the plain `git`, which is what a launcher wants anyway.
fn git_program() -> &'static OsStr {
    static PROGRAM: std::sync::OnceLock<OsString> = std::sync::OnceLock::new();
    PROGRAM.get_or_init(|| match direct_git() {
        Some(program) => program,
        None => OsString::from("git"),
    })
}

/// The real git behind the launcher `PATH` finds, when there is one.
#[cfg(windows)]
fn direct_git() -> Option<OsString> {
    let output = git_command(OsStr::new("git"))
        .arg("--exec-path")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let exec = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    // `<root>/mingw64/libexec/git-core`, or `<root>/libexec/git-core` on an
    // older install: the real binaries sit in the `bin` beside the libexec.
    let libexec = exec.parent()?;
    let root = libexec.parent()?;
    for candidate in [root.join("mingw64/bin/git.exe"), root.join("bin/git.exe")] {
        if candidate.is_file() {
            return Some(candidate.into_os_string());
        }
    }
    None
}

#[cfg(not(windows))]
fn direct_git() -> Option<OsString> {
    None
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
    /// Which reader classified this file.
    pub lang: Lang,
    pub name_test: bool,
    pub crate_test: bool,
    pub sibling_targets: Vec<String>,
    pub physical: u32,
    pub all: Counts,
    pub test_only: Counts,
    pub metrics: Metrics,
    pub metrics_all: Metrics,
    pub smells: Smells,
    pub smells_all: Smells,
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
    pub cognitive: u32,
    pub unsafe_kw: u32,
    pub test_fns: u32,
    pub halstead: Halstead,
    pub smells: Smells,
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

    /// The language this row was read with.
    pub fn lang(&self) -> Lang {
        self.facts.lang
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
            "cognitive" => self.stats.cognitive,
            "unsafe" => self.stats.unsafe_kw,
            "test_fns" => self.stats.test_fns,
            "unwrap" => self.stats.smells.unwrap,
            "expect" => self.stats.smells.expect,
            "panic" => self.stats.smells.panic,
            "todo" => self.stats.smells.todo,
            "unimplemented" => self.stats.smells.unimplemented,
            "allow" => self.stats.smells.allow,
            "smells" => self.stats.smells.total(),
            "uloc" => self.stats.uloc.unwrap_or(0),
            _ => 0,
        }
    }
}

/// Test-only path heuristics: a test directory, then the naming convention of
/// the language. Rust writes `tests.rs` next to `mod tests`; Python writes
/// `test_` in front and puts a `conftest.py` beside it; the JVM spells a test
/// file `FooTest`; the JS world keeps a `__tests__` folder and a `.spec.` stem.
pub fn is_test_file(path: &Path, lang: Lang) -> bool {
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let lower = stem.to_lowercase();
    // The convention every language shares: the file *is* a test.
    let shared = lower == "test"
        || lower == "tests"
        || lower.ends_with("_test")
        || lower.ends_with("_tests");
    // What the language spells on top of that.
    let named = match lang {
        Lang::Python => stem == "conftest" || lower.starts_with("test_"),
        Lang::Java | Lang::Kotlin | Lang::CSharp | Lang::Swift | Lang::Dart | Lang::Php => {
            stem.ends_with("Test") || stem.ends_with("Tests")
        }
        Lang::Ruby => lower.ends_with("_spec"),
        Lang::JavaScript | Lang::TypeScript | Lang::Svelte => {
            lower.ends_with(".test") || lower.ends_with(".spec")
        }
        _ => false,
    };
    let directories: &[&str] = match lang {
        Lang::JavaScript | Lang::TypeScript | Lang::Svelte => {
            &["tests", "benches", "test", "__tests__", "spec"]
        }
        Lang::Ruby => &["tests", "benches", "test", "spec"],
        _ => &["tests", "benches", "test"],
    };
    let in_test_dir = path.components().any(|part| {
        let name = part.as_os_str().to_string_lossy();
        directories.contains(&name.as_ref())
    });
    shared || named || in_test_dir
}

/// The language a discovered file is read with.
pub fn language_of(path: &Path) -> Lang {
    Lang::of(path).unwrap_or(Lang::Rust)
}

/// Read and analyse one file, reducing it to facts.
fn read_facts(path: &Path, root: &Path, read: ReadOptions) -> Option<FileFacts> {
    let text = read_text(path)?;
    let lang = language_of(path);
    // Each language reads with its own reader: Rust and Python follow their
    // grammar, everything else shares the generic scanner.
    let analysis: Analysis = lang.analyze(&text);
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
        lang,
        name_test: is_test_file(relative_path, lang),
        crate_test: analysis.crate_test,
        sibling_targets: analysis.sibling_targets,
        physical: analysis.total as u32,
        all,
        test_only,
        metrics: analysis.metrics,
        metrics_all: analysis.metrics_all,
        smells: analysis.smells,
        smells_all: analysis.smells_all,
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
                callback(Stage::Files, index + 1, total, path);
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
                    callback(Stage::Files, finished, total, path);
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
    let smells = if include_tests {
        facts.smells_all
    } else if forced_test {
        Smells::default()
    } else {
        facts.smells
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
        cognitive: metrics.cognitive,
        unsafe_kw: metrics.unsafe_kw,
        test_fns: metrics.test_fns,
        halstead: metrics.halstead,
        smells,
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
    pub cognitive: u32,
    pub unsafe_kw: u32,
    pub test_fns: u32,
    /// Test functions across the whole tree - business scope plus the files
    /// the scan excluded as test-only. The cards describe the repository, and
    /// a project whose tests live in `*_test.go` files would otherwise read
    /// as having none, which is exactly the number the reader is asking
    /// about. The text report keeps the business-only column.
    pub test_fns_all: u32,
    pub smells: Smells,
    /// Sum of the per-file Halstead volumes. Volume is additive enough for a
    /// project figure; the distinct-token counts behind it are not.
    pub volume: f64,
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
            "cognitive" => self.cognitive,
            "volume" => self.volume as u32,
            "unsafe" => self.unsafe_kw,
            "test_fns" => self.test_fns,
            "unwrap" => self.smells.unwrap,
            "expect" => self.smells.expect,
            "panic" => self.smells.panic,
            "todo" => self.smells.todo,
            "unimplemented" => self.smells.unimplemented,
            "allow" => self.smells.allow,
            "smells" => self.smells.total(),
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
        total.cognitive += stats.cognitive;
        total.volume += stats.halstead.volume();
        total.unsafe_kw += stats.unsafe_kw;
        total.test_fns += stats.test_fns;
        // Every file analysed (business or test-only) carries its
        // `metrics_all.test_fns`; a test file's own Stats zero the business
        // column, so the all-tree count reads it from the facts side.
        total.test_fns_all += if stats.forced_test {
            entry.facts.metrics_all.test_fns
        } else {
            stats.test_fns
        };
        total.smells.unwrap += stats.smells.unwrap;
        total.smells.expect += stats.smells.expect;
        total.smells.panic += stats.smells.panic;
        total.smells.todo += stats.smells.todo;
        total.smells.unimplemented += stats.smells.unimplemented;
        total.smells.allow += stats.smells.allow;
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

pub const SORT_KEYS: [&str; 18] = [
    "code",
    "lines",
    "blank",
    "comment",
    "test",
    "complexity",
    "cognitive",
    "unsafe",
    "test_fns",
    "smells",
    "unwrap",
    "expect",
    "panic",
    "todo",
    "unimplemented",
    "allow",
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
