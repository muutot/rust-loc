//! CI quality gates: `--fail-if <metric><op><value>`.
//!
//! A gate is evaluated against the business totals the report already carries,
//! so the same numbers a human reads decide the exit code.

use std::path::Path;

use crate::lexer;
use crate::report::{group, Report};
use crate::scan::{self, Totals};

/// The comparison an operator performs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Gt,
    Ge,
    Lt,
    Le,
    Eq,
    Ne,
}

impl Op {
    pub fn parse(text: &str) -> Option<Op> {
        match text {
            ">" => Some(Op::Gt),
            ">=" => Some(Op::Ge),
            "<" => Some(Op::Lt),
            "<=" => Some(Op::Le),
            "==" => Some(Op::Eq),
            "!=" => Some(Op::Ne),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Op::Gt => ">",
            Op::Ge => ">=",
            Op::Lt => "<",
            Op::Le => "<=",
            Op::Eq => "==",
            Op::Ne => "!=",
        }
    }

    /// True when the measured value violates the limit.
    pub fn violated(self, value: f64, limit: f64) -> bool {
        match self {
            Op::Gt => value > limit,
            Op::Ge => value >= limit,
            Op::Lt => value < limit,
            Op::Le => value <= limit,
            Op::Eq => value == limit,
            Op::Ne => value != limit,
        }
    }
}

/// Every counter a gate can look at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Metric {
    Files,
    Lines,
    Blank,
    Comment,
    Doc,
    Code,
    Test,
    Physical,
    Complexity,
    /// The complexity of the worst single function, forgiveness markers
    /// excluded.
    FnComplexity,
    Unsafe,
    TestFns,
    Uloc,
    Unwrap,
    Expect,
    Panic,
    Todo,
    Unimplemented,
    Allow,
    Smells,
}

impl Metric {
    pub fn parse(text: &str) -> Option<Metric> {
        let name = text.trim().to_ascii_lowercase().replace('-', "_");
        match name.as_str() {
            "files" => Some(Metric::Files),
            "lines" => Some(Metric::Lines),
            "blank" => Some(Metric::Blank),
            "comment" => Some(Metric::Comment),
            "doc" => Some(Metric::Doc),
            "code" => Some(Metric::Code),
            "test" => Some(Metric::Test),
            "physical" => Some(Metric::Physical),
            "complexity" => Some(Metric::Complexity),
            "fn_complexity" | "max_fn_complexity" => Some(Metric::FnComplexity),
            "unsafe" | "unsafe_kw" => Some(Metric::Unsafe),
            "test_fns" | "test_fn" => Some(Metric::TestFns),
            "uloc" => Some(Metric::Uloc),
            "unwrap" => Some(Metric::Unwrap),
            "expect" => Some(Metric::Expect),
            "panic" => Some(Metric::Panic),
            "todo" => Some(Metric::Todo),
            "unimplemented" => Some(Metric::Unimplemented),
            "allow" => Some(Metric::Allow),
            "smells" | "smell" => Some(Metric::Smells),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Metric::Files => "files",
            Metric::Lines => "lines",
            Metric::Blank => "blank",
            Metric::Comment => "comment",
            Metric::Doc => "doc",
            Metric::Code => "code",
            Metric::Test => "test",
            Metric::Physical => "physical",
            Metric::Complexity => "complexity",
            Metric::FnComplexity => "fn_complexity",
            Metric::Unsafe => "unsafe",
            Metric::TestFns => "test_fns",
            Metric::Uloc => "uloc",
            Metric::Unwrap => "unwrap",
            Metric::Expect => "expect",
            Metric::Panic => "panic",
            Metric::Todo => "todo",
            Metric::Unimplemented => "unimplemented",
            Metric::Allow => "allow",
            Metric::Smells => "smells",
        }
    }

    /// The key `Totals::column` understands.
    fn key(self) -> &'static str {
        match self {
            Metric::Files | Metric::FnComplexity => "files",
            other => other.as_str(),
        }
    }

    fn count(self, totals: &Totals) -> u32 {
        match self {
            Metric::Files | Metric::FnComplexity => 0,
            other => totals.column(other.key()),
        }
    }

    /// True for the counters a single function can own, so an annotation can
    /// point at its line rather than at the whole file.
    pub fn per_function(self) -> bool {
        matches!(self, Metric::Complexity | Metric::FnComplexity | Metric::Code)
    }

    /// Locating this metric needs the function table, which the scan only
    /// builds when something asks for it.
    pub fn collects_functions(self) -> bool {
        self.per_function()
    }

    /// The business file that contributes most to this metric, with the line
    /// of its worst function when a function owns the counter.
    pub fn blame(self, report: &Report, cwd: &Path, include_tests: bool) -> Option<Blame> {
        if self == Metric::Files {
            return None;
        }
        if self == Metric::FnComplexity {
            // Read the untruncated per-file table: the ranking the report
            // keeps may have cut the offender, and forgiven bodies never
            // count.
            let mut best: Option<(&scan::Entry, &lexer::FunctionInfo)> = None;
            for entry in report.business_entries() {
                for function in &entry.facts.functions {
                    if function.forgiven || (function.test && !include_tests) {
                        continue;
                    }
                    if best.map(|(_, top)| function.complexity > top.complexity) != Some(false)
                        && function.complexity > 0
                    {
                        best = Some((entry, function));
                    }
                }
            }
            let (entry, function) = best?;
            return Some(Blame {
                path: scan::relative(entry.path(), cwd),
                line: Some(function.line),
            });
        }
        let key = self.key();
        let worst = report
            .business_entries()
            .into_iter()
            .max_by_key(|entry| entry.column(key))?;
        if worst.column(key) == 0 {
            return None;
        }
        let path = scan::relative(worst.path(), cwd);
        let line = if self.per_function() {
            report
                .functions
                .iter()
                .filter(|row| row.path == path)
                .max_by_key(|row| match self {
                    Metric::Code => row.code,
                    _ => row.complexity,
                })
                .map(|row| row.line)
        } else {
            None
        };
        Some(Blame { path, line })
    }
}

/// The place a violated gate points at. A file scoped counter only knows the
/// path; the counters a function owns also know a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blame {
    pub path: String,
    pub line: Option<u32>,
}

impl Blame {
    /// "src/main.rs:42", or just the path when the line is unknown.
    pub fn label(&self) -> String {
        match self.line {
            Some(line) => format!("{}:{}", self.path, line),
            None => self.path.clone(),
        }
    }

    /// The "file=..." (and "line=...") part of a GitHub annotation, with the
    /// values escaped the way the workflow commands require.
    pub fn annotation(&self) -> String {
        match self.line {
            Some(line) => format!(",file={},line={line}", escape_property(&self.path)),
            None => format!(",file={}", escape_property(&self.path)),
        }
    }
}

/// Percent, comma and colon break the property list of a workflow command,
/// so they are the three characters that get escaped.
fn escape_property(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '%' => out.push_str("%25"),
            ',' => out.push_str("%2C"),
            ':' => out.push_str("%3A"),
            _ => out.push(character),
        }
    }
    out
}

/// One `--fail-if` rule.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gate {
    pub metric: Metric,
    pub op: Op,
    pub limit: f64,
}

impl Gate {
    /// Parse "code>100000" or "complexity >= 5000".
    pub fn parse(spec: &str) -> Option<Gate> {
        let trimmed = spec.trim();
        let at = trimmed
            .bytes()
            .position(|byte| matches!(byte, b'>' | b'<' | b'=' | b'!'))?;
        let rest = &trimmed[at..];
        let (op, tail) = match ["<=", ">=", "==", "!="]
            .iter()
            .find(|symbol| rest.starts_with(**symbol))
        {
            Some(symbol) => (Op::parse(symbol)?, &rest[2..]),
            None => (Op::parse(&rest[..1])?, &rest[1..]),
        };
        let metric = Metric::parse(&trimmed[..at])?;
        let limit = tail
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite() && *value >= 0.0)?;
        Some(Gate { metric, op, limit })
    }

    /// The measured value, or an error when the run never collected it.
    pub fn value(
        self,
        totals: &Totals,
        files: u32,
        uloc: Option<usize>,
        fn_complexity: Option<u32>,
    ) -> Result<f64, String> {
        match self.metric {
            Metric::Files => Ok(f64::from(files)),
            Metric::Uloc => match uloc {
                Some(value) => Ok(value as f64),
                None => Err("--fail-if uloc needs --uloc".to_string()),
            },
            Metric::FnComplexity => match fn_complexity {
                Some(value) => Ok(f64::from(value)),
                None => Err("--fail-if fn_complexity needs the function table".to_string()),
            },
            other => Ok(f64::from(other.count(totals))),
        }
    }

    pub fn violated(self, value: f64) -> bool {
        self.op.violated(value, self.limit)
    }

    /// "code 120,000 > 100,000"
    pub fn message(self, value: f64) -> String {
        format!(
            "{} {} {} {}",
            self.metric.as_str(),
            format_number(value),
            self.op.as_str(),
            format_number(self.limit)
        )
    }
}

/// Grouped whole numbers, or two decimals when the value is fractional.
fn format_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        group(value.abs().round() as u64)
    } else {
        format!("{:.2}", value)
    }
}

/// The complexity of the worst business function, forgiveness markers
/// excluded. `None` when the tree holds no function at all.
pub fn max_function_complexity(report: &Report, include_tests: bool) -> Option<u32> {
    report
        .business_entries()
        .iter()
        .flat_map(|entry| entry.facts.functions.iter())
        .filter(|function| !function.forgiven && (include_tests || !function.test))
        .map(|function| function.complexity)
        .max()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Smells;

    fn totals() -> Totals {
        Totals {
            lines: 100,
            blank: 10,
            comment: 20,
            doc: 5,
            code: 70,
            test: 4,
            physical: 200,
            complexity: 33,
            cognitive: 44,
            volume: 1234.5,
            unsafe_kw: 2,
            test_fns: 7,
            test_fns_all: 9,
            smells: Smells {
                unwrap: 3,
                expect: 1,
                panic: 1,
                todo: 0,
                unimplemented: 0,
                allow: 1,
            },
            uloc: Some(55),
        }
    }

    #[test]
    fn specs_parse_with_and_without_spaces() {
        assert_eq!(
            Gate::parse("code>100"),
            Some(Gate {
                metric: Metric::Code,
                op: Op::Gt,
                limit: 100.0
            })
        );
        assert_eq!(
            Gate::parse(" complexity >= 5000 "),
            Some(Gate {
                metric: Metric::Complexity,
                op: Op::Ge,
                limit: 5000.0
            })
        );
        assert_eq!(
            Gate::parse("test-fns==0").map(|gate| gate.metric),
            Some(Metric::TestFns)
        );
        assert_eq!(
            Gate::parse("unsafe_kw!=1").map(|gate| gate.op),
            Some(Op::Ne)
        );
    }

    #[test]
    fn bad_specs_are_rejected() {
        for spec in [
            "", "code", "code>", "nope>1", "code=1", "code>-1", "code>abc",
        ] {
            assert_eq!(Gate::parse(spec), None, "{}", spec);
        }
    }

    #[test]
    fn operators_compare_the_way_they_read() {
        assert!(Op::Gt.violated(11.0, 10.0));
        assert!(!Op::Gt.violated(10.0, 10.0));
        assert!(Op::Ge.violated(10.0, 10.0));
        assert!(Op::Lt.violated(9.0, 10.0));
        assert!(!Op::Lt.violated(10.0, 10.0));
        assert!(Op::Le.violated(10.0, 10.0));
        assert!(Op::Eq.violated(10.0, 10.0));
        assert!(Op::Ne.violated(9.0, 10.0));
    }

    #[test]
    fn metrics_read_the_business_totals() {
        let totals = totals();
        let read = |spec: &str| {
            let gate = Gate::parse(spec).unwrap();
            gate.value(&totals, 3, Some(55), None).unwrap()
        };
        assert_eq!(read("code>0"), 70.0);
        assert_eq!(read("files>0"), 3.0);
        assert_eq!(read("uloc>0"), 55.0);
        assert_eq!(read("complexity>0"), 33.0);
        assert_eq!(read("test_fns>0"), 7.0);
    }

    #[test]
    fn fn_complexity_reads_the_worst_function_and_needs_the_table() {
        let gate = Gate::parse("fn_complexity>0").unwrap();
        assert_eq!(gate.value(&totals(), 1, None, Some(12)).unwrap(), 12.0);
        assert!(gate.value(&totals(), 1, None, None).is_err());
        assert_eq!(
            Gate::parse("max-fn-complexity <= 25").map(|gate| gate.metric),
            Some(Metric::FnComplexity)
        );
    }

    #[test]
    fn smell_metrics_read_the_business_totals() {
        let totals = totals();
        let read = |spec: &str| {
            let gate = Gate::parse(spec).unwrap();
            gate.value(&totals, 3, Some(55), None).unwrap()
        };
        assert_eq!(read("smells>0"), 6.0);
        assert_eq!(read("unwrap>0"), 3.0);
        assert_eq!(read("allow>=1"), 1.0);
        assert_eq!(read("todo==0"), 0.0);
        assert_eq!(
            Gate::parse("unimplemented>0").map(|gate| gate.metric),
            Some(Metric::Unimplemented)
        );
        assert_eq!(
            Gate::parse("panic > 0").map(|gate| gate.message(2.0)),
            Some("panic 2 > 0".to_string())
        );
    }

    #[test]
    fn uloc_without_the_switch_is_an_error() {
        let gate = Gate::parse("uloc>10").unwrap();
        assert!(gate.value(&totals(), 1, None, None).is_err());
        assert_eq!(gate.value(&totals(), 1, Some(55), None).unwrap(), 55.0);
    }

    #[test]
    fn messages_read_like_a_sentence() {
        let gate = Gate::parse("code>120000").unwrap();
        assert_eq!(gate.message(130000.0), "code 130,000 > 120,000");
        let fractional = Gate::parse("code>1.5").unwrap();
        assert_eq!(fractional.message(2.0), "code 2 > 1.50");
    }
}
