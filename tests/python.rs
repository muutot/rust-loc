//! Python reader: line classes, test exclusion and the cheap metrics.

use rust_loc::lexer::Analysis;
use rust_loc::pylexer;

/// One compact flag string per physical line: code/comment/doc/test.
fn classes(text: &str) -> Vec<String> {
    let analysis = pylexer::analyze(text);
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
    pylexer::analyze(text)
}

#[test]
fn a_module_docstring_is_a_doc_comment() {
    let text = "\"\"\"Module doc.\n\nSecond line.\n\"\"\"\nvalue = 1\n";
    assert_eq!(classes(text), vec!["-XD-", "-XD-", "-XD-", "-XD-", "C---"]);
}

#[test]
fn a_shebang_does_not_stop_the_module_docstring() {
    // A comment is not a code line, so the string below is still the module
    // docstring even though it is not on the first line.
    let text = "#!/usr/bin/env python\n\"\"\"Doc.\"\"\"\nvalue = 1\n";
    assert_eq!(classes(text), vec!["-X--", "-XD-", "C---"]);
}

#[test]
fn a_prefixed_docstring_still_counts() {
    let text = "r\"\"\"Pattern.\"\"\"\nvalue = 1\n";
    assert_eq!(classes(text), vec!["-XD-", "C---"]);
}

#[test]
fn a_bare_multiline_string_is_code() {
    // Not the first token of its logical line, so it is a value, not a
    // docstring.
    let text = "query = \"\"\"\nselect 1\n\"\"\"\n";
    assert_eq!(classes(text), vec!["C---", "C---", "C---"]);
}

#[test]
fn a_function_docstring_is_a_doc_comment() {
    let text = "def f():\n    \"\"\"Doc.\"\"\"\n    return 1\n";
    assert_eq!(classes(text), vec!["C---", "-XD-", "C---"]);
}

#[test]
fn a_comment_behind_code_leaves_the_line_code() {
    let text = "value = 1  # note\n";
    assert_eq!(classes(text), vec!["CX--"]);
}

#[test]
fn a_backslash_joins_a_logical_line() {
    let text = "value = 1 + \\\n    2\nnext = 3\n";
    assert_eq!(classes(text), vec!["C---", "C---", "C---"]);
}

#[test]
fn brackets_join_a_logical_line() {
    let text = "value = (\n    1,\n    2,\n)\nnext = 3\n";
    assert_eq!(classes(text), vec!["C---", "C---", "C---", "C---", "C---"]);
}

#[test]
fn a_test_function_carries_its_body() {
    let text = "def compute(a, b):\n    return a + b\n\n\ndef test_compute():\n    assert compute(1, 2) == 3\n";
    assert_eq!(
        classes(text),
        vec!["C---", "C---", "----", "----", "C--T", "C--T"]
    );
}

#[test]
fn a_test_class_carries_its_methods() {
    let text = "class TestApi:\n    def helper(self):\n        return 1\n    def test_ok(self):\n        pass\n";
    assert_eq!(classes(text), vec!["C--T", "C--T", "C--T", "C--T", "C--T"]);
}

#[test]
fn a_decorated_test_is_test_code() {
    let text = "@pytest.mark.slow\ndef test_slow():\n    pass\n";
    assert_eq!(classes(text), vec!["C--T", "C--T", "C--T"]);
}

#[test]
fn a_plain_decorator_is_not_test_code() {
    let text = "@property\ndef value(self):\n    return 1\n";
    assert_eq!(classes(text), vec!["C---", "C---", "C---"]);
}

#[test]
fn a_one_line_test_body_is_marked() {
    let text = "def test_quick(): assert True\n";
    assert_eq!(classes(text), vec!["C--T"]);
}

#[test]
fn the_function_table_carries_params_and_depth() {
    let text = "def outer(a, b=1, *args, **kwargs):\n    if a:\n        for i in b:\n            print(i)\n    return a\n";
    let analysis = analyze(text);
    assert_eq!(analysis.functions.len(), 1);
    let outer = &analysis.functions[0];
    assert_eq!(outer.name, "outer");
    assert_eq!(outer.line, 1);
    assert_eq!(outer.params, 4);
    assert_eq!(outer.depth, 2);
    assert_eq!(outer.complexity, 2);
    assert_eq!(outer.code, 5);
    assert!(!outer.test);
}

#[test]
fn a_receiver_is_not_a_parameter() {
    let text = "class Api:\n    def get(self, url, timeout=1):\n        return url\n\n    @classmethod\n    def build(cls, name):\n        return name\n";
    let analysis = analyze(text);
    let names: Vec<(String, u32, u32)> = analysis
        .functions
        .iter()
        .map(|item| (item.name.clone(), item.params, item.line))
        .collect();
    assert_eq!(
        names,
        vec![("get".to_string(), 2, 2), ("build".to_string(), 1, 6),]
    );
}

#[test]
fn a_multi_line_signature_counts_its_parameters() {
    let text = "def outer(\n    a,\n    b,\n):\n    return a\n";
    let analysis = analyze(text);
    let outer = &analysis.functions[0];
    assert_eq!(outer.name, "outer");
    assert_eq!(outer.line, 1);
    assert_eq!(outer.params, 2);
}

#[test]
fn a_test_function_is_flagged_in_the_table() {
    let text = "def test_x():\n    pass\n";
    let analysis = analyze(text);
    assert_eq!(analysis.metrics.test_fns, 1);
    assert_eq!(analysis.metrics_all.test_fns, 1);
    assert!(analysis.functions[0].test);
}

#[test]
fn cognitive_complexity_grows_with_nesting() {
    let text = "if a:\n    if b:\n        pass\n";
    let analysis = analyze(text);
    assert_eq!(analysis.metrics.complexity, 2);
    assert_eq!(analysis.metrics.cognitive, 3);
}

#[test]
fn boolean_and_comparison_operators_are_branch_points() {
    let text = "if a and b or c:\n    pass\nif a == b:\n    pass\n";
    let analysis = analyze(text);
    assert_eq!(analysis.metrics.complexity, 5);
}

#[test]
fn a_match_arm_counts_when_it_opens_a_line() {
    let text = "match value:\n    case 1:\n        pass\n    case _:\n        pass\n";
    let analysis = analyze(text);
    assert_eq!(analysis.metrics.complexity, 2);
    // Each arm sits one level inside the match body.
    assert_eq!(analysis.metrics.cognitive, 4);
}

#[test]
fn a_branch_inside_a_test_is_not_business_complexity() {
    let text = "def test_x():\n    if a:\n        return\n";
    let analysis = analyze(text);
    assert_eq!(analysis.metrics.complexity, 0);
    assert_eq!(analysis.metrics_all.complexity, 1);
}

#[test]
fn lint_escapes_and_markers_are_smells() {
    let text =
        "# noqa: E501\nvalue = 1  # type: ignore\n# TODO: fix this\nraise NotImplementedError\n";
    let analysis = analyze(text);
    assert_eq!(analysis.smells.allow, 2);
    assert_eq!(analysis.smells.todo, 1);
    assert_eq!(analysis.smells.unimplemented, 1);
}

#[test]
fn smells_inside_a_test_stay_out_of_the_business_scope() {
    let text = "def test_x():\n    # noqa\n    return 1\n";
    let analysis = analyze(text);
    assert_eq!(analysis.smells_all.allow, 1);
    assert_eq!(analysis.smells.allow, 0);
}

#[test]
fn the_rust_only_metrics_stay_zero() {
    let analysis = analyze("unsafe = 1\nvalue = thing.unwrap()\n");
    assert_eq!(analysis.metrics.unsafe_kw, 0);
    assert_eq!(analysis.smells.unwrap, 0);
    assert_eq!(analysis.smells.expect, 0);
}

#[test]
fn an_empty_file_has_nothing_in_it() {
    let analysis = analyze("");
    assert_eq!(analysis.total, 0);
    assert_eq!(analysis.metrics.complexity, 0);
    assert!(analysis.functions.is_empty());
    assert_eq!(classes(""), Vec::<String>::new());
}

#[test]
fn a_file_that_ends_inside_a_body_still_closes_it() {
    let text = "def test_x():\n    pass";
    assert_eq!(classes(text), vec!["C--T", "C--T"]);
}
