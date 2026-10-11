//! Command line parsing and execution.
//!
//! Kept in the library (rather than `main.rs`) so the argument handling and
//! the rendered output can be exercised in-process by integration tests, the
//! same way the Python reference exposes `cli.main(argv)`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::baseline;
use crate::cocomo;
use crate::config;
use crate::gate;
use crate::options::{Format, Options};
use crate::{report, scan};

pub const USAGE: &str = "\
rust-loc - count Rust source lines with test code excluded

USAGE:
    rust-loc [OPTIONS] [ROOTS]...

ARGS:
    <ROOTS>...    directories to scan (default: .)

OPTIONS:
    --root <PATH>            add a scan root (repeatable)
    --config <PATH>          read this .rustloc.toml instead of the default
    --no-config              ignore any .rustloc.toml
    --snapshot <PATH>        save this scan for a later --baseline diff
    --baseline <PATH>        compare with a snapshot from --snapshot
    --format <FORMAT>        output format: text, json, csv, markdown [default: text]
    --json                   alias for --format json
    --by-file, --per-file    per-file breakdown
    --by-module              per-module breakdown
    --sort <KEY>             sort key for listings [default: code]
    --top <N>                limit listings to N rows
    --show-excluded          list excluded test-only files
    --include-tests          count test code too
    --exclude <GLOB>         exclude paths (repeatable)
    --ignore-dir <NAME>      skip directories with this name (repeatable)
    --no-ignore-dirs         do not skip the default ignored directories
    --gitignore              force gitignore filtering
    --no-gitignore           disable gitignore filtering
    --uloc                   compute ULOC and DRYness
    --no-complexity          hide the complexity estimate
    --functions              list functions ranked by complexity
    --top-functions <N>      rows in the function list [default: 20, 0 = all]
    --smells                 count unwrap/expect/panic!/todo!/#[allow] escapes
    --hotspots               rank files by complexity and git churn
    --metrics                add cognitive complexity, Halstead and MI
    --languages              list every language in the tree, not just Rust
    --exclude-generated      drop generated/minified files from the totals
    --mark-generated         tag listings with (gen)/(min) instead
    --min-gen-line-length <N>  average line length that marks a file minified
    --cocomo                 add a COCOMO cost estimate
    --cocomo-project-type <T>  organic, semi-detached, embedded, custom
    --cocomo-params <a,b,c,d>  custom COCOMO coefficients
    --fail-if <SPEC>         exit 1 when METRIC OP VALUE fails (repeatable)
    --ci                     print gate failures as GitHub annotations
    --jobs <N>               analysis threads [default: all cores]
    -h, --help               print this help
    -V, --version            print the version
";

/// What the parsed command line asks the process to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Help,
    Version,
    Report,
}

/// Options plus roots, resolved against the working directory.
#[derive(Debug)]
pub struct Invocation {
    pub action: Action,
    pub options: Options,
    pub roots: Vec<PathBuf>,
}

/// A path from the command line, resolved against the working directory.
fn resolve_path(value: &str, cwd: &Path) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

fn take(arguments: &[String], index: &mut usize, name: &str) -> Result<String, String> {
    *index += 1;
    arguments
        .get(*index)
        .cloned()
        .ok_or_else(|| format!("{} needs a value", name))
}

fn resolve_roots(requested: &[String], cwd: &Path) -> Result<Vec<PathBuf>, String> {
    let mut roots: Vec<PathBuf> = Vec::new();
    for raw in requested {
        let path = resolve_path(raw, cwd);
        if !path.exists() {
            return Err(format!("path does not exist: {}", raw));
        }
        roots.push(scan::resolve_path(&path));
    }
    Ok(roots)
}

/// `--config` and `--no-config` are read before anything else, because the
/// file they name supplies the defaults every other flag overrides.
fn config_request(arguments: &[String]) -> Result<(Option<String>, bool), String> {
    let mut path: Option<String> = None;
    let mut disabled = false;
    for (index, argument) in arguments.iter().enumerate() {
        match argument.as_str() {
            "--config" => {
                path = Some(
                    arguments
                        .get(index + 1)
                        .cloned()
                        .ok_or_else(|| "--config needs a value".to_string())?,
                );
            }
            "--no-config" => disabled = true,
            other => {
                if let Some(value) = other.strip_prefix("--config=") {
                    path = Some(value.to_string());
                }
            }
        }
    }
    Ok((path, disabled))
}

/// Flags that eat the argument after them, so the pre-pass can tell a scan
/// root apart from a flag value.
const VALUE_FLAGS: [&str; 15] = [
    "--snapshot",
    "--baseline",
    "--root",
    "--config",
    "--format",
    "--sort",
    "--top",
    "--jobs",
    "--exclude",
    "--ignore-dir",
    "--fail-if",
    "--min-gen-line-length",
    "--top-functions",
    "--cocomo-params",
    "--cocomo-project-type",
];

/// The first scan root on the command line, used to find the config before
/// the real parse has run.
fn first_positional(arguments: &[String]) -> Option<String> {
    let mut index = 0usize;
    while index < arguments.len() {
        let argument = arguments[index].as_str();
        if argument.starts_with('-') && argument != "-" {
            if VALUE_FLAGS.contains(&argument) {
                index += 1;
            }
        } else {
            return Some(argument.to_string());
        }
        index += 1;
    }
    None
}

/// `--config`, else `cwd/.rustloc.toml`, else the first scan root, so
/// `rust-loc ../some/project` picks up that project configuration.
fn locate_config(
    arguments: &[String],
    config_path: Option<&str>,
    no_config: bool,
    cwd: &Path,
) -> Result<Option<config::Config>, String> {
    if no_config {
        return Ok(None);
    }
    if let Some(loaded) = config::load(config_path.map(Path::new), cwd)? {
        return Ok(Some(loaded));
    }
    if config_path.is_some() {
        return Ok(None);
    }
    let Some(root) = first_positional(arguments) else {
        return Ok(None);
    };
    let path = Path::new(&root);
    let directory = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    config::load_from_dir(&directory)
}

/// Parse arguments (without the program name) against `cwd`.
pub fn parse(arguments: &[String], cwd: &Path) -> Result<Invocation, String> {
    let (config_path, no_config) = config_request(arguments)?;
    let config = locate_config(arguments, config_path.as_deref(), no_config, cwd)?;
    let mut options = Options::default();
    let mut positionals: Vec<String> = Vec::new();
    let mut extra_roots: Vec<String> = Vec::new();
    let mut config_roots: Vec<String> = Vec::new();
    let mut format_given = false;
    let mut ignore_dirs_given: BTreeSet<String> = BTreeSet::new();
    let mut no_ignore_dirs = false;
    let mut json_alias = false;
    if let Some(loaded) = &config {
        loaded.apply(&mut options)?;
        ignore_dirs_given.extend(loaded.ignore_dirs.iter().cloned());
        no_ignore_dirs = loaded.default_ignores == Some(false);
        config_roots = loaded.roots.clone();
        options.config = Some(loaded.path.clone());
    }

    let mut index = 0usize;
    while index < arguments.len() {
        let argument = arguments[index].clone();
        let argument = argument.as_str();
        match argument {
            "-h" | "--help" => {
                return Ok(Invocation {
                    action: Action::Help,
                    options,
                    roots: Vec::new(),
                })
            }
            "-V" | "--version" => {
                return Ok(Invocation {
                    action: Action::Version,
                    options,
                    roots: Vec::new(),
                })
            }
            "--json" => json_alias = true,
            "--config" => {
                let _ = take(arguments, &mut index, "--config")?;
            }
            "--no-config" => {}
            other if other.starts_with("--config=") => {}
            "--snapshot" => {
                let value = take(arguments, &mut index, "--snapshot")?;
                options.snapshot = Some(resolve_path(&value, cwd));
            }
            "--baseline" => {
                let value = take(arguments, &mut index, "--baseline")?;
                options.baseline = Some(baseline::Snapshot::load(&resolve_path(&value, cwd))?);
            }
            "--by-file" | "--per-file" => options.by_file = true,
            "--by-module" => options.by_module = true,
            "--show-excluded" => options.show_excluded = true,
            "--include-tests" => options.include_tests = true,
            "--uloc" => options.uloc = true,
            "--no-complexity" => options.complexity = false,
            "--functions" => options.functions = true,
            "--smells" => options.smells = true,
            "--hotspots" => options.hotspots = true,
            "--metrics" => options.metrics = true,
            "--languages" | "--langs" => options.languages = true,
            "--exclude-generated" | "--no-gen" => options.exclude_generated = true,
            "--mark-generated" => options.mark_generated = true,
            "--cocomo" => options.cocomo = true,
            "--fail-if" => {
                let value = take(arguments, &mut index, "--fail-if")?;
                options
                    .fail_if
                    .push(gate::Gate::parse(&value).ok_or_else(|| {
                        format!("--fail-if needs METRIC OP VALUE, got {}", value)
                    })?);
            }
            "--ci" => options.ci = true,
            "--cocomo-project-type" => {
                let value = take(arguments, &mut index, "--cocomo-project-type")?;
                let kind = cocomo::ProjectType::parse(&value)
                    .ok_or_else(|| format!("unknown COCOMO project type: {}", value))?;
                options.cocomo = true;
                if let Some(params) = kind.params() {
                    options.cocomo_params = params;
                }
                options.cocomo_project = kind;
            }
            "--cocomo-params" => {
                let value = take(arguments, &mut index, "--cocomo-params")?;
                options.cocomo_params = cocomo::Params::parse(&value)
                    .ok_or_else(|| format!("--cocomo-params needs a,b,c,d, got {}", value))?;
                options.cocomo = true;
                options.cocomo_project = cocomo::ProjectType::Custom;
            }
            "--min-gen-line-length" => {
                let value = take(arguments, &mut index, "--min-gen-line-length")?;
                options.min_gen_line_length = value
                    .parse::<i64>()
                    .map_err(|_| format!("--min-gen-line-length needs a number, got {}", value))?
                    .max(0) as usize;
            }
            "--top-functions" => {
                let value = take(arguments, &mut index, "--top-functions")?;
                options.top_functions = value
                    .parse::<i64>()
                    .map_err(|_| format!("--top-functions needs a number, got {}", value))?
                    .max(0) as usize;
                options.functions = true;
            }
            "--gitignore" => options.gitignore = true,
            "--no-gitignore" => options.gitignore = false,
            "--no-ignore-dirs" => no_ignore_dirs = true,
            "--root" => extra_roots.push(take(arguments, &mut index, "--root")?),
            "--format" => {
                let value = take(arguments, &mut index, "--format")?;
                options.format =
                    Format::parse(&value).ok_or_else(|| format!("unknown format: {}", value))?;
                format_given = true;
            }
            "--sort" => {
                let value = take(arguments, &mut index, "--sort")?;
                if !scan::SORT_KEYS.contains(&value.as_str()) {
                    return Err(format!("unknown sort key: {}", value));
                }
                options.sort = value;
            }
            "--top" => {
                let value = take(arguments, &mut index, "--top")?;
                options.top = value
                    .parse::<i64>()
                    .map_err(|_| format!("--top needs a number, got {}", value))?
                    .max(0) as usize;
            }
            "--jobs" => {
                let value = take(arguments, &mut index, "--jobs")?;
                options.jobs = value
                    .parse::<usize>()
                    .map_err(|_| format!("--jobs needs a number, got {}", value))?;
            }
            "--exclude" => options
                .exclude
                .push(take(arguments, &mut index, "--exclude")?),
            "--ignore-dir" => {
                ignore_dirs_given.insert(take(arguments, &mut index, "--ignore-dir")?);
            }
            other if other.starts_with("--format=") => {
                let value = &other["--format=".len()..];
                options.format =
                    Format::parse(value).ok_or_else(|| format!("unknown format: {}", value))?;
                format_given = true;
            }
            other if other.starts_with("--sort=") => {
                let value = &other["--sort=".len()..];
                if !scan::SORT_KEYS.contains(&value) {
                    return Err(format!("unknown sort key: {}", value));
                }
                options.sort = value.to_string();
            }
            other if other.starts_with("--top-functions=") => {
                let value = &other["--top-functions=".len()..];
                options.top_functions = value
                    .parse::<i64>()
                    .map_err(|_| format!("--top-functions needs a number, got {}", value))?
                    .max(0) as usize;
                options.functions = true;
            }
            other if other.starts_with("--min-gen-line-length=") => {
                let value = &other["--min-gen-line-length=".len()..];
                options.min_gen_line_length = value
                    .parse::<i64>()
                    .map_err(|_| format!("--min-gen-line-length needs a number, got {}", value))?
                    .max(0) as usize;
            }
            other if other.starts_with("--cocomo-params=") => {
                let value = &other["--cocomo-params=".len()..];
                options.cocomo_params = cocomo::Params::parse(value)
                    .ok_or_else(|| format!("--cocomo-params needs a,b,c,d, got {}", value))?;
                options.cocomo = true;
                options.cocomo_project = cocomo::ProjectType::Custom;
            }
            other if other.starts_with("--cocomo-project-type=") => {
                let value = &other["--cocomo-project-type=".len()..];
                let kind = cocomo::ProjectType::parse(value)
                    .ok_or_else(|| format!("unknown COCOMO project type: {}", value))?;
                options.cocomo = true;
                if let Some(params) = kind.params() {
                    options.cocomo_params = params;
                }
                options.cocomo_project = kind;
            }
            other if other.starts_with("--fail-if=") => {
                let value = &other["--fail-if=".len()..];
                options
                    .fail_if
                    .push(gate::Gate::parse(value).ok_or_else(|| {
                        format!("--fail-if needs METRIC OP VALUE, got {}", value)
                    })?);
            }
            other if other.starts_with('-') && other != "-" => {
                return Err(format!("unknown option: {}", other));
            }
            other => positionals.push(other.to_string()),
        }
        index += 1;
    }

    if json_alias && !format_given {
        options.format = Format::Json;
    }
    options.ignore_dirs = if no_ignore_dirs {
        ignore_dirs_given
    } else {
        let mut dirs = scan::default_ignore_dirs();
        dirs.extend(ignore_dirs_given);
        dirs
    };

    let mut requested = if !positionals.is_empty() {
        positionals
    } else if !config_roots.is_empty() {
        config_roots
    } else {
        vec![".".to_string()]
    };
    requested.extend(extra_roots);
    let roots = resolve_roots(&requested, cwd)?;

    Ok(Invocation {
        action: Action::Report,
        options,
        roots,
    })
}

/// Parse, scan and render. The returned text is exactly what belongs on
/// stdout, including the trailing newline Python's `print` would add; the
/// gate diagnostics are dropped, so `--format json` stays parseable.
pub fn run(arguments: &[String], cwd: &Path) -> Result<String, String> {
    run_checked(arguments, cwd).map(|(output, _diagnostics, _status)| output)
}

/// Like run, but also reports the gate diagnostics (which belong on stderr)
/// and the exit code the quality gates imply.
pub fn run_checked(arguments: &[String], cwd: &Path) -> Result<(String, String, u8), String> {
    let invocation = parse(arguments, cwd)?;
    match invocation.action {
        Action::Help => Ok((format!("{}\n", USAGE), String::new(), 0)),
        Action::Version => Ok((format!("rust-loc {}\n", crate::VERSION), String::new(), 0)),
        Action::Report => {
            let report = report::scan(&invocation.roots, &invocation.options, cwd, None);
            if let Some(path) = &invocation.options.snapshot {
                baseline::Snapshot::of(&report, cwd).save(path)?;
            }
            let mut rendered = report::render(&report, cwd);
            if invocation.options.format != Format::Csv {
                rendered.push('\n');
            }
            let (gate_text, status) = check_gates(&invocation.options, &report)?;
            Ok((rendered, gate_text, status))
        }
    }
}

/// Evaluate every `--fail-if` rule against the business totals.
fn check_gates(options: &Options, report: &report::Report) -> Result<(String, u8), String> {
    if options.fail_if.is_empty() {
        return Ok((String::new(), 0));
    }
    let files = report.business.len() as u32;
    let mut failures: Vec<(gate::Gate, f64)> = Vec::new();
    for rule in &options.fail_if {
        let value = rule.value(&report.summary, files, report.uloc_total)?;
        if rule.violated(value) {
            failures.push((*rule, value));
        }
    }
    if failures.is_empty() {
        return Ok((String::new(), 0));
    }
    let mut text = String::new();
    if options.ci {
        for (rule, value) in &failures {
            text.push_str(&format!(
                "::error title=rust-loc gate::{}\n",
                rule.message(*value)
            ));
        }
    } else {
        text.push_str("Quality gate failed\n");
        for (rule, value) in &failures {
            text.push_str(&format!("  {}\n", rule.message(*value)));
        }
    }
    Ok((text, 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_in(args: &[&str]) -> Result<Invocation, String> {
        let owned: Vec<String> = args.iter().map(|item| item.to_string()).collect();
        let cwd = std::env::current_dir().unwrap();
        parse(&owned, &cwd)
    }

    #[test]
    fn unknown_option_is_rejected() {
        assert!(parse_in(&["--nope"]).is_err());
    }

    #[test]
    fn missing_path_is_rejected() {
        assert!(parse_in(&["definitely/missing/path"]).is_err());
    }

    #[test]
    fn sort_key_must_be_known() {
        assert!(parse_in(&["--sort", "nope"]).is_err());
    }

    #[test]
    fn defaults_scan_the_current_directory() {
        let invocation = parse_in(&[]).unwrap();
        assert_eq!(invocation.action, Action::Report);
        assert_eq!(invocation.roots.len(), 1);
        assert_eq!(invocation.options.format, Format::Text);
        assert!(invocation.options.complexity);
        assert!(invocation.options.gitignore);
    }

    #[test]
    fn no_ignore_dirs_empties_the_default_set() {
        let invocation = parse_in(&["--no-ignore-dirs"]).unwrap();
        assert!(invocation.options.ignore_dirs.is_empty());
    }

    #[test]
    fn ignore_dir_extends_the_default_set() {
        let invocation = parse_in(&["--ignore-dir", "vendor"]).unwrap();
        assert!(invocation.options.ignore_dirs.contains("vendor"));
        assert!(invocation.options.ignore_dirs.contains("target"));
    }

    #[test]
    fn top_is_clamped_at_zero() {
        let invocation = parse_in(&["--top", "-5"]).unwrap();
        assert_eq!(invocation.options.top, 0);
    }

    #[test]
    fn equals_forms_are_accepted() {
        let invocation = parse_in(&["--format=json", "--sort=path"]).unwrap();
        assert_eq!(invocation.options.format, Format::Json);
        assert_eq!(invocation.options.sort, "path");
    }

    #[test]
    fn help_and_version_short_circuit() {
        assert_eq!(parse_in(&["--help"]).unwrap().action, Action::Help);
        assert_eq!(parse_in(&["-V"]).unwrap().action, Action::Version);
    }

    #[test]
    fn top_functions_implies_the_function_list() {
        let invocation = parse_in(&["--top-functions", "7"]).unwrap();
        assert!(invocation.options.functions);
        assert_eq!(invocation.options.top_functions, 7);
        assert!(parse_in(&["--functions"]).unwrap().options.functions);
        assert!(!parse_in(&[]).unwrap().options.functions);
        assert_eq!(
            parse_in(&["--top-functions=3"])
                .unwrap()
                .options
                .top_functions,
            3
        );
    }

    #[test]
    fn smells_switch_defaults_to_off() {
        assert!(!parse_in(&[]).unwrap().options.smells);
        assert!(parse_in(&["--smells"]).unwrap().options.smells);
    }

    #[test]
    fn smell_counters_are_sort_keys_too() {
        for key in ["smells", "unwrap", "expect", "allow"] {
            let invocation = parse_in(&["--sort", key]).unwrap();
            assert_eq!(invocation.options.sort, key);
        }
    }

    #[test]
    fn generated_switches_default_to_off() {
        let bare = parse_in(&[]).unwrap().options;
        assert!(!bare.exclude_generated);
        assert!(!bare.mark_generated);
        assert_eq!(
            bare.min_gen_line_length,
            crate::generated::DEFAULT_MIN_LINE_LENGTH
        );
        let marked = parse_in(&["--exclude-generated", "--mark-generated"])
            .unwrap()
            .options;
        assert!(marked.exclude_generated);
        assert!(marked.mark_generated);
        assert_eq!(
            parse_in(&["--no-gen", "--min-gen-line-length=40"])
                .unwrap()
                .options
                .min_gen_line_length,
            40
        );
    }

    #[test]
    fn cocomo_switches_are_parsed() {
        let bare = parse_in(&[]).unwrap().options;
        assert!(!bare.cocomo);
        assert_eq!(bare.cocomo_project, cocomo::ProjectType::Organic);
        assert_eq!(bare.cocomo_params, cocomo::ORGANIC);
        let embedded = parse_in(&["--cocomo-project-type=embedded"])
            .unwrap()
            .options;
        assert!(embedded.cocomo);
        assert_eq!(embedded.cocomo_params, cocomo::EMBEDDED);
        let custom = parse_in(&["--cocomo-params", "1,2,3,4"]).unwrap().options;
        assert_eq!(custom.cocomo_project, cocomo::ProjectType::Custom);
        assert_eq!(custom.cocomo_params.a, 1.0);
        assert!(parse_in(&["--cocomo-project-type", "nope"]).is_err());
        assert!(parse_in(&["--cocomo-params", "1,2,3"]).is_err());
    }

    #[test]
    fn the_first_positional_skips_flag_values() {
        let args = |items: &[&str]| {
            items
                .iter()
                .map(|item| item.to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            first_positional(&args(&["--format", "json", "src"])),
            Some("src".to_string())
        );
        assert_eq!(
            first_positional(&args(&["src", "--format", "json"])),
            Some("src".to_string())
        );
        assert_eq!(first_positional(&args(&["--no-gitignore"])), None);
        assert_eq!(first_positional(&args(&[])), None);
    }
}
