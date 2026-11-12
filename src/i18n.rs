//! UI language catalog: English and Chinese, Chinese by default.
//!
//! Every user visible string lives here, once per language, so the shell can
//! switch language at run time without any resource file or dependency.

use std::cell::Cell;

#[cfg(windows)]
use windows_sys::Win32::Globalization::GetUserDefaultUILanguage;

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
/// a language-only or mixed run does not claim to have counted Rust, and says
/// whether test code made it into the totals.
pub fn subtitle(sources: crate::sources::Sources, include_tests: bool) -> String {
    let catalog = t();
    let (en, zh) = match sources.single() {
        Some(lang) => (
            format!("{} business code statistics", lang.name()),
            format!("{} 业务代码统计", lang.name()),
        ),
        None => (
            "Business code statistics".to_string(),
            "业务代码统计".to_string(),
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
pub fn extensions(sources: crate::sources::Sources) -> String {
    match sources.single() {
        Some(lang) => format!(".{}", lang.extension()),
        None => sources
            .languages()
            .iter()
            .map(|lang| format!(".{}", lang.extension()))
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// What the selected languages call the test item the `Test fns` card counted.
pub fn test_item(sources: crate::sources::Sources) -> String {
    use crate::sources::Lang as Source;
    let (en, zh) = match sources.single() {
        Some(Source::Rust) => ("#[test] functions", "#[test] 函数"),
        Some(Source::Python) => ("def test_ functions", "def test_ 函数"),
        _ => ("test functions", "测试函数"),
    };
    let text = match lang() {
        Lang::En => en,
        Lang::Zh => zh,
    };
    text.to_string()
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

        impl Strings {
            /// Every entry as `(key, english, chinese)`, in declaration order.
            /// The `--dump-i18n` maintenance flag writes these out for the web
            /// front end, so the two language files cannot drift from here.
            pub fn pairs() -> Vec<(&'static str, &'static str, &'static str)> {
                vec![ $( (stringify!($name), $en, $zh), )* ]
            }
        }
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
        let sources = crate::sources::Sources::RUST;
        set_lang(Lang::Zh);
        assert!(subtitle(sources, false).ends_with("已排除测试代码"));
        assert!(subtitle(sources, true).ends_with("已包含测试代码"));
        set_lang(Lang::En);
        assert!(subtitle(sources, true).ends_with("test code included"));
        assert!(subtitle(sources, false).ends_with("test code excluded"));
        set_lang(Lang::Zh);
    }

    #[test]
    fn the_subtitle_names_the_language_it_read() {
        set_lang(Lang::En);
        assert!(subtitle(crate::sources::Sources::RUST, false).starts_with("Rust"));
        let go = crate::sources::Sources::parse("go").unwrap();
        assert!(subtitle(go, false).starts_with("Go"));
        assert!(subtitle(crate::sources::Sources::ALL, false).starts_with("Business"));
        set_lang(Lang::Zh);
    }

    #[test]
    fn the_scan_hints_speak_the_language_that_was_read() {
        use crate::sources::Sources;
        set_lang(Lang::En);
        assert_eq!(extensions(Sources::PYTHON), ".py");
        assert_eq!(extensions(Sources::parse("rust,go").unwrap()), ".rs .go");
        assert_eq!(test_item(Sources::RUST), "#[test] functions");
        assert_eq!(test_item(Sources::PYTHON), "def test_ functions");
        assert_eq!(test_item(Sources::ALL), "test functions");
        set_lang(Lang::Zh);
        assert_eq!(test_item(Sources::RUST), "#[test] 函数");
        assert_eq!(test_item(Sources::PYTHON), "def test_ 函数");
        assert_eq!(test_item(Sources::ALL), "测试函数");
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
    section_languages => ("LANGUAGES", "代码语言"),
    source_summary_hint => ("picked on the Settings screen", "在「设置」屏幕中勾选"),
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
    hint_extra_dirs => ("One directory name per line; same-named dirs are skipped anywhere", "每行一个目录名，任意层级中同名目录都会被跳过"),
    section_language => ("LANGUAGE", "界面语言"),
    button_scan => ("Scan  (F5)", "开始扫描  (F5)"),
    scan_all => ("All", "全部"),
    scan_files => ("Files only", "仅文件"),
    button_export => ("Export…", "导出…"),
    // -- tray --------------------------------------------------------------
    tray_open => ("Open rust-loc", "打开 rust-loc"),
    tray_quit => ("Quit", "退出"),
    // -- web ---------------------------------------------------------------
    tab_overview => ("Overview", "总览"),
    tab_authors => ("Authors", "作者"),
    tab_code => ("Code", "代码"),
    tab_settings => ("Settings", "设置"),
    tab_files => ("Files", "文件"),
    tab_functions => ("Functions", "函数"),
    button_save => ("Save", "保存"),
    button_save_scan => ("Save and scan", "保存并扫描"),
    button_remove => ("Remove", "移除"),
    button_add_folder => ("Add folder…", "添加文件夹…"),
    button_blame => ("Read authors from git", "从 git 读取作者"),
    export_markdown => ("Markdown", "Markdown"),
    export_json => ("JSON", "JSON"),
    export_csv => ("CSV", "CSV"),
    export_text => ("Text", "文本"),
    export_svg => ("SVG", "SVG"),
    export_pick_format => ("Pick export format", "选择导出格式"),
    hint_separator => ("Separate with spaces or commas", "空格或逗号分隔"),
    status_scanning => ("Scanning", "正在扫描"),
    status_blaming => ("Reading authors", "正在读取作者"),
    status_scan_failed => ("Scan failed", "扫描失败"),
    hint_no_scan => ("No scan yet — press Scan to read the tree", "尚未扫描——点击「扫描」读取代码树"),
    hint_no_git => ("This tree is not a git repository, so there are no authors to show", "该目录不是 git 仓库，无法显示作者"),
    hint_untracked => ("Untracked files", "未纳入 git 的文件"),
    hint_saved => ("Saved", "已保存"),
    hint_filter_files => ("Filter files…", "筛选文件…"),
    hint_filter_functions => ("Filter functions…", "筛选函数…"),
    limit_100 => ("Top 100", "前 100"),
    limit_300 => ("Top 300", "前 300"),
    limit_1000 => ("Top 1000", "前 1000"),
    limit_all => ("All rows", "全部"),
    card_code => ("Code", "代码"),
    card_total_lines => ("Total lines", "总行数"),
    card_physical => ("physical", "物理行"),
    card_comment => ("Comment", "注释"),
    card_doc => ("Doc comments", "文档注释"),
    card_blank => ("Blank", "空行"),
    card_test => ("Test code", "测试代码"),
    card_test_fns => ("Test functions", "测试函数"),
    card_functions => ("Functions", "函数数"),
    card_hotspots => ("Hotspot files", "热点文件"),
    card_authors => ("Authors", "作者数"),
    card_complexity => ("Complexity", "复杂度"),
    card_cognitive => ("Cognitive", "认知复杂度"),
    card_uloc => ("ULOC", "ULOC"),
    card_smells => ("Smells", "坏味道"),
    card_files => ("Files", "文件数"),
    col_path => ("Path", "路径"),
    col_lang => ("Language", "语言"),
    col_author => ("Author", "作者"),
    col_code => ("Code", "代码"),
    col_comment => ("Comment", "注释"),
    col_blank => ("Blank", "空行"),
    col_test => ("Test", "测试"),
    col_files => ("Files", "文件数"),
    col_complexity => ("Cx", "复杂度"),
    col_cognitive => ("Cog", "认知"),
    col_smells => ("Smells", "坏味道"),
    col_function => ("Function", "函数"),
    col_params => ("Params", "参数"),
    col_depth => ("Depth", "嵌套"),
    col_lead => ("Lead author", "主要作者"),
    col_share => ("Share", "占比"),
    chart_languages => ("Languages by code", "各语言代码量"),
    chart_authors => ("Authors by code", "各作者代码量"),
    table_authors => ("Per author", "按作者"),
    table_files => ("Per file", "按文件"),
    panel_metrics => ("Halstead metrics", "Halstead 指标"),
    panel_cocomo => ("Effort estimate (COCOMO)", "工作量估算（COCOMO）"),
    panel_dupes => ("Duplicated code", "重复代码"),
    panel_diff => ("Change since last scan", "较上次扫描的变化"),
    panel_coupling => ("Change coupling", "变更耦合"),
    panel_activity => ("Author activity", "作者活跃度"),
    panel_gates => ("Quality gates", "质量门禁"),
    coupling_examined => ("{n} commits examined", "共检查 {n} 个提交"),
    col_pair => ("File pair", "文件对"),
    col_commits => ("Commits", "提交次数"),
    panel_excluded => ("Excluded files", "被排除的文件"),
    metric_index => ("Index", "指数"),
    metric_volume => ("Volume", "体量"),
    metric_cognitive => ("Cognitive", "认知复杂度"),
    metric_tokens => ("Tokens", "词法单元"),
    cocomo_effort => ("Effort", "工作量"),
    cocomo_schedule => ("Schedule", "工期"),
    cocomo_people => ("People", "人数"),
    cocomo_cost => ("Cost", "成本"),
    unit_person_months => ("person-months", "人月"),
    unit_months => ("months", "月"),
    unit_files => ("files", "个文件"),
    unit_lines => ("lines", "行"),
    dupes_percent => ("Duplicated", "重复率"),
    dupes_lines => ("Duplicated lines", "重复行数"),
    diff_code => ("Code lines", "代码行"),
    section_filters => ("FILTERS", "筛选"),
    section_ui_lang => ("UI LANGUAGE", "界面语言"),
    field_exclude_ext => ("Skip extensions", "跳过扩展名"),
    field_max_size => ("Max file size (bytes)", "单文件大小上限（字节）"),
    field_exclude => ("Skip name parts", "跳过名称片段"),
    field_fail_if => ("Quality gates (fail-if)", "质量门禁（fail-if）"),
    hint_fail_if => ("e.g. code>120000, fn_complexity>60", "例如 code>120000, fn_complexity>60"),
    field_extra_dirs => ("Extra ignored dirs", "额外忽略目录"),
    picker_title => ("Choose a folder", "选择文件夹"),
    picker_go => ("Open", "打开"),
    picker_cancel => ("Cancel", "取消"),
    picker_choose => ("Choose this", "选这个"),
    picker_unreadable => ("Cannot read that directory", "无法读取该目录"),
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
    card_lines => ("Lines", "行数"),
    card_delta => ("vs last scan", "较上次扫描"),
    card_unsafe => ("Unsafe", "Unsafe"),
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
    group_languages => ("Languages", "代码语言"),
    group_filters => ("Filters", "筛选"),
    group_report => ("Report", "报告"),
    group_settings => ("Settings", "设置"),
    // -- settings screen ---------------------------------------------------
    languages_hint => ("tick every language the scan should read — the last one stays ticked", "勾选扫描应读取的语言——至少保留一种"),
    filters_hint => ("what a scan reads and what it skips — changes wait for the next scan", "扫描读取与跳过什么——修改后需重新扫描才生效"),
    report_hint => ("what the report adds on top of the counters — changes wait for the next scan", "报告在计数之外附加的内容——修改后需重新扫描才生效"),
    settings_filter_cue => ("filter languages…", "筛选语言…"),
    settings_all => ("Select all", "全选"),
    settings_none => ("Clear", "清空"),
    scope_exclude_label => ("Exclude globs", "排除通配符"),
    scope_extra_label => ("Extra ignored dirs", "额外忽略目录"),
    scope_ext_label => ("Excluded file types", "排除文件类型"),
    scope_size_label => ("Max file size (MB)", "单文件大小上限（MB）"),
    scope_dir_add => ("Add", "添加"),
    scope_dir_remove => ("Remove", "移除"),
    scope_dir_cue => ("a directory name, then Add", "输入目录名后点添加"),
    hint_exclude_ext => ("e.g. min.js, lock, map — 0 means no cap", "例如 min.js, lock, map"),
    hint_max_size => ("files larger than this are skipped; 0 reads every file", "超过该大小的文件会被跳过；0 表示不限制"),
    // -- stale cue --------------------------------------------------------
    stale_hint => ("settings changed — press Scan  (F5)", "设置已更改——按开始扫描（F5）更新"),
    // -- language pane -----------------------------------------------------
    source_title => ("Code languages", "代码语言"),
    source_hint => ("tick every language the scan should read — the last one stays ticked", "勾选扫描应读取的语言——至少保留一种"),
    source_filter_cue => ("filter languages…", "筛选语言…"),
    source_all => ("Select all", "全选"),
    source_none => ("Clear", "清空"),
    // -- table -------------------------------------------------------------
    table_per_file => ("Per-file breakdown", "按文件统计"),
    table_per_module => ("Per-module breakdown", "按模块统计"),
    table_functions => ("Worst functions by complexity", "复杂度最高的函数"),
    table_smells => ("Panic-shaped calls and lint escapes per file", "按文件统计恐慌式调用与 lint 逃逸"),
    table_excluded => ("Test-only files kept out of the totals", "未计入总计的纯测试文件"),
    table_author_totals => ("Lines each author still holds", "每位作者持有的代码行"),
    tab_modules => ("Modules", "模块"),
    tab_smells => ("Smells", "气味"),
    tab_excluded => ("Excluded tests", "排除的测试"),
    tab_author_totals => ("Per author", "按作者"),
    tab_author_files => ("Per file", "按文件"),
    label_file => ("File", "文件"),
    label_module => ("Module", "模块"),
    label_function => ("Function", "函数"),
    label_test_only_file => ("Test-only file", "纯测试文件"),
    label_author => ("Author", "作者"),
    col_lines => ("Lines", "行数"),
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
    col_index => ("No.", "序号"),
    col_delta => ("Delta", "变化"),
    col_churn => ("Churn", "改动行"),
    col_score => ("Score", "得分"),
    col_sites => ("Copy sites", "重复位置"),
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
    hint_no_authors => ("Author data not loaded yet; scan with authors or press the blame button", "尚未读取作者信息，可使用「全部」扫描或点击「从 git 读取作者」"),
    hint_last_roots => ("Previous scan roots (the report below was scanned with these)", "上次扫描目录（当前报告来自这些目录）"),
    hint_authors_reading => ("reading authors… {done} / {total} files", "正在统计作者… {done} / {total} 个文件"),
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
    overlay_title_export => ("Exporting the report", "正在导出报告"),
    overlay_files => ("{done} / {total} files", "{done} / {total} 个文件"),
    overlay_discovering => ("Discovering files", "正在发现文件"),
    overlay_stage_files => ("Reading sources", "读取源码"),
    overlay_stage_of => ("{stage} ({index}/{count})", "{stage}（{index}/{count}）"),
    overlay_stage_blame => ("Reading authors", "统计作者"),
    // -- menus -------------------------------------------------------------
    menu_export_as => ("Export table as {format}", "导出表格为 {format}"),
    menu_copy_command => ("Copy equivalent CLI command", "复制等效命令行"),
    ctx_open => ("Open file\tEnter", "打开文件\tEnter"),
    ctx_reveal => ("Reveal in Explorer", "在资源管理器中显示"),
    ctx_copy_path => ("Copy path", "复制路径"),
}
