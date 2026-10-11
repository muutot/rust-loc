#!/usr/bin/env python3
"""Count Rust source lines with test code excluded.

A Rust-only line counter in the spirit of tokei / scc / cloc / cargo-count, plus
one capability those tools do not offer: first-class exclusion of test code so
the numbers describe shipped business logic.

Core columns match the competitors (files / lines / blanks / comments / code).
Rust extras:

* ``test``        lines inside ``#[cfg(test)]`` items or test-only files;
* ``complexity``  approximate cyclomatic complexity using the same Rust branch
                  token list scc uses in its language config;
* ``unsafe``      naive ``unsafe`` keyword counter (cargo-count style);
* ``test fns``    number of ``#[test]``-family functions;
* ``uloc``        unique lines (``sort -u | wc -l`` over non-blank lines), see
                  https://cmcenroe.me/2018/12/14/uloc.html.

Test exclusion rules:

* test-only files: ``tests``/``benches`` directories, ``tests.rs``, ``test.rs``,
  ``*_tests.rs``, ``*_test.rs``, siblings referenced by ``#[cfg(test)] mod x;``
  (honouring ``#[path = "..."]``), and files with a crate-level
  ``#![cfg(test)]``;
* ``#[cfg(test)]`` and ``#[cfg(all(test, ...))]`` items inside ordinary files
  (the whole balanced item is removed). ``#[cfg(any(test, ...))]`` and
  ``#[cfg(not(test))]`` are kept because they also build outside tests.

Examples::

    rust-loc
    rust-loc --by-file --sort complexity --top 15
    rust-loc --format markdown --by-module
    rust-loc --format csv --by-file > loc.csv
    rust-loc --uloc
    rust-loc src-tauri/src src-tauri/crates
"""

from __future__ import annotations

import argparse
import csv as csvlib
import fnmatch
import io
import json
import re
import subprocess
import sys
from pathlib import Path

try:  # normal package import
    from . import __version__
except ImportError:  # executed as a standalone file
    __version__ = "0.1.0"

DEFAULT_ROOTS = (".",)
DEFAULT_IGNORE_DIRS = (
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
)

TEST_DIR_NAMES = {"tests", "benches", "test"}
TEST_FILE_RE = re.compile(r"^(?:tests?|.*_tests?)$")
ATTR_RE = re.compile(r"#\s*!?\s*\[(?P<body>[^\]]*)\]")
CFG_TEST_RE = re.compile(r"^\s*cfg\s*\((?P<inner>.*)\)\s*$", re.S)
PATH_ATTR_RE = re.compile(r'^path\s*=\s*"(?P<file>[^"]+)"$')
TEST_ATTR_RE = re.compile(r"^(?:[A-Za-z_]\w*::)*(?:test|rstest)$")
RAW_STRING_RE = re.compile(r'(?:b|c|br|cr)?r(?P<hashes>#*)"')
IDENT_RE = re.compile(r"[A-Za-z_]\w*")

COMPLEXITY_KEYWORDS = frozenset({"if", "else", "for", "while", "loop", "match"})
COMPLEXITY_OPERATORS = ("&&", "||", "==", "!=")
METRIC_KEYS = ("complexity", "unsafe", "test_fns")


def physical_lines(text: str) -> int:
    if not text:
        return 0
    return text.count("\n") + (0 if text.endswith("\n") else 1)


def hidden_process() -> dict:
    """Extra Popen arguments that keep console children (git) invisible.

    Without this a GUI launch (pythonw / the rust-loc-ui script) shows a black
    console window flashing for every git call.
    """
    if sys.platform != "win32":
        return {}
    startupinfo = subprocess.STARTUPINFO()
    startupinfo.dwFlags |= subprocess.STARTF_USESHOWWINDOW
    startupinfo.wShowWindow = 0  # SW_HIDE
    return {"creationflags": 0x08000000, "startupinfo": startupinfo}  # CREATE_NO_WINDOW


def is_test_only_cfg(inner: str) -> bool:
    """``test`` and ``all(test, ...)`` select test builds only."""
    stripped = inner.strip()
    if stripped == "test":
        return True
    if stripped.startswith("all("):
        body = stripped[stripped.index("(") + 1 : stripped.rindex(")")]
        return any(part.strip() == "test" for part in body.split(","))
    return False


def is_test_file(path: Path) -> bool:
    if any(part in TEST_DIR_NAMES for part in path.parts):
        return True
    return bool(TEST_FILE_RE.match(path.stem))


def analyze(text: str) -> dict:
    """Tokenise Rust source into per-line classes plus Rust metrics."""
    total = physical_lines(text)
    code = [False] * total
    comment = [False] * total
    doc = [False] * total
    spans: list[tuple[int, int]] = []

    complexity = complexity_all = 0
    unsafe = unsafe_all = 0
    test_fns = test_fns_all = 0

    length = len(text)
    line = 0
    i = 0
    CODE, LINE_COMMENT, BLOCK_COMMENT, STRING, RAW_STRING, CHAR = range(6)
    state = CODE
    block_depth = 0
    raw_hashes = 0
    escaped = False
    comment_is_doc = False
    pending_test = None
    pending_is_mod = False
    pending_mod_name = None
    pending_path = None
    pending_attrs: list[str] = []
    skipping_test = False
    test_depth = 0
    test_start = 0
    crate_test = False
    sibling_targets: list[str] = []

    while i < length:
        ch = text[i]

        if state == LINE_COMMENT:
            if ch == "\n":
                state = CODE
                line += 1
            else:
                comment[line] = True
                if comment_is_doc:
                    doc[line] = True
            i += 1
            continue

        if state == BLOCK_COMMENT:
            comment[line] = True
            if comment_is_doc:
                doc[line] = True
            if ch == "\n":
                line += 1
                i += 1
                continue
            if ch == "/" and text.startswith("*", i + 1):
                block_depth += 1
                i += 2
                continue
            if ch == "*" and text.startswith("/", i + 1):
                block_depth -= 1
                i += 2
                if block_depth == 0:
                    state = CODE
                    comment_is_doc = False
                continue
            i += 1
            continue

        if state == STRING:
            code[line] = True
            if ch == "\n":
                line += 1
            elif escaped:
                escaped = False
            elif ch == "\\":
                escaped = True
            elif ch == '"':
                state = CODE
            i += 1
            continue

        if state == RAW_STRING:
            code[line] = True
            if ch == "\n":
                line += 1
                i += 1
                continue
            if ch == '"' and text.startswith("#" * raw_hashes, i + 1):
                i += 1 + raw_hashes
                state = CODE
                continue
            i += 1
            continue

        if state == CHAR:
            code[line] = True
            if escaped:
                escaped = False
            elif ch == "\\":
                escaped = True
            elif ch == "'":
                state = CODE
            i += 1
            continue

        # ---- CODE state ----
        if ch == "\n":
            line += 1
            i += 1
            continue

        if ch == "#":
            match = ATTR_RE.match(text, i)
            if match:
                body = match.group("body").strip()
                if not skipping_test:
                    cfg = CFG_TEST_RE.match(body)
                    if cfg and is_test_only_cfg(cfg.group("inner")):
                        if text.startswith("#!", i):
                            crate_test = True
                        else:
                            pending_test = line
                    elif PATH_ATTR_RE.match(body):
                        pending_path = PATH_ATTR_RE.match(body).group("file")
                    else:
                        pending_attrs.append(body)
                end = match.end()
                extra = text.count("\n", i, end)
                for index in range(line, min(line + extra + 1, total)):
                    code[index] = True
                line += extra
                i = end
                continue

        if ch.isspace():
            i += 1
            continue

        if text.startswith("//", i):
            comment[line] = True
            comment_is_doc = (
                text.startswith("///", i) and not text.startswith("////", i)
            ) or text.startswith("//!", i)
            if comment_is_doc:
                doc[line] = True
            state = LINE_COMMENT
            i += 2
            continue

        if text.startswith("/*", i):
            comment[line] = True
            comment_is_doc = (
                text.startswith("/**", i) and not text.startswith("/***", i)
            ) or text.startswith("/*!", i)
            if comment_is_doc:
                doc[line] = True
            block_depth = 1
            state = BLOCK_COMMENT
            i += 2
            continue

        raw = RAW_STRING_RE.match(text, i)
        if raw:
            code[line] = True
            raw_hashes = len(raw.group("hashes"))
            i = raw.end()
            state = RAW_STRING
            continue

        if ch == '"':
            code[line] = True
            escaped = False
            state = STRING
            i += 1
            continue

        if ch == "'":
            code[line] = True
            if i + 1 < length and (
                text[i + 1] == "\\"
                or (i + 2 < length and text[i + 2] == "'" and text[i + 1] != "\n")
            ):
                escaped = False
                state = CHAR
            i += 1
            continue

        ident = IDENT_RE.match(text, i)
        if ident:
            code[line] = True
            word = ident.group(0)
            if word in COMPLEXITY_KEYWORDS:
                complexity_all += 1
                if not skipping_test:
                    complexity += 1
            elif word == "unsafe":
                unsafe_all += 1
                if not skipping_test:
                    unsafe += 1
            if word == "fn":
                if any(TEST_ATTR_RE.match(attr) for attr in pending_attrs):
                    test_fns_all += 1
                    if not skipping_test:
                        test_fns += 1
                pending_attrs = []
            elif pending_test is not None:
                if word == "mod":
                    pending_is_mod = True
                    pending_mod_name = None
                elif pending_is_mod and pending_mod_name is None:
                    pending_mod_name = word
            i = ident.end()
            continue

        operator = next((op for op in COMPLEXITY_OPERATORS if text.startswith(op, i)), None)
        if operator:
            code[line] = True
            complexity_all += 1
            if not skipping_test:
                complexity += 1
            i += len(operator)
            continue

        code[line] = True

        if skipping_test:
            if ch == "{":
                test_depth += 1
            elif ch == "}":
                test_depth -= 1
                if test_depth == 0:
                    spans.append((test_start, line))
                    skipping_test = False
                    pending_test = None
                    pending_is_mod = False
                    pending_mod_name = None
                    pending_path = None
                    pending_attrs = []
            i += 1
            continue

        if pending_test is not None:
            if ch == "{":
                test_start = pending_test
                skipping_test = True
                test_depth = 1
            elif ch == ";":
                spans.append((pending_test, line))
                if pending_is_mod:
                    target = pending_path
                    if not target and pending_mod_name:
                        target = pending_mod_name + ".rs"
                    if target:
                        sibling_targets.append(target)
                pending_test = None
                pending_is_mod = False
                pending_mod_name = None
                pending_path = None
                pending_attrs = []
            i += 1
            continue

        if ch in ";{}":
            pending_is_mod = False
            pending_mod_name = None
            pending_path = None
            pending_attrs = []

        i += 1

    if skipping_test:
        spans.append((test_start, total - 1))
    elif pending_test is not None:
        spans.append((pending_test, total - 1))

    is_test = [False] * total
    for start, end in spans:
        for index in range(max(start, 0), min(end, total - 1) + 1):
            is_test[index] = True

    return {
        "lines": text.split("\n")[:total],
        "total": total,
        "code": code,
        "comment": comment,
        "doc": doc,
        "is_test": is_test,
        "crate_test": crate_test,
        "sibling_targets": sibling_targets,
        "metrics": {"complexity": complexity, "unsafe": unsafe, "test_fns": test_fns},
        "metrics_all": {
            "complexity": complexity_all,
            "unsafe": unsafe_all,
            "test_fns": test_fns_all,
        },
    }

def summarize(entry: dict, siblings: set, include_tests: bool, want_uloc: bool) -> dict:
    """Reduce one analyzed file to business-code counters."""
    analysis = entry["analysis"]
    total = analysis["total"]
    forced_test = entry["name_test"] or entry["path"] in siblings or analysis["crate_test"]

    code = comment = doc = blank = test = 0
    unique: set[str] = set()
    for index in range(total):
        is_test = True if forced_test else analysis["is_test"][index]
        stripped = analysis["lines"][index].strip()
        if is_test and not include_tests:
            test += 1
        elif analysis["code"][index]:
            code += 1
        elif analysis["comment"][index]:
            comment += 1
            if analysis["doc"][index]:
                doc += 1
        else:
            blank += 1
        if want_uloc and stripped and not (is_test and not include_tests):
            unique.add(stripped)

    if forced_test:
        source = analysis["metrics_all"] if include_tests else None
    else:
        source = analysis["metrics_all"] if include_tests else analysis["metrics"]
    metrics = {key: (source[key] if source else 0) for key in METRIC_KEYS}

    return {
        "lines": blank + comment + code,
        "blank": blank,
        "comment": comment,
        "doc": doc,
        "code": code,
        "test": test,
        "physical": total,
        "complexity": metrics["complexity"],
        "unsafe": metrics["unsafe"],
        "test_fns": metrics["test_fns"],
        "uloc": len(unique) if want_uloc else None,
        "unique": unique if want_uloc else None,
        "forced_test": forced_test,
    }


def resolve_siblings(entries) -> set:
    siblings: set[Path] = set()
    for entry in entries:
        base = entry["path"].parent
        for target in entry["analysis"]["sibling_targets"]:
            resolved = base / target if target.endswith(".rs") else base / (target + ".rs")
            siblings.add(resolved.resolve())
    return siblings


def matches_exclude(path: Path, root: Path, patterns) -> bool:
    if not patterns:
        return False
    rel = path.relative_to(root).as_posix()
    absolute = path.as_posix()
    for pattern in patterns:
        trimmed = pattern.rstrip("/")
        if fnmatch.fnmatch(rel, pattern) or fnmatch.fnmatch(absolute, pattern):
            return True
        if rel == trimmed or fnmatch.fnmatch(rel, trimmed + "/*"):
            return True
    return False


def discover(root: Path, options) -> list:
    found = []
    for path in sorted(root.rglob("*.rs")):
        if not path.is_file():
            continue
        if any(part in options.ignore_dirs for part in path.parts):
            continue
        if matches_exclude(path, root, options.exclude):
            continue
        found.append(path)
    return found


def apply_gitignore(root: Path, files, enabled: bool) -> list:
    """Drop .gitignored paths using `git ls-files`, degrading to no-op."""
    files = list(files)
    if not enabled or not files:
        return files
    try:
        top = subprocess.run(
            ["git", "-C", str(root), "rev-parse", "--show-toplevel"],
            capture_output=True,
            text=True,
            timeout=20,
            **hidden_process(),
        )
        if top.returncode != 0:
            return files
        top_path = Path(top.stdout.strip())
        listing = subprocess.run(
            [
                "git",
                "-C",
                str(top_path),
                "ls-files",
                "-z",
                "--cached",
                "--others",
                "--exclude-standard",
                "--",
                "*.rs",
            ],
            capture_output=True,
            text=True,
            timeout=120,
            **hidden_process(),
        )
        if listing.returncode != 0:
            return files
        kept = {(top_path / name).resolve() for name in listing.stdout.split("\0") if name}
        return [path for path in files if path.resolve() in kept]
    except (OSError, subprocess.SubprocessError):
        return files


def collect_files(root: Path, files, on_progress=None) -> list:
    """Analyse already discovered ``files``, reporting progress per file."""
    entries = []
    total = len(files)
    for index, path in enumerate(files, 1):
        try:
            text = path.read_text(encoding="utf-8-sig", errors="replace")
        except OSError:
            text = None
        if text is not None:
            entries.append(
                {
                    "path": path.resolve(),
                    "root": root,
                    "name_test": is_test_file(Path(*path.relative_to(root).parts)),
                    "analysis": analyze(text),
                }
            )
        if on_progress is not None:
            on_progress(index, total, path)
    return entries


def collect(root: Path, options, on_progress=None) -> list:
    files = apply_gitignore(root, discover(root, options), options.gitignore)
    return collect_files(root, files, on_progress)


def resolve_roots(raw_paths, cwd: Path) -> list:
    """Turn user supplied paths into resolved, existing directories."""
    roots = []
    for raw in raw_paths:
        root = Path(raw)
        if not root.is_absolute():
            root = cwd / root
        if not root.exists():
            raise ValueError("path does not exist: " + str(raw))
        roots.append(root.resolve())
    return roots


def relative(path: Path, cwd: Path) -> str:
    try:
        return path.relative_to(cwd).as_posix()
    except ValueError:
        return path.as_posix()


def group_key(path: Path, root: Path) -> str:
    rel = path.relative_to(root)
    return rel.parts[0] if len(rel.parts) > 1 else "(root)"


def totals(entries) -> dict:
    keys = (
        "lines",
        "blank",
        "comment",
        "doc",
        "code",
        "test",
        "physical",
        "complexity",
        "unsafe",
        "test_fns",
    )
    return {key: sum(entry["stats"][key] for entry in entries) for key in keys}

SORT_KEYS = (
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
)


def share(value: int, base: int) -> str:
    return f"{value / base * 100:.1f}%" if base else "0.0%"


def union_uloc(entries):
    sets = [entry["stats"]["unique"] for entry in entries if entry["stats"]["unique"] is not None]
    if not sets:
        return None
    merged: set[str] = set()
    for item in sets:
        merged |= item
    return len(merged)


def sort_entries(entries, key: str) -> list:
    if key == "path":
        return sorted(entries, key=lambda item: item["path"].as_posix())
    if key == "uloc":
        return sorted(entries, key=lambda item: item["stats"]["uloc"] or 0, reverse=True)
    return sorted(entries, key=lambda item: item["stats"].get(key, 0), reverse=True)


def table_row(label: str, values, label_width: int = 44, width: int = 10) -> str:
    return f"{label:<{label_width}}" + "".join(f"{value:>{width},}" for value in values)


def file_columns(options) -> list:
    columns = ["lines", "blank", "comment", "code", "test"]
    if options.complexity:
        columns.append("complexity")
    if options.uloc:
        columns.append("uloc")
    return columns


def file_header(options) -> list:
    header = ["Lines", "Blank", "Comment", "Code", "Test"]
    if options.complexity:
        header.append("Cplx")
    if options.uloc:
        header.append("ULOC")
    return header


def module_groups(business, cwd):
    grouped: dict[tuple[str, str], list] = {}
    for entry in business:
        key = (relative(entry["root"], cwd), group_key(entry["path"], entry["root"]))
        grouped.setdefault(key, []).append(entry)
    rows = []
    for (root, group), items in grouped.items():
        stats = totals(items)
        stats["uloc"] = union_uloc(items)
        rows.append((root + "/" + group, stats))
    return sorted(rows, key=lambda row: row[1]["code"], reverse=True)


def render_text(options, cwd, roots, business, summary, uloc_total, all_lines, excluded):
    out: list[str] = []
    columns = file_columns(options)
    if options.by_file:
        ordered = sort_entries(business, options.sort)
        if options.top:
            ordered = ordered[: options.top]
        header = file_header(options)
        labels = [relative(entry["path"], cwd) for entry in ordered]
        width = max([44, len("File")] + [len(label) for label in labels])
        out.append(f"{'File':<{width}}" + "".join(f"{name:>10}" for name in header))
        out.append("-" * (width + 10 * len(header)))
        for label, entry in zip(labels, ordered):
            stats = entry["stats"]
            out.append(table_row(label, [(stats[key] or 0) for key in columns], width))
        out.append("")
    if options.by_module:
        header = file_header(options)
        groups = module_groups(business, cwd)
        labels = [label for label, _ in groups]
        width = max([44, len("Module")] + [len(label) for label in labels])
        out.append(f"{'Module':<{width}}" + "".join(f"{name:>10}" for name in header))
        out.append("-" * (width + 10 * len(header)))
        for label, stats in groups:
            out.append(table_row(label, [(stats[key] or 0) for key in columns], width))
        out.append("")

    out.append("Rust source lines" if options.include_tests else "Rust business code (tests excluded)")
    out.append("Roots: " + ", ".join(relative(root, cwd) for root in roots))
    out.append(
        "Scope: "
        + ("gitignore-aware" if options.gitignore else "no gitignore filtering")
        + f", {len(options.ignore_dirs)} ignored directory names"
    )
    out.append("")
    lines = summary["lines"]
    out.append(f"{'Files':<22}{len(business):>12,}")
    out.append(f"{'Lines':<22}{lines:>12,}")
    out.append(f"{'  Blank':<22}{summary['blank']:>12,}   {share(summary['blank'], lines)}")
    out.append(
        f"{'  Comment':<22}{summary['comment']:>12,}   {share(summary['comment'], lines)}"
        + (f"   (doc {summary['doc']:,})" if summary["doc"] else "")
    )
    out.append(f"{'  Code':<22}{summary['code']:>12,}   {share(summary['code'], lines)}")
    out.append("")
    out.append("Rust metrics")
    if options.complexity:
        out.append(f"{'  Complexity (est.)':<22}{summary['complexity']:>12,}")
    out.append(f"{'  Unsafe':<22}{summary['unsafe']:>12,}")
    out.append(f"{'  Test functions':<22}{summary['test_fns']:>12,}")
    if options.uloc:
        out.append(
            f"{'  ULOC':<22}{uloc_total:>12,}   {share(uloc_total, lines)} of lines (DRYness)"
        )
    out.append("")
    if options.include_tests:
        out.append("Test code included (--include-tests)")
    else:
        out.append("Excluded test code")
        out.append(
            f"  Test-only files          {excluded['files']:>6,} file(s), {excluded['file_lines']:,} lines"
        )
        out.append(f"  #[cfg(test)] items       {'':>6}          {excluded['item_lines']:,} lines")
    out.append("")
    out.append(f"{'All Rust lines':<22}{all_lines:>12,}")
    if options.show_excluded and excluded["paths"]:
        out.append("")
        out.append("Excluded test-only files")
        for path, count in excluded["paths"]:
            out.append(f"  {path} ({count:,} lines)")
    return out


def render_markdown(options, cwd, roots, business, summary, uloc_total, all_lines, excluded):
    title = "Rust source lines" if options.include_tests else "Rust business code (tests excluded)"
    out = [f"### {title}", ""]
    out.append("Roots: " + ", ".join(f"`{relative(root, cwd)}`" for root in roots))
    out.append("")
    out.append("| Metric | Value |")
    out.append("| --- | ---: |")
    rows = [
        ("Files", len(business)),
        ("Lines", summary["lines"]),
        ("Blank", summary["blank"]),
        ("Comment", summary["comment"]),
        ("Code", summary["code"]),
    ]
    if options.complexity:
        rows.append(("Complexity (est.)", summary["complexity"]))
    rows.append(("Unsafe", summary["unsafe"]))
    rows.append(("Test functions", summary["test_fns"]))
    if options.uloc:
        rows.append(("ULOC", uloc_total))
    for name, value in rows:
        out.append(f"| {name} | {value:,} |")
    out.append("")
    if not options.include_tests:
        out.append(
            f"Excluded tests: {excluded['files']:,} test-only files "
            f"({excluded['file_lines']:,} lines), {excluded['item_lines']:,} lines "
            "inside `#[cfg(test)]` items. All Rust lines: "
            f"{all_lines:,}."
        )
    out.append("")
    header = file_header(options)
    columns = file_columns(options)
    if options.by_file:
        ordered = sort_entries(business, options.sort)
        if options.top:
            ordered = ordered[: options.top]
        out.append("| File | " + " | ".join(header) + " |")
        out.append("| --- | " + " | ".join("---:" for _ in header) + " |")
        for entry in ordered:
            stats = entry["stats"]
            values = " | ".join(f"{stats[key] or 0:,}" for key in columns)
            out.append(f"| `{relative(entry['path'], cwd)}` | {values} |")
        out.append("")
    if options.by_module:
        out.append("| Module | " + " | ".join(header) + " |")
        out.append("| --- | " + " | ".join("---:" for _ in header) + " |")
        for label, stats in module_groups(business, cwd):
            values = " | ".join(f"{stats[key] or 0:,}" for key in columns)
            out.append(f"| `{label}` | {values} |")
        out.append("")
    return out


def render_csv(options, cwd, business) -> str:
    columns = ["lines", "blank", "comment", "doc", "code", "test"]
    if options.complexity:
        columns.append("complexity")
    columns.extend(["unsafe", "test_fns"])
    if options.uloc:
        columns.append("uloc")
    buffer = io.StringIO(newline="")
    writer = csvlib.writer(buffer, lineterminator="\n")
    writer.writerow(["path"] + columns)
    if options.by_file:
        ordered = sort_entries(business, options.sort)
        if options.top:
            ordered = ordered[: options.top]
        for entry in ordered:
            stats = entry["stats"]
            writer.writerow(
                [relative(entry["path"], cwd)] + [stats[key] if stats[key] is not None else 0 for key in columns]
            )
    else:
        stats = totals(business)
        stats["uloc"] = union_uloc(business)
        writer.writerow(["TOTAL"] + [stats[key] if stats[key] is not None else 0 for key in columns])
    return buffer.getvalue()


def render_json(options, cwd, roots, business, entries, summary, uloc_total, all_lines, excluded) -> str:
    stats_keys = (
        "lines",
        "blank",
        "comment",
        "doc",
        "code",
        "test",
        "physical",
        "complexity",
        "unsafe",
        "test_fns",
        "uloc",
    )
    payload = {
        "roots": [relative(root, cwd) for root in roots],
        "scope": {
            "gitignore": options.gitignore,
            "ignore_dirs": sorted(options.ignore_dirs),
            "exclude": options.exclude,
            "include_tests": options.include_tests,
        },
        "summary": {
            "files": len(business),
            **{key: summary[key] for key in stats_keys if key != "uloc"},
            "uloc": uloc_total,
            "dryness_percent": round(uloc_total / summary["lines"] * 100, 1)
            if options.uloc and summary["lines"]
            else None,
        },
        "excluded": None if options.include_tests else dict(excluded, paths=None),
        "all_rust_lines": all_lines,
        "files": [
            {
                "path": relative(entry["path"], cwd),
                "test_only": entry["stats"]["forced_test"],
                **{key: entry["stats"][key] for key in stats_keys if key != "uloc"},
                "uloc": entry["stats"]["uloc"],
            }
            for entry in entries
        ],
    }
    return json.dumps(payload, indent=2, ensure_ascii=False)


def make_options(
    *,
    by_file: bool = False,
    by_module: bool = False,
    sort: str = "code",
    top: int = 0,
    show_excluded: bool = False,
    include_tests: bool = False,
    exclude=(),
    ignore_dirs=None,
    gitignore: bool = True,
    uloc: bool = False,
    complexity: bool = True,
    format: str = "text",
) -> argparse.Namespace:
    """Build the option namespace shared by the CLI and the desktop UI."""
    return argparse.Namespace(
        by_file=by_file,
        by_module=by_module,
        sort=sort,
        top=max(int(top), 0),
        show_excluded=show_excluded,
        include_tests=include_tests,
        exclude=list(exclude),
        ignore_dirs=set(DEFAULT_IGNORE_DIRS) if ignore_dirs is None else set(ignore_dirs),
        gitignore=gitignore,
        uloc=uloc,
        complexity=complexity,
        format=format,
    )


def scan(roots, options, cwd: Path, on_progress=None, keep_analysis: bool = False) -> dict:
    """Analyse every root and reduce it to the numbers all renderers share.

    ``on_progress`` is called as ``on_progress(done, total, path)``, where
    ``done`` counts analysed files across all roots, so a UI can show progress
    without duplicating the walking logic.  The per-line arrays dominate the
    memory footprint, so they are dropped after summarising unless
    ``keep_analysis`` asks for them.
    """
    discovered = [
        (root, apply_gitignore(root, discover(root, options), options.gitignore)) for root in roots
    ]
    total = sum(len(files) for _, files in discovered)

    entries: list = []
    done = 0
    for root, files in discovered:
        if on_progress is None:
            entries.extend(collect_files(root, files))
        else:
            entries.extend(
                collect_files(
                    root,
                    files,
                    on_progress=lambda index, _total, path, base=done: on_progress(
                        base + index, total, path
                    ),
                )
            )
        done += len(files)

    siblings = resolve_siblings(entries)
    for entry in entries:
        entry["stats"] = summarize(entry, siblings, options.include_tests, options.uloc)
        if not keep_analysis:
            entry.pop("analysis", None)

    business = [
        entry for entry in entries if options.include_tests or not entry["stats"]["forced_test"]
    ]
    excluded_entries = [entry for entry in entries if entry["stats"]["forced_test"]]
    summary = totals(business)
    uloc_total = union_uloc(business) if options.uloc else None
    all_lines = sum(entry["stats"]["physical"] for entry in entries)
    excluded = {
        "files": len(excluded_entries),
        "file_lines": sum(entry["stats"]["physical"] for entry in excluded_entries),
        "item_lines": sum(
            entry["stats"]["test"] for entry in entries if not entry["stats"]["forced_test"]
        ),
        "paths": sorted(
            (relative(entry["path"], cwd), entry["stats"]["physical"]) for entry in excluded_entries
        ),
    }
    return {
        "roots": list(roots),
        "options": options,
        "entries": entries,
        "business": business,
        "excluded_entries": excluded_entries,
        "summary": summary,
        "uloc_total": uloc_total,
        "all_lines": all_lines,
        "excluded": excluded,
    }


def render_report(options, cwd: Path, report: dict) -> str:
    """Render a scan report in ``options.format``."""
    roots = report["roots"]
    if options.format == "json":
        return render_json(
            options,
            cwd,
            roots,
            report["business"],
            report["entries"],
            report["summary"],
            report["uloc_total"],
            report["all_lines"],
            report["excluded"],
        )
    if options.format == "csv":
        return render_csv(options, cwd, report["business"])
    if options.format == "markdown":
        return "\n".join(
            render_markdown(
                options,
                cwd,
                roots,
                report["business"],
                report["summary"],
                report["uloc_total"],
                report["all_lines"],
                report["excluded"],
            )
        )
    return "\n".join(
        render_text(
            options,
            cwd,
            roots,
            report["business"],
            report["summary"],
            report["uloc_total"],
            report["all_lines"],
            report["excluded"],
        )
    )


def render_as(options, cwd: Path, report: dict, fmt: str) -> str:
    """Render an existing report in another format (used by the UI export menu)."""
    clone = argparse.Namespace(**vars(options))
    clone.format = fmt
    return render_report(clone, cwd, report)


def options_from_args(args) -> argparse.Namespace:
    ignore_dirs = set(args.ignore_dir)
    if not args.no_ignore_dirs:
        ignore_dirs |= set(DEFAULT_IGNORE_DIRS)
    return make_options(
        by_file=args.by_file,
        by_module=args.by_module,
        sort=args.sort,
        top=args.top,
        show_excluded=args.show_excluded,
        include_tests=args.include_tests,
        exclude=args.exclude,
        ignore_dirs=ignore_dirs,
        gitignore=args.gitignore is not False,
        uloc=args.uloc,
        complexity=not args.no_complexity,
        format="json" if args.json else args.format,
    )


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="rust-loc",
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("--version", action="version", version="rust-loc " + __version__)
    parser.add_argument(
        "roots",
        nargs="*",
        default=None,
        help="directories to scan (default: " + ", ".join(DEFAULT_ROOTS) + ")",
    )
    parser.add_argument("--root", action="append", default=[], help="add a scan root (repeatable)")
    parser.add_argument(
        "--format",
        choices=("text", "json", "csv", "markdown"),
        default="text",
        help="output format (default: text)",
    )
    parser.add_argument("--json", action="store_true", help="alias for --format json")
    parser.add_argument(
        "--by-file", "--per-file", dest="by_file", action="store_true", help="per-file breakdown"
    )
    parser.add_argument("--by-module", action="store_true", help="per-module breakdown")
    parser.add_argument("--sort", choices=SORT_KEYS, default="code", help="sort key for listings")
    parser.add_argument("--top", type=int, default=0, help="limit listings to N rows")
    parser.add_argument("--show-excluded", action="store_true", help="list excluded test files")
    parser.add_argument("--include-tests", action="store_true", help="count test code too")
    parser.add_argument(
        "--exclude", action="append", default=[], metavar="GLOB", help="exclude paths (repeatable)"
    )
    parser.add_argument(
        "--ignore-dir",
        action="append",
        default=[],
        metavar="NAME",
        help="skip directories with this name (repeatable)",
    )
    parser.add_argument("--no-ignore-dirs", action="store_true", help="do not skip default dirs")
    parser.add_argument("--gitignore", dest="gitignore", action="store_true", default=None)
    parser.add_argument("--no-gitignore", dest="gitignore", action="store_false")
    parser.add_argument("--uloc", action="store_true", help="compute ULOC and DRYness")
    parser.add_argument("--no-complexity", action="store_true", help="hide complexity estimate")
    parser.add_argument("--ui", action="store_true", help="open the native desktop UI (tkinter)")
    return parser


def main(argv=None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    cwd = Path.cwd()

    requested = list(args.roots) if args.roots else list(DEFAULT_ROOTS)
    requested.extend(args.root)
    try:
        roots = resolve_roots(requested, cwd)
    except ValueError as exc:
        parser.error(str(exc))

    options = options_from_args(args)

    if args.ui:
        try:  # normal package import
            from . import ui
        except ImportError:
            try:  # executed as a standalone file
                import ui  # type: ignore[no-redef]
            except ImportError:
                parser.error("the desktop UI needs tkinter, which this Python does not provide")
        return ui.run(roots=roots, options=options, cwd=cwd, roots_explicit=bool(args.roots or args.root))

    report = scan(roots, options, cwd)
    if options.format == "csv":
        sys.stdout.write(render_report(options, cwd, report))
        return 0
    print(render_report(options, cwd, report))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
