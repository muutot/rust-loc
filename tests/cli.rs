//! End-to-end tests for the rust-loc CLI.
//!
//! Ported from `python/tests/test_cli.py` so the Rust implementation is held
//! to the same behaviour as the Python reference it replaced.

mod json;

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use rust_loc::cli;
use rust_loc::options::{Format, Options};
use rust_loc::report::{self, Report};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A scratch directory that removes itself when the test ends.
struct Workspace {
    path: PathBuf,
}

impl Workspace {
    fn new() -> Workspace {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let name = format!(
            "rust-loc-test-{}-{}-{}",
            std::process::id(),
            nanos,
            COUNTER.fetch_add(1, Ordering::SeqCst)
        );
        let path = std::env::temp_dir().join(name);
        std::fs::create_dir_all(&path).unwrap();
        Workspace { path }
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.path.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, text).unwrap();
        path
    }

    /// `[root, --no-gitignore, ...extra]`, run in-process. Output is returned
    /// exactly as it would reach stdout.
    fn run(&self, extra: &[&str]) -> String {
        let mut args: Vec<String> = vec![
            self.path.display().to_string(),
            "--no-gitignore".to_string(),
        ];
        args.extend(extra.iter().map(|item| item.to_string()));
        cli::run(&args, &self.path).expect("cli::run failed")
    }

    fn invocation(&self, extra: &[&str]) -> cli::Invocation {
        let mut args: Vec<String> = vec![self.path.display().to_string()];
        args.extend(extra.iter().map(|item| item.to_string()));
        cli::parse(&args, &self.path).expect("cli::parse failed")
    }

    fn report(&self, extra: &[&str]) -> Report {
        let invocation = self.invocation(extra);
        report::scan(&invocation.roots, &invocation.options, &self.path, None)
    }

    fn document(&self, extra: &[&str]) -> json::Value {
        let mut args = vec!["--format", "json"];
        args.extend_from_slice(extra);
        json::parse(&self.run(&args))
    }

    fn summary(&self, extra: &[&str]) -> json::Value {
        self.document(extra).get("summary").unwrap().clone()
    }

    fn find(&self, document: &json::Value, suffix: &str) -> json::Value {
        document
            .get("files")
            .unwrap()
            .items()
            .iter()
            .find(|entry| entry.get("path").unwrap().text().ends_with(suffix))
            .unwrap_or_else(|| panic!("no json entry ending with {}", suffix))
            .clone()
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

const BASIC: &str = "// plain\n\nfn main() {\n    let x = 1; // trailing\n}\n";
const DOCS: &str = "/// docs\n// plain\nfn main() {}\n";
const NESTED: &str = "/* outer /* inner */ still comment */\nfn a() {}\n";
const RAW_BRACES: &str = "const S: &str = r#\"{ \"not code\" }\"#;\n";
const WITH_TESTS: &str = "pub fn value() -> u32 {\n    1\n}\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn works() {\n        assert_eq!(1, 1);\n    }\n}\n";
const SMALL_WITH_TESTS: &str = "fn a() {}\n\n#[cfg(test)]\nmod tests {\n    fn b() {}\n}\n";

#[test]
fn counts_blank_comment_and_code() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BASIC);
    let summary = workspace.summary(&[]);
    assert_eq!(summary.get("blank").unwrap().integer(), 1);
    assert_eq!(summary.get("comment").unwrap().integer(), 1);
    assert_eq!(summary.get("code").unwrap().integer(), 3);
    assert_eq!(summary.get("lines").unwrap().integer(), 5);
}

#[test]
fn doc_comments_are_reported_separately() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", DOCS);
    let summary = workspace.summary(&[]);
    assert_eq!(summary.get("comment").unwrap().integer(), 2);
    assert_eq!(summary.get("doc").unwrap().integer(), 1);
    assert_eq!(summary.get("code").unwrap().integer(), 1);
}

#[test]
fn nested_block_comment_is_one_comment() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", NESTED);
    let summary = workspace.summary(&[]);
    assert_eq!(summary.get("comment").unwrap().integer(), 1);
    assert_eq!(summary.get("code").unwrap().integer(), 1);
}

#[test]
fn raw_string_with_braces_stays_code() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", RAW_BRACES);
    let summary = workspace.summary(&[]);
    assert_eq!(summary.get("code").unwrap().integer(), 1);
    assert_eq!(summary.get("comment").unwrap().integer(), 0);
    assert_eq!(summary.get("test").unwrap().integer(), 0);
}

#[test]
fn cfg_test_module_is_excluded() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", WITH_TESTS);
    let summary = workspace.summary(&[]);
    assert_eq!(summary.get("code").unwrap().integer(), 3);
    assert_eq!(summary.get("test").unwrap().integer(), 7);
}

#[test]
fn crate_level_cfg_test_excludes_whole_file() {
    let workspace = Workspace::new();
    workspace.write(
        "src/crate_inner.rs",
        "#![cfg(test)]\n\n#[test]\nfn a() {}\n",
    );
    let document = workspace.document(&[]);
    assert_eq!(
        document
            .get("summary")
            .unwrap()
            .get("files")
            .unwrap()
            .integer(),
        0
    );
    assert_eq!(
        document
            .get("excluded")
            .unwrap()
            .get("files")
            .unwrap()
            .integer(),
        1
    );
}

#[test]
fn path_attribute_sibling_becomes_test_only() {
    let workspace = Workspace::new();
    workspace.write("src/hidden/support.rs", "pub fn helper() -> u32 { 1 }\n");
    workspace.write(
        "src/lib.rs",
        "#[cfg(test)]\n#[path = \"hidden/support.rs\"]\nmod support;\n",
    );
    let document = workspace.document(&[]);
    assert!(workspace
        .find(&document, "hidden/support.rs")
        .get("test_only")
        .unwrap()
        .flag());
    assert_eq!(
        document
            .get("summary")
            .unwrap()
            .get("lines")
            .unwrap()
            .integer(),
        0
    );
}

#[test]
fn plain_path_attribute_does_not_leak_into_later_test_mod() {
    let workspace = Workspace::new();
    workspace.write("src/misc.rs", "pub fn misc() -> u32 { 1 }\n");
    workspace.write("src/helper.rs", "pub fn helper() -> u32 { 2 }\n");
    workspace.write(
        "src/lib.rs",
        "#[path = \"misc.rs\"]\nmod misc;\n\n#[cfg(test)]\nmod helper;\n",
    );
    let document = workspace.document(&[]);
    assert!(!workspace
        .find(&document, "src/misc.rs")
        .get("test_only")
        .unwrap()
        .flag());
    assert!(workspace
        .find(&document, "src/helper.rs")
        .get("test_only")
        .unwrap()
        .flag());
}

#[test]
fn any_test_and_not_test_are_kept() {
    let workspace = Workspace::new();
    workspace.write(
        "src/lib.rs",
        "#[cfg(any(test, feature = \"x\"))]\npub fn a() {}\n\n#[cfg(not(test))]\npub fn b() {}\n",
    );
    let summary = workspace.summary(&[]);
    assert_eq!(summary.get("code").unwrap().integer(), 4);
    assert_eq!(summary.get("test").unwrap().integer(), 0);
}

#[test]
fn test_named_files_and_tests_dir_are_excluded() {
    let workspace = Workspace::new();
    workspace.write("src/foo_tests.rs", "pub fn a() {}\n");
    workspace.write("src/tests/helper.rs", "pub fn b() {}\n");
    workspace.write("src/lib.rs", "pub fn c() {}\n");
    let document = workspace.document(&[]);
    assert_eq!(
        document
            .get("summary")
            .unwrap()
            .get("files")
            .unwrap()
            .integer(),
        1
    );
    assert_eq!(
        document
            .get("excluded")
            .unwrap()
            .get("files")
            .unwrap()
            .integer(),
        2
    );
}

#[test]
fn fake_cfg_test_inside_string_and_comment_is_ignored() {
    let workspace = Workspace::new();
    workspace.write(
        "src/lib.rs",
        "fn main() {\n    let s = \"#[cfg(test)] mod fake { }\";\n    // #[cfg(test)] mod fake2 { }\n    let t = \"}\";\n}\n",
    );
    let summary = workspace.summary(&[]);
    assert_eq!(summary.get("test").unwrap().integer(), 0);
    assert_eq!(summary.get("code").unwrap().integer(), 4);
    assert_eq!(summary.get("comment").unwrap().integer(), 1);
}

#[test]
fn include_tests_adds_test_lines_back() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", SMALL_WITH_TESTS);
    let plain = workspace.summary(&[]);
    let included = workspace.summary(&["--include-tests"]);
    assert_eq!(
        included.get("lines").unwrap().integer(),
        plain.get("lines").unwrap().integer() + plain.get("test").unwrap().integer()
    );
    assert_eq!(included.get("test").unwrap().integer(), 0);
}

#[test]
fn complexity_uses_scc_rust_branch_tokens() {
    let workspace = Workspace::new();
    workspace.write(
        "src/lib.rs",
        "fn f(a: u32) -> u32 {\n    if a == 1 && a != 2 {\n        1\n    } else {\n        2\n    }\n}\n",
    );
    let summary = workspace.summary(&[]);
    assert_eq!(summary.get("complexity").unwrap().integer(), 5);
}

#[test]
fn unsafe_and_test_function_counters() {
    let workspace = Workspace::new();
    workspace.write(
        "src/lib.rs",
        "pub unsafe fn f() {\n    unsafe { }\n}\n\n#[test]\nfn t() {}\n",
    );
    let summary = workspace.summary(&[]);
    assert_eq!(summary.get("unsafe").unwrap().integer(), 2);
    assert_eq!(summary.get("test_fns").unwrap().integer(), 1);
}

#[test]
fn uloc_counts_unique_non_blank_lines() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", "fn a() {}\nfn a() {}\n\nfn b() {}\n");
    let summary = workspace.summary(&["--uloc"]);
    assert_eq!(summary.get("uloc").unwrap().integer(), 2);
}

#[test]
fn exclude_glob_skips_matching_paths() {
    let workspace = Workspace::new();
    workspace.write("src/keep.rs", "fn a() {}\n");
    workspace.write("src/skip/other.rs", "fn b() {}\n");
    let document = workspace.document(&["--exclude", "*/skip/*"]);
    assert_eq!(
        document
            .get("summary")
            .unwrap()
            .get("files")
            .unwrap()
            .integer(),
        1
    );
}

#[test]
fn text_output_lists_metrics() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", "fn a() {}\n");
    let out = workspace.run(&[]);
    assert!(
        out.contains("Rust business code (tests excluded)"),
        "{}",
        out
    );
    assert!(out.contains("Complexity (est.)"), "{}", out);
}

#[test]
fn csv_has_header_and_total_row() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", "fn a() {}\n");
    let out = workspace.run(&["--format", "csv"]);
    let rows: Vec<&str> = out.trim().lines().collect();
    assert_eq!(rows[0].split(',').next().unwrap(), "path");
    assert_eq!(rows[1].split(',').next().unwrap(), "TOTAL");
}

#[test]
fn markdown_contains_table() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", "fn a() {}\n");
    let out = workspace.run(&["--format", "markdown"]);
    assert!(out.contains("| Code |"), "{}", out);
}

#[test]
fn no_complexity_hides_column() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", "fn a() {}\n");
    let shown = workspace.run(&["--by-file"]);
    let hidden = workspace.run(&["--by-file", "--no-complexity"]);
    assert!(shown.contains("Cplx"), "{}", shown);
    assert!(!hidden.contains("Cplx"), "{}", hidden);
}

#[test]
fn version_flag_prints_the_crate_version() {
    let workspace = Workspace::new();
    let out = workspace.run(&["--version"]);
    assert!(out.contains("rust-loc"), "{}", out);
    assert!(out.contains(env!("CARGO_PKG_VERSION")), "{}", out);
}

#[test]
fn help_flag_documents_every_switch() {
    let workspace = Workspace::new();
    let out = workspace.run(&["--help"]);
    for flag in ["--by-file", "--by-module", "--uloc", "--jobs", "--exclude"] {
        assert!(out.contains(flag), "{} missing from help", flag);
    }
}

#[test]
fn scan_returns_the_same_numbers_as_the_cli() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", SMALL_WITH_TESTS);
    let report = workspace.report(&[]);
    assert_eq!(report.summary.code, 1);
    assert_eq!(report.summary.test, 4);
    assert_eq!(report.business.len(), 1);
    assert_eq!(report.excluded_entries.len(), 0);
}

#[test]
fn scan_reports_progress_for_every_file() {
    let workspace = Workspace::new();
    workspace.write("src/a.rs", "fn a() {}\n");
    workspace.write("src/b.rs", "fn b() {}\n");
    let seen = std::sync::Mutex::new(Vec::new());
    let callback = |done: usize, total: usize, _path: &std::path::Path| {
        seen.lock().unwrap().push((done, total));
    };
    let options = Options {
        gitignore: false,
        ..Options::default()
    };
    let invocation = workspace.invocation(&[]);
    report::scan(
        &invocation.roots,
        &options,
        &workspace.path,
        Some(&callback),
    );
    let mut seen = seen.into_inner().unwrap();
    seen.sort();
    assert_eq!(seen, vec![(1, 2), (2, 2)]);
}

#[test]
fn scan_records_the_roots_and_options() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", "fn a() {}\n");
    let report = workspace.report(&["--uloc"]);
    assert_eq!(report.roots.len(), 1);
    assert!(report.options.uloc);
    assert_eq!(report.uloc_total, Some(1));
}

#[test]
fn render_dispatches_to_every_format() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", "fn a() {}\n");
    let invocation = workspace.invocation(&[]);
    let report = report::scan(
        &invocation.roots,
        &invocation.options,
        &workspace.path,
        None,
    );

    assert!(report::render(&report, &workspace.path).contains("Rust business code"));

    let csv = report.render_as(Format::Csv, &workspace.path);
    assert!(csv.starts_with("path,"), "{}", csv);

    assert!(report
        .render_as(Format::Markdown, &workspace.path)
        .contains("| Code |"));

    let document = json::parse(&report.render_as(Format::Json, &workspace.path));
    assert_eq!(
        document
            .get("summary")
            .unwrap()
            .get("files")
            .unwrap()
            .integer(),
        1
    );
}

#[test]
fn parse_carries_every_switch() {
    let workspace = Workspace::new();
    let invocation = workspace.invocation(&[
        "--by-file",
        "--sort",
        "lines",
        "--top",
        "5",
        "--uloc",
        "--include-tests",
        "--no-complexity",
        "--no-gitignore",
        "--exclude",
        "*/gen/*",
        "--ignore-dir",
        "tmp",
    ]);
    let options = invocation.options;
    assert!(options.by_file);
    assert_eq!(options.sort, "lines");
    assert_eq!(options.top, 5);
    assert!(options.uloc);
    assert!(options.include_tests);
    assert!(!options.complexity);
    assert!(!options.gitignore);
    assert_eq!(options.exclude, vec!["*/gen/*".to_string()]);
    assert!(options.ignore_dirs.contains("tmp"));
    assert!(options.ignore_dirs.contains("target"));
}

#[test]
fn no_ignore_dirs_switch_empties_the_default_set() {
    let workspace = Workspace::new();
    let options = workspace
        .invocation(&["--no-ignore-dirs", "--ignore-dir", "tmp"])
        .options;
    assert_eq!(options.ignore_dirs.len(), 1);
    assert!(options.ignore_dirs.contains("tmp"));
}

#[test]
fn jobs_switch_is_honoured() {
    let workspace = Workspace::new();
    assert_eq!(
        workspace
            .invocation(&["--jobs", "3"])
            .options
            .worker_count(),
        3
    );
    assert!(workspace.invocation(&[]).options.worker_count() >= 1);
}

#[test]
fn missing_root_is_rejected() {
    let workspace = Workspace::new();
    let arguments = vec![workspace
        .path
        .join("definitely-missing")
        .display()
        .to_string()];
    let error = cli::run(&arguments, &workspace.path).unwrap_err();
    assert!(error.contains("does not exist"), "{}", error);
}
