"""Native desktop UI for rust-loc (tkinter/ttk, no web stack).

A single window over a rust_loc.cli scan report:

* sidebar with scan roots, scope switches and filters;
* KPI cards for the headline numbers;
* hand drawn canvases for line composition and the biggest modules;
* sortable per-file, per-module and excluded-test tables;
* export of the current result as text, Markdown, CSV or JSON.

Scanning runs on a worker thread and reports back through a queue, so the
window stays responsive on large trees.  Start it with ``rust-loc --ui``,
``rust-loc-ui`` or ``python -m rust_loc.ui``.
"""

from __future__ import annotations

import os
import queue
import sys
import threading
import time
import tkinter as tk
from pathlib import Path
from tkinter import filedialog, font as tkfont, messagebox, simpledialog, ttk

try:  # normal package import
    from . import __version__, cli
    from . import ui_model as model
except ImportError:  # executed as a standalone file
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    from rust_loc import __version__, cli
    from rust_loc import ui_model as model

WINDOW_TITLE = "rust-loc — Rust code statistics"
TABS = (("files", "Files"), ("modules", "Modules"), ("excluded", "Excluded tests"))
TABLE_TITLES = {
    "files": "Per-file breakdown",
    "modules": "Per-module breakdown",
    "excluded": "Test-only files kept out of the totals",
}
EXPORT_FORMATS = (
    ("text", "Text", ".txt"),
    ("markdown", "Markdown", ".md"),
    ("csv", "CSV", ".csv"),
    ("json", "JSON", ".json"),
)
CHART_ROWS = 12
CARD_COLUMNS = 5
OVERLAY_DELAY_MS = 220


def _enable_dpi_awareness() -> None:
    """Ask Windows for a crisp, non-blurry UI; harmless elsewhere."""
    if sys.platform != "win32":
        return
    import ctypes

    try:
        ctypes.windll.shcore.SetProcessDpiAwareness(1)
    except (AttributeError, OSError):
        try:
            ctypes.windll.user32.SetProcessDPIAware()
        except (AttributeError, OSError):
            pass


def _pick_fonts(root: tk.Misc) -> tuple:
    """Choose a UI and a monospace family, and align the Tk named fonts."""
    families = set(tkfont.families(root))
    ui_family = None
    for name in ("Segoe UI Variable Text", "Segoe UI", "Noto Sans", "DejaVu Sans"):
        if name in families:
            ui_family = name
            break
    if ui_family is None:
        ui_family = tkfont.nametofont("TkDefaultFont").actual("family")
    for name in ("TkDefaultFont", "TkTextFont", "TkMenuFont", "TkHeadingFont"):
        try:
            tkfont.nametofont(name).configure(family=ui_family, size=9)
        except tk.TclError:
            pass
    mono = "Courier"
    for name in ("Cascadia Mono", "Consolas", "DejaVu Sans Mono", "Courier New"):
        if name in families:
            mono = name
            break
    return ui_family, mono


def _dpi_scale(window: tk.Misc) -> float:
    """Display scale relative to 96 DPI (Tk scaling is pixels per point)."""
    try:
        scaling = float(window.tk.call("tk", "scaling"))
    except tk.TclError:
        return 1.0
    return max(scaling / (4 / 3), 0.5)


def _place_window(window: tk.Tk, width: int, height: int, minimum=(1080, 660)) -> None:
    """Size, centre and bound the window; sizes are logical pixels."""
    window.update_idletasks()
    scale = _dpi_scale(window)
    screen_w = int(window.winfo_screenwidth())
    screen_h = int(window.winfo_screenheight())
    width = int(min(width * scale, max(screen_w - 80, 640)))
    height = int(min(height * scale, max(screen_h - 120, 480)))
    left = max((screen_w - width) // 2, 0)
    top = max((screen_h - height) // 3, 0)
    window.minsize(min(int(minimum[0] * scale), width), min(int(minimum[1] * scale), height))
    window.geometry(f"{width}x{height}+{left}+{top}")


def _ellipsize(text: str, font: tkfont.Font, max_px: int) -> str:
    if font.measure(text) <= max_px:
        return text
    trimmed = text
    while trimmed and font.measure(trimmed + "…") > max_px:
        trimmed = trimmed[:-1]
    return trimmed + "…" if trimmed else "…"


class LocApp:
    """Single window explorer for rust-loc scan reports."""

    def __init__(self, root: tk.Tk, roots, options, cwd: Path, roots_explicit: bool = False):
        self.root = root
        self.cwd = Path(cwd)
        self.options = options
        self.initial_options = options
        self.roots_explicit = roots_explicit
        self.report = None
        self.exit_code = 0

        self.roots: list[Path] = [Path(item) for item in (roots or [])]
        self.queue: queue.Queue = queue.Queue()
        self.worker = None
        self._scan_started = 0.0
        self._chart_job = None
        self._rescan_job = None
        self._search_job = None
        self._overlay_job = None
        self._sync_job = None
        self.overlay = None

        self.family, self.mono = _pick_fonts(root)
        self._make_fonts()

        self._columns = model.table_columns(options.complexity, options.uloc)
        self._keys = model.column_keys(self._columns)
        self._rows: dict = {"files": [], "modules": [], "excluded": []}
        self._modules: list = []
        self._trees: dict = {}
        self._tab_labels: dict = {}
        self._tab_underlines: dict = {}
        self._tab_frames: dict = {}
        self._active = "files"
        self._card_widgets: dict = {}
        self._sort_key = "code"
        self._sort_desc = True

        self.status_var = tk.StringVar(value="Ready")
        self.scope_var = tk.StringVar(value="")
        self.header_var = tk.StringVar(value="")
        self.search_var = tk.StringVar(value="")
        self.limit_var = tk.StringVar(value="300")
        self.var_gitignore = tk.BooleanVar(value=True)
        self.var_default_ignores = tk.BooleanVar(value=True)
        self.var_include_tests = tk.BooleanVar(value=False)
        self.var_complexity = tk.BooleanVar(value=True)
        self.var_uloc = tk.BooleanVar(value=False)
        self.var_exclude = tk.StringVar(value="")
        self.var_extra_dirs = tk.StringVar(value="")

        self._build_style()
        self._build_header()
        self._build_status()
        self._build_body()
        self._bind_keys()
        self._load_settings()
        self.root.protocol("WM_DELETE_WINDOW", self.close)

    # ------------------------------------------------------------------ setup
    def _make_fonts(self) -> None:
        self.title_font = tkfont.Font(family=self.family, size=16, weight="bold")
        self.label_font = tkfont.Font(family=self.family, size=9, weight="bold")
        self.small_font = tkfont.Font(family=self.family, size=9)
        self.tiny_font = tkfont.Font(family=self.family, size=8)
        self.tiny_bold_font = tkfont.Font(family=self.family, size=8, weight="bold")
        self.value_font = tkfont.Font(family=self.mono, size=15, weight="bold")
        self.mono_small = tkfont.Font(family=self.mono, size=9)
        self.small_bold_font = tkfont.Font(family=self.family, size=8, weight="bold")
        self.overlay_font = tkfont.Font(family=self.family, size=13, weight="bold")

    def _build_style(self) -> None:
        style = ttk.Style(self.root)
        if "clam" in style.theme_names():
            style.theme_use("clam")
        style.configure(".", background=model.PANEL_BG, foreground=model.TEXT, font=self.small_font)
        style.configure("TFrame", background=model.PANEL_BG)
        style.configure("TLabel", background=model.PANEL_BG, foreground=model.TEXT)
        style.configure("Card.TLabel", background=model.CARD_BG, foreground=model.TEXT)
        style.configure("CardMuted.TLabel", background=model.CARD_BG, foreground=model.MUTED, font=self.tiny_font)
        style.configure(
            "Accent.TButton",
            background=model.ACCENT,
            foreground="#ffffff",
            borderwidth=0,
            focusthickness=0,
            relief="flat",
            padding=(12, 9),
            font=self.label_font,
        )
        style.map(
            "Accent.TButton",
            background=[
                ("pressed", model.ACCENT_DARK),
                ("active", "#3b82f6"),
                ("disabled", "#a8c4f8"),
            ],
            foreground=[("disabled", "#eef2ff")],
        )
        style.configure(
            "Ghost.TButton",
            background=model.SUBTLE_BG,
            foreground=model.TEXT,
            borderwidth=0,
            focusthickness=0,
            relief="flat",
            padding=(8, 6),
            font=self.tiny_font,
        )
        style.map("Ghost.TButton", background=[("pressed", "#dbe1ea"), ("active", "#e4e9f1")])
        style.configure(
            "Ghost.TMenubutton",
            background=model.SUBTLE_BG,
            foreground=model.TEXT,
            borderwidth=0,
            relief="flat",
            padding=(8, 7),
            font=self.tiny_font,
        )
        style.map("Ghost.TMenubutton", background=[("pressed", "#dbe1ea"), ("active", "#e4e9f1")])
        for name in ("Field.TEntry", "Field.TCombobox"):
            style.configure(
                name,
                fieldbackground="#ffffff",
                background="#ffffff",
                foreground=model.TEXT,
                bordercolor=model.BORDER,
                lightcolor=model.BORDER,
                darkcolor=model.BORDER,
                arrowcolor=model.MUTED,
                padding=5,
                relief="flat",
            )
        style.map(
            "Field.TCombobox",
            fieldbackground=[("readonly", "#ffffff")],
            selectbackground=[("readonly", "#ffffff")],
            selectforeground=[("readonly", model.TEXT)],
        )
        style.configure(
            "Card.TCheckbutton",
            background=model.CARD_BG,
            foreground=model.TEXT,
            focusthickness=0,
            padding=(0, 3),
            font=self.small_font,
        )
        style.map(
            "Card.TCheckbutton",
            background=[("active", model.CARD_BG)],
            indicatorcolor=[("selected", model.ACCENT), ("!selected", "#ffffff")],
        )
        style.configure(
            "Treeview",
            background=model.CARD_BG,
            fieldbackground=model.CARD_BG,
            foreground=model.TEXT,
            borderwidth=0,
            rowheight=24,
            font=self.small_font,
        )
        style.map("Treeview", background=[("selected", model.SELECT_BG)], foreground=[("selected", model.SELECT_FG)])
        style.configure(
            "Treeview.Heading",
            background="#f1f4f9",
            foreground=model.MUTED,
            borderwidth=0,
            relief="flat",
            padding=(8, 6),
            font=self.tiny_bold_font,
        )
        style.map("Treeview.Heading", background=[("active", "#e6ebf3")])
        for name in ("Vertical.TScrollbar", "Horizontal.TScrollbar"):
            style.configure(
                name,
                background="#ccd3e0",
                troughcolor="#f7f9fc",
                bordercolor="#f7f9fc",
                lightcolor="#ccd3e0",
                darkcolor="#ccd3e0",
                arrowcolor="#98a2b3",
                borderwidth=0,
                relief="flat",
                arrowsize=11,
                gripsize=0,
            )
            style.map(name, background=[("pressed", model.MUTED), ("active", "#b4bccc")])
        style.configure(
            "Scan.Horizontal.TProgressbar",
            troughcolor="#e7ebf2",
            background=model.ACCENT,
            lightcolor=model.ACCENT,
            darkcolor=model.ACCENT,
            borderwidth=0,
        )
        style.configure(
            "Veil.Horizontal.TProgressbar",
            troughcolor=model.VEIL_TROUGH,
            background=model.VEIL_ACCENT,
            lightcolor=model.VEIL_ACCENT,
            darkcolor=model.VEIL_ACCENT,
            borderwidth=0,
        )

    def _build_header(self) -> None:
        header = tk.Frame(self.root, bg=model.CARD_BG)
        header.pack(fill="x")
        inner = tk.Frame(header, bg=model.CARD_BG, padx=16, pady=10)
        inner.pack(fill="x")
        left = tk.Frame(inner, bg=model.CARD_BG)
        left.pack(side="left")
        tk.Label(left, text="rust-loc", bg=model.CARD_BG, fg=model.ACCENT, font=self.title_font).pack(side="left")
        tk.Label(left, text="v" + __version__, bg=model.CARD_BG, fg=model.MUTED, font=self.tiny_font).pack(
            side="left", padx=(6, 0), pady=(7, 0)
        )
        tk.Label(
            left,
            text="Rust business code statistics · test code excluded",
            bg=model.CARD_BG,
            fg=model.MUTED,
            font=self.small_font,
        ).pack(side="left", padx=(14, 0), pady=(3, 0))
        tk.Label(inner, textvariable=self.header_var, bg=model.CARD_BG, fg=model.TEXT, font=self.mono_small).pack(
            side="right", pady=(3, 0)
        )
        tk.Frame(self.root, bg=model.BORDER, height=1).pack(fill="x")

    def _build_status(self) -> None:
        bar = tk.Frame(self.root, bg=model.CARD_BG, padx=14, pady=6)
        bar.pack(fill="x", side="bottom")
        tk.Label(bar, textvariable=self.status_var, bg=model.CARD_BG, fg=model.MUTED, font=self.tiny_font, anchor="w").pack(
            side="left"
        )
        self.progress = ttk.Progressbar(bar, mode="determinate", style="Scan.Horizontal.TProgressbar", length=170)
        tk.Label(bar, textvariable=self.scope_var, bg=model.CARD_BG, fg=model.MUTED, font=self.tiny_font, anchor="e").pack(
            side="right"
        )
        tk.Frame(self.root, bg=model.BORDER, height=1).pack(fill="x", side="bottom")

    def _build_body(self) -> None:
        body = tk.Frame(self.root, bg=model.PANEL_BG)
        body.pack(fill="both", expand=True)
        sidebar = tk.Frame(body, bg=model.CARD_BG, width=316, highlightbackground=model.BORDER, highlightthickness=1)
        sidebar.pack(side="left", fill="y")
        sidebar.pack_propagate(False)
        self._build_sidebar(sidebar)
        panel = tk.Frame(body, bg=model.PANEL_BG)
        panel.pack(side="left", fill="both", expand=True, padx=12, pady=12)
        self._build_results(panel)

    def _build_sidebar(self, parent: tk.Frame) -> None:
        pad = tk.Frame(parent, bg=model.CARD_BG, padx=14, pady=12)
        pad.pack(fill="both", expand=True)

        self.export_menu = tk.Menu(self.root, tearoff=0, font=self.small_font)
        for key, label, _extension in EXPORT_FORMATS:
            self.export_menu.add_command(label="Export table as " + label, command=lambda k=key: self.export("table", k))
        self.export_menu.add_separator()
        for key, label, _extension in EXPORT_FORMATS:
            self.export_menu.add_command(
                label="Export summary as " + label, command=lambda k=key: self.export("summary", k)
            )
        self.export_menu.add_separator()
        self.export_menu.add_command(label="Copy summary as Markdown", command=self.copy_summary)
        self.export_menu.add_command(label="Copy equivalent CLI command", command=self.copy_command)

        actions = tk.Frame(pad, bg=model.CARD_BG)
        actions.pack(side="bottom", fill="x", pady=(12, 0))
        self.scan_button = ttk.Button(actions, text="Scan  (F5)", style="Accent.TButton", command=self.start_scan)
        self.scan_button.pack(fill="x")
        self.export_button = ttk.Menubutton(actions, text="Export…", style="Ghost.TMenubutton", menu=self.export_menu)
        self.export_button.pack(fill="x", pady=(6, 0))

        self._section(pad, "SCAN ROOTS")
        holder = tk.Frame(pad, bg=model.CARD_BG, highlightbackground=model.BORDER, highlightthickness=1)
        holder.pack(fill="x", pady=(4, 6))
        self.roots_list = tk.Listbox(
            holder,
            height=5,
            activestyle="none",
            borderwidth=0,
            highlightthickness=0,
            bg=model.GRID_BG,
            fg=model.TEXT,
            selectbackground=model.SELECT_BG,
            selectforeground=model.SELECT_FG,
            font=self.tiny_font,
            selectmode="extended",
        )
        self.roots_list.pack(fill="x")
        self.roots_list.bind("<Double-Button-1>", self._open_root)
        buttons = tk.Frame(pad, bg=model.CARD_BG)
        buttons.pack(fill="x")
        for index, (text, command) in enumerate(
            (("Add folder…", self.add_folder), ("Paste path…", self.add_path), ("Remove", self.remove_root))
        ):
            button = ttk.Button(buttons, text=text, style="Ghost.TButton", command=command)
            button.pack(side="left", fill="x", expand=True, padx=(0, 4 if index < 2 else 0))

        self._section(pad, "SCOPE")
        for text, variable in (
            ("Respect .gitignore", self.var_gitignore),
            ("Skip default ignored dirs", self.var_default_ignores),
            ("Include test code", self.var_include_tests),
            ("Complexity estimate", self.var_complexity),
            ("ULOC / DRYness", self.var_uloc),
        ):
            ttk.Checkbutton(
                pad, text=text, variable=variable, style="Card.TCheckbutton", command=self._on_option_change
            ).pack(anchor="w", fill="x")

        self._section(pad, "EXCLUDE GLOBS")
        exclude_entry = ttk.Entry(pad, textvariable=self.var_exclude, style="Field.TEntry")
        exclude_entry.pack(fill="x", pady=(4, 2))
        exclude_entry.bind("<Return>", lambda _event: self.start_scan())
        tk.Label(pad, text="e.g. */generated/*, src/legacy", bg=model.CARD_BG, fg=model.MUTED, font=self.tiny_font, anchor="w").pack(
            fill="x"
        )

        self._section(pad, "EXTRA IGNORED DIRS")
        dirs_entry = ttk.Entry(pad, textvariable=self.var_extra_dirs, style="Field.TEntry")
        dirs_entry.pack(fill="x", pady=(4, 2))
        dirs_entry.bind("<Return>", lambda _event: self.start_scan())
        tk.Label(pad, text="directory names skipped anywhere", bg=model.CARD_BG, fg=model.MUTED, font=self.tiny_font, anchor="w").pack(
            fill="x"
        )

    def _section(self, parent: tk.Frame, text: str) -> None:
        tk.Label(parent, text=text, bg=model.CARD_BG, fg=model.MUTED, font=self.tiny_bold_font, anchor="w").pack(
            fill="x", pady=(12, 0)
        )

    def _build_results(self, panel: tk.Frame) -> None:
        panel.columnconfigure(0, weight=1, uniform="charts")
        panel.columnconfigure(1, weight=1, uniform="charts")
        panel.rowconfigure(2, weight=1, minsize=200)
        self.canvas_composition = self._chart_card(panel, 0, "Line composition")
        self.canvas_modules = self._chart_card(panel, 1, "Top modules by code")
        self.cards_frame = tk.Frame(panel, bg=model.PANEL_BG)
        self.cards_frame.grid(row=1, column=0, columnspan=2, sticky="ew", pady=(10, 0))
        for column in range(CARD_COLUMNS):
            self.cards_frame.columnconfigure(column, weight=1, uniform="cards")
        self._build_table_card(panel)

    def _chart_card(self, parent: tk.Frame, column: int, title: str) -> tk.Canvas:
        card = tk.Frame(parent, bg=model.CARD_BG, highlightbackground=model.BORDER, highlightthickness=1)
        card.grid(row=0, column=column, sticky="nsew", padx=(0, 8) if column == 0 else (0, 0))
        head = tk.Frame(card, bg=model.CARD_BG)
        head.pack(fill="x", padx=12, pady=(9, 0))
        tk.Label(head, text=title, bg=model.CARD_BG, fg=model.TEXT, font=self.label_font, anchor="w").pack(side="left")
        canvas = tk.Canvas(card, bg=model.CARD_BG, highlightthickness=0, borderwidth=0, height=118)
        canvas.pack(fill="both", expand=True, padx=12, pady=(4, 10))
        canvas.bind("<Configure>", self._schedule_charts)
        return canvas

    def _build_table_card(self, panel: tk.Frame) -> None:
        card = tk.Frame(panel, bg=model.CARD_BG, highlightbackground=model.BORDER, highlightthickness=1)
        card.grid(row=2, column=0, columnspan=2, sticky="nsew", pady=(10, 0))
        head = tk.Frame(card, bg=model.CARD_BG)
        head.pack(fill="x", padx=12, pady=(10, 2))
        self.table_title = tk.Label(
            head, text=TABLE_TITLES["files"], bg=model.CARD_BG, fg=model.TEXT, font=self.label_font, anchor="w"
        )
        self.table_title.pack(side="left")
        right = tk.Frame(head, bg=model.CARD_BG)
        right.pack(side="right")
        ttk.Label(right, text="Filter", style="CardMuted.TLabel").pack(side="left", padx=(0, 6))
        self.search_entry = ttk.Entry(right, textvariable=self.search_var, style="Field.TEntry", width=24)
        self.search_entry.pack(side="left")
        ttk.Label(right, text="Top", style="CardMuted.TLabel").pack(side="left", padx=(10, 6))
        limit_box = ttk.Combobox(
            right, textvariable=self.limit_var, values=model.LIMIT_CHOICES, width=5, state="readonly", style="Field.TCombobox"
        )
        limit_box.pack(side="left")
        limit_box.bind("<<ComboboxSelected>>", lambda _event: self._fill_table())

        strip = tk.Frame(card, bg=model.CARD_BG)
        strip.pack(fill="x", padx=12, pady=(8, 0))
        for tab, title in TABS:
            cell = tk.Frame(strip, bg=model.CARD_BG)
            cell.pack(side="left", padx=(0, 4))
            label = tk.Label(
                cell,
                text=title,
                bg=model.CARD_BG,
                fg=model.MUTED,
                font=self.small_font,
                padx=12,
                pady=6,
                cursor="hand2",
            )
            label.pack(fill="x")
            rule = tk.Frame(cell, bg=model.CARD_BG, height=2)
            rule.pack(fill="x")
            label.bind("<Button-1>", lambda _event, name=tab: self._select_tab(name))
            label.bind("<Enter>", lambda _event, name=tab: self._hover_tab(name, True))
            label.bind("<Leave>", lambda _event, name=tab: self._hover_tab(name, False))
            self._tab_labels[tab] = label
            self._tab_underlines[tab] = rule
        tk.Frame(card, bg=model.BORDER, height=1).pack(fill="x")
        self.tab_area = tk.Frame(card, bg=model.CARD_BG)
        self.tab_area.pack(fill="both", expand=True, padx=8, pady=(4, 2))
        self.tab_area.rowconfigure(0, weight=1)
        self.tab_area.columnconfigure(0, weight=1)
        for tab, _title in TABS:
            area = tk.Frame(self.tab_area, bg=model.CARD_BG)
            area.grid(row=0, column=0, sticky="nsew")
            self._make_table(area, tab)
            self._tab_frames[tab] = area
        self._select_tab("files", force=True)
        self.table_hint = tk.Label(card, text="", bg=model.CARD_BG, fg=model.MUTED, font=self.tiny_font, anchor="w")
        self.table_hint.pack(fill="x", padx=12, pady=(2, 8))
        self._configure_tables()
        self.search_var.trace_add("write", self._on_search_change)

    def _make_table(self, parent: tk.Frame, tab: str) -> None:
        wrapper = tk.Frame(parent, bg=model.CARD_BG)
        wrapper.pack(fill="both", expand=True)
        tree = ttk.Treeview(wrapper, show="headings", selectmode="browse")
        yscroll = ttk.Scrollbar(wrapper, orient="vertical", command=tree.yview)
        xscroll = ttk.Scrollbar(wrapper, orient="horizontal", command=tree.xview)
        tree.configure(yscrollcommand=yscroll.set, xscrollcommand=xscroll.set)
        tree.grid(row=0, column=0, sticky="nsew")
        yscroll.grid(row=0, column=1, sticky="ns")
        xscroll.grid(row=1, column=0, sticky="ew")
        wrapper.rowconfigure(0, weight=1)
        wrapper.columnconfigure(0, weight=1)
        tree.tag_configure("odd", background=model.GRID_BG)
        tree.tag_configure("even", background=model.CARD_BG)
        tree.bind("<Double-Button-1>", self._on_row_open)
        tree.bind("<Return>", self._on_row_open)
        tree.bind("<Button-3>", self._on_row_menu)
        self._trees[tab] = tree

    def _bind_keys(self) -> None:
        self.root.bind("<F5>", lambda _event: self.start_scan())
        self.root.bind("<Control-o>", lambda _event: self.add_folder())
        self.root.bind("<Control-f>", lambda _event: self._focus_search())
        self.root.bind("<Control-e>", lambda _event: self._popup_export())
        self.root.bind("<Control-Tab>", lambda _event: self._cycle_tab(1))
        self.root.bind("<Escape>", self._on_escape)
        self.root.bind("<Configure>", self._on_root_configure)
        self.root.bind("<Map>", self._on_root_map)
        self.root.bind("<Unmap>", self._on_root_unmap)

    # -------------------------------------------------------------- tab state
    def _active_tab(self) -> str:
        return self._active

    def _select_tab(self, tab: str, force: bool = False) -> None:
        # Switch the visible table and repaint the segmented tab strip.
        if tab not in self._tab_frames:
            return
        if tab == self._active and not force:
            return
        self._active = tab
        for name, area in self._tab_frames.items():
            if name == tab:
                area.grid()
            else:
                area.grid_remove()
        for name, label in self._tab_labels.items():
            active = name == tab
            label.configure(fg=model.ACCENT if active else model.MUTED)
            self._tab_underlines[name].configure(bg=model.ACCENT if active else model.CARD_BG)
        self._on_tab_change()

    def _hover_tab(self, tab: str, entering: bool) -> None:
        if tab == self._active:
            return
        label = self._tab_labels.get(tab)
        if label is not None:
            label.configure(fg=model.TEXT if entering else model.MUTED)

    def _cycle_tab(self, step: int) -> None:
        names = [name for name, _title in TABS]
        self._select_tab(names[(names.index(self._active) + step) % len(names)])

    def _tab_columns(self, tab: str):
        if tab == "excluded":
            return model.label_columns(tab, model.EXCLUDED_COLUMNS, "Test-only file")
        title = "File" if tab == "files" else "Module"
        return model.label_columns(tab, self._columns, title)

    def _tab_keys(self, tab: str) -> list:
        return model.column_keys(self._tab_columns(tab))[1:]

    def _tab_titles(self, tab: str) -> dict:
        return {key: title for key, title, _width in self._tab_columns(tab)}

    def _on_tab_change(self, _event=None) -> None:
        if self.report is None or not self._trees:
            return
        tab = self._active_tab()
        self.table_title.configure(text=TABLE_TITLES[tab])
        self._fill_table()

    def _configure_tables(self) -> None:
        for tab, _title in TABS:
            tree = self._trees[tab]
            specs = self._tab_columns(tab)
            tree.configure(columns=[key for key, _t, _w in specs], show="headings")
            for key, title, width in specs:
                tree.heading(key, text=title, command=lambda k=key, t=tab: self._on_sort(t, k))
                tree.column(
                    key,
                    width=width,
                    minwidth=60,
                    stretch=key == model.LABEL_KEY,
                    anchor="w" if key == model.LABEL_KEY else "e",
                )

    def _on_sort(self, tab: str, key: str) -> None:
        if tab != self._active_tab():
            return
        if self._sort_key == key:
            self._sort_desc = not self._sort_desc
        else:
            self._sort_key = key
            self._sort_desc = key != model.LABEL_KEY
        self._fill_table()

    def _fill_table(self) -> None:
        if self.report is None:
            return
        tab = self._active_tab()
        tree = self._trees[tab]
        keys = self._tab_keys(tab)
        rows = model.filter_rows(self._rows[tab], self.search_var.get())
        matched = len(rows)
        column = keys.index(self._sort_key) if self._sort_key in keys else -1
        descending = self._sort_desc if column >= 0 else False
        rows = model.sort_rows(rows, column, descending)
        limit = model.parse_limit(self.limit_var.get())
        shown = rows if tab == "excluded" or not limit else model.limit_rows(rows, limit)

        tree.delete(*tree.get_children())
        for index, (label, values) in enumerate(shown):
            tree.insert("", "end", values=(label, *values), tags=("odd" if index % 2 else "even",))
        self._update_headings(tab, keys)
        self.table_hint.configure(text=self._hint_for(tab, len(shown), matched))

    def _update_headings(self, tab: str, keys) -> None:
        tree = self._trees[tab]
        arrow = " ▼" if self._sort_desc else " ▲"
        for key, title, _width in self._tab_columns(tab):
            tree.heading(key, text=title + (arrow if key == self._sort_key else ""))

    def _hint_for(self, tab: str, shown: int, matched: int) -> str:
        if tab == "excluded":
            excluded = (self.report or {}).get("excluded") or {}
            files = int(excluded.get("files") or 0)
            if not files:
                return "no test-only files were detected"
            return (
                f"{model.human_int(files)} test-only files · "
                f"{model.human_int(excluded.get('file_lines'))} lines · plus "
                f"{model.human_int(excluded.get('item_lines'))} lines inside #[cfg(test)] items"
            )
        return model.table_hint(shown, matched, self._sort_key, self._sort_desc, self._tab_titles(tab))

    def _on_search_change(self, *_args) -> None:
        if self._search_job:
            self.root.after_cancel(self._search_job)
        self._search_job = self.root.after(180, self._run_search)

    def _run_search(self) -> None:
        self._search_job = None
        self._fill_table()

    def _focus_search(self) -> None:
        self.search_entry.focus_set()

    def _on_escape(self, _event=None) -> None:
        if self.search_var.get():
            self.search_var.set("")

    # ------------------------------------------------------------------ scan
    def _collect_options(self):
        ignore_dirs = set(cli.DEFAULT_IGNORE_DIRS) if self.var_default_ignores.get() else set()
        ignore_dirs |= set(model.parse_list(self.var_extra_dirs.get()))
        return cli.make_options(
            include_tests=self.var_include_tests.get(),
            exclude=model.parse_list(self.var_exclude.get()),
            ignore_dirs=ignore_dirs,
            gitignore=self.var_gitignore.get(),
            uloc=self.var_uloc.get(),
            complexity=self.var_complexity.get(),
            show_excluded=True,
        )

    def _on_option_change(self) -> None:
        if self.report is not None:
            self._schedule_rescan()

    def _schedule_rescan(self) -> None:
        if self._rescan_job:
            self.root.after_cancel(self._rescan_job)
        self._rescan_job = self.root.after(320, self._rescan_now)

    def _rescan_now(self) -> None:
        self._rescan_job = None
        self.start_scan()

    def start_scan(self) -> None:
        if self.worker is not None:
            return
        roots = list(self.roots)
        if not roots:
            messagebox.showwarning("No scan roots", "Add at least one folder to scan.", parent=self.root)
            return
        missing = [str(root) for root in roots if not root.exists()]
        if missing:
            messagebox.showerror("Folder not found", "\n".join(missing), parent=self.root)
            return
        self.options = self._collect_options()
        self.queue = queue.Queue()
        self._scan_started = time.perf_counter()
        self.scan_button.configure(state="disabled", text="Scanning…")
        self.progress.configure(maximum=1, value=0)
        self._show_progress(True)
        self._set_status("Scanning…")
        self._schedule_overlay()
        self.worker = threading.Thread(target=self._run_scan, args=(roots, self.options), daemon=True)
        self.worker.start()
        self.root.after(90, self._poll)

    def _run_scan(self, roots, options) -> None:
        try:
            report = cli.scan(roots, options, self.cwd, on_progress=self._report_progress)
        except Exception as exc:  # surfaced in the window instead of dying silently
            self.queue.put(("error", exc))
            return
        self.queue.put(("done", report))

    def _report_progress(self, done: int, total: int, path) -> None:
        if done == total or done % 5 == 0:
            self.queue.put(("progress", done, total, str(path)))

    def _poll(self) -> None:
        latest = None
        payload = None
        try:
            while True:
                message = self.queue.get_nowait()
                if message[0] == "progress":
                    latest = message
                else:
                    payload = message
                    break
        except queue.Empty:
            pass

        if latest is not None and payload is None:
            _tag, done, total, path = latest
            self.progress.configure(maximum=max(total, 1), value=done)
            self._set_overlay_progress(done, total, path)
            short = path if len(path) <= 46 else "…" + path[-45:]
            self._set_status(f"Scanning… {done:,}/{total:,} files — {short}")

        if payload is None:
            self.root.after(90, self._poll)
            return

        self.worker = None
        self.scan_button.configure(state="normal", text="Scan  (F5)")
        self._show_progress(False)
        self._hide_overlay()
        if payload[0] == "error":
            self._set_status("Scan failed")
            messagebox.showerror("Scan failed", str(payload[1]), parent=self.root)
            return
        self._apply_report(payload[1], time.perf_counter() - self._scan_started)
        self._save_settings()

    def _apply_report(self, report: dict, elapsed: float) -> None:
        self.report = report
        options = report.get("options") or self.options
        self._columns = model.table_columns(options.complexity, options.uloc)
        self._keys = model.column_keys(self._columns)
        self._rows = {
            "files": model.file_rows(report["business"], self.cwd, self._keys),
            "modules": model.group_rows(report["business"], self.cwd, self._keys),
            "excluded": model.excluded_rows(report["excluded"]),
        }
        self._modules = model.chart_modules(report["business"], self.cwd, CHART_ROWS)
        self._configure_tables()
        self._render_cards(model.summary_cards(report))
        self._fill_table()
        self._draw_charts()
        self.header_var.set(model.header_summary(report))
        self.scope_var.set(model.scope_line(options, report["roots"]))
        self._set_status(
            f"Scanned {model.human_int(len(report['entries']))} files in {elapsed:.2f} s"
            f" · {model.human_int(model.test_line_total(report['excluded']))} test lines kept out"
        )

    # ---------------------------------------------------------------- render
    def _render_cards(self, cards) -> None:
        keys = [card["key"] for card in cards]
        if keys != list(self._card_widgets):
            for widget in self._card_widgets.values():
                widget["frame"].destroy()
            self._card_widgets = {}
            for index, card in enumerate(cards):
                row, column = divmod(index, CARD_COLUMNS)
                frame = tk.Frame(
                    self.cards_frame, bg=model.CARD_BG, highlightbackground=model.BORDER, highlightthickness=1
                )
                frame.grid(row=row, column=column, sticky="nsew", padx=(0, 8), pady=(0, 8))
                tk.Frame(frame, bg=model.TONE.get(card["key"], model.ACCENT), width=3).pack(side="left", fill="y")
                inner = tk.Frame(frame, bg=model.CARD_BG, padx=10, pady=7)
                inner.pack(side="left", fill="both", expand=True)
                tk.Label(
                    inner, text=card["label"].upper(), bg=model.CARD_BG, fg=model.MUTED, font=self.tiny_bold_font, anchor="w"
                ).pack(fill="x")
                value = tk.Label(inner, text="—", bg=model.CARD_BG, fg=model.TEXT, font=self.value_font, anchor="w")
                value.pack(fill="x")
                hint = tk.Label(inner, text="", bg=model.CARD_BG, fg=model.MUTED, font=self.tiny_font, anchor="w")
                hint.pack(fill="x")
                self._card_widgets[card["key"]] = {"frame": frame, "value": value, "hint": hint}
        for card in cards:
            widget = self._card_widgets[card["key"]]
            widget["value"].configure(text=card["value"])
            widget["hint"].configure(text=card["hint"])

    def _schedule_charts(self, _event=None) -> None:
        if self._chart_job:
            self.root.after_cancel(self._chart_job)
        self._chart_job = self.root.after(70, self._draw_charts)

    def _draw_charts(self) -> None:
        self._chart_job = None
        self._draw_composition()
        self._draw_modules()

    def _draw_composition(self) -> None:
        canvas = self.canvas_composition
        canvas.delete("all")
        width = canvas.winfo_width()
        height = canvas.winfo_height()
        if width < 80 or height < 40:
            return
        segments = model.composition((self.report or {}).get("summary") or {})
        total = sum(value for _label, value, _color in segments)
        x0, x1 = 2, width - 2
        bar_h = 22
        if total <= 0:
            canvas.create_text(x0, 12, anchor="nw", text="Run a scan to see the line mix", fill=model.MUTED, font=self.tiny_font)
            return
        cursor = x0
        for _label, value, color in segments:
            if value <= 0:
                continue
            span = (x1 - x0) * value / total
            canvas.create_rectangle(cursor, 4, cursor + span, 4 + bar_h, fill=color, outline="")
            cursor += span
        legend_top = 4 + bar_h + 12
        column_width = (x1 - x0) / 2
        for index, (label, value, color) in enumerate(segments):
            row, column = divmod(index, 2)
            left = x0 + column * column_width
            top = legend_top + row * 21
            if top + 14 > height:
                break
            canvas.create_rectangle(left, top + 3, left + 9, top + 12, fill=color, outline="")
            canvas.create_text(left + 15, top + 7, anchor="w", text=label, fill=model.MUTED, font=self.tiny_font)
            canvas.create_text(
                left + column_width - 10,
                top + 7,
                anchor="e",
                text=model.human_int(value) + "   " + model.human_share(value, total),
                fill=model.TEXT,
                font=self.small_bold_font,
            )

    def _draw_modules(self) -> None:
        canvas = self.canvas_modules
        canvas.delete("all")
        width = canvas.winfo_width()
        height = canvas.winfo_height()
        if width < 100 or height < 40:
            return
        modules = list(self._modules)
        if not modules:
            canvas.create_text(2, 12, anchor="nw", text="No data yet", fill=model.MUTED, font=self.tiny_font)
            return
        row_h = 20
        visible = max(1, min(len(modules), int((height - 4) // row_h)))
        modules = modules[:visible]
        peak = max(value for _label, value in modules) or 1
        value_width = max(self.small_bold_font.measure(model.human_int(value)) for _label, value in modules)
        label_width = int(min(width * 0.42, max(self.tiny_font.measure(label) for label, _value in modules)))
        track_x0 = 2 + label_width + 8
        track_x1 = max(track_x0 + 40, width - value_width - 12)
        for index, (label, value) in enumerate(modules):
            top = 4 + index * row_h
            middle = top + row_h / 2 - 1
            canvas.create_text(
                2, middle, anchor="w", text=_ellipsize(label, self.tiny_font, label_width), fill=model.TEXT, font=self.tiny_font
            )
            canvas.create_rectangle(track_x0, middle - 5, track_x1, middle + 5, fill="#f1f4f9", outline="")
            span = (track_x1 - track_x0) * (value / peak)
            if span > 0:
                canvas.create_rectangle(track_x0, middle - 5, track_x0 + span, middle + 5, fill=model.ACCENT, outline="")
            canvas.create_text(
                width - 2,
                middle,
                anchor="e",
                text=model.human_int(value),
                fill=model.MUTED,
                font=self.small_bold_font,
            )

    # ---------------------------------------------------------------- actions
    def _refresh_roots(self) -> None:
        self.roots_list.delete(0, "end")
        for root in self.roots:
            self.roots_list.insert("end", model.display_path(root, self.cwd))

    def add_folder(self) -> None:
        initial = str(self.roots[-1]) if self.roots else str(self.cwd)
        chosen = filedialog.askdirectory(parent=self.root, title="Add scan root", initialdir=initial, mustexist=True)
        if not chosen:
            return
        path = Path(chosen).resolve()
        if path not in self.roots:
            self.roots.append(path)
            self._refresh_roots()
        self._schedule_rescan()

    def add_path(self) -> None:
        answer = simpledialog.askstring(
            "Add roots", "Paste one or more folders (separated by ; or newlines):", parent=self.root
        )
        if not answer:
            return
        added = 0
        for raw in model.parse_paths(answer):
            path = Path(raw)
            if not path.is_absolute():
                path = self.cwd / path
            if not path.exists():
                messagebox.showwarning("Folder not found", str(raw), parent=self.root)
                continue
            path = path.resolve()
            if path not in self.roots:
                self.roots.append(path)
                added += 1
        if added:
            self._refresh_roots()
            self._schedule_rescan()

    def remove_root(self) -> None:
        selection = list(self.roots_list.curselection())
        if not selection:
            return
        for index in reversed(selection):
            del self.roots[index]
        self._refresh_roots()
        self._schedule_rescan()

    def _open_root(self, _event=None) -> None:
        selection = self.roots_list.curselection()
        if selection:
            self._open_path(self.roots[selection[0]])

    def _open_path(self, path) -> None:
        try:
            os.startfile(str(path))  # noqa: S606 - Windows shell open
        except AttributeError:
            messagebox.showinfo("Path", str(path), parent=self.root)
        except OSError as exc:
            messagebox.showerror("Cannot open", f"{path}\n{exc}", parent=self.root)

    def _on_row_open(self, _event=None) -> None:
        tree = self._trees[self._active_tab()]
        selection = tree.selection()
        if selection:
            self._open_label(tree.item(selection[0], "values")[0])

    def _open_label(self, label: str) -> None:
        for entry in (self.report or {}).get("entries", []):
            if model.display_path(entry["path"], self.cwd) == label:
                self._open_path(entry["path"])
                return
        candidate = Path(label)
        if not candidate.is_absolute():
            candidate = self.cwd / candidate
        if candidate.exists():
            self._open_path(candidate)
        else:
            self._set_status(label)

    def _on_row_menu(self, event) -> None:
        tree = event.widget
        row = tree.identify_row(event.y)
        if not row:
            return
        tree.selection_set(row)
        values = tree.item(row, "values")
        menu = tk.Menu(self.root, tearoff=0, font=self.small_font)
        menu.add_command(label="Open", command=lambda: self._open_label(values[0]))
        menu.add_command(label="Copy path", command=lambda: self._copy(str(values[0])))
        menu.add_command(label="Copy row", command=lambda: self._copy("\t".join(str(value) for value in values)))
        try:
            menu.tk_popup(event.x_root, event.y_root)
        finally:
            menu.grab_release()

    def _copy(self, text: str) -> None:
        self.root.clipboard_clear()
        self.root.clipboard_append(text)
        self._set_status("Copied to clipboard")

    def copy_summary(self) -> None:
        if self.report is None:
            messagebox.showinfo("Nothing to copy", "Run a scan first.", parent=self.root)
            return
        options = self._export_options("summary")
        self._copy(cli.render_as(options, self.cwd, self.report, "markdown"))
        self._set_status("Summary copied as Markdown")

    def copy_command(self) -> None:
        self._copy(model.cli_command(self.options, self.roots, self.cwd))
        self._set_status("Equivalent CLI command copied")

    def export(self, target: str, fmt: str) -> None:
        if self.report is None:
            messagebox.showinfo("Nothing to export", "Run a scan first.", parent=self.root)
            return
        extension = dict((key, ext) for key, _label, ext in EXPORT_FORMATS)[fmt]
        path = filedialog.asksaveasfilename(
            parent=self.root,
            title="Export report",
            initialfile=f"rust-loc-{target}{extension}",
            defaultextension=extension,
            filetypes=[(fmt.upper(), "*" + extension), ("All files", "*.*")],
        )
        if not path:
            return
        options = self._export_options("table" if target == "table" else "summary")
        try:
            text = cli.render_as(options, self.cwd, self.report, fmt)
            Path(path).write_text(text if text.endswith("\n") else text + "\n", encoding="utf-8")
        except OSError as exc:
            messagebox.showerror("Export failed", str(exc), parent=self.root)
            return
        self._set_status("Exported " + target + " as " + fmt.upper() + " → " + path)

    def _export_options(self, target: str):
        base = self._collect_options()
        if target == "table":
            base.by_file = self._active_tab() == "files"
            base.by_module = self._active_tab() == "modules"
        return base

    def _popup_export(self) -> None:
        try:
            self.export_menu.tk_popup(self.root.winfo_pointerx(), self.root.winfo_pointery())
        finally:
            self.export_menu.grab_release()

    def _set_status(self, text: str) -> None:
        self.status_var.set(text)

    def _show_progress(self, visible: bool) -> None:
        if visible:
            self.progress.pack(side="right", padx=(10, 0))
        else:
            self.progress.pack_forget()

    # ---------------------------------------------------------------- overlay
    def _ensure_overlay(self) -> tk.Toplevel:
        """Lazily build the translucent scan mask (a borderless top level)."""
        if self.overlay is not None and self.overlay.winfo_exists():
            return self.overlay
        overlay = tk.Toplevel(self.root)
        overlay.withdraw()
        overlay.overrideredirect(True)
        overlay.configure(bg=model.VEIL)
        try:
            overlay.attributes("-alpha", 0.82)
        except tk.TclError:
            pass
        body = tk.Frame(overlay, bg=model.VEIL, padx=36, pady=28)
        body.place(relx=0.5, rely=0.5, anchor="center")
        tk.Label(
            body,
            text="Scanning Rust sources",
            bg=model.VEIL,
            fg=model.VEIL_TEXT,
            font=self.overlay_font,
            anchor="w",
        ).pack(fill="x")
        self.overlay_count = tk.Label(
            body, text="starting…", bg=model.VEIL, fg=model.VEIL_MUTED, font=self.small_font, anchor="w"
        )
        self.overlay_count.pack(fill="x", pady=(4, 10))
        self.overlay_bar = ttk.Progressbar(
            body, style="Veil.Horizontal.TProgressbar", mode="determinate", maximum=1, value=0, length=430
        )
        self.overlay_bar.pack(fill="x")
        self.overlay_path = tk.Label(
            body, text="", bg=model.VEIL, fg=model.VEIL_MUTED, font=self.tiny_font, anchor="w", width=72
        )
        self.overlay_path.pack(fill="x", pady=(8, 0))
        self.overlay = overlay
        return overlay

    def _schedule_overlay(self) -> None:
        """Show the mask only when the scan is slow enough to be noticeable."""
        if self._overlay_job:
            self.root.after_cancel(self._overlay_job)
        self._overlay_job = self.root.after(OVERLAY_DELAY_MS, self._show_overlay_if_scanning)

    def _show_overlay_if_scanning(self) -> None:
        self._overlay_job = None
        if self.worker is not None:
            self._show_overlay()

    def _show_overlay(self) -> None:
        overlay = self._ensure_overlay()
        self._sync_overlay()
        if self.root.state() == "iconic":
            return
        overlay.deiconify()
        overlay.lift()

    def _hide_overlay(self) -> None:
        if self._overlay_job:
            self.root.after_cancel(self._overlay_job)
            self._overlay_job = None
        if self.overlay is not None and self.overlay.winfo_exists():
            self.overlay.withdraw()

    def _sync_overlay(self, _event=None) -> None:
        """Keep the mask exactly on top of the window's client area."""
        self._sync_job = None
        overlay = self.overlay
        if overlay is None or not overlay.winfo_exists():
            return
        if self.root.state() == "iconic":
            overlay.withdraw()
            return
        overlay.geometry(
            "{}x{}+{}+{}".format(
                self.root.winfo_width(),
                self.root.winfo_height(),
                self.root.winfo_rootx(),
                self.root.winfo_rooty(),
            )
        )

    def _on_root_configure(self, event) -> None:
        if event.widget is not self.root or self.overlay is None:
            return
        if self._sync_job:
            self.root.after_cancel(self._sync_job)
        self._sync_job = self.root.after(60, self._sync_overlay)

    def _on_root_map(self, event) -> None:
        if event.widget is self.root and self.worker is not None:
            self._show_overlay()

    def _on_root_unmap(self, event) -> None:
        if event.widget is self.root and self.overlay is not None and self.overlay.winfo_exists():
            self.overlay.withdraw()

    def _set_overlay_progress(self, done: int, total: int, path: str) -> None:
        if self.overlay is None or not self.overlay.winfo_exists():
            return
        self.overlay_bar.configure(maximum=max(total, 1), value=done)
        self.overlay_count.configure(text=model.human_int(done) + " / " + model.human_int(total) + " files")
        self.overlay_path.configure(text=_ellipsize(path, self.tiny_font, 400))

    # --------------------------------------------------------------- settings
    def _state(self) -> dict:
        return {
            "roots": [str(root) for root in self.roots],
            "gitignore": bool(self.var_gitignore.get()),
            "default_ignores": bool(self.var_default_ignores.get()),
            "include_tests": bool(self.var_include_tests.get()),
            "complexity": bool(self.var_complexity.get()),
            "uloc": bool(self.var_uloc.get()),
            "exclude": self.var_exclude.get(),
            "extra_ignore_dirs": self.var_extra_dirs.get(),
            "limit": self.limit_var.get(),
        }

    def _save_settings(self) -> None:
        model.save_settings(self._state())

    def _seed_from_options(self, options) -> dict:
        """Only command line values that differ from the defaults count as intent.

        A flag equal to its default carries no information, so a previously
        saved UI state keeps winning in that case.
        """
        defaults = cli.make_options()
        seed: dict = {}
        for key in ("include_tests", "uloc", "complexity", "gitignore"):
            if getattr(options, key) != getattr(defaults, key):
                seed[key] = bool(getattr(options, key))
        if not set(defaults.ignore_dirs) <= set(options.ignore_dirs):
            seed["default_ignores"] = False
        extra = sorted(set(options.ignore_dirs) - set(defaults.ignore_dirs))
        if extra:
            seed["extra_ignore_dirs"] = ", ".join(extra)
        if options.exclude:
            seed["exclude"] = ", ".join(options.exclude)
        return seed

    def _load_settings(self) -> None:
        state = dict(model.load_settings())
        state.update(self._seed_from_options(self.initial_options))
        for key, variable in (
            ("gitignore", self.var_gitignore),
            ("default_ignores", self.var_default_ignores),
            ("include_tests", self.var_include_tests),
            ("complexity", self.var_complexity),
            ("uloc", self.var_uloc),
        ):
            if isinstance(state.get(key), bool):
                variable.set(state[key])
        for key, variable in (("exclude", self.var_exclude), ("extra_ignore_dirs", self.var_extra_dirs)):
            if isinstance(state.get(key), str):
                variable.set(state[key])
        if state.get("limit") in model.LIMIT_CHOICES:
            self.limit_var.set(state["limit"])
        if not self.roots or not self.roots_explicit:
            saved = [Path(str(raw)) for raw in state.get("roots", []) or []]
            existing = [path for path in saved if path.exists()]
            if existing:
                self.roots = existing
        if not self.roots:
            self.roots = [self.cwd]
        self._refresh_roots()
        self.options = self._collect_options()

    def close(self) -> None:
        self._hide_overlay()
        self._save_settings()
        self.root.destroy()


def run(roots=None, options=None, cwd=None, roots_explicit: bool = False) -> int:
    """Open the desktop UI and block until the window is closed."""
    _enable_dpi_awareness()
    try:
        root = tk.Tk()
    except tk.TclError as exc:
        print("rust-loc: cannot open the desktop UI (" + str(exc) + ")", file=sys.stderr)
        return 1
    root.title(WINDOW_TITLE)
    root.configure(bg=model.PANEL_BG)
    _place_window(root, 1240, 850)
    app = LocApp(
        root,
        roots,
        options or cli.make_options(),
        Path(cwd) if cwd else Path.cwd(),
        roots_explicit=roots_explicit,
    )
    root.after(150, app.start_scan)
    root.mainloop()
    return app.exit_code


def main(argv=None) -> int:
    """Entry point for the rust-loc-ui GUI script."""
    arguments = list(sys.argv[1:] if argv is None else argv)
    cwd = Path.cwd()
    roots = []
    for raw in arguments:
        path = Path(raw)
        if not path.is_absolute():
            path = cwd / path
        if path.exists():
            roots.append(path.resolve())
    return run(roots=roots or None, options=cli.make_options(), cwd=cwd, roots_explicit=bool(roots))


if __name__ == "__main__":
    raise SystemExit(main())
