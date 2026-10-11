//! End-to-end tests for the MCP server.
//!
//! Each case drives `mcp::handle` with one line of JSON-RPC and reads the one
//! answer back, exactly as the stdio binary does. The last case runs the real
//! binary so help, version and the pipe itself are covered too.

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use rust_loc::history::{History, Sample};
use rust_loc::json::{self, Value};
use rust_loc::{cli, mcp, report};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A scratch tree with two sources and one test binary, removed on drop.
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
            "rust-loc-mcp-{}-{}-{}",
            std::process::id(),
            nanos,
            COUNTER.fetch_add(1, Ordering::SeqCst)
        );
        let path = std::env::temp_dir().join(name);
        std::fs::create_dir_all(&path).unwrap();
        let workspace = Workspace { path };
        workspace.write(
            "src/main.rs",
            "fn main() {\n    // greet\n    println!(\"hi\");\n}\n",
        );
        workspace.write(
            "src/lib.rs",
            "pub fn double(value: u32) -> u32 {\n    if value > 2 {\n        value * 2\n    } else {\n        value\n    }\n}\n",
        );
        workspace.write(
            "tests/cli.rs",
            "#[test]\nfn works() {\n    // a test, not business code\n    assert!(true);\n}\n",
        );
        workspace
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.path.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, text).unwrap();
        path
    }

    fn root(&self) -> String {
        self.path.display().to_string()
    }

    fn scan(&self) -> report::Report {
        let args = vec![self.root(), "--no-gitignore".to_string()];
        let invocation = cli::parse(&args, &self.path).expect("cli::parse failed");
        report::scan(&invocation.roots, &invocation.options, &self.path, None)
    }

    fn ask(&self, id: u32, method: &str, params: &str) -> Value {
        let line = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":{},\"method\":\"{}\",\"params\":{}}}",
            id, method, params
        );
        let answer = mcp::handle(&line, &self.path).expect("the server stayed silent");
        json::parse(&answer).expect("the answer is not JSON")
    }

    fn call(&self, name: &str, arguments: &str) -> Value {
        self.ask(
            3,
            "tools/call",
            &format!("{{\"name\":\"{}\",\"arguments\":{}}}", name, arguments),
        )
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn field<'a>(value: &'a Value, name: &str) -> &'a Value {
    value
        .get(name)
        .unwrap_or_else(|| panic!("no {} in {:?}", name, value))
}

/// The text a tool answered with, parsed as JSON.
fn payload(answer: &Value) -> Value {
    json::parse(&text_of(answer)).expect("tool text is not JSON")
}

fn text_of(answer: &Value) -> String {
    field(field(answer, "result"), "content").items()[0]
        .get("text")
        .expect("no text")
        .text()
        .expect("text is not a string")
        .to_string()
}

fn failed(answer: &Value) -> bool {
    field(field(answer, "result"), "isError")
        .boolean()
        .expect("isError is not a bool")
}

/// The strings of `key`, an array whose entries are plain strings.
fn strings(value: &Value, key: &str) -> Vec<String> {
    field(value, key)
        .items()
        .iter()
        .map(|item| item.text().expect("not a string").to_string())
        .collect()
}

/// The `item` field of every entry of `key`.
fn names(value: &Value, key: &str, item: &str) -> Vec<String> {
    field(value, key)
        .items()
        .iter()
        .map(|entry| field(entry, item).text().expect("not a string").to_string())
        .collect()
}

#[test]
fn broken_json_is_a_parse_error() {
    let workspace = Workspace::new();
    let answer = mcp::handle("{oops", &workspace.path).expect("no answer");
    let answer = json::parse(&answer).unwrap();
    assert_eq!(
        field(field(&answer, "error"), "code").number(),
        Some(-32700.0)
    );
    assert!(field(&answer, "id").is_null());
}

#[test]
fn initialize_echoes_the_client_version() {
    let workspace = Workspace::new();
    let answer = workspace.ask(1, "initialize", "{\"protocolVersion\":\"2025-06-18\"}");
    let result = field(&answer, "result");
    assert_eq!(field(result, "protocolVersion").text(), Some("2025-06-18"));
    assert_eq!(
        field(field(result, "serverInfo"), "name").text(),
        Some("rust-loc")
    );
    assert_eq!(
        field(field(result, "serverInfo"), "version").text(),
        Some(rust_loc::VERSION)
    );
    assert!(!field(field(result, "capabilities"), "tools").is_null());
    assert!(!field(result, "instructions")
        .text()
        .expect("no instructions")
        .is_empty());

    let plain = workspace.ask(1, "initialize", "{}");
    assert_eq!(
        field(field(&plain, "result"), "protocolVersion").text(),
        Some(mcp::PROTOCOL)
    );
}

#[test]
fn ping_answers_an_empty_result() {
    let workspace = Workspace::new();
    let answer = workspace.ask(2, "ping", "{}");
    assert!(answer.get("result").is_some());
    assert!(answer.get("error").is_none());
}

#[test]
fn tools_list_names_scan_gate_and_history() {
    let workspace = Workspace::new();
    let answer = workspace.ask(4, "tools/list", "{}");
    let result = field(&answer, "result");
    assert_eq!(names(result, "tools", "name"), ["scan", "gate", "history"]);
    for tool in field(result, "tools").items() {
        assert_eq!(
            field(field(tool, "inputSchema"), "type").text(),
            Some("object")
        );
        assert!(tool.get("description").and_then(Value::text).is_some());
    }
    let gate = field(result, "tools")
        .items()
        .iter()
        .find(|tool| field(tool, "name").text() == Some("gate"))
        .expect("no gate tool");
    assert_eq!(strings(field(gate, "inputSchema"), "required"), ["rules"]);
    assert!(field(field(gate, "inputSchema"), "properties")
        .get("rules")
        .is_some());
    let scan = field(result, "tools")
        .items()
        .iter()
        .find(|tool| field(tool, "name").text() == Some("scan"))
        .expect("no scan tool");
    let format = field(field(field(scan, "inputSchema"), "properties"), "format");
    assert_eq!(strings(format, "enum"), ["json", "text", "markdown", "csv"]);
}

#[test]
fn scan_counts_the_business_tree() {
    let workspace = Workspace::new();
    let answer = workspace.call("scan", "{}");
    assert!(!failed(&answer));
    let document = payload(&answer);
    let summary = field(&document, "summary");
    assert!(field(summary, "code").number().unwrap() > 0.0);
    assert!(field(summary, "lines").number().unwrap() >= field(summary, "code").number().unwrap());
    assert_eq!(field(summary, "files").number(), Some(2.0));
    let entries = field(&document, "files").items();
    let listed = names(&document, "files", "path");
    assert!(listed.iter().any(|path| path.ends_with("main.rs")));
    assert!(listed.iter().any(|path| path.ends_with("lib.rs")));
    let test_entry = entries
        .iter()
        .find(|entry| field(entry, "path").text().unwrap().contains("tests/"))
        .expect("the test file is not listed");
    assert_eq!(field(test_entry, "test_only").boolean(), Some(true));
    assert!(field(test_entry, "test").number().unwrap() > 0.0);

    let with_tests = payload(&workspace.call("scan", "{\"include_tests\":true}"));
    let tests = field(&with_tests, "summary");
    assert_eq!(field(tests, "files").number(), Some(3.0));
    assert_eq!(field(tests, "test").number(), Some(0.0));
    assert!(field(tests, "lines").number().unwrap() > field(summary, "lines").number().unwrap());
    assert!(field(&with_tests, "excluded").is_null());

    let text = workspace.call("scan", "{\"format\":\"text\"}");
    assert!(!failed(&text));
    assert!(text_of(&text).contains("Rust"));

    let switched =
        payload(&workspace.call("scan", "{\"smells\":true,\"functions\":true,\"uloc\":true}"));
    assert!(!field(field(&switched, "summary"), "smells").is_null());
    assert!(!field(field(&switched, "summary"), "uloc").is_null());
    assert!(!field(&switched, "functions").items().is_empty());
}

#[test]
fn gate_names_the_worst_file() {
    let workspace = Workspace::new();
    let passed = payload(&workspace.call("gate", "{\"rules\":[\"code>1000000\"]}"));
    assert_eq!(field(&passed, "failed").boolean(), Some(false));
    assert!(field(&passed, "rules").items()[0]
        .get("worst")
        .unwrap()
        .is_null());

    let answer = workspace.call("gate", "{\"rules\":[\"code>1000000\",\"code>0\"]}");
    assert!(!failed(&answer));
    let document = payload(&answer);
    assert_eq!(field(&document, "failed").boolean(), Some(true));
    let rules = field(&document, "rules").items();
    assert_eq!(rules.len(), 2);
    assert_eq!(field(&rules[0], "passed").boolean(), Some(true));
    assert_eq!(field(&rules[1], "passed").boolean(), Some(false));
    assert_eq!(field(&rules[1], "metric").text(), Some("code"));
    let worst = field(&rules[1], "worst").text().expect("no blame");
    assert!(
        worst.starts_with("src/") && worst.ends_with(":1"),
        "unexpected blame: {}",
        worst
    );

    let broken = workspace.call("gate", "{\"rules\":[\"nonsense\"]}");
    assert!(failed(&broken));
    assert!(text_of(&broken).contains("nonsense"));
}

#[test]
fn history_reads_a_recorded_series() {
    let workspace = Workspace::new();
    let mut history = History::load(&workspace.path.join("history.tsv")).unwrap();
    history.record(Sample::of(&workspace.scan(), 1_700_000_000));
    workspace.write("src/extra.rs", "fn extra() -> u32 {\n    1\n}\n");
    history.record(Sample::of(&workspace.scan(), 1_700_003_600));
    history.save().unwrap();

    let answer = payload(&workspace.call("history", "{\"path\":\"history.tsv\"}"));
    assert_eq!(field(&answer, "samples").number(), Some(2.0));
    assert_eq!(field(&answer, "metric").text(), Some("code"));
    assert_eq!(field(&answer, "values").items().len(), 2);
    assert!(field(&answer, "delta").number().unwrap() > 0.0);
    assert_eq!(
        field(&answer, "sparkline").text().unwrap().chars().count(),
        2
    );
    assert!(field(&answer, "span").text().unwrap().contains("->"));
    for trend in [
        "files",
        "lines",
        "blank",
        "comment",
        "code",
        "complexity",
        "smells",
    ] {
        assert_eq!(
            field(field(&answer, "series"), trend).items().len(),
            2,
            "series is missing {}",
            trend
        );
    }

    let metric =
        payload(&workspace.call("history", "{\"path\":\"history.tsv\",\"metric\":\"files\"}"));
    assert_eq!(field(&metric, "metric").text(), Some("files"));
    assert_eq!(field(&metric, "values").items().len(), 2);

    let empty = payload(&workspace.call("history", "{\"path\":\"nothing.tsv\"}"));
    assert_eq!(field(&empty, "samples").number(), Some(0.0));
    assert!(field(&empty, "values").items().is_empty());

    assert!(failed(&workspace.call("history", "{}")));
    assert!(failed(&workspace.call(
        "history",
        "{\"path\":\"history.tsv\",\"metric\":\"nonsense\"}"
    )));
}

#[test]
fn notifications_and_unknown_calls_are_answered_correctly() {
    let workspace = Workspace::new();
    assert!(mcp::handle(
        "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}",
        &workspace.path
    )
    .is_none());
    assert!(mcp::handle(
        "{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"idle\"}",
        &workspace.path
    )
    .is_some());

    let unknown = workspace.ask(5, "does/not/exist", "{}");
    assert_eq!(
        field(field(&unknown, "error"), "code").number(),
        Some(-32601.0)
    );

    let tool = workspace.call("nope", "{}");
    assert!(failed(&tool));
    assert!(text_of(&tool).contains("unknown tool"));
    assert_eq!(
        names(
            &workspace.ask(6, "resources/list", "{}"),
            "result",
            "resources"
        ),
        Vec::<String>::new()
    );
    assert_eq!(
        names(&workspace.ask(7, "prompts/list", "{}"), "result", "prompts"),
        Vec::<String>::new()
    );
}

#[test]
fn the_binary_speaks_the_protocol_on_stdio() {
    let exe = env!("CARGO_BIN_EXE_rust-loc-mcp");
    let help = std::process::Command::new(exe)
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("rust-loc-mcp"));

    let version = std::process::Command::new(exe)
        .arg("--version")
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&version.stdout).contains(rust_loc::VERSION));

    let unknown = std::process::Command::new(exe)
        .arg("--nope")
        .output()
        .unwrap();
    assert_eq!(unknown.status.code(), Some(2));

    let missing = std::process::Command::new(exe)
        .arg(std::env::temp_dir().join("rust-loc-does-not-exist"))
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(1));

    let mut child = std::process::Command::new(exe)
        .arg(std::env::temp_dir())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let lines = concat!(
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\",\"params\":{}}\n",
    );
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(lines.as_bytes())
        .unwrap();
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&output.stdout);
    let answers: Vec<&str> = text.lines().collect();
    assert_eq!(answers.len(), 2, "one answer per request: {:?}", text);
    assert_eq!(
        field(&json::parse(answers[0]).unwrap(), "id").number(),
        Some(1.0)
    );
    assert_eq!(
        field(&json::parse(answers[1]).unwrap(), "id").number(),
        Some(2.0)
    );
}
