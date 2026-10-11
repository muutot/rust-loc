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

    /// Like `run`, but keeps the exit code the quality gates produced.
    fn checked(&self, extra: &[&str]) -> (String, u8) {
        let mut args: Vec<String> = vec![
            self.path.display().to_string(),
            "--no-gitignore".to_string(),
        ];
        args.extend(extra.iter().map(|item| item.to_string()));
        let (output, diagnostics, status) =
            cli::run_checked(&args, &self.path).expect("cli::run_checked failed");
        (format!("{}{}", output, diagnostics), status)
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

const SMELLY: &str = "#[allow(dead_code)]\nfn a(x: Option<u32>) -> u32 {\n    if x.is_none() {\n        panic!(\"boom\");\n    }\n    let y: Result<u32, ()> = Ok(1);\n    y.expect(\"bad\") + x.unwrap()\n}\n";

#[test]
fn smells_count_panic_shaped_calls_and_lint_escapes() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", SMELLY);
    let summary = workspace.summary(&["--smells"]);
    let smells = summary.get("smells").unwrap();
    assert_eq!(smells.get("unwrap").unwrap().integer(), 1);
    assert_eq!(smells.get("expect").unwrap().integer(), 1);
    assert_eq!(smells.get("panic").unwrap().integer(), 1);
    assert_eq!(smells.get("allow").unwrap().integer(), 1);
    assert_eq!(smells.get("todo").unwrap().integer(), 0);
    assert_eq!(smells.get("total").unwrap().integer(), 4);
    let file = workspace.find(&workspace.document(&["--smells"]), "src/lib.rs");
    let file_smells = file.get("smells").unwrap();
    assert_eq!(file_smells.get("unwrap").unwrap().integer(), 1);
    assert_eq!(file_smells.get("total").unwrap().integer(), 4);
}

#[test]
fn smells_stay_out_of_the_default_report() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", SMELLY);
    assert!(workspace.summary(&[]).get("smells").is_none());
    let text = workspace.run(&[]);
    assert!(!text.contains("Rust smells"), "{}", text);
    let shown = workspace.run(&["--smells"]);
    assert!(shown.contains("Rust smells"), "{}", shown);
    assert!(shown.contains("unimplemented!()"), "{}", shown);
    assert!(shown.contains("#[allow(..)]"), "{}", shown);
    let markdown = workspace.run(&["--format", "markdown", "--smells"]);
    assert!(markdown.contains("| unwrap() | 1 |"), "{}", markdown);
    assert!(markdown.contains("| Smells (total) | 4 |"), "{}", markdown);
}

#[test]
fn smells_add_csv_columns() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", SMELLY);
    let out = workspace.run(&["--format", "csv", "--smells"]);
    let header = out.lines().next().unwrap();
    assert!(
        header.ends_with("unwrap,expect,panic,todo,unimplemented,allow,smells"),
        "{}",
        header
    );
    let total = out.lines().nth(1).unwrap();
    assert!(total.ends_with(",1,1,1,0,0,1,4"), "{}", total);
}

#[test]
fn smells_can_gate_the_build_without_the_report_switch() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", SMELLY);
    let (out, status) = workspace.checked(&["--fail-if", "unwrap>0"]);
    assert_eq!(status, 1);
    assert!(out.contains("unwrap 1 > 0"), "{}", out);
    assert_eq!(workspace.checked(&["--fail-if", "smells==4"]).1, 1);
    assert_eq!(workspace.checked(&["--fail-if", "unwrap<1"]).1, 0);
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

const REPO_CONFIG: &str = concat!(
    "# team defaults\n",
    "[scope]\n",
    "exclude = [\"*/skip/*\"]\n",
    "[report]\n",
    "format = \"json\"\n",
    "smells = true\n",
    "[gate]\n",
    "fail_if = [\"unwrap > 0\"]\n"
);

#[test]
fn a_repo_config_supplies_the_defaults() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", SMELLY);
    workspace.write("src/skip/me.rs", "fn a() {}\n");
    workspace.write(".rustloc.toml", REPO_CONFIG);

    let invocation = workspace.invocation(&[]);
    assert_eq!(invocation.options.format, Format::Json);
    assert!(invocation.options.smells);
    assert_eq!(invocation.options.exclude, ["*/skip/*"]);
    assert_eq!(invocation.options.fail_if.len(), 1);
    assert!(invocation.options.config.is_some());

    let document = json::parse(&workspace.run(&[]));
    assert_eq!(
        document
            .get("summary")
            .unwrap()
            .get("files")
            .unwrap()
            .integer(),
        1
    );
    assert!(document
        .get("config")
        .unwrap()
        .text()
        .ends_with(".rustloc.toml"));

    let (out, status) = workspace.checked(&[]);
    assert_eq!(status, 1, "{}", out);
    assert!(out.contains("unwrap 1 > 0"), "{}", out);
}

#[test]
fn the_command_line_still_wins_over_the_repo_config() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", "fn a() {}\n");
    workspace.write(
        ".rustloc.toml",
        "[scope]\nexclude = [\"*/generated/*\"]\n[report]\nformat = \"csv\"\ntop = 3\n",
    );
    let invocation = workspace.invocation(&[
        "--format",
        "markdown",
        "--no-complexity",
        "--top",
        "9",
        "--exclude",
        "*/extra/*",
    ]);
    assert_eq!(invocation.options.format, Format::Markdown);
    assert!(!invocation.options.complexity);
    assert_eq!(invocation.options.top, 9);
    assert_eq!(
        invocation.options.exclude,
        ["*/generated/*", "*/extra/*"],
        "the config excludes come first, the command line adds to them"
    );

    let bare = workspace.invocation(&["--no-config"]);
    assert!(bare.options.config.is_none());
    assert_eq!(bare.options.format, Format::Text);
    assert!(bare.options.exclude.is_empty());
    assert_eq!(bare.options.top, 0);
}

#[test]
fn config_roots_are_used_when_none_are_given() {
    let workspace = Workspace::new();
    workspace.write("alpha/a.rs", "fn a() {}\n");
    workspace.write("beta/b.rs", "fn b() {}\n");
    workspace.write(".rustloc.toml", "[scope]\nroots = [\"alpha\"]\n");

    let args: Vec<String> = vec!["--no-gitignore".to_string()];
    let invocation = cli::parse(&args, &workspace.path).unwrap();
    assert_eq!(invocation.roots.len(), 1);
    assert!(
        invocation.roots[0].ends_with("alpha"),
        "{:?}",
        invocation.roots
    );
    let args: Vec<String> = vec![
        "--no-gitignore".to_string(),
        "--format".to_string(),
        "json".to_string(),
    ];
    let document = json::parse(&cli::run(&args, &workspace.path).unwrap());
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
fn the_config_is_found_beside_the_scan_root() {
    let workspace = Workspace::new();
    workspace.write("proj/src/lib.rs", "fn a() {}\n");
    workspace.write("proj/.rustloc.toml", "[report]\nformat = \"json\"\n");
    let mut args: Vec<String> = vec![
        workspace.path.join("proj").display().to_string(),
        "--no-gitignore".to_string(),
    ];
    let document = json::parse(&cli::run(&args, &workspace.path).unwrap());
    assert_eq!(
        document
            .get("summary")
            .unwrap()
            .get("files")
            .unwrap()
            .integer(),
        1
    );
    assert!(document
        .get("config")
        .unwrap()
        .text()
        .ends_with(".rustloc.toml"));

    args.push("--no-config".to_string());
    assert!(cli::run(&args, &workspace.path)
        .unwrap()
        .starts_with("Rust business code"));
}

#[test]
fn an_explicit_config_path_must_exist() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", "fn a() {}\n");
    let args: Vec<String> = vec![
        workspace.path.display().to_string(),
        "--config".to_string(),
        "nope.toml".to_string(),
    ];
    let error = cli::parse(&args, &workspace.path).unwrap_err();
    assert!(error.contains("config file not found"), "{}", error);
}

#[test]
fn a_broken_config_stops_the_run_and_names_the_line() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", "fn a() {}\n");
    workspace.write(".rustloc.toml", "[scope]\nnope = 1\n");
    let args: Vec<String> = vec![workspace.path.display().to_string()];
    let error = cli::parse(&args, &workspace.path).unwrap_err();
    assert!(error.contains(".rustloc.toml:2"), "{}", error);
    assert!(error.contains("unknown key"), "{}", error);
}

#[test]
fn a_snapshot_records_the_business_files() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BASIC);
    let out = workspace.run(&["--snapshot", "base.txt"]);
    assert!(out.contains("Snapshot written: base.txt"), "{}", out);
    let text = std::fs::read_to_string(workspace.path.join("base.txt")).unwrap();
    assert!(text.starts_with("# rust-loc snapshot v1"), "{}", text);
    assert!(text.contains("src/lib.rs"), "{}", text);
}

#[test]
fn a_baseline_reports_what_changed() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BASIC);
    workspace.write("src/gone.rs", "fn gone() {}\n");
    workspace.run(&["--snapshot", "base.txt"]);
    workspace.write("src/lib.rs", &format!("{}{}", BASIC, "fn extra() {}\n"));
    std::fs::remove_file(workspace.path.join("src/gone.rs")).unwrap();
    workspace.write("src/new.rs", "fn new() {}\n");

    let text = workspace.run(&["--baseline", "base.txt"]);
    assert!(
        text.contains("Since baseline base.txt (1 added, 1 deleted, 1 changed)"),
        "{}",
        text
    );
    assert!(text.contains("Largest moves"), "{}", text);
    assert!(text.contains("+1  src/new.rs"), "{}", text);
    assert!(text.contains("-1  src/gone.rs"), "{}", text);

    let document = workspace.document(&["--baseline", "base.txt"]);
    let baseline = document.get("baseline").unwrap();
    assert_eq!(baseline.get("added").unwrap().integer(), 1);
    assert_eq!(baseline.get("deleted").unwrap().integer(), 1);
    assert_eq!(baseline.get("changed").unwrap().integer(), 1);
    assert_eq!(baseline.get("path").unwrap().text(), "base.txt");
    assert_eq!(
        baseline.get("totals").unwrap().get("code").unwrap().items()[1].integer(),
        document
            .get("summary")
            .unwrap()
            .get("code")
            .unwrap()
            .integer()
    );

    let csv = workspace.run(&["--baseline", "base.txt", "--by-file", "--format", "csv"]);
    assert!(csv.contains("code_delta"), "{}", csv);
    assert!(csv.lines().any(|line| line.ends_with(",1")), "{}", csv);
}

#[test]
fn a_broken_baseline_stops_the_run() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BASIC);
    workspace.write("base.txt", "not a snapshot\n");
    let args: Vec<String> = vec![
        workspace.path.display().to_string(),
        "--no-gitignore".to_string(),
        "--baseline".to_string(),
        "base.txt".to_string(),
    ];
    let error = cli::parse(&args, &workspace.path).unwrap_err();
    assert!(error.contains("base.txt:1"), "{}", error);
    assert!(
        error.contains("missing the # rust-loc snapshot v1 header"),
        "{}",
        error
    );
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

const BRANCHY: &str = "fn plain() {}\nfn branchy(a: u32, b: u32) -> u32 {\n    if a == 1 {\n        a\n    } else if b != 2 {\n        b\n    } else {\n        0\n    }\n}\n";

const GENERATED: &str = "// Code generated by tool. DO NOT EDIT.\nfn a() {\n    let x = 1;\n}\n";

#[test]
fn functions_flag_lists_the_worst_first() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BRANCHY);
    let text = workspace.run(&["--functions"]);
    let start = text.find("Functions").expect("no Functions section");
    let section = &text[start..];
    assert!(section.contains("2 function(s) in scope"), "{}", section);
    let big = section.find("branchy").expect("branchy missing");
    let small = section.find("plain").expect("plain missing");
    assert!(big < small, "{}", section);
}

#[test]
fn functions_stay_hidden_without_the_flag() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BRANCHY);
    assert!(!workspace.run(&[]).contains("Functions"));
    assert!(!workspace.document(&[]).get("functions").is_some());
}

#[test]
fn functions_json_reports_the_metrics() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BRANCHY);
    let document = workspace.document(&["--functions"]);
    assert_eq!(document.get("functions_total").unwrap().integer(), 2);
    let functions = document.get("functions").unwrap().items();
    assert_eq!(functions.len(), 2);
    let top = &functions[0];
    assert_eq!(top.get("name").unwrap().text(), "branchy");
    assert_eq!(top.get("line").unwrap().integer(), 2);
    assert_eq!(top.get("params").unwrap().integer(), 2);
    assert_eq!(top.get("complexity").unwrap().integer(), 6);
    assert_eq!(top.get("depth").unwrap().integer(), 2);
    assert_eq!(top.get("code").unwrap().integer(), 9);
    assert!(top.get("path").unwrap().text().ends_with("lib.rs"));
}

#[test]
fn top_functions_limits_the_rows_but_keeps_the_total() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", "fn a() {}\nfn b() {}\nfn c() {}\n");
    let document = workspace.document(&["--top-functions", "2"]);
    assert_eq!(document.get("functions_total").unwrap().integer(), 3);
    assert_eq!(document.get("functions").unwrap().items().len(), 2);
}

#[test]
fn disabled_test_functions_are_left_out_of_the_ranking() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", "fn business() {}\n#[test]\nfn covers() {}\n");
    let document = workspace.document(&["--functions"]);
    let functions = document.get("functions").unwrap().items();
    assert_eq!(functions.len(), 1);
    assert_eq!(functions[0].get("name").unwrap().text(), "business");
    let with_tests = workspace.document(&["--functions", "--include-tests"]);
    assert_eq!(with_tests.get("functions").unwrap().items().len(), 2);
}

#[test]
fn exclude_generated_drops_marker_files_from_the_totals() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BASIC);
    workspace.write("src/bindings.rs", GENERATED);
    assert_eq!(workspace.summary(&[]).get("code").unwrap().integer(), 6);
    assert_eq!(
        workspace
            .summary(&["--exclude-generated"])
            .get("code")
            .unwrap()
            .integer(),
        3
    );
    let text = workspace.run(&["--exclude-generated"]);
    assert!(text.contains("Excluded generated code"), "{}", text);
    assert!(text.contains("1 file(s)"), "{}", text);
    let document = workspace.document(&["--exclude-generated"]);
    assert_eq!(
        document
            .get("generated_excluded")
            .unwrap()
            .get("files")
            .unwrap()
            .integer(),
        1
    );
}

#[test]
fn mark_generated_tags_listings_instead_of_dropping() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BASIC);
    workspace.write("src/bindings.rs", GENERATED);
    let text = workspace.run(&["--mark-generated", "--by-file"]);
    assert!(text.contains("bindings.rs (gen)"), "{}", text);
    assert!(!text.contains("lib.rs (gen)"), "{}", text);
    let document = workspace.document(&["--mark-generated"]);
    assert_eq!(
        workspace
            .find(&document, "bindings.rs")
            .get("generated")
            .unwrap()
            .text(),
        "gen"
    );
    assert_eq!(
        workspace
            .summary(&["--mark-generated"])
            .get("code")
            .unwrap()
            .integer(),
        6
    );
}

#[test]
fn minified_files_are_detected_and_the_threshold_is_configurable() {
    let workspace = Workspace::new();
    let long = format!("fn a() {{ let s = \"{}\"; }}\n", "x".repeat(400));
    workspace.write("src/bundle.rs", &long);
    let text = workspace.run(&["--mark-generated", "--by-file"]);
    assert!(text.contains("bundle.rs (min)"), "{}", text);
    let raised = workspace.run(&[
        "--mark-generated",
        "--min-gen-line-length",
        "1000",
        "--by-file",
    ]);
    assert!(!raised.contains("(min)"), "{}", raised);
}

#[test]
fn cocomo_estimate_appears_only_when_asked_for() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BASIC);
    assert!(!workspace.run(&[]).contains("COCOMO"));
    let text = workspace.run(&["--cocomo"]);
    assert!(text.contains("COCOMO estimate (organic)"), "{}", text);
    assert!(text.contains("Cost to develop"), "{}", text);
}

#[test]
fn cocomo_project_type_changes_the_estimate() {
    let workspace = Workspace::new();
    let mut source = String::new();
    for index in 0..2000 {
        source.push_str(&format!("fn f{}() {{}}\n", index));
    }
    workspace.write("src/big.rs", &source);
    let organic = workspace.document(&["--cocomo"]);
    let embedded = workspace.document(&["--cocomo-project-type", "embedded"]);
    let cost = |document: &json::Value| {
        document
            .get("cocomo")
            .unwrap()
            .get("cost_usd")
            .unwrap()
            .number()
    };
    let project = |document: &json::Value| {
        document
            .get("cocomo")
            .unwrap()
            .get("project")
            .unwrap()
            .text()
            .to_string()
    };
    assert_eq!(project(&organic), "organic");
    assert_eq!(project(&embedded), "embedded");
    assert!(cost(&organic) > 1000.0);
    assert!(cost(&embedded) > cost(&organic));
}

#[test]
fn custom_cocomo_params_drive_the_formula() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BASIC);
    let document = workspace.document(&["--cocomo-params", "1,1,1,1"]);
    let cocomo = document.get("cocomo").unwrap();
    assert_eq!(cocomo.get("project").unwrap().text(), "custom");
    assert_eq!(cocomo.get("people").unwrap().number(), 1.0);
    assert_eq!(cocomo.get("cost_usd").unwrap().number(), 7.4);
    assert!(workspace
        .run(&["--cocomo-params", "1,1,1,1"])
        .contains("$7"));
}

#[test]
fn bad_cocomo_arguments_are_rejected() {
    let workspace = Workspace::new();
    let arguments = |extra: &[&str]| {
        let mut args = vec![workspace.path.display().to_string()];
        args.extend(extra.iter().map(|item| item.to_string()));
        args
    };
    assert!(cli::parse(
        &arguments(&["--cocomo-project-type", "nope"]),
        &workspace.path
    )
    .is_err());
    assert!(cli::parse(&arguments(&["--cocomo-params", "1,2,3"]), &workspace.path).is_err());
}

#[test]
fn fail_if_reports_violations_and_the_exit_status() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BASIC);
    let (text, status) = workspace.checked(&["--fail-if", "code>1"]);
    assert_eq!(status, 1);
    assert!(text.contains("Quality gate failed"), "{}", text);
    assert!(text.contains("code 3 > 1"), "{}", text);
    assert_eq!(workspace.checked(&["--fail-if", "code>100"]).1, 0);
    assert_eq!(
        workspace
            .checked(&["--fail-if", "code>100"])
            .0
            .matches("Quality gate")
            .count(),
        0
    );
    assert_eq!(workspace.checked(&["--fail-if=code>=3"]).1, 1);
    assert_eq!(workspace.checked(&["--fail-if", "code<1"]).1, 0);
    assert_eq!(workspace.checked(&["--fail-if", "complexity==0"]).1, 1);
    assert_eq!(workspace.checked(&["--fail-if", "files>1"]).1, 0);
}

#[test]
fn ci_mode_prints_github_annotations() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BASIC);
    let (text, status) = workspace.checked(&["--fail-if", "code>1", "--ci"]);
    assert_eq!(status, 1);
    assert!(
        text.contains("::error title=rust-loc gate::code 3 > 1"),
        "{}",
        text
    );
    assert!(!text.contains("Quality gate failed"), "{}", text);
}

#[test]
fn gate_diagnostics_stay_out_of_stdout() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BASIC);
    let args: Vec<String> = vec![
        workspace.path.display().to_string(),
        "--no-gitignore".to_string(),
        "--format".to_string(),
        "json".to_string(),
        "--fail-if".to_string(),
        "code>1".to_string(),
    ];
    let (output, diagnostics, status) =
        cli::run_checked(&args, &workspace.path).expect("cli::run_checked failed");
    assert_eq!(status, 1);
    let document = json::parse(&output);
    assert_eq!(
        document
            .get("summary")
            .unwrap()
            .get("code")
            .unwrap()
            .integer(),
        3
    );
    assert!(
        diagnostics.contains("Quality gate failed"),
        "{}",
        diagnostics
    );
    assert!(!output.contains("Quality gate"), "{}", output);
    assert!(diagnostics.contains("code 3 > 1"), "{}", diagnostics);
}

#[test]
fn fail_if_rejects_bad_specs_and_needs_uloc() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BASIC);
    let arguments = |extra: &[&str]| {
        let mut args = vec![workspace.path.display().to_string()];
        args.extend(extra.iter().map(|item| item.to_string()));
        args
    };
    assert!(cli::run(&arguments(&["--fail-if", "nope"]), &workspace.path).is_err());
    let error = cli::run(&arguments(&["--fail-if", "uloc>1"]), &workspace.path).unwrap_err();
    assert!(error.contains("--uloc"), "{}", error);
    assert_eq!(workspace.checked(&["--uloc", "--fail-if", "uloc>1"]).1, 1);
}
