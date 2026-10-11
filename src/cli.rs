//! Command line parsing and execution.
//!
//! Kept in the library (rather than `main.rs`) so the argument handling and
//! the rendered output can be exercised in-process by integration tests, the
//! same way the Python reference exposes `cli.main(argv)`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

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
        let candidate = PathBuf::from(raw);
        let path = if candidate.is_absolute() {
            candidate
        } else {
            cwd.join(&candidate)
        };
        if !path.exists() {
            return Err(format!("path does not exist: {}", raw));
        }
        roots.push(scan::resolve_path(&path));
    }
    Ok(roots)
}

/// Parse arguments (without the program name) against `cwd`.
pub fn parse(arguments: &[String], cwd: &Path) -> Result<Invocation, String> {
    let mut options = Options::default();
    let mut positionals: Vec<String> = Vec::new();
    let mut extra_roots: Vec<String> = Vec::new();
    let mut format_given = false;
    let mut gitignore_given = false;
    let mut ignore_dirs_given: BTreeSet<String> = BTreeSet::new();
    let mut no_ignore_dirs = false;
    let mut json_alias = false;

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
            "--by-file" | "--per-file" => options.by_file = true,
            "--by-module" => options.by_module = true,
            "--show-excluded" => options.show_excluded = true,
            "--include-tests" => options.include_tests = true,
            "--uloc" => options.uloc = true,
            "--no-complexity" => options.complexity = false,
            "--gitignore" => {
                options.gitignore = true;
                gitignore_given = true;
            }
            "--no-gitignore" => {
                options.gitignore = false;
                gitignore_given = true;
            }
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
    if !gitignore_given {
        options.gitignore = true;
    }
    options.ignore_dirs = if no_ignore_dirs {
        ignore_dirs_given
    } else {
        let mut dirs = scan::default_ignore_dirs();
        dirs.extend(ignore_dirs_given);
        dirs
    };

    let mut requested = if positionals.is_empty() {
        vec![".".to_string()]
    } else {
        positionals
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
/// stdout, including the trailing newline Python's `print` would add.
pub fn run(arguments: &[String], cwd: &Path) -> Result<String, String> {
    let invocation = parse(arguments, cwd)?;
    match invocation.action {
        Action::Help => Ok(format!("{}\n", USAGE)),
        Action::Version => Ok(format!("rust-loc {}\n", crate::VERSION)),
        Action::Report => {
            let report = report::scan(&invocation.roots, &invocation.options, cwd, None);
            let rendered = report::render(&report, cwd);
            Ok(if invocation.options.format == Format::Csv {
                rendered
            } else {
                format!("{}\n", rendered)
            })
        }
    }
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
}
