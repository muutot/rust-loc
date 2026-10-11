"""End-to-end tests for the rust-loc CLI."""

from __future__ import annotations

import contextlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from rust_loc import cli  # noqa: E402


def run_cli(*args):
    """Run the CLI in-process and capture stdout."""

    buffer = io.StringIO()
    with contextlib.redirect_stdout(buffer):
        code = cli.main(list(args))
    return code, buffer.getvalue()


class CliTestCase(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)

    def tearDown(self):
        self._tmp.cleanup()

    def write(self, name, text):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
        return path

    def json_summary(self, *args):
        code, out = run_cli(str(self.root), "--no-gitignore", "--format", "json", *args)
        self.assertEqual(code, 0, out)
        return json.loads(out)["summary"]

    def json_data(self, *args):
        code, out = run_cli(str(self.root), "--no-gitignore", "--format", "json", *args)
        self.assertEqual(code, 0, out)
        return json.loads(out)

    def find(self, data, suffix):
        for entry in data["files"]:
            if entry["path"].endswith(suffix):
                return entry
        self.fail("no json entry ending with " + suffix)


class LineClassificationTests(CliTestCase):
    def test_counts_blank_comment_and_code(self):
        self.write("src/lib.rs", "// plain\n\nfn main() {\n    let x = 1; // trailing\n}\n")
        summary = self.json_summary()
        self.assertEqual(summary["blank"], 1)
        self.assertEqual(summary["comment"], 1)
        self.assertEqual(summary["code"], 3)
        self.assertEqual(summary["lines"], 5)

    def test_doc_comments_are_reported_separately(self):
        self.write("src/lib.rs", "/// docs\n// plain\nfn main() {}\n")
        summary = self.json_summary()
        self.assertEqual(summary["comment"], 2)
        self.assertEqual(summary["doc"], 1)
        self.assertEqual(summary["code"], 1)

    def test_nested_block_comment_is_one_comment(self):
        self.write("src/lib.rs", "/* outer /* inner */ still comment */\nfn a() {}\n")
        summary = self.json_summary()
        self.assertEqual(summary["comment"], 1)
        self.assertEqual(summary["code"], 1)

    def test_raw_string_with_braces_stays_code(self):
        self.write("src/lib.rs", 'const S: &str = r#"{ "not code" }"#;\n')
        summary = self.json_summary()
        self.assertEqual(summary["code"], 1)
        self.assertEqual(summary["comment"], 0)
        self.assertEqual(summary["test"], 0)


class TestExclusionTests(CliTestCase):
    def test_cfg_test_module_is_excluded(self):
        self.write(
            "src/lib.rs",
            "pub fn value() -> u32 {\n    1\n}\n\n"
            "#[cfg(test)]\nmod tests {\n    #[test]\n    fn works() {\n        assert_eq!(1, 1);\n    }\n}\n",
        )
        summary = self.json_summary()
        self.assertEqual(summary["code"], 3)
        self.assertEqual(summary["test"], 7)

    def test_crate_level_cfg_test_excludes_whole_file(self):
        self.write("src/crate_inner.rs", "#![cfg(test)]\n\n#[test]\nfn a() {}\n")
        data = self.json_data()
        self.assertEqual(data["summary"]["files"], 0)
        self.assertEqual(data["excluded"]["files"], 1)

    def test_path_attribute_sibling_becomes_test_only(self):
        self.write("src/hidden/support.rs", "pub fn helper() -> u32 { 1 }\n")
        self.write("src/lib.rs", '#[cfg(test)]\n#[path = "hidden/support.rs"]\nmod support;\n')
        data = self.json_data()
        self.assertTrue(self.find(data, "hidden/support.rs")["test_only"])
        self.assertEqual(data["summary"]["lines"], 0)

    def test_plain_path_attribute_does_not_leak_into_later_test_mod(self):
        self.write("src/misc.rs", "pub fn misc() -> u32 { 1 }\n")
        self.write("src/helper.rs", "pub fn helper() -> u32 { 2 }\n")
        self.write("src/lib.rs", '#[path = "misc.rs"]\nmod misc;\n\n#[cfg(test)]\nmod helper;\n')
        data = self.json_data()
        self.assertFalse(self.find(data, "src/misc.rs")["test_only"])
        self.assertTrue(self.find(data, "src/helper.rs")["test_only"])

    def test_any_test_and_not_test_are_kept(self):
        self.write(
            "src/lib.rs",
            '#[cfg(any(test, feature = "x"))]\npub fn a() {}\n\n#[cfg(not(test))]\npub fn b() {}\n',
        )
        summary = self.json_summary()
        self.assertEqual(summary["code"], 4)
        self.assertEqual(summary["test"], 0)

    def test_test_named_files_and_tests_dir_are_excluded(self):
        self.write("src/foo_tests.rs", "pub fn a() {}\n")
        self.write("src/tests/helper.rs", "pub fn b() {}\n")
        self.write("src/lib.rs", "pub fn c() {}\n")
        data = self.json_data()
        self.assertEqual(data["summary"]["files"], 1)
        self.assertEqual(data["excluded"]["files"], 2)

    def test_fake_cfg_test_inside_string_and_comment_is_ignored(self):
        self.write(
            "src/lib.rs",
            'fn main() {\n'
            '    let s = "#[cfg(test)] mod fake { }";\n'
            "    // #[cfg(test)] mod fake2 { }\n"
            '    let t = "}";\n'
            "}\n",
        )
        summary = self.json_summary()
        self.assertEqual(summary["test"], 0)
        self.assertEqual(summary["code"], 4)
        self.assertEqual(summary["comment"], 1)

    def test_include_tests_adds_test_lines_back(self):
        self.write("src/lib.rs", "fn a() {}\n\n#[cfg(test)]\nmod tests {\n    fn b() {}\n}\n")
        plain = self.json_summary()
        with_tests = self.json_summary("--include-tests")
        self.assertEqual(with_tests["lines"], plain["lines"] + plain["test"])
        self.assertEqual(self.json_summary("--include-tests")["test"], 0)


class MetricTests(CliTestCase):
    def test_complexity_uses_scc_rust_branch_tokens(self):
        self.write(
            "src/lib.rs",
            "fn f(a: u32) -> u32 {\n    if a == 1 && a != 2 {\n        1\n    } else {\n        2\n    }\n}\n",
        )
        summary = self.json_summary()
        self.assertEqual(summary["complexity"], 5)

    def test_unsafe_and_test_function_counters(self):
        self.write(
            "src/lib.rs",
            "pub unsafe fn f() {\n    unsafe { }\n}\n\n#[test]\nfn t() {}\n",
        )
        summary = self.json_summary()
        self.assertEqual(summary["unsafe"], 2)
        self.assertEqual(summary["test_fns"], 1)

    def test_uloc_counts_unique_non_blank_lines(self):
        self.write("src/lib.rs", "fn a() {}\nfn a() {}\n\nfn b() {}\n")
        summary = self.json_summary("--uloc")
        self.assertEqual(summary["uloc"], 2)

    def test_exclude_glob_skips_matching_paths(self):
        self.write("src/keep.rs", "fn a() {}\n")
        self.write("src/skip/other.rs", "fn b() {}\n")
        data = self.json_data("--exclude", "*/skip/*")
        self.assertEqual(data["summary"]["files"], 1)


class OutputFormatTests(CliTestCase):
    def setUp(self):
        super().setUp()
        self.write("src/lib.rs", "fn a() {}\n")

    def test_text_output_lists_metrics(self):
        code, out = run_cli(str(self.root), "--no-gitignore")
        self.assertEqual(code, 0)
        self.assertIn("Rust business code (tests excluded)", out)
        self.assertIn("Complexity (est.)", out)

    def test_csv_has_header_and_total_row(self):
        code, out = run_cli(str(self.root), "--no-gitignore", "--format", "csv")
        self.assertEqual(code, 0)
        rows = out.strip().splitlines()
        self.assertEqual(rows[0].split(",")[0], "path")
        self.assertEqual(rows[1].split(",")[0], "TOTAL")

    def test_markdown_contains_table(self):
        code, out = run_cli(str(self.root), "--no-gitignore", "--format", "markdown")
        self.assertEqual(code, 0)
        self.assertIn("| Code |", out)

    def test_no_complexity_hides_column(self):
        _, shown = run_cli(str(self.root), "--no-gitignore", "--by-file")
        _, hidden = run_cli(str(self.root), "--no-gitignore", "--by-file", "--no-complexity")
        self.assertIn("Cplx", shown)
        self.assertNotIn("Cplx", hidden)

    def test_version_flag(self):
        buffer = io.StringIO()
        with contextlib.redirect_stdout(buffer), self.assertRaises(SystemExit) as ctx:
            cli.main(["--version"])
        self.assertEqual(ctx.exception.code, 0)
        self.assertIn("rust-loc", buffer.getvalue())


class ScanApiTests(CliTestCase):
    def test_scan_returns_the_same_numbers_as_the_cli(self):
        self.write("src/lib.rs", "fn a() {}\n\n#[cfg(test)]\nmod tests {\n    fn b() {}\n}\n")
        report = cli.scan([self.root], cli.make_options(gitignore=False), Path.cwd())
        self.assertEqual(report["summary"]["code"], 1)
        self.assertEqual(report["summary"]["test"], 4)
        self.assertEqual(len(report["business"]), 1)
        self.assertEqual(len(report["excluded_entries"]), 0)

    def test_scan_drops_the_per_line_arrays_unless_asked(self):
        self.write("src/lib.rs", "fn a() {}\n")
        options = cli.make_options(gitignore=False)
        report = cli.scan([self.root], options, Path.cwd())
        self.assertNotIn("analysis", report["entries"][0])
        kept = cli.scan([self.root], options, Path.cwd(), keep_analysis=True)
        self.assertIn("analysis", kept["entries"][0])

    def test_scan_reports_progress_for_every_file(self):
        self.write("src/a.rs", "fn a() {}\n")
        self.write("src/b.rs", "fn b() {}\n")
        seen = []
        cli.scan(
            [self.root],
            cli.make_options(gitignore=False),
            Path.cwd(),
            on_progress=lambda done, total, path: seen.append((done, total)),
        )
        self.assertEqual(seen, [(1, 2), (2, 2)])

    def test_scan_records_the_roots_and_options(self):
        self.write("src/lib.rs", "fn a() {}\n")
        options = cli.make_options(gitignore=False, uloc=True)
        report = cli.scan([self.root], options, Path.cwd())
        self.assertEqual(report["roots"], [self.root])
        self.assertIs(report["options"], options)
        self.assertEqual(report["uloc_total"], 1)

    def test_render_report_matches_the_dedicated_renderers(self):
        self.write("src/lib.rs", "fn a() {}\n")
        options = cli.make_options(gitignore=False)
        report = cli.scan([self.root], options, Path.cwd())
        self.assertEqual(
            cli.render_report(options, Path.cwd(), report),
            "\n".join(
                cli.render_text(
                    options,
                    Path.cwd(),
                    report["roots"],
                    report["business"],
                    report["summary"],
                    report["uloc_total"],
                    report["all_lines"],
                    report["excluded"],
                )
            ),
        )
        options.format = "csv"
        self.assertEqual(cli.render_report(options, Path.cwd(), report), cli.render_csv(options, Path.cwd(), report["business"]))
        options.format = "markdown"
        self.assertIn("| Code |", cli.render_report(options, Path.cwd(), report))
        options.format = "json"
        self.assertEqual(json.loads(cli.render_report(options, Path.cwd(), report))["summary"]["files"], 1)

    def test_render_as_leaves_the_options_alone(self):
        self.write("src/lib.rs", "fn a() {}\n")
        options = cli.make_options(gitignore=False)
        report = cli.scan([self.root], options, Path.cwd())
        cli.render_as(options, Path.cwd(), report, "json")
        self.assertEqual(options.format, "text")

    def test_make_options_defaults_and_clamping(self):
        options = cli.make_options()
        self.assertTrue(options.gitignore)
        self.assertTrue(options.complexity)
        self.assertFalse(options.uloc)
        self.assertFalse(options.include_tests)
        self.assertEqual(options.top, 0)
        self.assertEqual(options.ignore_dirs, set(cli.DEFAULT_IGNORE_DIRS))
        self.assertEqual(options.format, "text")
        self.assertEqual(cli.make_options(top=-5).top, 0)
        self.assertEqual(cli.make_options(top="12").top, 12)
        self.assertEqual(cli.make_options(ignore_dirs=()).ignore_dirs, set())

    def test_options_from_args_carries_every_switch(self):
        args = cli.build_parser().parse_args(
            ["--by-file", "--sort", "lines", "--top", "5", "--uloc", "--include-tests", "--no-complexity",
             "--no-gitignore", "--exclude", "*/gen/*", "--ignore-dir", "tmp", "src"]
        )
        options = cli.options_from_args(args)
        self.assertTrue(options.by_file)
        self.assertEqual(options.sort, "lines")
        self.assertEqual(options.top, 5)
        self.assertTrue(options.uloc)
        self.assertTrue(options.include_tests)
        self.assertFalse(options.complexity)
        self.assertFalse(options.gitignore)
        self.assertEqual(options.exclude, ["*/gen/*"])
        self.assertEqual(options.ignore_dirs, set(cli.DEFAULT_IGNORE_DIRS) | {"tmp"})

    def test_no_ignore_dirs_switch_empties_the_default_set(self):
        args = cli.build_parser().parse_args(["--no-ignore-dirs", "--ignore-dir", "tmp"])
        self.assertEqual(cli.options_from_args(args).ignore_dirs, {"tmp"})

    def test_resolve_roots_rejects_missing_paths(self):
        self.assertEqual(cli.resolve_roots(["."], self.root), [self.root])
        with self.assertRaises(ValueError):
            cli.resolve_roots(["definitely-missing-dir"], self.root)

    def test_ui_flag_is_registered(self):
        args = cli.build_parser().parse_args(["--ui", "src"])
        self.assertTrue(args.ui)
        self.assertEqual(args.roots, ["src"])


if __name__ == "__main__":
    unittest.main()
