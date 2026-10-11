"""Presentation helpers for the native desktop UI.

Everything in this module is plain Python that turns a rust_loc.cli scan report
into rows, cards, chart segments and settings files.  The tkinter layer in
rust_loc.ui stays a thin rendering shell on top, which keeps this logic
testable on a headless machine.
"""

from __future__ import annotations

import json
import os
import re
from pathlib import Path

from .cli import DEFAULT_IGNORE_DIRS, module_groups

# Flat, light palette shared by every widget.
PANEL_BG = "#f3f5f9"
CARD_BG = "#ffffff"
BORDER = "#e2e6ee"
GRID_BG = "#fbfcfe"
TEXT = "#1f2430"
MUTED = "#6b7280"
ACCENT = "#2563eb"
ACCENT_DARK = "#1d4ed8"
SUBTLE_BG = "#eef1f6"
SELECT_BG = "#dbe7fe"
SELECT_FG = "#10214a"

# Translucent scan mask.
VEIL = "#0b1220"
VEIL_TROUGH = "#243352"
VEIL_TEXT = "#e8edf8"
VEIL_MUTED = "#9aa9c6"
VEIL_ACCENT = "#60a5fa"

COLOR_CODE = "#2563eb"
COLOR_DOC = "#8b5cf6"
COLOR_COMMENT = "#22c55e"
COLOR_BLANK = "#cbd5e1"
COLOR_TEST = "#f59e0b"

TONE = {
    "files": ACCENT,
    "lines": ACCENT,
    "code": COLOR_CODE,
    "comment": COLOR_COMMENT,
    "blank": "#94a3b8",
    "test": COLOR_TEST,
    "complexity": "#7c3aed",
    "unsafe": "#dc2626",
    "test_fns": "#0891b2",
    "uloc": "#16a34a",
}

# (stats key, column title, pixel width); order matches the CLI tables.
TABLE_COLUMNS = (
    ("lines", "Lines", 78),
    ("blank", "Blank", 70),
    ("comment", "Comment", 84),
    ("code", "Code", 74),
    ("test", "Test", 70),
    ("complexity", "Cplx", 68),
    ("uloc", "ULOC", 74),
)
TABLE_BASE_KEYS = ("lines", "blank", "comment", "code", "test")
EXCLUDED_COLUMNS = (("lines", "Lines", 90),)
LABEL_KEY = "__label__"
LABEL_WIDTH = 520

LIMIT_CHOICES = ("100", "300", "1000", "All")


def human_int(value) -> str:
    """Thousands separated integer, tolerant of None."""
    try:
        return f"{int(value or 0):,}"
    except (TypeError, ValueError):
        return str(value)


def human_share(value, base) -> str:
    if not base:
        return "0.0%"
    return f"{value / base * 100:.1f}%"


def display_path(path, cwd) -> str:
    """cwd relative posix path when possible, absolute otherwise."""
    candidate = Path(path)
    try:
        return candidate.relative_to(cwd).as_posix()
    except ValueError:
        return candidate.as_posix()


def parse_list(text) -> list[str]:
    """Split a free-form field into unique tokens (globs, directory names)."""
    result: list[str] = []
    for part in re.split(r"[,;\s]+", text or ""):
        token = part.strip()
        if token and token not in result:
            result.append(token)
    return result


def parse_paths(text) -> list[str]:
    """Split pasted roots.  Windows paths may contain commas, so only ';' splits."""
    result: list[str] = []
    for part in re.split(r"[;\r\n]+", text or ""):
        token = part.strip().strip('"')
        if token and token not in result:
            result.append(token)
    return result


def parse_limit(text) -> int:
    """Row limit from the Top selector; 0 means every row."""
    raw = str(text or "").strip().lower()
    if not raw or raw.startswith("all"):
        return 0
    try:
        return max(int(raw), 0)
    except ValueError:
        return 0


def table_columns(complexity: bool = True, uloc: bool = False):
    """Column specs for the current option set, in CLI order."""
    keys = set(TABLE_BASE_KEYS)
    if complexity:
        keys.add("complexity")
    if uloc:
        keys.add("uloc")
    return [spec for spec in TABLE_COLUMNS if spec[0] in keys]


def column_keys(columns) -> list[str]:
    return [spec[0] for spec in columns]


def label_columns(tab: str, columns, label_title: str):
    """Column specs including the leading label column."""
    return [(LABEL_KEY, label_title, LABEL_WIDTH), *columns]


def file_rows(entries, cwd, keys) -> list[tuple[str, list[int]]]:
    """One (label, numeric values) row per file."""
    return [
        (display_path(entry["path"], cwd), [int(entry["stats"].get(key) or 0) for key in keys])
        for entry in entries
    ]


def group_rows(business, cwd, keys) -> list[tuple[str, list[int]]]:
    """One (module label, numeric values) row per top level module."""
    return [
        (label, [int(stats.get(key) or 0) for key in keys])
        for label, stats in module_groups(business, cwd)
    ]


def excluded_rows(excluded) -> list[tuple[str, list[int]]]:
    """Test-only files with their line counts, biggest first."""
    rows = [(str(path), [int(lines or 0)]) for path, lines in (excluded.get("paths") or [])]
    return sorted(rows, key=lambda row: (-row[1][0], row[0]))


def sort_rows(rows, column: int, descending: bool) -> list:
    """Sort display rows; column < 0 sorts by the label.

    Ties keep an ascending label order in both directions, which a stable
    second pass guarantees.
    """
    if column < 0:
        return sorted(rows, key=lambda row: row[0].casefold())
    ordered = sorted(rows, key=lambda row: row[0].casefold())
    return sorted(ordered, key=lambda row: row[1][column], reverse=descending)


def filter_rows(rows, query: str) -> list:
    """Case insensitive substring filter over the label column."""
    needle = (query or "").strip().casefold()
    if not needle:
        return list(rows)
    return [row for row in rows if needle in row[0].casefold()]


def limit_rows(rows, limit: int) -> list:
    if not limit or limit <= 0:
        return list(rows)
    return list(rows[:limit])


def table_hint(shown: int, matched: int, sort_key: str, descending: bool, titles) -> str:
    arrow = "▼" if descending else "▲"
    return (
        f"showing {human_int(shown)} of {human_int(matched)} rows · "
        f"sorted by {titles.get(sort_key, sort_key)} {arrow}"
    )


def composition(summary) -> list[tuple[str, int, str]]:
    """Segments of the line composition bar, in draw order."""
    comment = int(summary.get("comment") or 0)
    doc = int(summary.get("doc") or 0)
    return [
        ("Code", int(summary.get("code") or 0), COLOR_CODE),
        ("Doc comments", doc, COLOR_DOC),
        ("Comments", comment - doc, COLOR_COMMENT),
        ("Blank", int(summary.get("blank") or 0), COLOR_BLANK),
    ]


def test_line_total(excluded) -> int:
    """Test-only file lines plus lines inside #[cfg(test)] items."""
    return int(excluded.get("file_lines") or 0) + int(excluded.get("item_lines") or 0)


def summary_cards(report) -> list[dict]:
    """KPI cards for the headline numbers of a scan report."""
    options = report.get("options")
    summary = report["summary"]
    excluded = report["excluded"]
    lines = int(summary.get("lines") or 0)
    include_tests = bool(getattr(options, "include_tests", False))
    cards = [
        {
            "key": "files",
            "label": "Files",
            "value": human_int(len(report["business"])),
            "hint": human_int(len(report["entries"])) + " .rs files scanned",
        },
        {
            "key": "lines",
            "label": "Lines",
            "value": human_int(lines),
            "hint": "test code included" if include_tests else "business code only",
        },
        {
            "key": "code",
            "label": "Code",
            "value": human_int(summary.get("code")),
            "hint": human_share(summary.get("code") or 0, lines) + " of lines",
        },
        {
            "key": "comment",
            "label": "Comment",
            "value": human_int(summary.get("comment")),
            "hint": human_share(summary.get("comment") or 0, lines)
            + " · doc "
            + human_int(summary.get("doc")),
        },
        {
            "key": "blank",
            "label": "Blank",
            "value": human_int(summary.get("blank")),
            "hint": human_share(summary.get("blank") or 0, lines) + " of lines",
        },
        {
            "key": "test",
            "label": "Test lines",
            "value": human_int(test_line_total(excluded)),
            "hint": "included in totals" if include_tests else "excluded from totals",
        },
    ]
    if getattr(options, "complexity", True):
        cards.append(
            {
                "key": "complexity",
                "label": "Complexity",
                "value": human_int(summary.get("complexity")),
                "hint": "estimated branch points",
            }
        )
    cards.append(
        {
            "key": "unsafe",
            "label": "Unsafe",
            "value": human_int(summary.get("unsafe")),
            "hint": "unsafe keywords",
        }
    )
    cards.append(
        {
            "key": "test_fns",
            "label": "Test fns",
            "value": human_int(summary.get("test_fns")),
            "hint": "#[test] functions",
        }
    )
    if getattr(options, "uloc", False):
        total = int(report.get("uloc_total") or 0)
        cards.append(
            {
                "key": "uloc",
                "label": "ULOC",
                "value": human_int(total),
                "hint": "DRYness " + human_share(total, lines),
            }
        )
    return cards


def chart_modules(business, cwd, limit: int = 12) -> list[tuple[str, int]]:
    """Biggest modules by code, ready for the bar chart."""
    rows = [
        (label, int(stats.get("code") or 0))
        for label, stats in module_groups(business, cwd)[:limit]
    ]
    return shorten_labels(rows)


def shorten_labels(rows) -> list[tuple[str, int]]:
    """Drop the directory prefix every chart label shares."""
    rows = list(rows)
    if len(rows) < 2:
        return rows
    prefix = os.path.commonprefix([label for label, _value in rows])
    cut = prefix.rfind("/")
    if cut <= 0:
        return rows
    prefix = prefix[: cut + 1]
    return [(("…/" + label[len(prefix) :]) if label.startswith(prefix) else label, value) for label, value in rows]


def header_summary(report) -> str:
    summary = report["summary"]
    return " · ".join(
        (
            human_int(summary.get("code")) + " code",
            human_int(summary.get("comment")) + " comment",
            human_int(summary.get("blank")) + " blank",
        )
    )


def scope_line(options, roots) -> str:
    count = len(list(roots))
    parts = [f"{count} root" + ("" if count == 1 else "s")]
    parts.append("gitignore-aware" if options.gitignore else "gitignore ignored")
    parts.append(f"{len(options.ignore_dirs)} ignored dir names")
    if options.exclude:
        parts.append(f"{len(options.exclude)} exclude glob(s)")
    if options.include_tests:
        parts.append("tests included")
    return " · ".join(parts)


def quote_arg(value) -> str:
    text = str(value)
    if any(character in text for character in ' \t"'):
        return '"' + text + '"'
    return text


def cli_command(options, roots, cwd) -> str:
    """Reproduce the current UI state as a rust-loc command line."""
    parts = ["rust-loc"]
    for root in roots:
        parts.append(quote_arg(display_path(root, cwd)))
    for pattern in options.exclude:
        parts.extend(["--exclude", quote_arg(pattern)])
    default = set(DEFAULT_IGNORE_DIRS)
    current = set(options.ignore_dirs)
    if not default <= current:
        parts.append("--no-ignore-dirs")
    for name in sorted(current - default):
        parts.extend(["--ignore-dir", quote_arg(name)])
    if not options.gitignore:
        parts.append("--no-gitignore")
    if options.include_tests:
        parts.append("--include-tests")
    if options.uloc:
        parts.append("--uloc")
    if not options.complexity:
        parts.append("--no-complexity")
    return " ".join(parts)


def settings_path() -> Path:
    """%APPDATA%/rust-loc/ui.json on Windows, XDG or ~/.config elsewhere."""
    base = os.environ.get("APPDATA") or os.environ.get("XDG_CONFIG_HOME")
    root = Path(base) if base else Path.home() / ".config"
    return root / "rust-loc" / "ui.json"


def load_settings(path=None) -> dict:
    target = Path(path) if path else settings_path()
    try:
        data = json.loads(target.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return {}
    return data if isinstance(data, dict) else {}


def save_settings(data, path=None) -> bool:
    target = Path(path) if path else settings_path()
    try:
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    except OSError:
        return False
    return True
