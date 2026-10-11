//! Tokeniser edge cases, including the regressions found while matching the
//! Python reference byte for byte.

use std::path::Path;

use rust_loc::lexer::{self, Analysis};
use rust_loc::scan;

/// One compact flag string per physical line: code/comment/doc/test.
fn classes(text: &str) -> Vec<String> {
    let analysis = lexer::analyze(text);
    (0..analysis.total)
        .map(|index| {
            let mut flags = String::with_capacity(4);
            flags.push(if analysis.code[index] { 'C' } else { '-' });
            flags.push(if analysis.comment[index] { 'X' } else { '-' });
            flags.push(if analysis.doc[index] { 'D' } else { '-' });
            flags.push(if analysis.is_test[index] { 'T' } else { '-' });
            flags
        })
        .collect()
}

fn analyze(text: &str) -> Analysis {
    lexer::analyze(text)
}

#[test]
fn raw_string_after_an_opening_token_stays_code() {
    // Regression: the opener used to be searched for one or two bytes ahead, so
    // `(r"a//b")` looked like a `cr`-prefixed literal and the `//` started a
    // comment. Anything that is not `r`, `br` or `cr` at this position is not a
    // raw-string opener.
    for text in [
        "let a = (r\"a//b\");\n",
        "let a = r\"a//b\";\n",
        "let a = [r\"a//b\"];\n",
        "    r#\"https?://x\"#,\n",
    ] {
        assert_eq!(classes(text), vec!["C---"], "{}", text);
    }
}

#[test]
fn raw_string_prefixes_are_recognised_at_token_start() {
    assert_eq!(classes("let a = br\"x//y\";\n"), vec!["C---"]);
    assert_eq!(classes("let a = cr\"x//y\";\n"), vec!["C---"]);
    assert_eq!(classes("let a = r#\"x\"#;\n"), vec!["C---"]);
    assert_eq!(classes("let a = br##\"x\"##;\n"), vec!["C---"]);
}

#[test]
fn an_identifier_ending_in_cr_does_not_start_a_raw_string() {
    // `xcr` is one identifier, so the quotes are an ordinary string literal.
    assert_eq!(classes("let xcr = 1; xcr\"a//b\";\n"), vec!["C---"]);
}

#[test]
fn raw_string_spans_lines_and_hash_count_matters() {
    let text = "let a = r#\"line one\nline // two\n\"#;\n";
    assert_eq!(classes(text), vec!["C---", "C---", "C---"]);
}

#[test]
fn quotes_inside_a_plain_string_are_escaped() {
    let text = "let s = \"a \\\" // \\\" b\";\nlet t = 1;\n";
    assert_eq!(classes(text), vec!["C---", "C---"]);
}

#[test]
fn comment_markers_inside_strings_are_not_comments() {
    let text = "let s = \"// not a comment\";\nlet t = \"/* nor this */\";\n";
    assert_eq!(classes(text), vec!["C---", "C---"]);
    assert_eq!(analyze(text).metrics.complexity, 0);
}

#[test]
fn doc_comment_forms_are_detected() {
    assert_eq!(classes("/// doc\n"), vec!["-XD-"]);
    assert_eq!(classes("//! inner doc\n"), vec!["-XD-"]);
    assert_eq!(classes("/** block doc */\n"), vec!["-XD-"]);
    assert_eq!(classes("/*! inner block doc */\n"), vec!["-XD-"]);
    assert_eq!(classes("// plain\n"), vec!["-X--"]);
    assert_eq!(classes("//// not doc\n"), vec!["-X--"]);
    assert_eq!(classes("/*** not doc */\n"), vec!["-X--"]);
}

#[test]
fn nested_block_comments_close_once() {
    let text = "/* outer /* inner */ still comment */\nfn a() {}\n";
    assert_eq!(classes(text), vec!["-X--", "C---"]);
}

#[test]
fn lifetimes_do_not_open_char_literals() {
    let text = "fn f<'a>(x: &'a str) -> &'a str { x }\nfn g() {}\n";
    assert_eq!(classes(text), vec!["C---", "C---"]);
}

#[test]
fn char_literals_containing_quotes_and_slashes_close() {
    let text = "let a = '\\'';\nlet b = '/';\nlet c = '\"';\nfn d() {}\n";
    assert_eq!(classes(text), vec!["C---", "C---", "C---", "C---"]);
}

#[test]
fn cfg_test_items_are_recorded_as_test_code() {
    let text = "fn a() {}\n#[cfg(test)]\nmod tests {\n    fn b() {}\n}\n";
    // The attribute line itself belongs to the excluded span.
    assert_eq!(classes(text), vec!["C---", "C--T", "C--T", "C--T", "C--T"]);
}

#[test]
fn cfg_all_test_is_excluded_but_any_and_not_are_kept() {
    let excluded = "#[cfg(all(test, feature = \"x\"))]\nfn a() {}\n";
    assert_eq!(classes(excluded), vec!["C--T", "C--T"]);
    for kept in [
        "#[cfg(any(test, feature = \"x\"))]\nfn a() {}\n",
        "#[cfg(not(test))]\nfn a() {}\n",
    ] {
        assert_eq!(classes(kept), vec!["C---", "C---"], "{}", kept);
    }
}

#[test]
fn crate_level_cfg_test_flags_the_whole_file() {
    let analysis = analyze("#![cfg(test)]\nfn a() {}\n");
    assert!(analysis.crate_test);
}

#[test]
fn cfg_test_mod_records_sibling_targets() {
    let plain = analyze("#[cfg(test)]\nmod helper;\n");
    assert_eq!(plain.sibling_targets, vec!["helper.rs".to_string()]);
    let pathed = analyze("#[cfg(test)]\n#[path = \"hidden/support.rs\"]\nmod support;\n");
    assert_eq!(
        pathed.sibling_targets,
        vec!["hidden/support.rs".to_string()]
    );
}

#[test]
fn a_plain_path_attribute_is_not_a_sibling_target() {
    let analysis = analyze("#[path = \"misc.rs\"]\nmod misc;\n");
    assert!(analysis.sibling_targets.is_empty());
    assert!(!analyze("#[path = \"misc.rs\"]\nmod misc;\n").crate_test);
}

#[test]
fn complexity_counts_scc_branch_tokens() {
    let text = "fn f(a: u32) -> u32 {\n    if a == 1 && a != 2 {\n        1\n    } else {\n        2\n    }\n}\n";
    let analysis = analyze(text);
    assert_eq!(analysis.metrics.complexity, 5);
    assert_eq!(analysis.metrics_all.complexity, 5);
}

#[test]
fn test_attributes_count_test_functions() {
    let text = "#[test]\nfn a() {}\n#[tokio::test]\nasync fn b() {}\n#[rstest]\nfn c() {}\n";
    assert_eq!(analyze(text).metrics.test_fns, 3);
    assert_eq!(
        analyze("#[test]\nfn a() {}\nfn plain() {}\n")
            .metrics
            .test_fns,
        1
    );
}

#[test]
fn unsafe_keywords_are_counted() {
    let text = "pub unsafe fn f() {\n    unsafe { }\n}\n";
    assert_eq!(analyze(text).metrics.unsafe_kw, 2);
}

#[test]
fn test_code_metrics_are_split_from_business_metrics() {
    let text = "fn a() {}\n#[cfg(test)]\nmod tests {\n    fn b() { if 1 == 1 {} }\n}\n";
    let analysis = analyze(text);
    assert_eq!(analysis.metrics_all.complexity, 2);
    assert_eq!(analysis.metrics.complexity, 0);
}

#[test]
fn physical_lines_follow_wc_semantics() {
    assert_eq!(lexer::physical_lines(""), 0);
    assert_eq!(lexer::physical_lines("a\n"), 1);
    assert_eq!(lexer::physical_lines("a\nb"), 2);
    assert_eq!(lexer::physical_lines("a\n\n"), 2);
}

#[test]
fn newlines_are_normalised_before_counting() {
    let text = lexer::normalize_newlines("a\r\nb\rc\n");
    assert_eq!(text, "a\nb\nc\n");
    assert_eq!(lexer::physical_lines(&text), 3);
}

#[test]
fn a_fake_cfg_test_inside_a_string_is_ignored() {
    let text = "fn main() {\n    let s = \"#[cfg(test)] mod fake { }\";\n}\n";
    let analysis = analyze(text);
    assert!(!analysis.is_test.iter().any(|flag| *flag));
}

#[test]
fn unbalanced_trailing_cfg_test_marks_the_rest_of_the_file() {
    let text = "fn a() {}\n#[cfg(test)]\nmod tests {\n";
    assert_eq!(classes(text), vec!["C---", "C--T", "C--T"]);
}

#[test]
fn test_only_file_names_are_recognised() {
    for name in [
        "tests/x.rs",
        "benches/x.rs",
        "src/tests.rs",
        "src/test.rs",
        "src/foo_tests.rs",
        "src/foo_test.rs",
    ] {
        assert!(
            scan::is_test_file(Path::new(name)),
            "{} should be test-only",
            name
        );
    }
    for name in [
        "src/lib.rs",
        "src/main.rs",
        "src/benches_helper.rs",
        "src/latest.rs",
    ] {
        assert!(
            !scan::is_test_file(Path::new(name)),
            "{} should not be test-only",
            name
        );
    }
}

#[test]
fn empty_files_are_blank() {
    let analysis = analyze("");
    assert_eq!(analysis.total, 0);
    assert_eq!(classes(""), Vec::<String>::new());
}

fn function_names(analysis: &Analysis) -> Vec<&str> {
    analysis
        .functions
        .iter()
        .map(|function| function.name.as_str())
        .collect()
}

#[test]
fn functions_record_name_line_params_and_code() {
    let text = "fn one() {}\nfn two(a: u32, b: &str) -> u32 {\n    a\n}\n";
    let analysis = analyze(text);
    assert_eq!(function_names(&analysis), ["one", "two"]);
    assert_eq!(analysis.functions[0].line, 1);
    assert_eq!(analysis.functions[0].params, 0);
    assert_eq!(analysis.functions[0].code, 1);
    assert_eq!(analysis.functions[1].line, 2);
    assert_eq!(analysis.functions[1].params, 2);
    assert_eq!(analysis.functions[1].code, 3);
}

#[test]
fn generic_commas_and_return_types_do_not_inflate_parameters() {
    let text = "fn f(a: HashMap<u32, u32>, b: Vec<Vec<u8>>, c: u8) -> Result<u8, ()> {}\n";
    let analysis = analyze(text);
    assert_eq!(analysis.functions[0].params, 3);
}

#[test]
fn function_complexity_and_nesting_depth_are_per_body() {
    let text =
        "fn f(a: u32) -> u32 {\n    if a == 1 {\n        for _ in 0..2 {}\n    }\n    0\n}\n";
    let analysis = analyze(text);
    assert_eq!(analysis.functions[0].complexity, 3);
    assert_eq!(analysis.functions[0].depth, 3);
}

#[test]
fn nested_functions_are_measured_separately() {
    let text = "fn outer() {\n    fn inner() {\n        if true {}\n    }\n    inner();\n}\n";
    let analysis = analyze(text);
    assert_eq!(function_names(&analysis), ["inner", "outer"]);
    assert_eq!(analysis.functions[0].complexity, 1);
    assert_eq!(analysis.functions[1].complexity, 0);
}

#[test]
fn smells_count_panic_calls_and_lint_escapes() {
    let text = "#[allow(dead_code)]\nfn a(maybe: Option<u32>) -> u32 {\n    if maybe.is_none() {\n        panic!(\"boom\");\n    }\n    let x = maybe.unwrap();\n    let y: Result<u32, ()> = Ok(x);\n    y.expect(\"bad\")\n}\n";
    let analysis = analyze(text);
    assert_eq!(analysis.smells.unwrap, 1);
    assert_eq!(analysis.smells.expect, 1);
    assert_eq!(analysis.smells.panic, 1);
    assert_eq!(analysis.smells.allow, 1);
    assert_eq!(analysis.smells.total(), 4);
    assert_eq!(analysis.smells_all.total(), 4);
}

#[test]
fn todo_and_unimplemented_count_only_as_macros() {
    let text =
        "fn a() {\n    todo!();\n    unimplemented!();\n    let todo = 1;\n    let _ = todo;\n}\n";
    let analysis = analyze(text);
    assert_eq!(analysis.smells.todo, 1);
    assert_eq!(analysis.smells.unimplemented, 1);
    assert_eq!(analysis.smells.total(), 2);
}

#[test]
fn test_attributed_functions_are_flagged_but_still_counted() {
    let text = "#[test]\nfn covers() {}\nfn business() {}\n";
    let analysis = analyze(text);
    assert_eq!(function_names(&analysis), ["covers", "business"]);
    assert!(analysis.functions[0].test);
    assert!(!analysis.functions[1].test);
}

#[test]
fn functions_inside_cfg_test_modules_are_not_recorded() {
    let text = "fn business() {}\n#[cfg(test)]\nmod tests {\n    fn helper() {}\n}\n";
    let analysis = analyze(text);
    assert_eq!(function_names(&analysis), ["business"]);
}

#[test]
fn function_pointer_fields_do_not_register_as_functions() {
    let text = "struct S {\n    f: fn(u32) -> u32,\n    g: Option<fn() -> u32>,\n}\nfn real() {}\n";
    let analysis = analyze(text);
    assert_eq!(function_names(&analysis), ["real"]);
}

#[test]
fn trait_method_declarations_without_a_body_are_skipped() {
    let text = "trait T {\n    fn value(&self) -> u32;\n    fn set(&mut self, value: u32);\n}\n";
    let analysis = analyze(text);
    assert!(analysis.functions.is_empty());
}

#[test]
fn where_clauses_stay_part_of_the_signature() {
    let text = "fn f<T>(x: T) -> T\nwhere\n    T: Clone,\n{\n    x\n}\n";
    let analysis = analyze(text);
    assert_eq!(analysis.functions.len(), 1);
    assert_eq!(analysis.functions[0].name, "f");
    assert_eq!(analysis.functions[0].params, 1);
    assert_eq!(analysis.functions[0].code, 6);
}

#[test]
fn an_unclosed_function_body_is_flushed_at_eof() {
    let text = "fn open() {\n    let value = 1;\n";
    let analysis = analyze(text);
    assert_eq!(function_names(&analysis), ["open"]);
    assert_eq!(analysis.functions[0].code, 2);
}
