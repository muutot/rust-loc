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
use rust_loc::scan::Stage;

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
fn config_can_pick_the_source_languages() {
    let workspace = Workspace::new();
    workspace.write("tool/app.py", PY_SOURCE);
    workspace.write(".rustloc.toml", "[scope]\nsources = \"python\"\n");

    let args: Vec<String> = vec![
        "--no-gitignore".to_string(),
        "--format".to_string(),
        "json".to_string(),
    ];
    let document = json::parse(&cli::run(&args, &workspace.path).unwrap());
    assert_eq!(
        document
            .get("scope")
            .unwrap()
            .get("sources")
            .unwrap()
            .text(),
        "python"
    );
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
fn an_unknown_source_selection_in_the_config_is_an_error() {
    let workspace = Workspace::new();
    workspace.write(".rustloc.toml", "[scope]\nsources = \"cobol\"\n");
    let args: Vec<String> = vec!["--no-gitignore".to_string()];
    let error = cli::parse(&args, &workspace.path).unwrap_err();
    assert!(error.contains("unknown source selection"), "{}", error);
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
fn history_records_a_sample_and_charts_it() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BASIC);
    let first = workspace.run(&["--history", "history.tsv"]);
    assert!(first.contains("History written: history.tsv"), "{}", first);
    assert!(first.contains("Scan history - 1 scan"), "{}", first);

    let second = workspace.run(&["--history", "history.tsv"]);
    assert!(second.contains("Scan history - 2 scans"), "{}", second);
    assert!(second.contains("code"), "{}", second);

    let text = std::fs::read_to_string(workspace.path.join("history.tsv")).unwrap();
    assert!(text.starts_with("# rust-loc history v1\n"), "{}", text);
    assert_eq!(text.lines().count(), 1 + 1 + 2, "{}", text);
}

#[test]
fn a_broken_history_stops_the_run() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BASIC);
    workspace.write("history.tsv", "not a history\n");
    let args: Vec<String> = vec![
        workspace.path.display().to_string(),
        "--no-gitignore".to_string(),
        "--history".to_string(),
        "history.tsv".to_string(),
    ];
    let error = cli::run(&args, &workspace.path).unwrap_err();
    assert!(error.contains("history.tsv:1"), "{}", error);
    assert!(
        error.contains("missing the # rust-loc history v1 header"),
        "{}",
        error
    );
}

#[test]
fn history_json_carries_the_series() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BASIC);
    workspace.document(&["--history", "history.tsv"]);
    let raw = workspace.run(&["--format", "json", "--history", "history.tsv"]);
    assert!(raw.contains("\"sparkline\""), "{}", raw);
    assert!(raw.contains("\u{2584}\u{2584}"), "{}", raw);

    let document = json::parse(&raw);
    let history = document.get("history").unwrap();
    assert_eq!(history.get("path").unwrap().text(), "history.tsv");
    assert_eq!(history.get("samples").unwrap().integer(), 2);
    let code = history.get("series").unwrap().get("code").unwrap();
    assert_eq!(code.items().len(), 2);
    assert_eq!(code.items()[0].integer(), 3);
}

#[test]
fn history_stays_out_of_the_report_without_the_switch() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BASIC);
    let out = workspace.run(&[]);
    assert!(!out.contains("Scan history"), "{}", out);
    assert!(!out.contains("history.tsv"), "{}", out);
}

#[test]
fn help_flag_documents_every_switch() {
    let workspace = Workspace::new();
    let out = workspace.run(&["--help"]);
    for flag in [
        "--by-file",
        "--by-module",
        "--uloc",
        "--jobs",
        "--exclude",
        "--sources",
    ] {
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
    let callback = |stage: Stage, done: usize, total: usize, _path: &std::path::Path| {
        seen.lock().unwrap().push((stage, done, total));
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
    seen.sort_by_key(|entry| entry.1);
    // A scan without `--authors` has only the read stage to report, and it
    // must not count it out of a total it never reaches.
    assert_eq!(seen, vec![(Stage::Files, 1, 2), (Stage::Files, 2, 2)]);
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
        "--hotspots",
    ]);
    let options = invocation.options;
    assert!(options.hotspots);
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
fn ci_mode_points_at_the_worst_file_and_function() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BASIC);
    let (text, status) = workspace.checked(&["--fail-if", "code>1", "--ci"]);
    assert_eq!(status, 1);
    assert!(
        text.contains("::error,file=src/lib.rs,line=3,title=rust-loc gate::code 3 > 1"),
        "{}",
        text
    );
    assert!(!text.contains("Quality gate failed"), "{}", text);
}

#[test]
fn a_file_scoped_gate_annotates_the_path_alone() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BASIC);
    let (text, status) = workspace.checked(&["--fail-if", "blank>0", "--ci"]);
    assert_eq!(status, 1);
    assert!(
        text.contains("::error,file=src/lib.rs,title=rust-loc gate::blank 1 > 0"),
        "{}",
        text
    );
    assert!(!text.contains("line="), "{}", text);
}

#[test]
fn a_plain_gate_names_the_worst_file() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BASIC);
    let (text, status) = workspace.checked(&["--fail-if", "code>1"]);
    assert_eq!(status, 1);
    assert!(text.contains("worst: src/lib.rs:3"), "{}", text);
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

/// True when a git subcommand succeeds. History tests skip when git is absent
/// so the suite still passes on a machine without it.
fn git_ok(dir: &std::path::Path, args: &[&str]) -> bool {
    std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[test]
fn hotspots_stay_hidden_unless_asked_for() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BASIC);
    let out = workspace.run(&[]);
    assert!(!out.contains("Git hotspots"), "{}", out);
}

#[test]
fn hotspots_without_a_repository_print_an_empty_section() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BASIC);
    let out = workspace.run(&["--hotspots"]);
    assert!(out.contains("Git hotspots"), "{}", out);
    assert!(out.contains("no git history"), "{}", out);
}

#[test]
fn authors_stay_hidden_unless_asked_for() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BASIC);
    let out = workspace.run(&[]);
    assert!(!out.contains("Authors"), "{}", out);
}

#[test]
fn authors_without_a_repository_say_so() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BASIC);
    if git_ok(&workspace.path, &["rev-parse", "--show-toplevel"]) {
        return; // the temp tree happens to live inside a repository
    }
    let out = workspace.run(&["--authors"]);
    assert!(out.contains("Authors"), "{}", out);
    assert!(out.contains("not a git repository"), "{}", out);
}

/// Commit the tree as `name`, so blame has somebody to credit.
fn commit_as(workspace: &Workspace, name: &str, email: &str, message: &str) -> bool {
    git_ok(&workspace.path, &["add", "-A"]);
    let mail = format!("user.email={}", email);
    let who = format!("user.name={}", name);
    git_ok(
        &workspace.path,
        &["-c", &mail, "-c", &who, "commit", "-q", "-m", message],
    )
}

#[test]
fn authors_split_the_business_lines_of_a_repository() {
    let workspace = Workspace::new();
    workspace.write("src/first.rs", "fn a() {}\n// one\n\nfn b() {}\n");
    if !git_ok(&workspace.path, &["init", "-q"]) {
        return; // git is not installed; nothing to assert
    }
    assert!(commit_as(&workspace, "Ada", "ada@example.com", "first"));
    workspace.write("src/second.rs", "fn c() {}\n");
    assert!(commit_as(&workspace, "Bob", "bob@example.com", "second"));

    let document = workspace.document(&["--authors"]);
    assert!(document.get("authors_available").unwrap().flag());
    assert_eq!(document.get("authors_untracked").unwrap().integer(), 0);
    let rows = document.get("authors").unwrap().items();
    let find = |name: &str| {
        rows.iter()
            .find(|row| row.get("name").unwrap().text() == name)
            .unwrap_or_else(|| panic!("no author {}", name))
            .clone()
    };
    let ada = find("Ada");
    assert_eq!(ada.get("code").unwrap().integer(), 2);
    assert_eq!(ada.get("comment").unwrap().integer(), 1);
    assert_eq!(ada.get("blank").unwrap().integer(), 1);
    assert_eq!(ada.get("lines").unwrap().integer(), 4);
    assert_eq!(ada.get("files").unwrap().integer(), 1);
    let bob = find("Bob");
    assert_eq!(bob.get("code").unwrap().integer(), 1);
    assert_eq!(bob.get("lines").unwrap().integer(), 1);
    // Every business line is credited to somebody, so the halves add up.
    let blamed = ada.get("lines").unwrap().integer() + bob.get("lines").unwrap().integer();
    assert_eq!(
        blamed,
        document
            .get("summary")
            .unwrap()
            .get("lines")
            .unwrap()
            .integer()
    );
}

#[test]
fn blame_reports_the_stage_that_outlasts_the_read() {
    let workspace = Workspace::new();
    workspace.write("src/first.rs", "fn a() {}\n");
    workspace.write("src/second.rs", "fn b() {}\n");
    if !git_ok(&workspace.path, &["init", "-q"]) {
        return; // git is not installed; nothing to assert
    }
    assert!(commit_as(&workspace, "Ada", "ada@example.com", "first"));

    let seen = std::sync::Mutex::new(Vec::new());
    let callback = |stage: Stage, done: usize, total: usize, path: &std::path::Path| {
        seen.lock()
            .unwrap()
            .push((stage, done, total, path.display().to_string()));
    };
    let invocation = workspace.invocation(&["--no-gitignore", "--authors"]);
    report::scan(
        &invocation.roots,
        &invocation.options,
        &workspace.path,
        Some(&callback),
    );

    // The read fills its bar first, then blaming the same files takes over.
    // Both have to reach the UI, or the bar sits at 100% while the slow half
    // of the scan is still running.
    let seen = seen.into_inner().unwrap();
    let of = |stage: Stage| -> Vec<(usize, usize, String)> {
        seen.iter()
            .filter(|entry| entry.0 == stage)
            .map(|entry| (entry.1, entry.2, entry.3.clone()))
            .collect()
    };
    assert_eq!(
        of(Stage::Files).len(),
        2,
        "the read reports every file: {seen:?}"
    );
    // The caption names the file from the scan root, not from the drive.
    let mut blamed = of(Stage::Blame);
    blamed.sort();
    assert_eq!(
        blamed,
        vec![
            (1, 2, "src/first.rs".to_string()),
            (2, 2, "src/second.rs".to_string()),
        ]
    );
    assert!(of(Stage::Files)
        .iter()
        .all(|entry| entry.2.starts_with("src/")));
}

#[test]
fn blame_follows_the_file_rather_than_the_shell() {
    let workspace = Workspace::new();
    workspace.write("src/first.rs", "fn a() {}\n");
    if !git_ok(&workspace.path, &["init", "-q"]) {
        return; // git is not installed; nothing to assert
    }
    assert!(commit_as(&workspace, "Ada", "ada@example.com", "first"));

    // Asked from a directory that is not the repository at all, the blame still
    // finds it from the file, which is how the window asks for it.
    let elsewhere = std::env::temp_dir();
    if git_ok(&elsewhere, &["rev-parse", "--show-toplevel"]) {
        return; // the temp tree happens to live inside a repository
    }
    let args = vec![
        workspace.path.display().to_string(),
        "--no-gitignore".to_string(),
        "--authors".to_string(),
        "--format".to_string(),
        "json".to_string(),
    ];
    let out = cli::run(&args, &elsewhere).expect("cli::run failed");
    let document = json::parse(&out);
    assert!(document.get("authors_available").unwrap().flag());
    let rows = document.get("authors").unwrap().items();
    let names: Vec<String> = rows
        .iter()
        .map(|row| row.get("name").unwrap().text().to_string())
        .collect();
    assert!(names.iter().any(|name| name == "Ada"), "{:?}", names);
}

#[test]
fn a_file_lists_every_author_that_touched_it() {
    let workspace = Workspace::new();
    workspace.write("src/shared.rs", "fn a() {}\n");
    if !git_ok(&workspace.path, &["init", "-q"]) {
        return;
    }
    assert!(commit_as(&workspace, "Ada", "ada@example.com", "first"));
    workspace.write("src/shared.rs", "fn a() {}\nfn b() {}\n");
    assert!(commit_as(&workspace, "Bob", "bob@example.com", "second"));

    let document = workspace.document(&["--authors", "--by-file"]);
    let files = document.get("file_authors").unwrap().items();
    let entry = files
        .iter()
        .find(|file| file.get("path").unwrap().text() == "src/shared.rs")
        .expect("shared.rs is missing from the per-file report")
        .clone();
    let names: Vec<String> = entry
        .get("authors")
        .unwrap()
        .items()
        .iter()
        .map(|share| share.get("name").unwrap().text().to_string())
        .collect();
    assert_eq!(names.len(), 2, "{:?}", names);
    assert!(names.iter().any(|n| n == "Ada"), "{:?}", names);
    assert!(names.iter().any(|n| n == "Bob"), "{:?}", names);
    let text = workspace.run(&["--authors", "--by-file"]);
    assert!(text.contains("Per-file authors"), "{}", text);
    assert!(text.contains("src/shared.rs"), "{}", text);
}

#[test]
fn hotspots_rank_files_by_complexity_and_churn() {
    let workspace = Workspace::new();
    workspace.write("src/busy.rs", &"fn busy() {}\n".repeat(40));
    workspace.write("src/idle.rs", &"fn idle() {}\n".repeat(40));
    if !git_ok(&workspace.path, &["init", "-q"]) {
        return; // git is not installed; nothing to assert
    }
    let commit = |message: &str| {
        git_ok(&workspace.path, &["add", "-A"]);
        git_ok(
            &workspace.path,
            &[
                "-c",
                "user.email=test@example.com",
                "-c",
                "user.name=Test",
                "commit",
                "-q",
                "-m",
                message,
            ],
        );
    };
    commit("first");
    for round in 1..=2 {
        let mut text = "fn busy() {}\n".repeat(40);
        for line in 0..10 * round {
            text.push_str(&format!("// touch {line}\n"));
        }
        workspace.write("src/busy.rs", &text);
        commit(&format!("touch {round}"));
    }
    let out = workspace.run(&["--hotspots", "--format", "json", "--top", "10"]);
    let document = json::parse(&out);
    let rows = document.get("hotspots").unwrap().items();
    assert_eq!(
        document.get("hotspot_total").unwrap().integer(),
        rows.len() as u64
    );
    let busy = rows
        .iter()
        .find(|row| row.get("path").unwrap().text() == "src/busy.rs")
        .expect("busy.rs should be ranked");
    assert_eq!(busy.get("commits").unwrap().integer(), 3);
    assert!(busy.get("churn").unwrap().integer() >= 60);
    assert_eq!(
        rows[0].get("path").unwrap().text(),
        "src/busy.rs",
        "the most churned file should lead"
    );
}

#[test]
fn metrics_stay_hidden_unless_asked_for() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BASIC);
    let out = workspace.run(&[]);
    assert!(!out.contains("Maintainability"), "{}", out);
    let options = workspace.invocation(&["--metrics"]).options;
    assert!(options.metrics);
    assert!(options.column_keys().contains(&"cognitive"));
    assert!(options.column_titles().contains(&"Cog"));
}

#[test]
fn metrics_add_the_maintainability_card() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", SMELLY);
    let out = workspace.run(&["--metrics"]);
    for label in [
        "Maintainability",
        "MI score",
        "Halstead volume",
        "Tokens",
        "Cognitive complexity",
        "Methods / arguments",
    ] {
        assert!(out.contains(label), "{} missing from:\n{}", label, out);
    }
}

#[test]
fn metrics_json_reports_the_index_and_the_halstead_counts() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", SMELLY);
    let out = workspace.run(&["--metrics", "--format", "json"]);
    let document = json::parse(&out);
    let row = document.get("maintainability").unwrap();
    assert!(row.get("mi").unwrap().number() > 0.0);
    assert!(!row.get("band").unwrap().text().is_empty());
    assert!(row.get("halstead_volume").unwrap().number() > 0.0);
    assert!(row.get("operators").unwrap().integer() > 0);
    assert!(row.get("operands").unwrap().integer() > 0);
    assert!(row.get("cognitive").unwrap().integer() > 0);
    assert_eq!(row.get("methods").unwrap().integer(), 1);
    assert_eq!(row.get("arguments").unwrap().integer(), 1);
    assert_eq!(
        row.get("sloc").unwrap().integer(),
        document
            .get("summary")
            .unwrap()
            .get("code")
            .unwrap()
            .integer()
    );
}

#[test]
fn functions_gain_a_cognitive_column_with_metrics() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BRANCHY);
    let shown = workspace.run(&["--functions", "--metrics"]);
    assert!(shown.contains("Cog"), "{}", shown);
    let plain = workspace.run(&["--functions"]);
    assert!(!plain.contains("Cog"), "{}", plain);
    let out = workspace.run(&["--functions", "--metrics", "--format", "json"]);
    let document = json::parse(&out);
    let functions = document.get("functions").unwrap().items();
    let branchy = functions
        .iter()
        .find(|row| row.get("name").unwrap().text() == "branchy")
        .expect("branchy should be listed");
    assert_eq!(branchy.get("cognitive").unwrap().integer(), 3);
}

#[test]
fn metrics_do_not_disturb_the_default_columns() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BASIC);
    let options = workspace.invocation(&[]).options;
    assert!(!options.column_keys().contains(&"cognitive"));
    assert!(options.column_keys().contains(&"complexity"));
}

#[test]
fn languages_stay_hidden_unless_asked_for() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BASIC);
    let hidden = workspace.run(&[]);
    assert!(!hidden.contains("Languages"), "{}", hidden);
    assert!(workspace.invocation(&["--languages"]).options.languages);
}

#[test]
fn languages_count_a_mixed_tree() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", "fn a() {}\n");
    workspace.write("web/app.ts", "// note\nexport const x = 1;\n");
    workspace.write("README.md", "# Title\n\nText.\n");
    workspace.write("assets/logo.png", "not really a png\n");
    let out = workspace.run(&["--languages"]);
    for name in ["Languages", "Rust", "TypeScript", "Markdown"] {
        assert!(out.contains(name), "{} missing from:\n{}", name, out);
    }
    assert!(!out.contains("png"), "{}", out);
}

#[test]
fn languages_json_reports_files_and_line_classes() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", "fn a() {}\n");
    workspace.write("web/app.ts", "// note\nexport const x = 1;\n");
    let out = workspace.run(&["--languages", "--format", "json"]);
    let document = json::parse(&out);
    let rows = document.get("languages").unwrap().items();
    let typescript = rows
        .iter()
        .find(|row| row.get("name").unwrap().text() == "TypeScript")
        .expect("TypeScript should be listed");
    assert_eq!(typescript.get("files").unwrap().integer(), 1);
    assert_eq!(typescript.get("lines").unwrap().integer(), 2);
    assert_eq!(typescript.get("comment").unwrap().integer(), 1);
    assert_eq!(typescript.get("code").unwrap().integer(), 1);
    assert!(document.get("language_code").unwrap().integer() >= 2);
}

/// Two files whose bodies are the same six lines, so the clone detector has
/// exactly one hit to report.
const CLONE_RIGHT: &str = "pub fn alpha(x: u32) -> u32 {\n    let mut total = x;\n    total += 1;\n    total *= 2;\n    total -= 3;\n    total /= 4;\n    total\n}\n";
const CLONE_LEFT: &str = "pub fn beta(x: u32) -> u32 {\n    let mut total = x;\n    total += 1;\n    total *= 2;\n    total -= 3;\n    total /= 4;\n    total\n}\n";

#[test]
fn duplicates_stay_hidden_unless_asked_for() {
    let workspace = Workspace::new();
    workspace.write("src/a.rs", CLONE_RIGHT);
    workspace.write("src/b.rs", CLONE_LEFT);
    let out = workspace.run(&[]);
    assert!(!out.contains("Duplicated code"), "{}", out);
    let options = workspace.invocation(&["--duplicates"]).options;
    assert!(options.duplicates);
}

#[test]
fn duplicates_report_the_copied_block_and_its_sites() {
    let workspace = Workspace::new();
    workspace.write("src/a.rs", CLONE_RIGHT);
    workspace.write("src/b.rs", CLONE_LEFT);
    let out = workspace.run(&["--duplicates"]);
    assert!(out.contains("Duplicated code"), "{}", out);
    assert!(out.contains("6 lines, 2 site(s)"), "{}", out);
    assert!(out.contains("src/a.rs:2"), "{}", out);
    assert!(out.contains("src/b.rs:2"), "{}", out);
}

#[test]
fn duplicates_json_carries_the_clone_count() {
    let workspace = Workspace::new();
    workspace.write("src/a.rs", CLONE_RIGHT);
    workspace.write("src/b.rs", CLONE_LEFT);
    let document = workspace.document(&["--duplicates"]);
    let dupes = document.get("duplicates").unwrap();
    assert_eq!(dupes.get("window").unwrap().integer(), 6);
    assert_eq!(dupes.get("clone_count").unwrap().integer(), 1);
    assert_eq!(dupes.get("duplicated_lines").unwrap().integer(), 12);
    assert_eq!(dupes.get("normalised_lines").unwrap().integer(), 14);
    assert!(dupes.get("percent").unwrap().number() > 85.0);
    let clusters = dupes.get("clusters").unwrap().items();
    assert_eq!(clusters.len(), 1);
    assert_eq!(clusters[0].get("lines").unwrap().integer(), 6);
    assert_eq!(clusters[0].get("sites").unwrap().items().len(), 2);
}

#[test]
fn dup_lines_grows_a_block_back_to_its_real_length() {
    let workspace = Workspace::new();
    workspace.write("src/a.rs", CLONE_RIGHT);
    workspace.write("src/b.rs", CLONE_LEFT);
    let document = workspace.document(&["--duplicates", "--dup-lines", "3"]);
    let dupes = document.get("duplicates").unwrap();
    assert_eq!(dupes.get("window").unwrap().integer(), 3);
    assert_eq!(dupes.get("clone_count").unwrap().integer(), 1);
    let clusters = dupes.get("clusters").unwrap().items();
    assert_eq!(clusters[0].get("lines").unwrap().integer(), 6);
}

#[test]
fn duplicates_ignore_test_only_files() {
    let workspace = Workspace::new();
    workspace.write("src/a.rs", CLONE_RIGHT);
    workspace.write("src/only_tests.rs", &format!("#![cfg(test)]\n{CLONE_LEFT}"));
    let out = workspace.run(&["--duplicates"]);
    assert!(out.contains("0 copy(ies)"), "{}", out);
    assert!(!out.contains("only_tests.rs"), "{}", out);
}

/// The same six lines again, but living inside `#[cfg(test)]` modules.
const CLONE_IN_TESTS_ONE: &str = "pub fn a() {}\n\n#[cfg(test)]\nmod tests {\n    fn one() {\n        let mut total = x;\n        total += 1;\n        total *= 2;\n        total -= 3;\n        total /= 4;\n        total\n    }\n}\n";
const CLONE_IN_TESTS_TWO: &str = "pub fn b() {}\n\n#[cfg(test)]\nmod tests {\n    fn two() {\n        let mut total = x;\n        total += 1;\n        total *= 2;\n        total -= 3;\n        total /= 4;\n        total\n    }\n}\n";

#[test]
fn duplicates_skip_cfg_test_items_unless_tests_are_included() {
    let workspace = Workspace::new();
    workspace.write("src/a.rs", CLONE_IN_TESTS_ONE);
    workspace.write("src/b.rs", CLONE_IN_TESTS_TWO);
    let hidden = workspace.document(&["--duplicates"]);
    let dupes = hidden.get("duplicates").unwrap();
    assert_eq!(dupes.get("clone_count").unwrap().integer(), 0);
    let shown = workspace.document(&["--duplicates", "--include-tests"]);
    let dupes = shown.get("duplicates").unwrap();
    assert_eq!(dupes.get("clone_count").unwrap().integer(), 1);
    assert_eq!(dupes.get("duplicated_lines").unwrap().integer(), 12);
}

#[test]
fn svg_export_is_a_standalone_document() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BASIC);
    let out = workspace.run(&["--format", "svg"]);
    assert!(out.starts_with("<svg "), "{}", out);
    assert!(out.ends_with("</svg>\n"), "{}", out);
    assert!(out.contains("Top files by code"), "{}", out);
    assert!(out.contains("font-family"), "{}", out);
    assert!(!out.contains("<html"), "{}", out);
}

#[test]
fn svg_stays_out_of_the_other_formats() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BASIC);
    assert!(!workspace.run(&[]).contains("<svg "));
    assert_eq!(
        workspace.invocation(&["--format", "svg"]).options.format,
        Format::Svg
    );
}

#[test]
fn a_badge_is_one_svg_line_sized_to_its_text() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BASIC);
    let out = workspace.run(&["--badge"]);
    assert!(out.starts_with("<svg "), "{}", out);
    assert!(out.ends_with("</svg>\n"), "{}", out);
    assert_eq!(out.matches('\n').count(), 1, "{}", out);
    assert!(out.contains("code lines"), "{}", out);
    assert!(!out.contains("<html"), "{}", out);
}

#[test]
fn the_badge_figure_matches_the_report() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BASIC);
    let code = workspace.summary(&[]).get("code").unwrap().integer();
    let out = workspace.run(&["--badge=code"]);
    assert!(out.contains(&format!("code lines: {code}")), "{}", out);
}

#[test]
fn a_badge_label_can_be_replaced_and_is_escaped() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", BASIC);
    let out = workspace.run(&["--badge-metric", "files", "--badge-label", "a<b"]);
    assert!(out.contains("a&lt;b: 1"), "{}", out);
    assert!(!out.contains("a<b"), "{}", out);
}

#[test]
fn a_smells_badge_turns_the_smell_count_on() {
    let workspace = Workspace::new();
    workspace.write(
        "src/lib.rs",
        "fn a() {\n    let x = Option::<u8>::None.unwrap();\n}\n",
    );
    let out = workspace.run(&["--badge-metric", "smells"]);
    assert!(out.contains("smells: 1"), "{}", out);
}

const PY_SOURCE: &str = "import os\n\n\ndef add(a, b):\n    return a + b\n";
const PY_TEST: &str = "def test_add():\n    assert add(1, 2) == 3\n";

#[test]
fn an_unknown_badge_figure_is_an_error() {
    let workspace = Workspace::new();
    let args: Vec<String> = vec![
        workspace.path.display().to_string(),
        "--badge=nope".to_string(),
    ];
    let error = cli::parse(&args, &workspace.path).unwrap_err();
    assert!(error.contains("known figure"), "{}", error);
}

#[test]
fn python_files_are_left_alone_by_default() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", "fn main() {}\n");
    workspace.write("tool/app.py", PY_SOURCE);
    let summary = workspace.summary(&[]);
    assert_eq!(summary.get("files").unwrap().integer(), 1);
    assert_eq!(summary.get("code").unwrap().integer(), 1);
}

#[test]
fn the_python_selection_reads_a_python_tree() {
    let workspace = Workspace::new();
    workspace.write("tool/app.py", PY_SOURCE);
    workspace.write("tests/test_app.py", PY_TEST);
    let summary = workspace.summary(&["--sources", "python"]);
    assert_eq!(summary.get("files").unwrap().integer(), 1);
    assert_eq!(summary.get("code").unwrap().integer(), 3);
    assert_eq!(summary.get("blank").unwrap().integer(), 2);
    assert_eq!(summary.get("test").unwrap().integer(), 0);
}

#[test]
fn the_all_selection_reads_both_languages() {
    let workspace = Workspace::new();
    workspace.write("src/lib.rs", "fn main() {}\n");
    workspace.write("tool/app.py", PY_SOURCE);
    let summary = workspace.summary(&["--sources", "all"]);
    assert_eq!(summary.get("files").unwrap().integer(), 2);
    assert_eq!(summary.get("code").unwrap().integer(), 4);
}

#[test]
fn the_scope_names_the_languages_it_read() {
    let workspace = Workspace::new();
    workspace.write("tool/app.py", PY_SOURCE);
    let out = workspace.run(&["--sources", "python"]);
    assert!(
        out.contains("Python business code (tests excluded)"),
        "{}",
        out
    );
    assert!(out.contains("Sources: Python"), "{}", out);
    assert!(out.contains("Python metrics"), "{}", out);
    assert!(!out.contains("Unsafe"), "{}", out);
    let both = workspace.run(&["--sources", "all"]);
    assert!(both.contains("Sources: Rust and Python"), "{}", both);
    assert!(both.contains("Metrics"), "{}", both);
    assert!(!both.contains("Rust smells"), "{}", both);
}

#[test]
fn the_json_scope_records_the_selection() {
    let workspace = Workspace::new();
    workspace.write("tool/app.py", PY_SOURCE);
    let document = workspace.document(&["--sources", "python"]);
    assert_eq!(
        document
            .get("scope")
            .unwrap()
            .get("sources")
            .unwrap()
            .text(),
        "python"
    );
}

#[test]
fn an_unknown_sources_value_is_an_error() {
    let workspace = Workspace::new();
    let args: Vec<String> = vec![
        workspace.path.display().to_string(),
        "--sources".to_string(),
        "cobol".to_string(),
    ];
    let error = cli::parse(&args, &workspace.path).unwrap_err();
    assert!(error.contains("rust, python or all"), "{}", error);
}
