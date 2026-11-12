//! JSON payloads for the web front end.
//!
//! Every struct here is a view over the analysis types: the core keeps no
//! serde derives, so the API shape can move without dragging the pipeline
//! along. Exports do not go through this layer at all — they reuse the very
//! renderers the CLI prints with, byte for byte.

use std::path::Path;

use serde::Serialize;

use crate::authors;
use crate::baseline;
use crate::history;
use crate::lexer;
use crate::options::Format;
use crate::report::Report;
use crate::scan;

/// Everything a finished scan leaves behind: the report itself (exports re-render
/// it), the samples the history chart plots and the wall clock it took.
pub struct ScanResult {
    pub report: Report,
    pub history: Vec<history::Sample>,
    pub elapsed: f64,
    pub generated_at: u64,
}

impl ScanResult {
    /// Record the finished scan into the per-root history file, so the next
    /// scan charts a series and not a lone point.
    pub fn record_history(&mut self, roots: &[String]) {
        let path = crate::settings::history_path(roots);
        let mut history = history::History::load(&path).unwrap_or_else(|_| history::History {
            path: path.clone(),
            samples: Vec::new(),
        });
        history.record(history::Sample::of(&self.report, self.generated_at));
        let _ = history.save();
        self.history = history.samples;
    }
}

#[derive(Serialize)]
pub struct ReportDto {
    pub roots: Vec<String>,
    pub generated_at: u64,
    pub elapsed: f64,
    pub summary: SummaryDto,
    pub all_lines: u32,
    pub uloc_total: Option<usize>,
    /// One row per business file, ranked by code lines.
    pub files: Vec<FileDto>,
    pub excluded_files: usize,
    pub excluded_file_lines: u32,
    pub excluded_item_lines: u32,
    pub excluded_paths: Vec<ExcludedPathDto>,
    pub generated_files: usize,
    pub generated_lines: u32,
    pub functions: Vec<FunctionDto>,
    pub function_count: usize,
    pub cocomo: Option<CocomoDto>,
    pub hotspots: Vec<HotspotDto>,
    pub coupling: Option<CouplingDto>,
    /// Each configured `--fail-if` rule with the value it measured; the page
    /// shows whether the scan would pass CI.
    pub gates: Vec<GateDto>,
    pub metrics: Option<MetricsDto>,
    pub languages: Vec<LanguageDto>,
    pub duplicates: Option<DupesDto>,
    pub diff: Option<DiffDto>,
    pub history: Vec<SampleDto>,
    /// Whether the scan roots sit inside a git repository; the page uses it
    /// to decide whether the "read authors" action can possibly succeed.
    pub git_available: bool,
}

#[derive(Serialize)]
pub struct GateDto {
    /// The spec as configured, e.g. `complexity<5000`.
    pub spec: String,
    pub value: f64,
    pub limit: f64,
    pub op: String,
    /// True when the rule holds (or could not be measured and blocks nothing).
    pub ok: bool,
    /// Why the value could not be read, e.g. uloc without `--uloc`.
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct SummaryDto {
    pub lines: u32,
    pub blank: u32,
    pub comment: u32,
    pub doc: u32,
    pub code: u32,
    pub test: u32,
    pub physical: u32,
    pub complexity: u32,
    pub cognitive: u32,
    pub unsafe_kw: u32,
    pub test_fns: u32,
    /// Test functions across the whole tree, the files the scan excluded as
    /// test-only included - the card's number when tests live in their own
    /// files, the way Go projects keep them.
    pub test_fns_all: u32,
    pub smells: SmellsDto,
    pub volume: f64,
    pub uloc: Option<usize>,
}

#[derive(Serialize)]
pub struct SmellsDto {
    pub unwrap: u32,
    pub expect: u32,
    pub panic: u32,
    pub todo: u32,
    pub unimplemented: u32,
    pub allow: u32,
    pub total: u32,
}

impl SmellsDto {
    fn of(smells: lexer::Smells) -> SmellsDto {
        SmellsDto {
            unwrap: smells.unwrap,
            expect: smells.expect,
            panic: smells.panic,
            todo: smells.todo,
            unimplemented: smells.unimplemented,
            allow: smells.allow,
            total: smells.total(),
        }
    }
}

#[derive(Serialize)]
pub struct FileDto {
    pub path: String,
    /// The directory the file sits in, relative to its scan root; files at the
    /// root itself read `(root)`.
    pub module: String,
    pub lang: &'static str,
    pub lines: u32,
    pub blank: u32,
    pub comment: u32,
    pub doc: u32,
    pub code: u32,
    pub test: u32,
    pub physical: u32,
    pub complexity: u32,
    pub cognitive: u32,
    pub unsafe_kw: u32,
    pub test_fns: u32,
    pub smells: SmellsDto,
    pub uloc: Option<u32>,
    /// `(gen)` or `(min)`, the same tag the tables print.
    pub generated: &'static str,
    pub delta: Option<i64>,
}

#[derive(Serialize)]
pub struct ExcludedPathDto {
    pub path: String,
    pub lines: u32,
}

#[derive(Serialize)]
pub struct FunctionDto {
    pub path: String,
    pub name: String,
    pub line: u32,
    pub code: u32,
    pub complexity: u32,
    pub cognitive: u32,
    pub params: u32,
    pub depth: u32,
}

#[derive(Serialize)]
pub struct CocomoDto {
    pub project: &'static str,
    pub effort: f64,
    pub schedule: f64,
    pub people: f64,
    pub cost: f64,
}

#[derive(Serialize)]
pub struct HotspotDto {
    pub path: String,
    pub code: u32,
    pub complexity: u32,
    pub commits: u64,
    pub churn: u64,
    pub score: u64,
}

#[derive(Serialize)]
pub struct CouplingPairDto {
    pub a: String,
    pub b: String,
    pub commits: u64,
}

#[derive(Serialize)]
pub struct CouplingDto {
    /// How many commits the analysis walked.
    pub commits: usize,
    /// The strongest pairs first.
    pub pairs: Vec<CouplingPairDto>,
}

impl CouplingDto {
    fn of(coupling: &crate::coupling::Coupling) -> CouplingDto {
        CouplingDto {
            commits: coupling.commits,
            pairs: coupling
                .top(20)
                .iter()
                .map(|pair| CouplingPairDto {
                    a: pair.a.clone(),
                    b: pair.b.clone(),
                    commits: pair.commits,
                })
                .collect(),
        }
    }
}

#[derive(Serialize)]
pub struct MetricsDto {
    pub index: f64,
    pub volume: f64,
    pub cognitive: u32,
    pub sloc: u32,
    pub operators: u32,
    pub operands: u32,
    pub tokens: u32,
    pub methods: usize,
    pub arguments: u64,
    pub band: &'static str,
}

#[derive(Serialize)]
pub struct LanguageDto {
    pub name: &'static str,
    pub files: u32,
    pub lines: u32,
    pub blank: u32,
    pub comment: u32,
    pub code: u32,
}

#[derive(Serialize)]
pub struct DupesDto {
    pub window: usize,
    pub duplicated_lines: u32,
    pub code_lines: u32,
    pub percent: f64,
    /// The biggest groups, capped so one pathological tree cannot flood the
    /// page: two hundred clusters name every copy a reader will chase.
    pub clusters: Vec<ClusterDto>,
}

#[derive(Serialize)]
pub struct ClusterDto {
    pub lines: u32,
    pub sites: Vec<SiteDto>,
}

#[derive(Serialize)]
pub struct SiteDto {
    pub path: String,
    pub line: u32,
}

#[derive(Serialize)]
pub struct BaselineRowDto {
    pub lines: u32,
    pub code: u32,
    pub complexity: u32,
    pub smells: u32,
}

#[derive(Serialize)]
pub struct FileDeltaDto {
    pub path: String,
    pub old: Option<BaselineRowDto>,
    pub new: Option<BaselineRowDto>,
    pub delta: i64,
}

#[derive(Serialize)]
pub struct DiffDto {
    pub path: String,
    pub old: BaselineRowDto,
    pub new: BaselineRowDto,
    pub files: Vec<FileDeltaDto>,
}

#[derive(Serialize)]
pub struct SampleDto {
    pub when: u64,
    pub files: u32,
    pub lines: u32,
    pub blank: u32,
    pub comment: u32,
    pub code: u32,
    pub complexity: u32,
    pub smells: u32,
}

/// The author tables: per author across the scan, then per file.
#[derive(Serialize)]
pub struct AuthorsDto {
    pub available: bool,
    pub untracked: usize,
    pub rows: Vec<AuthorRowDto>,
    pub files: Vec<AuthorFileDto>,
    /// Commits per author per month, from the scan that carried the blame.
    pub activity: Option<ActivityDto>,
}

#[derive(Serialize)]
pub struct AuthorRowDto {
    pub name: String,
    pub email: String,
    pub files: usize,
    pub blank: u32,
    pub comment: u32,
    pub doc: u32,
    pub code: u32,
    pub test: u32,
}

#[derive(Serialize)]
pub struct AuthorFileDto {
    pub path: String,
    pub shares: Vec<ShareDto>,
}

#[derive(Serialize)]
pub struct ShareDto {
    pub name: String,
    pub email: String,
    pub blank: u32,
    pub comment: u32,
    pub doc: u32,
    pub code: u32,
    pub test: u32,
}

#[derive(Serialize)]
pub struct ActivityRowDto {
    pub name: String,
    /// Commits per month, aligned with the months on the activity.
    pub commits: Vec<u32>,
}

#[derive(Serialize)]
pub struct ActivityDto {
    /// Months ascending, `YYYY-MM`, the most recent ones.
    pub months: Vec<String>,
    pub rows: Vec<ActivityRowDto>,
}

impl ActivityDto {
    fn of(activity: &crate::activity::Activity) -> ActivityDto {
        ActivityDto {
            months: activity.months.clone(),
            rows: activity
                .rows
                .iter()
                .map(|row| ActivityRowDto {
                    name: row.name.clone(),
                    commits: row.commits.clone(),
                })
                .collect(),
        }
    }
}

fn row_of(path: &Path, root: &Path) -> (String, String) {
    let relative = scan::relative(path, root);
    let module = match relative.rsplit_once('/') {
        Some((parent, _)) if !parent.is_empty() => parent.to_string(),
        _ => "(root)".to_string(),
    };
    (relative, module)
}

impl ReportDto {
    pub fn of(scan: &ScanResult) -> ReportDto {
        let report = &scan.report;
        let mut files: Vec<FileDto> = Vec::new();
        for entry in report.business_entries() {
            let (path, module) = row_of(entry.path(), entry.root());
            files.push(FileDto {
                path,
                module,
                lang: entry.facts.lang.name(),
                lines: entry.stats.lines,
                blank: entry.stats.blank,
                comment: entry.stats.comment,
                doc: entry.stats.doc,
                code: entry.stats.code,
                test: entry.stats.test,
                physical: entry.stats.physical,
                complexity: entry.stats.complexity,
                cognitive: entry.stats.cognitive,
                unsafe_kw: entry.stats.unsafe_kw,
                test_fns: entry.stats.test_fns,
                smells: SmellsDto::of(entry.stats.smells),
                uloc: entry.stats.uloc,
                generated: entry.stats.generated.tag(),
                delta: None,
            });
        }
        files.sort_by(|a, b| b.code.cmp(&a.code).then(a.path.cmp(&b.path)));

        let summary = &report.summary;
        let cocomo = report.cocomo.as_ref().map(|row| CocomoDto {
            project: row.project,
            effort: row.estimate.effort,
            schedule: row.estimate.schedule,
            people: row.estimate.people,
            cost: row.estimate.cost,
        });
        let metrics = report.metrics.as_ref().map(|row| MetricsDto {
            index: row.index,
            volume: row.volume,
            cognitive: row.cognitive,
            sloc: row.sloc,
            operators: row.operators,
            operands: row.operands,
            tokens: row.tokens(),
            methods: row.methods,
            arguments: row.arguments,
            band: row.band(),
        });
        let duplicates = report.duplicates.as_ref().map(|dupes| DupesDto {
            window: dupes.window,
            duplicated_lines: dupes.duplicated_lines,
            code_lines: dupes.code_lines,
            percent: dupes.percent(),
            clusters: dupes
                .clusters
                .iter()
                .take(200)
                .map(|cluster| ClusterDto {
                    lines: cluster.lines,
                    sites: cluster
                        .sites
                        .iter()
                        .map(|site| SiteDto {
                            path: site.path.clone(),
                            line: site.line,
                        })
                        .collect(),
                })
                .collect(),
        });
        let diff = report.diff.as_ref().map(|diff| DiffDto {
            path: diff.path.to_string_lossy().into_owned(),
            old: row_dto(&diff.old),
            new: row_dto(&diff.new),
            files: diff
                .files
                .iter()
                .map(|file| FileDeltaDto {
                    path: file.path.clone(),
                    old: file.old.as_ref().map(|row| row_dto(row)),
                    new: file.new.as_ref().map(|row| row_dto(row)),
                    delta: code_delta(file),
                })
                .collect(),
        });
        // The same evaluation the CI exit code uses, so the page shows what
        // `--fail-if` would decide.
        let gates: Vec<GateDto> = report
            .options
            .fail_if
            .iter()
            .map(|rule| {
                let fn_complexity = if rule.metric == crate::gate::Metric::FnComplexity {
                    Some(
                        crate::gate::max_function_complexity(report, report.options.include_tests)
                            .unwrap_or(0),
                    )
                } else {
                    None
                };
                let files = report.business.len() as u32;
                let spec = format!(
                    "{}{}{}",
                    rule.metric.as_str(),
                    rule.op.as_str(),
                    rule.limit
                );
                match rule.value(&report.summary, files, report.uloc_total, fn_complexity) {
                    Ok(value) => GateDto {
                        spec,
                        value,
                        limit: rule.limit,
                        op: rule.op.as_str().to_string(),
                        ok: !rule.violated(value),
                        error: None,
                    },
                    Err(error) => GateDto {
                        spec,
                        value: 0.0,
                        limit: rule.limit,
                        op: rule.op.as_str().to_string(),
                        ok: true,
                        error: Some(error),
                    },
                }
            })
            .collect();
        ReportDto {
            roots: report
                .roots
                .iter()
                .map(|root| root.to_string_lossy().into_owned())
                .collect(),
            generated_at: scan.generated_at,
            elapsed: scan.elapsed,
            summary: SummaryDto {
                lines: summary.lines,
                blank: summary.blank,
                comment: summary.comment,
                doc: summary.doc,
                code: summary.code,
                test: summary.test,
                physical: summary.physical,
                complexity: summary.complexity,
                cognitive: summary.cognitive,
                unsafe_kw: summary.unsafe_kw,
                test_fns: summary.test_fns,
                test_fns_all: summary.test_fns_all,
                smells: SmellsDto::of(summary.smells),
                volume: summary.volume,
                uloc: summary.uloc,
            },
            all_lines: report.all_lines,
            uloc_total: report.uloc_total,
            files,
            excluded_files: report.excluded.files,
            excluded_file_lines: report.excluded.file_lines,
            excluded_item_lines: report.excluded.item_lines,
            excluded_paths: report
                .excluded
                .paths
                .iter()
                .map(|(path, lines)| ExcludedPathDto {
                    path: path.clone(),
                    lines: *lines,
                })
                .collect(),
            generated_files: report.excluded.generated_files,
            generated_lines: report.excluded.generated_lines,
            functions: report
                .functions
                .iter()
                .map(|row| FunctionDto {
                    path: row.path.clone(),
                    name: row.name.clone(),
                    line: row.line,
                    code: row.code,
                    complexity: row.complexity,
                    cognitive: row.cognitive,
                    params: row.params,
                    depth: row.depth,
                })
                .collect(),
            function_count: report.function_count,
            cocomo,
            hotspots: report
                .hotspots
                .iter()
                .map(|row| HotspotDto {
                    path: row.path.clone(),
                    code: row.code,
                    complexity: row.complexity,
                    commits: row.commits,
                    churn: row.churn,
                    score: row.score,
                })
                .collect(),
            coupling: report.coupling.as_ref().map(CouplingDto::of),
            gates,
            metrics,
            languages: report
                .languages
                .iter()
                .map(|row| LanguageDto {
                    name: row.name,
                    files: row.files,
                    lines: row.lines,
                    blank: row.blank,
                    comment: row.comment,
                    code: row.code,
                })
                .collect(),
            duplicates,
            diff,
            git_available: report
                .roots
                .first()
                .is_some_and(|root| crate::scan::run_git(root, &["rev-parse", "--show-toplevel"]).is_some()),
            history: scan
                .history
                .iter()
                .map(|sample| SampleDto {
                    when: sample.when,
                    files: sample.files,
                    lines: sample.lines,
                    blank: sample.blank,
                    comment: sample.comment,
                    code: sample.code,
                    complexity: sample.complexity,
                    smells: sample.smells,
                })
                .collect(),
        }
    }
}

fn row_dto(row: &baseline::Row) -> BaselineRowDto {
    BaselineRowDto {
        lines: row.lines,
        code: row.code,
        complexity: row.complexity,
        smells: row.smells,
    }
}

fn code_delta(file: &baseline::FileDelta) -> i64 {
    let new = file.new.as_ref().map(|row| i64::from(row.code)).unwrap_or(0);
    let old = file.old.as_ref().map(|row| i64::from(row.code)).unwrap_or(0);
    new - old
}

impl AuthorsDto {
    pub fn of(blame: &authors::Authors, activity: Option<&crate::activity::Activity>) -> AuthorsDto {
        AuthorsDto {
            available: blame.available,
            untracked: blame.untracked,
            activity: activity.map(ActivityDto::of),
            rows: blame
                .rows
                .iter()
                .map(|row| AuthorRowDto {
                    name: row.name.clone(),
                    email: row.email.clone(),
                    files: row.files,
                    blank: row.lines.blank,
                    comment: row.lines.comment,
                    doc: row.lines.doc,
                    code: row.lines.code,
                    test: row.lines.test,
                })
                .collect(),
            files: blame
                .files
                .iter()
                .map(|file| AuthorFileDto {
                    path: scan::posix(&file.path),
                    shares: file
                        .authors
                        .iter()
                        .map(|share| ShareDto {
                            name: share.name.clone(),
                            email: share.email.clone(),
                            blank: share.lines.blank,
                            comment: share.lines.comment,
                            doc: share.lines.doc,
                            code: share.lines.code,
                            test: share.lines.test,
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}

/// The export formats the web page offers, matching the CLI's `--format`.
pub fn export_format(name: &str) -> Option<Format> {
    Format::parse(name)
}

/// Media type and suggested filename for an export format.
pub fn export_content_type(format: Format) -> (&'static str, &'static str) {
    match format {
        Format::Text => ("text/plain; charset=utf-8", "rust-loc.txt"),
        Format::Json => ("application/json", "rust-loc.json"),
        Format::Csv => ("text/csv; charset=utf-8", "rust-loc.csv"),
        Format::Markdown => ("text/markdown; charset=utf-8", "rust-loc.md"),
        Format::Svg => ("image/svg+xml", "rust-loc.svg"),
        Format::Html => ("text/html; charset=utf-8", "rust-loc.html"),
    }
}
