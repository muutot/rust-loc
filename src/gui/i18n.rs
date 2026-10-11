//! UI language catalog: English and Chinese, Chinese by default.
//!
//! Every user visible string lives here, once per language, so the shell can
//! switch language at run time without any resource file or dependency.

use std::cell::Cell;

use crate::gui::sys::GetUserDefaultUILanguage;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    En,
    Zh,
}

/// Languages in the order the selector offers them.
pub const LANGS: [Lang; 2] = [Lang::Zh, Lang::En];

impl Lang {
    #[cfg(test)]
    pub fn from_code(value: &str) -> Lang {
        match value.trim().to_ascii_lowercase().as_str() {
            "en" | "en-us" | "en-gb" | "english" => Lang::En,
            _ => Lang::Zh,
        }
    }

    pub fn from_index(index: usize) -> Lang {
        *LANGS.get(index).unwrap_or(&Lang::Zh)
    }

    pub fn index(self) -> usize {
        LANGS.iter().position(|lang| *lang == self).unwrap_or(0)
    }

    /// Name of the language in that language, for the selector.
    pub fn native(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Zh => "中文",
        }
    }
}

thread_local! {
    static CURRENT: Cell<Lang> = const { Cell::new(Lang::Zh) };
}

pub fn set_lang(lang: Lang) {
    CURRENT.with(|cell| cell.set(lang));
}

pub fn lang() -> Lang {
    CURRENT.with(|cell| cell.get())
}

/// The header subtitle: it names the languages the current selection reads, so
/// a Python-only or mixed run does not claim to have counted Rust, and says
/// whether test code made it into the totals.
pub fn subtitle(sources: crate::sources::Sources, include_tests: bool) -> String {
    let catalog = t();
    let (en, zh) = match sources {
        crate::sources::Sources::Rust => ("Rust business code statistics", "Rust 业务代码统计"),
        crate::sources::Sources::Python => {
            ("Python business code statistics", "Python 业务代码统计")
        }
        crate::sources::Sources::All => (
            "Rust and Python business code statistics",
            "Rust 与 Python 业务代码统计",
        ),
    };
    // The tail follows the switch: a scan that keeps test code in its totals
    // must not claim to have left it out.
    let tail = if include_tests {
        catalog.subtitle_tests_included
    } else {
        catalog.subtitle_tests_excluded
    };
    let head = match lang() {
        Lang::En => en,
        Lang::Zh => zh,
    };
    format!("{} · {}", head, tail)
}

/// The extension the file card counts: what the selection read, not the
/// language this tool happens to be written in.
pub fn extensions(sources: crate::sources::Sources) -> &'static str {
    match sources {
        crate::sources::Sources::Rust => ".rs",
        crate::sources::Sources::Python => ".py",
        crate::sources::Sources::All => ".rs / .py",
    }
}

/// What the selected language calls the test item the `Test fns` card counted.
pub fn test_item(sources: crate::sources::Sources) -> &'static str {
    let (en, zh) = match sources {
        crate::sources::Sources::Rust => ("#[test] functions", "#[test] 函数"),
        crate::sources::Sources::Python => ("def test_ functions", "def test_ 函数"),
        crate::sources::Sources::All => ("test functions", "测试函数"),
    };
    match lang() {
        Lang::En => en,
        Lang::Zh => zh,
    }
}

/// The catalog for the active language.
pub fn t() -> &'static Strings {
    match lang() {
        Lang::En => &EN,
        Lang::Zh => &ZH,
    }
}

/// Declare one field per string, with the English and Chinese text together so
/// the two languages cannot drift apart.
macro_rules! catalog {
    ($( $name:ident => ($en:expr, $zh:expr) ),* $(,)?) => {
        #[allow(dead_code)]
        pub struct Strings {
            $( pub $name: &'static str, )*
        }
        pub const EN: Strings = Strings { $( $name: $en, )* };
        pub const ZH: Strings = Strings { $( $name: $zh, )* };
    };
}

/// Replace `{name}` placeholders; unknown braces are left alone.
pub fn fill(template: &str, parts: &[(&str, String)]) -> String {
    let mut out = template.to_string();
    for (key, value) in parts {
        out = out.replace(&format!("{{{key}}}"), value);
    }
    out
}

impl Strings {
    /// Root count with the plural forms English needs.
    pub fn roots(&self, n: usize) -> String {
        let template = if n == 1 {
            self.scope_root_one
        } else {
            self.scope_root_many
        };
        fill(template, &[("n", n.to_string())])
    }

    pub fn added_roots(&self, n: usize) -> String {
        let template = if n == 1 {
            self.status_added_one
        } else {
            self.status_added_many
        };
        fill(template, &[("n", n.to_string())])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_replaces_placeholders() {
        let text = fill("a {x} b {x}", &[("x", "1".to_string())]);
        assert_eq!(text, "a 1 b 1");
    }

    #[test]
    fn language_round_trips() {
        assert_eq!(Lang::from_code("EN"), Lang::En);
        assert_eq!(Lang::from_code("zh"), Lang::Zh);
        assert_eq!(Lang::from_code("nonsense"), Lang::Zh);
        assert_eq!(Lang::from_index(1), Lang::En);
        assert_eq!(Lang::En.index(), 1);
    }

    #[test]
    fn the_subtitle_follows_the_test_switch() {
        let sources = crate::sources::Sources::Rust;
        set_lang(Lang::Zh);
        assert!(subtitle(sources, false).ends_with("已排除测试代码"));
        assert!(subtitle(sources, true).ends_with("已包含测试代码"));
        set_lang(Lang::En);
        assert!(subtitle(sources, true).ends_with("test code included"));
        assert!(subtitle(sources, false).ends_with("test code excluded"));
        set_lang(Lang::Zh);
    }

    #[test]
    fn the_scan_hints_speak_the_language_that_was_read() {
        use crate::sources::Sources;
        set_lang(Lang::En);
        assert_eq!(extensions(Sources::Python), ".py");
        assert_eq!(extensions(Sources::All), ".rs / .py");
        assert_eq!(test_item(Sources::Rust), "#[test] functions");
        assert_eq!(test_item(Sources::Python), "def test_ functions");
        assert_eq!(test_item(Sources::All), "test functions");
        set_lang(Lang::Zh);
        assert_eq!(test_item(Sources::Rust), "#[test] 函数");
        assert_eq!(test_item(Sources::Python), "def test_ 函数");
        assert_eq!(test_item(Sources::All), "测试函数");
    }

    #[test]
    fn catalog_lookup_switches_language() {
        set_lang(Lang::En);
        assert_eq!(t().button_scan, "Scan  (F5)");
        set_lang(Lang::Zh);
        assert_eq!(t().section_language, "界面语言");
    }
}

/// Language of the current Windows user; anything Chinese counts as Chinese,
/// every other UI language falls back to English.
pub fn system_lang() -> Lang {
    let id = unsafe { GetUserDefaultUILanguage() } as u32;
    // The primary language id lives in the low ten bits: LANG_CHINESE == 4.
    if id & 0x3FF == 4 {
        Lang::Zh
    } else {
        Lang::En
    }
}

catalog! {
    // -- shell -------------------------------------------------------------
    window_title => ("rust-loc — source code statistics", "rust-loc — 代码统计"),
    no_scan_yet => ("no scan yet", "尚未扫描"),
    subtitle_tests_excluded => ("test code excluded", "已排除测试代码"),
    subtitle_tests_included => ("test code included", "已包含测试代码"),
    header_code => ("code", "代码"),
    header_comment => ("comment", "注释"),
    header_blank => ("blank", "空行"),
    // -- sidebar -----------------------------------------------------------
    section_roots => ("SCAN ROOTS", "扫描根目录"),
    add_folder => ("Add folder…", "添加文件夹…"),
    paste_path => ("Paste path…", "粘贴路径…"),
    remove => ("Remove", "移除"),
    section_sources => ("SOURCES", "代码来源"),
    section_scope => ("SCOPE", "统计范围"),
    opt_gitignore => ("Respect .gitignore", "遵循 .gitignore"),
    opt_default_ignores => ("Skip default ignored dirs", "跳过默认忽略目录"),
    opt_include_tests => ("Include test code", "包含测试代码"),
    opt_complexity => ("Complexity estimate", "复杂度估算"),
    opt_uloc => ("ULOC / DRYness", "ULOC / 重复率"),
    section_report => ("REPORT OPTIONS", "报告选项"),
    opt_exclude_generated => ("Skip generated / minified", "跳过生成或压缩文件"),
    opt_cocomo => ("COCOMO cost estimate", "COCOMO 成本估算"),
    opt_authors => ("Author blame (git)", "作者归属（git blame）"),
    section_exclude => ("EXCLUDE GLOBS", "排除通配符"),
    hint_exclude => ("e.g. */generated/*, src/legacy", "例如 */generated/*, src/legacy"),
    section_extra_dirs => ("EXTRA IGNORED DIRS", "额外忽略目录"),
    hint_extra_dirs => ("directory names skipped anywhere", "任意层级中同名的目录都会被跳过"),
    section_language => ("LANGUAGE", "界面语言"),
    button_scan => ("Scan  (F5)", "开始扫描  (F5)"),
    button_export => ("Export…", "导出…"),
    // -- charts ------------------------------------------------------------
    chart_history => ("Scan history", "扫描历史"),
    chart_history_delta => ("since the last scan", "较上次扫描"),
    chart_composition => ("Line composition", "行构成"),
    chart_modules => ("Top modules by code", "代码量最大的模块"),
    chart_empty_composition => ("Run a scan to see the line mix", "运行扫描后显示行构成"),
    chart_empty_modules => ("No data yet", "暂无数据"),
    seg_code => ("Code", "代码"),
    seg_doc => ("Doc comments", "文档注释"),
    seg_comment => ("Comments", "注释"),
    seg_blank => ("Blank", "空行"),
    hover_segment => ("{name} · {value} · {share}", "{name} · {value} · {share}"),
    hover_module => ("{name} · {value} code", "{name} · {value} 行代码"),
    // -- KPI cards ---------------------------------------------------------
    card_files => ("Files", "文件"),
    card_lines => ("Lines", "行数"),
    card_code => ("Code", "代码"),
    card_comment => ("Comment", "注释"),
    card_blank => ("Blank", "空行"),
    card_test => ("Test lines", "测试行"),
    card_delta => ("vs last scan", "较上次扫描"),
    card_complexity => ("Complexity", "复杂度"),
    card_test_fns => ("Test fns", "测试函数"),
    card_unsafe => ("Unsafe", "Unsafe"),
    card_uloc => ("ULOC", "ULOC"),
    card_cost => ("Dev cost", "开发成本"),
    card_effort => ("Effort", "工作量"),
    card_people => ("Staff", "人员"),
    card_schedule => ("Schedule", "工期"),
    card_placeholder_value => ("—", "—"),
    hint_placeholder => ("run a scan", "运行扫描后显示"),
    hint_files_scanned => ("{n} {ext} files scanned", "已扫描 {n} 个 {ext} 文件"),
    hint_business_only => ("business code only", "仅统计业务代码"),
    hint_tests_included => ("test code included", "已包含测试代码"),
    hint_delta_files => ("{added} added, {deleted} deleted", "新增 {added}，删除 {deleted}"),
    hint_of_lines => ("{share} of lines", "占全部行数 {share}"),
    hint_comment_doc => ("{share} · doc {n}", "{share} · 文档 {n}"),
    hint_excluded_totals => ("excluded from totals", "未计入总计"),
    hint_included_totals => ("included in totals", "已计入总计"),
    hint_complexity => ("estimated branch points", "估算分支点"),
    hint_unsafe => ("unsafe keywords", "unsafe 关键字"),
    hint_uloc => ("DRYness {share}", "重复率 {share}"),
    hint_cocomo_cost => ("COCOMO {project} estimate", "COCOMO {project} 估算"),
    hint_effort => ("person-months", "人月"),
    hint_people => ("average team size", "平均团队规模"),
    hint_schedule => ("calendar months", "日历月"),
    hint_generated_excluded => ("{files} generated files · {lines} lines skipped", "已跳过 {files} 个生成文件 · {lines} 行"),
    // -- panes -------------------------------------------------------------
    group_overview => ("Overview", "概览"),
    group_code => ("Code", "代码统计"),
    // -- table -------------------------------------------------------------
    table_per_file => ("Per-file breakdown", "按文件统计"),
    table_per_module => ("Per-module breakdown", "按模块统计"),
    table_functions => ("Worst functions by complexity", "复杂度最高的函数"),
    table_smells => ("Panic-shaped calls and lint escapes per file", "按文件统计恐慌式调用与 lint 逃逸"),
    table_excluded => ("Test-only files kept out of the totals", "未计入总计的纯测试文件"),
    table_authors => ("Lines by author, per file", "按文件统计作者代码行"),
    table_author_totals => ("Lines each author still holds", "每位作者持有的代码行"),
    tab_files => ("Files", "文件"),
    tab_modules => ("Modules", "模块"),
    tab_functions => ("Functions", "函数"),
    tab_smells => ("Smells", "气味"),
    tab_excluded => ("Excluded tests", "排除的测试"),
    tab_authors => ("Authors", "作者"),
    tab_author_totals => ("Per author", "按作者"),
    tab_author_files => ("Per file", "按文件"),
    label_file => ("File", "文件"),
    label_module => ("Module", "模块"),
    label_function => ("Function", "函数"),
    label_test_only_file => ("Test-only file", "纯测试文件"),
    label_author => ("Author", "作者"),
    col_lines => ("Lines", "行数"),
    col_blank => ("Blank", "空行"),
    col_comment => ("Comment", "注释"),
    col_code => ("Code", "代码"),
    col_test => ("Test", "测试"),
    col_cplx => ("Cplx", "复杂度"),
    col_uloc => ("ULOC", "ULOC"),
    col_fn_args => ("Args", "参数"),
    col_fn_depth => ("Depth", "深度"),
    col_fn_where => ("Location", "位置"),
    col_unwrap => ("unwrap", "unwrap 处"),
    col_expect => ("expect", "expect 处"),
    col_panic => ("panic!", "panic! 处"),
    col_todo => ("todo!", "todo! 处"),
    col_unimplemented => ("unimpl", "unimpl 处"),
    col_allow => ("allow", "allow 处"),
    col_smells => ("Total", "合计"),
    col_author => ("Author", "作者"),
    col_files => ("Files", "文件"),
    col_delta => ("Delta", "变化"),
    filter_label => ("Filter", "筛选"),
    filter_cue => ("Filter paths…", "按路径筛选…"),
    filter_cue_author => ("Filter authors…", "按作者筛选…"),
    top_label => ("Top", "上限"),
    top_all => ("All", "全部"),
    hint_sorted => ("sorted by {column} {arrow}", "按 {column} 排序 {arrow}"),
    hint_showing => ("showing {shown} of {matched} rows", "显示 {shown} / {matched} 行"),
    hint_no_test_files => ("no test-only files were detected", "未发现纯测试文件"),
    hint_excluded => ("{files} test-only files · {lines} lines · plus {inner} lines inside test items", "{files} 个纯测试文件 · {lines} 行 · 另有 {inner} 行位于测试项内"),
    hint_functions => ("{total} functions in scope · showing the {shown} most complex", "共 {total} 个函数 · 显示其中复杂度最高的 {shown} 个"),
    hint_no_functions => ("no functions were detected", "未检测到函数"),
    hint_smells => ("{total} smells across {matched} files · showing {shown}", "{matched} 个文件共 {total} 处 · 显示 {shown}"),
    hint_no_smells => ("no panic-shaped calls or lint escapes found", "未发现恐慌式调用或 lint 逃逸"),
    hint_authors => ("{authors} author(s) across {files} blamed files · showing {shown}", "{files} 个文件、{authors} 位作者 · 显示 {shown}"),
    hint_no_authors => ("no author data - turn on Author blame and scan", "暂无作者数据 — 请打开作者归属后重新扫描"),
    table_empty => ("no rows yet — run a scan", "暂无数据 — 请先运行扫描"),
    // -- status bar --------------------------------------------------------
    scope_root_one => ("{n} root", "{n} 个根目录"),
    scope_root_many => ("{n} roots", "{n} 个根目录"),
    scope_gitignore_on => ("gitignore-aware", "遵循 gitignore"),
    scope_gitignore_off => ("gitignore ignored", "不读取 gitignore"),
    scope_ignored_dirs => ("{n} ignored dir names", "{n} 个忽略目录名"),
    scope_excludes => ("{n} exclude glob(s)", "{n} 个排除通配符"),
    scope_tests_included => ("tests included", "已包含测试"),
    status_scanned => ("Scanned {files} files in {seconds} s · {test} test lines kept out", "已扫描 {files} 个文件，用时 {seconds} s · 排除 {test} 行测试代码"),
    status_scanned_tests => ("Scanned {files} files in {seconds} s · {test} test lines included", "已扫描 {files} 个文件，用时 {seconds} s · 已包含 {test} 行测试代码"),
    status_ready => ("Ready", "就绪"),
    status_scanning => ("Scanning…", "正在扫描…"),
    status_no_root => ("No root selected", "未选择根目录"),
    status_not_folder => ("Not a folder: {paths}", "不是文件夹：{paths}"),
    status_folder_listed => ("Folder already listed", "该文件夹已在列表中"),
    status_added_one => ("Added {n} root", "已添加 {n} 个根目录"),
    status_added_many => ("Added {n} roots", "已添加 {n} 个根目录"),
    status_no_clipboard_folder => ("Clipboard has no folder path", "剪贴板中没有文件夹路径"),
    status_copied_command => ("Copied the equivalent CLI command", "已复制等效命令行"),
    status_copied_path => ("Copied the file path", "已复制文件路径"),
    status_no_row => ("Select a row first", "请先选择一行"),
    status_clipboard_failed => ("Could not open the clipboard", "无法打开剪贴板"),
    status_nothing_to_copy => ("Nothing to copy yet - run a scan first", "暂无可复制内容 — 请先运行扫描"),
    status_nothing_to_export => ("Nothing to export yet - run a scan first", "暂无可导出内容 — 请先运行扫描"),
    status_exported => ("Exported to {path}", "已导出到 {path}"),
    status_export_failed => ("Export failed: {error}", "导出失败：{error}"),
    // -- scan overlay ------------------------------------------------------
    overlay_title => ("Scanning sources", "正在扫描源码"),
    overlay_files => ("{done} / {total} files", "{done} / {total} 个文件"),
    overlay_discovering => ("Discovering files", "正在发现文件"),
    overlay_stage_files => ("Reading sources", "读取源码"),
    overlay_stage_blame => ("Reading authors", "统计作者"),
    // -- menus -------------------------------------------------------------
    menu_export_as => ("Export table as {format}", "导出表格为 {format}"),
    menu_copy_command => ("Copy equivalent CLI command", "复制等效命令行"),
    ctx_open => ("Open file\tEnter", "打开文件\tEnter"),
    ctx_reveal => ("Reveal in Explorer", "在资源管理器中显示"),
    ctx_copy_path => ("Copy path", "复制路径"),
}
