//! A minimal Model Context Protocol server: JSON-RPC over stdio.
//!
//! Three tools answer what an agent would otherwise have to run the CLI for:
//! `scan` counts a tree, `gate` checks budgets such as `code>100000`, and
//! `history` reads a recorded series. Every tool goes through the library, so
//! a tool call cannot drift from the report the command line prints.

use std::path::{Path, PathBuf};

use crate::json::{self, Value};
use crate::options::Options;
use crate::{cli, history, report, scan};

/// Protocol version answered when the client does not name one.
pub const PROTOCOL: &str = "2024-11-05";

/// One line of input in, one JSON-RPC answer out. `None` means the message was
/// a notification, which is acted on but never answered.
pub fn handle(line: &str, cwd: &Path) -> Option<String> {
    let request = match json::parse(line) {
        Ok(value) => value,
        Err(error) => {
            return Some(failure(
                &Value::Null,
                -32700,
                &format!("parse error: {}", error),
            ))
        }
    };
    let id = request.get("id")?.clone();
    let method = request
        .get("method")
        .and_then(Value::text)
        .unwrap_or_default();
    let params = request.get("params").cloned().unwrap_or(Value::Null);
    match method {
        "initialize" => Some(success(&id, &initialize(&params))),
        "ping" => Some(success(&id, "{}")),
        "tools/list" => Some(success(&id, &tools())),
        "tools/call" => Some(success(&id, &call(&params, cwd))),
        "resources/list" => Some(success(&id, "{\"resources\":[]}")),
        "prompts/list" => Some(success(&id, "{\"prompts\":[]}")),
        other => Some(failure(&id, -32601, &format!("unknown method: {}", other))),
    }
}

fn success(id: &Value, result: &str) -> String {
    String::from("{\"jsonrpc\":\"2.0\",\"id\":") + &id.render() + ",\"result\":" + result + "}"
}

fn failure(id: &Value, code: i32, message: &str) -> String {
    String::from("{\"jsonrpc\":\"2.0\",\"id\":")
        + &id.render()
        + ",\"error\":{\"code\":"
        + &code.to_string()
        + ",\"message\":"
        + &json::quote(message)
        + "}}"
}

/// What the client is told the server is for, in one paragraph.
const INSTRUCTIONS: &str = "rust-loc counts source lines with test code excluded (Rust by default; set \"sources\" to \"python\" or \"all\"). Use `scan` for a tree's counters (business code, comments, blank lines, complexity, smells, per-file rows), `gate` to check budgets such as \"code>100000\", and `history` to read the series a previous `--history` run recorded.";

fn initialize(params: &Value) -> String {
    // Answer with the revision the client asked for: the tools use nothing
    // that changed between them, and hearing its own version keeps the
    // session alive.
    let version = params
        .get("protocolVersion")
        .and_then(Value::text)
        .unwrap_or(PROTOCOL);
    object(&[
        ("protocolVersion", json::quote(version)),
        (
            "capabilities",
            String::from("{\"tools\":{\"listChanged\":false}}"),
        ),
        (
            "serverInfo",
            object(&[
                ("name", json::quote("rust-loc")),
                ("version", json::quote(crate::VERSION)),
            ]),
        ),
        ("instructions", json::quote(INSTRUCTIONS)),
    ])
}

/// The scope and report switches every scanning tool shares.
const SCOPE_PROPERTIES: &str = concat!(
    "\"roots\":{\"type\":\"array\",\"items\":{\"type\":\"string\"},",
    "\"description\":\"Directories to scan; defaults to the server working directory.\"},",
    "\"sources\":{\"type\":\"string\",\"enum\":[\"rust\",\"python\",\"all\"],",
    "\"description\":\"Source languages to read; Rust by default.\"},",
    "\"include_tests\":{\"type\":\"boolean\",\"description\":\"Count test code as well.\"},",
    "\"exclude\":{\"type\":\"array\",\"items\":{\"type\":\"string\"},",
    "\"description\":\"Glob patterns to skip.\"},",
    "\"exclude_generated\":{\"type\":\"boolean\",",
    "\"description\":\"Drop generated and minified files from the totals.\"},",
    "\"uloc\":{\"type\":\"boolean\",\"description\":\"Add the unique line (DRYness) count.\"},",
    "\"smells\":{\"type\":\"boolean\",",
    "\"description\":\"Add the unwrap/expect/panic/todo/unimplemented/allow counters.\"},",
    "\"functions\":{\"type\":\"boolean\",\"description\":\"Rank functions by complexity.\"},",
    "\"metrics\":{\"type\":\"boolean\",",
    "\"description\":\"Add cognitive complexity, Halstead volume and the maintainability index.\"},",
    "\"languages\":{\"type\":\"boolean\",\"description\":\"Break the tree down by language.\"},",
    "\"duplicates\":{\"type\":\"boolean\",\"description\":\"Find blocks of copied lines.\"},",
    "\"hotspots\":{\"type\":\"boolean\",",
    "\"description\":\"Rank files by complexity times git churn.\"},",
    "\"cocomo\":{\"type\":\"boolean\",\"description\":\"Add a COCOMO cost estimate.\"}"
);

const SCAN_DESCRIPTION: &str = "Count the business code of a directory (Rust, or Python with \"sources\") and return the report: files, lines, blank, comment, code, test, physical, complexity, unsafe, test_fns and, when asked for, ULOC, smells, function metrics, language shares, duplicates, hotspots, author shares and an estimate. JSON by default.";
const GATE_DESCRIPTION: &str = "Check quality budgets against a scan. Every rule is written METRIC OP VALUE, for example \"code>100000\" or \"complexity>=5000\", and the answer names the file (and function) to blame for each failure.";
const HISTORY_DESCRIPTION: &str = "Read a scan history written by `--history` or by the window and return the series, its sparkline and the change since the previous scan.";

fn tools() -> String {
    let scan = tool(
        "scan",
        SCAN_DESCRIPTION,
        &(String::from("{\"type\":\"object\",\"properties\":{")
            + SCOPE_PROPERTIES
            + ",\"format\":{\"type\":\"string\",\"enum\":[\"json\",\"text\",\"markdown\",\"csv\"],"
            + "\"description\":\"Report shape; JSON by default.\"}}}"),
    );
    let gate = tool(
        "gate",
        GATE_DESCRIPTION,
        &(String::from("{\"type\":\"object\",\"properties\":{")
            + SCOPE_PROPERTIES
            + ",\"rules\":{\"type\":\"array\",\"items\":{\"type\":\"string\"},"
            + "\"description\":\"Rules like code>100000.\"}},\"required\":[\"rules\"]}"),
    );
    let history = tool(
        "history",
        HISTORY_DESCRIPTION,
        "{\"type\":\"object\",\"properties\":{\"path\":{\"type\":\"string\",\"description\":\"History file written by --history.\"},\"metric\":{\"type\":\"string\",\"description\":\"Counter to draw: code, lines, files, comment, blank, complexity or smells.\"}},\"required\":[\"path\"]}",
    );
    String::from("{\"tools\":[") + &scan + "," + &gate + "," + &history + "]}"
}

fn tool(name: &str, description: &str, schema: &str) -> String {
    String::from("{\"name\":")
        + &json::quote(name)
        + ",\"description\":"
        + &json::quote(description)
        + ",\"inputSchema\":"
        + schema
        + "}"
}

fn call(params: &Value, cwd: &Path) -> String {
    let name = params.get("name").and_then(Value::text).unwrap_or_default();
    let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
    let answer = match name {
        "scan" => scan_tool(&arguments, cwd),
        "gate" => gate_tool(&arguments, cwd),
        "history" => history_tool(&arguments, cwd),
        other => Err(format!(
            "unknown tool: {}; this server offers scan, gate and history",
            other
        )),
    };
    match answer {
        Ok(text) => content(&text, false),
        Err(error) => content(&error, true),
    }
}

fn content(text: &str, is_error: bool) -> String {
    let flag = if is_error { "true" } else { "false" };
    String::from("{\"content\":[{\"type\":\"text\",\"text\":")
        + &json::quote(text)
        + "}],\"isError\":"
        + flag
        + "}"
}

fn scan_tool(arguments: &Value, cwd: &Path) -> Result<String, String> {
    let args = arguments_to_argv(arguments, true)?;
    let (report, options) = run(&args, cwd)?;
    Ok(report.render_as(options.format, cwd))
}

fn gate_tool(arguments: &Value, cwd: &Path) -> Result<String, String> {
    let rules: Vec<String> = arguments
        .get("rules")
        .map(Value::items)
        .unwrap_or(&[])
        .iter()
        .map(|item| string_of(item, "rules"))
        .collect::<Result<_, _>>()?;
    if rules.is_empty() {
        return Err("gate needs at least one rule, for example code>100000".to_string());
    }
    let mut args = arguments_to_argv(arguments, false)?;
    for rule in &rules {
        args.push("--fail-if".to_string());
        args.push(rule.clone());
    }
    let (report, options) = run(&args, cwd)?;
    let mut failed = false;
    let mut items: Vec<String> = Vec::with_capacity(options.fail_if.len());
    for rule in &options.fail_if {
        let value = rule.value(
            &report.summary,
            report.business.len() as u32,
            report.uloc_total,
        )?;
        let violated = rule.violated(value);
        failed |= violated;
        items.push(object(&[
            ("rule", json::quote(&rule.message(value))),
            ("metric", json::quote(rule.metric.as_str())),
            ("limit", number(rule.limit)),
            ("value", number(value)),
            ("passed", (!violated).to_string()),
            (
                "worst",
                if violated {
                    rule.metric
                        .blame(&report, cwd)
                        .map(|spot| json::quote(&spot.label()))
                        .unwrap_or_else(|| "null".to_string())
                } else {
                    "null".to_string()
                },
            ),
        ]));
    }
    Ok(object(&[
        ("failed", failed.to_string()),
        ("rules", array(&items)),
    ]))
}

fn history_tool(arguments: &Value, cwd: &Path) -> Result<String, String> {
    let path = arguments
        .get("path")
        .and_then(Value::text)
        .ok_or_else(|| "history needs a path to the file --history wrote".to_string())?;
    let resolved = resolve(path, cwd);
    let history = history::History::load(&resolved)?;
    let metric = match arguments.get("metric").and_then(Value::text) {
        Some(name) => history::Trend::parse(name)
            .ok_or_else(|| format!("unknown metric: {}; try code, lines or smells", name))?,
        None => history::Trend::Code,
    };
    let values = history.series(metric, history::LIMIT);
    let latest = values.last().copied().unwrap_or(0);
    let before = history
        .previous()
        .map(|sample| sample.value(metric))
        .unwrap_or(latest);
    let series: Vec<String> = history::Trend::all()
        .iter()
        .map(|trend| {
            json::quote(trend.as_str())
                + ":"
                + &array(&numbers(&history.series(*trend, history::LIMIT)))
        })
        .collect();
    Ok(object(&[
        ("path", json::quote(&scan::relative(&resolved, cwd))),
        ("samples", history.samples.len().to_string()),
        ("span", json::quote(&history.span())),
        ("metric", json::quote(metric.as_str())),
        ("values", array(&numbers(&values))),
        (
            "sparkline",
            json::quote(&history::sparkline(
                &history.series(metric, history::SPARK_WIDTH),
            )),
        ),
        ("delta", (i64::from(latest) - i64::from(before)).to_string()),
        ("series", object_from(&series)),
    ]))
}

/// The switches a tool argument can turn on, in the order they are printed.
const SWITCHES: [(&str, &str); 11] = [
    ("include_tests", "--include-tests"),
    ("exclude_generated", "--exclude-generated"),
    ("mark_generated", "--mark-generated"),
    ("uloc", "--uloc"),
    ("smells", "--smells"),
    ("functions", "--functions"),
    ("metrics", "--metrics"),
    ("languages", "--languages"),
    ("duplicates", "--duplicates"),
    ("hotspots", "--hotspots"),
    ("cocomo", "--cocomo"),
];

/// Turn the tool arguments into the command line the library already parses,
/// so a tool call and a shell command cannot drift apart.
fn arguments_to_argv(arguments: &Value, default_json: bool) -> Result<Vec<String>, String> {
    let mut args: Vec<String> = Vec::new();
    for item in arguments.get("roots").map(Value::items).unwrap_or(&[]) {
        args.push(string_of(item, "roots")?);
    }
    if let Some(name) = arguments.get("sources").and_then(Value::text) {
        args.push("--sources".to_string());
        args.push(name.to_string());
    }
    for (key, flag) in SWITCHES {
        if arguments.get(key).and_then(Value::boolean).unwrap_or(false) {
            args.push(flag.to_string());
        }
    }
    for item in arguments.get("exclude").map(Value::items).unwrap_or(&[]) {
        args.push("--exclude".to_string());
        args.push(string_of(item, "exclude")?);
    }
    match arguments.get("format").and_then(Value::text) {
        Some(name) => {
            args.push("--format".to_string());
            args.push(name.to_string());
        }
        None if default_json => {
            args.push("--format".to_string());
            args.push("json".to_string());
        }
        None => {}
    }
    Ok(args)
}

/// Parse and scan, exactly like the command line would.
fn run(args: &[String], cwd: &Path) -> Result<(report::Report, Options), String> {
    let invocation = cli::parse(args, cwd)?;
    let report = report::scan(&invocation.roots, &invocation.options, cwd, None);
    Ok((report, invocation.options))
}

fn string_of(value: &Value, what: &str) -> Result<String, String> {
    value
        .text()
        .map(str::to_string)
        .ok_or_else(|| format!("{} must be a string", what))
}

fn resolve(path: &str, cwd: &Path) -> PathBuf {
    let candidate = Path::new(path);
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        cwd.join(candidate)
    }
}

/// Whole numbers where they fit, fractions otherwise.
fn number(value: f64) -> String {
    Value::Number(value).render()
}

fn numbers(values: &[u32]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

fn object(fields: &[(&str, String)]) -> String {
    let mut out = String::from("{");
    for (index, (key, value)) in fields.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&json::quote(key));
        out.push(':');
        out.push_str(value);
    }
    out.push('}');
    out
}

/// An object from ready made `"key":value` pieces.
fn object_from(fields: &[String]) -> String {
    String::from("{") + &fields.join(",") + "}"
}

fn array(items: &[String]) -> String {
    String::from("[") + &items.join(",") + "]"
}
