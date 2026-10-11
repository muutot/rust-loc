"""Tests for the tkinter-free presentation helpers of the desktop UI."""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from rust_loc import cli  # noqa: E402
from rust_loc import ui_model as model  # noqa: E402


class ParseTests(unittest.TestCase):
    def test_parse_list_splits_and_dedupes(self):
        self.assertEqual(model.parse_list("a, b  c;d\nb"), ["a", "b", "c", "d"])

    def test_parse_list_of_empty_text(self):
        self.assertEqual(model.parse_list(""), [])
        self.assertEqual(model.parse_list(None), [])

    def test_parse_paths_keeps_commas_and_drops_quotes(self):
        self.assertEqual(model.parse_paths('"C:/a b"; D:/c,d'), ["C:/a b", "D:/c,d"])

    def test_parse_limit(self):
        self.assertEqual(model.parse_limit("All"), 0)
        self.assertEqual(model.parse_limit("300"), 300)
        self.assertEqual(model.parse_limit("junk"), 0)
        self.assertEqual(model.parse_limit(""), 0)


class ColumnTests(unittest.TestCase):
    def test_columns_follow_the_cli_options(self):
        keys = model.column_keys(model.table_columns(complexity=False, uloc=False))
        self.assertEqual(keys, ["lines", "blank", "comment", "code", "test"])
        keys = model.column_keys(model.table_columns(complexity=True, uloc=True))
        self.assertEqual(keys[-2:], ["complexity", "uloc"])

    def test_label_columns_prepend_the_name_column(self):
        specs = model.label_columns("files", model.table_columns(), "File")
        self.assertEqual(specs[0][0], model.LABEL_KEY)
        self.assertEqual(specs[0][1], "File")


class RowTests(unittest.TestCase):
    def rows(self):
        return [("b.rs", [10, 1]), ("a.rs", [10, 2]), ("c.rs", [5, 9])]

    def test_sort_rows_by_value_breaks_ties_on_the_label(self):
        self.assertEqual(
            [row[0] for row in model.sort_rows(self.rows(), 0, True)], ["a.rs", "b.rs", "c.rs"]
        )
        self.assertEqual(
            [row[0] for row in model.sort_rows(self.rows(), 0, False)], ["c.rs", "a.rs", "b.rs"]
        )

    def test_sort_rows_by_label_is_always_ascending(self):
        self.assertEqual(
            [row[0] for row in model.sort_rows(self.rows(), -1, True)], ["a.rs", "b.rs", "c.rs"]
        )

    def test_filter_rows_is_case_insensitive(self):
        self.assertEqual([row[0] for row in model.filter_rows(self.rows(), "A")], ["a.rs"])
        self.assertEqual(len(model.filter_rows(self.rows(), "  ")), 3)

    def test_limit_rows(self):
        self.assertEqual(len(model.limit_rows(self.rows(), 2)), 2)
        self.assertEqual(len(model.limit_rows(self.rows(), 0)), 3)

    def test_file_rows_use_cwd_relative_paths(self):
        entries = [{"path": Path("C:/repo/src/lib.rs"), "stats": {"code": 5, "uloc": None}}]
        self.assertEqual(
            model.file_rows(entries, Path("C:/repo"), ["code", "uloc"]), [("src/lib.rs", [5, 0])]
        )

    def test_excluded_rows_sort_by_size(self):
        excluded = {"paths": [("b.rs", 10), ("a.rs", 30)]}
        self.assertEqual(model.excluded_rows(excluded), [("a.rs", [30]), ("b.rs", [10])])
        self.assertEqual(model.excluded_rows({}), [])


class CardTests(unittest.TestCase):
    def report(self):
        return {
            "options": cli.make_options(uloc=True),
            "entries": [{"path": Path("x.rs"), "stats": {"forced_test": False}}],
            "business": [{"path": Path("x.rs"), "stats": {"forced_test": False}}],
            "summary": {
                "lines": 100,
                "blank": 10,
                "comment": 20,
                "doc": 5,
                "code": 70,
                "complexity": 7,
                "unsafe": 1,
                "test_fns": 2,
            },
            "excluded": {"files": 1, "file_lines": 4, "item_lines": 6, "paths": [("t.rs", 4)]},
            "uloc_total": 50,
        }

    def test_cards_cover_every_headline_number(self):
        cards = {card["key"]: card for card in model.summary_cards(self.report())}
        self.assertEqual(cards["files"]["value"], "1")
        self.assertEqual(cards["code"]["value"], "70")
        self.assertEqual(cards["test"]["value"], "10")
        self.assertEqual(cards["uloc"]["hint"], "DRYness 50.0%")

    def test_optional_cards_follow_the_options(self):
        report = self.report()
        report["options"] = cli.make_options(uloc=False, complexity=False)
        keys = [card["key"] for card in model.summary_cards(report)]
        self.assertNotIn("uloc", keys)
        self.assertNotIn("complexity", keys)
        self.assertIn("unsafe", keys)

    def test_test_line_total_adds_files_and_items(self):
        self.assertEqual(model.test_line_total(self.report()["excluded"]), 10)

    def test_composition_splits_doc_from_comments(self):
        segments = model.composition({"code": 70, "comment": 20, "doc": 5, "blank": 10})
        self.assertEqual(segments[0][:2], ("Code", 70))
        self.assertEqual(segments[1][:2], ("Doc comments", 5))
        self.assertEqual(segments[2][:2], ("Comments", 15))
        self.assertEqual(segments[3][:2], ("Blank", 10))


class TextTests(unittest.TestCase):
    def test_human_numbers(self):
        self.assertEqual(model.human_int(1234567), "1,234,567")
        self.assertEqual(model.human_int(None), "0")
        self.assertEqual(model.human_share(1, 4), "25.0%")
        self.assertEqual(model.human_share(1, 0), "0.0%")

    def test_display_path_prefers_relative(self):
        self.assertEqual(model.display_path(Path("C:/repo/src/a.rs"), Path("C:/repo")), "src/a.rs")

    def test_table_hint_mentions_the_sort_column(self):
        titles = {model.LABEL_KEY: "File", "code": "Code"}
        hint = model.table_hint(10, 20, "code", True, titles)
        self.assertIn("showing 10 of 20 rows", hint)
        self.assertIn("Code", hint)

    def test_header_and_scope_lines(self):
        report = {"summary": {"code": 3, "comment": 2, "blank": 1}}
        self.assertEqual(model.header_summary(report), "3 code · 2 comment · 1 blank")
        self.assertEqual(
            model.scope_line(cli.make_options(gitignore=False, include_tests=True), [Path("C:/repo")]),
            "1 root · gitignore ignored · 15 ignored dir names · tests included",
        )

    def test_shorten_labels_trims_the_shared_prefix(self):
        rows = [("src-tauri/src/platform", 10), ("src-tauri/src/storage", 5)]
        self.assertEqual(
            model.shorten_labels(rows), [("…/platform", 10), ("…/storage", 5)]
        )
        self.assertEqual(model.shorten_labels([("only", 1), ("only2", 2)])[0][0], "only")


class CommandTests(unittest.TestCase):
    def test_cli_command_reproduces_the_scope(self):
        options = cli.make_options(
            gitignore=False, uloc=True, include_tests=True, exclude=["*/gen/*"], ignore_dirs=set()
        )
        command = model.cli_command(options, [Path("C:/repo")], Path("C:/repo"))
        self.assertIn("rust-loc .", command)
        self.assertIn("--exclude */gen/*", command)
        self.assertIn("--no-gitignore", command)
        self.assertIn("--include-tests", command)
        self.assertIn("--uloc", command)
        self.assertIn("--no-ignore-dirs", command)

    def test_cli_command_quotes_paths_with_spaces(self):
        self.assertIn('"my repo"', model.cli_command(cli.make_options(), [Path("C:/my repo")], Path("C:/")))

    def test_extra_ignore_dirs_are_rendered(self):
        options = cli.make_options(ignore_dirs=set(cli.DEFAULT_IGNORE_DIRS) | {"generated"})
        self.assertIn(
            "--ignore-dir generated", model.cli_command(options, [Path("C:/repo")], Path("C:/repo"))
        )

    def test_plain_command_has_no_switches(self):
        self.assertEqual(
            model.cli_command(cli.make_options(), [Path("C:/repo")], Path("C:/")), "rust-loc repo"
        )


class SettingsTests(unittest.TestCase):
    def test_settings_round_trip(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "nested" / "ui.json"
            payload = {"roots": ["C:/repo"], "uloc": True}
            self.assertTrue(model.save_settings(payload, path))
            self.assertEqual(model.load_settings(path), payload)

    def test_missing_or_broken_settings_are_ignored(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "ui.json"
            self.assertEqual(model.load_settings(path), {})
            path.write_text("{ not json", encoding="utf-8")
            self.assertEqual(model.load_settings(path), {})
            path.write_text("[1, 2]", encoding="utf-8")
            self.assertEqual(model.load_settings(path), {})

    def test_settings_path_lives_under_the_app_data_dir(self):
        self.assertEqual(model.settings_path().name, "ui.json")


if __name__ == "__main__":
    unittest.main()
