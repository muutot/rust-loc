//! Repository configuration: a small `.rustloc.toml` reader.
//!
//! Teams pin their scan settings in the repository so a local run and CI agree
//! on the scope, the excluded globs and the quality gates. The reader
//! understands exactly the TOML subset those settings need: `[section]`
//! headers, `key = value` pairs, `#` comments, basic and literal strings,
//! integers, floats, booleans and flat arrays, on one line or several.
//!
//! Anything else - an unknown section, an unknown key, a value of the wrong
//! shape - is an error naming the file, the line and the key, so a typo never
//! silently changes a report.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::cocomo;
use crate::gate;
use crate::options::{Format, Options};

/// The file `load` looks for in the working directory.
pub const CONFIG_FILE_NAME: &str = ".rustloc.toml";

/// The sections the reader accepts.
const SECTIONS: [&str; 3] = ["scope", "report", "gate"];

/// A parsed `.rustloc.toml`. Every key is optional, so a missing key leaves
/// the command line default alone.
#[derive(Debug, Default, Clone)]
pub struct Config {
    /// Where the values came from; echoed in errors and in the report.
    pub path: PathBuf,
    pub roots: Vec<String>,
    pub exclude: Option<Vec<String>>,
    pub ignore_dirs: Vec<String>,
    pub default_ignores: Option<bool>,
    pub gitignore: Option<bool>,
    pub include_tests: Option<bool>,
    pub sources: Option<crate::sources::Sources>,
    pub exclude_ext: BTreeSet<String>,
    pub max_file_size_mb: Option<u64>,
    pub exclude_generated: Option<bool>,
    pub mark_generated: Option<bool>,
    pub min_gen_line_length: Option<usize>,
    pub format: Option<Format>,
    pub by_file: Option<bool>,
    pub by_module: Option<bool>,
    pub show_excluded: Option<bool>,
    pub sort: Option<String>,
    pub top: Option<usize>,
    pub complexity: Option<bool>,
    pub uloc: Option<bool>,
    pub functions: Option<bool>,
    pub top_functions: Option<usize>,
    pub smells: Option<bool>,
    pub authors: Option<bool>,
    pub cocomo: Option<bool>,
    pub cocomo_project: Option<cocomo::ProjectType>,
    pub cocomo_params: Option<cocomo::Params>,
    pub jobs: Option<usize>,
    pub fail_if: Vec<String>,
}

impl Config {
    /// Fold the configured values into `options`. Command line flags are
    /// applied afterwards, so anything given there still wins; the list valued
    /// keys (exclude globs, ignored directories, gates) only ever add to what
    /// the config asked for.
    pub fn apply(&self, options: &mut Options) -> Result<(), String> {
        macro_rules! set {
            ($($field:ident),* $(,)?) => {
                $( if let Some(value) = self.$field { options.$field = value; } )*
            };
        }
        set!(
            gitignore,
            include_tests,
            sources,
            exclude_generated,
            mark_generated,
            by_file,
            by_module,
            show_excluded,
            complexity,
            uloc,
            functions,
            smells,
            authors,
            cocomo,
            min_gen_line_length,
            max_file_size_mb,
            top,
            jobs,
        );
        // The list valued keys only ever add to what the config asked for, the
        // way the repeatable command line flags do.
        for name in &self.exclude_ext {
            options.exclude_ext.insert(name.clone());
        }
        if let Some(value) = self.top_functions {
            options.top_functions = value;
        }
        if let Some(value) = &self.sort {
            options.sort = value.clone();
        }
        if let Some(value) = self.format {
            options.format = value;
        }
        if let Some(value) = self.cocomo_project {
            options.cocomo_project = value;
        }
        if let Some(value) = self.cocomo_params {
            options.cocomo_params = value;
        }
        if let Some(values) = &self.exclude {
            options.exclude = values.clone();
        }
        for spec in &self.fail_if {
            let rule = gate::Gate::parse(spec).ok_or_else(|| {
                format!(
                    "{}: `fail_if` needs METRIC OP VALUE, got {}",
                    self.path.display(),
                    spec
                )
            })?;
            options.fail_if.push(rule);
        }
        Ok(())
    }
}

/// Read the config named by `--config`, or the one in `cwd` when the flag is
/// absent. `Ok(None)` means "no config, use the defaults".
pub fn load(explicit: Option<&Path>, cwd: &Path) -> Result<Option<Config>, String> {
    let path = match explicit {
        Some(path) => {
            let full = if path.is_absolute() {
                path.to_path_buf()
            } else {
                cwd.join(path)
            };
            if !full.is_file() {
                return Err(format!("config file not found: {}", full.display()));
            }
            full
        }
        None => {
            let candidate = cwd.join(CONFIG_FILE_NAME);
            if !candidate.is_file() {
                return Ok(None);
            }
            candidate
        }
    };
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {}", path.display(), error))?;
    parse(&text, &path).map(Some)
}

/// Read `dir/.rustloc.toml` when it exists, so a scan root can supply its own
/// configuration. `Ok(None)` means the directory has no config file.
pub fn load_from_dir(dir: &Path) -> Result<Option<Config>, String> {
    let candidate = dir.join(CONFIG_FILE_NAME);
    if !candidate.is_file() {
        return Ok(None);
    }
    load(Some(&candidate), dir)
}

/// Parse the subset of TOML a scan configuration needs.
pub fn parse(text: &str, path: &Path) -> Result<Config, String> {
    let text = text.trim_start_matches('\u{feff}');
    let mut config = Config {
        path: path.to_path_buf(),
        ..Config::default()
    };
    let mut section = String::new();
    let mut statements: Vec<(usize, String)> = Vec::new();
    let mut buffer = String::new();
    let mut start = 1usize;
    for (index, raw) in text.lines().enumerate() {
        let line = strip_comment(raw);
        if buffer.trim().is_empty() && line.trim().is_empty() {
            continue;
        }
        if buffer.trim().is_empty() {
            start = index + 1;
        }
        buffer.push_str(&line);
        buffer.push(' ');
        match bracket_depth(&buffer) {
            None => return Err(error(path, index + 1, "unterminated string")),
            Some(depth) if depth > 0 => continue,
            Some(_) => {}
        }
        statements.push((start, buffer.trim().to_string()));
        buffer.clear();
    }
    if !buffer.trim().is_empty() {
        return Err(error(path, start, "unterminated array"));
    }
    for (line, statement) in statements {
        apply(&mut config, &mut section, &statement, line)?;
    }
    Ok(config)
}

fn error(path: &Path, line: usize, message: &str) -> String {
    format!("{}:{}: {}", path.display(), line, message)
}

/// One statement: a section header or a `key = value` pair.
fn apply(
    config: &mut Config,
    section: &mut String,
    statement: &str,
    line: usize,
) -> Result<(), String> {
    let path = config.path.clone();
    if statement.starts_with('[') {
        if !statement.ends_with(']') || statement.starts_with("[[") {
            return Err(error(&path, line, "malformed section header"));
        }
        let name = statement
            .trim_start_matches('[')
            .trim_end_matches(']')
            .trim()
            .to_lowercase();
        if !SECTIONS.contains(&name.as_str()) {
            return Err(error(
                &path,
                line,
                &format!(
                    "unknown section [{}]; expected one of {}",
                    name,
                    SECTIONS.join(", ")
                ),
            ));
        }
        *section = name;
        return Ok(());
    }
    let (key, value) = statement
        .split_once('=')
        .ok_or_else(|| error(&path, line, "expected `key = value`"))?;
    let key = key.trim().to_lowercase().replace('-', "_");
    let value = value.trim();
    if value.is_empty() {
        return Err(error(&path, line, &format!("`{}` needs a value", key)));
    }
    let unknown = || {
        error(
            &path,
            line,
            &format!("unknown key `{}` in [{}]", key, section),
        )
    };
    let boolean = || boolean(&path, line, &key, value);
    let count = || count(&path, line, &key, value);
    let string = || string(&path, line, &key, value);
    let strings = || string_list(&path, line, &key, value);
    match (section.as_str(), key.as_str()) {
        ("scope", "roots") => config.roots = strings()?,
        ("scope", "exclude") => config.exclude = Some(strings()?),
        ("scope", "ignore_dirs") => config.ignore_dirs = strings()?,
        ("scope", "default_ignores") => config.default_ignores = Some(boolean()?),
        ("scope", "gitignore") => config.gitignore = Some(boolean()?),
        ("scope", "include_tests") => config.include_tests = Some(boolean()?),
        ("scope", "sources") => {
            // One quoted string naming a language, a comma separated list, an
            // array of names, or `all`: whichever spelling the team prefers.
            let items = match string_list(&path, line, &key, value) {
                Ok(items) => items,
                Err(_) => vec![string()?],
            };
            let name = items.join(",");
            config.sources = Some(
                crate::sources::Sources::parse(&name)
                    .filter(|selection| !selection.is_empty())
                    .ok_or_else(|| {
                        let message = format!("unknown source selection `{}`", name);
                        error(&path, line, &message)
                    })?,
            );
        }
        ("scope", "exclude_ext") => {
            let items = match string_list(&path, line, &key, value) {
                Ok(items) => items,
                Err(_) => vec![string()?],
            };
            for name in items {
                for name in name.split(',') {
                    let name = name.trim().trim_start_matches('.').to_ascii_lowercase();
                    if !name.is_empty() {
                        config.exclude_ext.insert(name);
                    }
                }
            }
        }
        ("scope", "max_file_size") => {
            config.max_file_size_mb = Some(
                count()?
                    .try_into()
                    .map_err(|_| error(&path, line, "`max_file_size` is too large"))?,
            );
        }
        ("scope", "exclude_generated") => config.exclude_generated = Some(boolean()?),
        ("scope", "mark_generated") => config.mark_generated = Some(boolean()?),
        ("scope", "min_gen_line_length") => config.min_gen_line_length = Some(count()?),
        ("report", "format") => {
            let name = string()?;
            config.format = Some(
                Format::parse(&name)
                    .ok_or_else(|| error(&path, line, &format!("unknown format `{}`", name)))?,
            );
        }
        ("report", "sort") => {
            let name = string()?;
            if !crate::scan::SORT_KEYS.contains(&name.as_str()) {
                return Err(error(&path, line, &format!("unknown sort key `{}`", name)));
            }
            config.sort = Some(name);
        }
        ("report", "by_file") | ("report", "per_file") => config.by_file = Some(boolean()?),
        ("report", "by_module") => config.by_module = Some(boolean()?),
        ("report", "show_excluded") => config.show_excluded = Some(boolean()?),
        ("report", "top") => config.top = Some(count()?),
        ("report", "complexity") => config.complexity = Some(boolean()?),
        ("report", "uloc") => config.uloc = Some(boolean()?),
        ("report", "functions") => config.functions = Some(boolean()?),
        ("report", "top_functions") => {
            config.top_functions = Some(count()?);
            config.functions = Some(true);
        }
        ("report", "smells") => config.smells = Some(boolean()?),
        ("report", "authors") => config.authors = Some(boolean()?),
        ("report", "cocomo") => config.cocomo = Some(boolean()?),
        ("report", "cocomo_project") => {
            let name = string()?;
            let kind = cocomo::ProjectType::parse(&name)
                .ok_or_else(|| error(&path, line, &format!("unknown COCOMO type `{}`", name)))?;
            if let Some(params) = kind.params() {
                config.cocomo_params = Some(params);
            }
            config.cocomo_project = Some(kind);
            config.cocomo = Some(true);
        }
        ("report", "cocomo_params") => {
            let params = number_list(&path, line, &key, value)?;
            if params.len() != 4 {
                return Err(error(&path, line, "cocomo_params needs a, b, c, d"));
            }
            config.cocomo_params = Some(cocomo::Params {
                a: params[0],
                b: params[1],
                c: params[2],
                d: params[3],
            });
            config.cocomo_project = Some(cocomo::ProjectType::Custom);
            config.cocomo = Some(true);
        }
        ("report", "jobs") => config.jobs = Some(count()?),
        ("gate", "fail_if") => config.fail_if = strings()?,
        ("", _) => return Err(error(&path, line, "put `key = value` under a [section]")),
        _ => return Err(unknown()),
    }
    Ok(())
}

/// A `true`/`false` value.
fn boolean(path: &Path, line: usize, key: &str, value: &str) -> Result<bool, String> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(error(
            path,
            line,
            &format!("`{}` needs true or false, got {}", key, value),
        )),
    }
}

/// A non negative integer.
fn count(path: &Path, line: usize, key: &str, value: &str) -> Result<usize, String> {
    let cleaned: String = value.chars().filter(|ch| *ch != '_').collect();
    match cleaned.parse::<i64>() {
        Ok(number) if number >= 0 => Ok(number as usize),
        _ => Err(error(
            path,
            line,
            &format!("`{}` needs a whole number, got {}", key, value),
        )),
    }
}

/// A basic or literal string.
fn string(path: &Path, line: usize, key: &str, value: &str) -> Result<String, String> {
    parse_string(value).ok_or_else(|| {
        error(
            path,
            line,
            &format!("`{}` needs a quoted string, got {}", key, value),
        )
    })
}

/// An array of strings.
fn string_list(path: &Path, line: usize, key: &str, value: &str) -> Result<Vec<String>, String> {
    let items = split_array(value).ok_or_else(|| {
        error(
            path,
            line,
            &format!("`{}` needs an array, got {}", key, value),
        )
    })?;
    items
        .iter()
        .map(|item| {
            parse_string(item).ok_or_else(|| {
                error(
                    path,
                    line,
                    &format!("`{}` needs quoted strings, got {}", key, item),
                )
            })
        })
        .collect()
}

/// An array of numbers.
fn number_list(path: &Path, line: usize, key: &str, value: &str) -> Result<Vec<f64>, String> {
    let items = split_array(value).ok_or_else(|| {
        error(
            path,
            line,
            &format!("`{}` needs an array, got {}", key, value),
        )
    })?;
    items
        .iter()
        .map(|item| {
            item.replace('_', "").parse::<f64>().map_err(|_| {
                error(
                    path,
                    line,
                    &format!("`{}` needs numbers, got {}", key, item),
                )
            })
        })
        .collect()
}

/// Strip a `#` comment, leaving `#` inside a string alone.
fn strip_comment(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for ch in line.chars() {
        if let Some(open) = quote {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' && open == '"' {
                escaped = true;
            } else if ch == open {
                quote = None;
            }
            continue;
        }
        match ch {
            '#' => break,
            '"' | '\'' => {
                quote = Some(ch);
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}

/// Open `[` count outside strings; `None` for an unterminated string.
fn bracket_depth(text: &str) -> Option<i32> {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for ch in text.chars() {
        if let Some(open) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' && open == '"' {
                escaped = true;
            } else if ch == open {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => quote = Some(ch),
            '[' => depth += 1,
            ']' => depth -= 1,
            _ => {}
        }
    }
    if quote.is_some() {
        None
    } else {
        Some(depth)
    }
}

/// Split `[a, b]` into its trimmed elements; `None` when it is not an array.
fn split_array(value: &str) -> Option<Vec<String>> {
    let inner = value.strip_prefix('[')?.strip_suffix(']')?;
    let mut items: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for ch in inner.chars() {
        if let Some(open) = quote {
            current.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' && open == '"' {
                escaped = true;
            } else if ch == open {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => {
                quote = Some(ch);
                current.push(ch);
            }
            ',' => {
                if !current.trim().is_empty() {
                    items.push(current.trim().to_string());
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if quote.is_some() {
        return None;
    }
    if !current.trim().is_empty() {
        items.push(current.trim().to_string());
    }
    Some(items)
}

/// A quoted value, `"` with escapes or `'` as written.
fn parse_string(value: &str) -> Option<String> {
    let mut chars = value.chars();
    let quote = chars.next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    if !value.ends_with(quote) || value.chars().count() < 2 {
        return None;
    }
    let body: String = chars.collect();
    let body = &body[..body.len() - 1];
    if quote == '\'' {
        return Some(body.to_string());
    }
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next()? {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            '"' => out.push('"'),
            '\\' => out.push('\\'),
            _ => return None,
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> Config {
        parse(text, Path::new(".rustloc.toml")).expect("config should parse")
    }

    fn refused(text: &str) -> String {
        parse(text, Path::new(".rustloc.toml")).expect_err("config should be refused")
    }

    #[test]
    fn a_full_file_maps_onto_the_options() {
        let config = parsed(
            "# team defaults\n\
             [scope]\n\
             roots = [\"src\", 'crates'] # both quote styles\n\
             exclude = [\"*/generated/*\"]\n\
             ignore_dirs = [\"vendor\"]\n\
             default_ignores = true\n\
             gitignore = true\n\
             include_tests = false\n\
             min_gen_line_length = 200\n\
             [report]\n\
             format = \"markdown\"\n\
             sort = \"smells\"\n\
             by_file = true\n\
             top = 50\n\
             uloc = true\n\
             smells = true\n\
             complexity = false\n\
             top_functions = 5\n\
             authors = true\n\
             cocomo_project = \"embedded\"\n\
             jobs = 4\n\
             [gate]\n\
             fail_if = [\"code>120000\", \"complexity >= 6000\"]\n",
        );
        assert_eq!(config.roots, ["src", "crates"]);
        assert_eq!(config.exclude, Some(vec!["*/generated/*".to_string()]));
        assert_eq!(config.ignore_dirs, ["vendor"]);
        assert_eq!(config.min_gen_line_length, Some(200));
        assert_eq!(config.format, Some(Format::Markdown));
        assert_eq!(config.sort.as_deref(), Some("smells"));
        assert_eq!(config.top_functions, Some(5));
        assert_eq!(config.authors, Some(true));
        assert_eq!(config.functions, Some(true));
        assert_eq!(config.cocomo, Some(true));
        assert_eq!(config.cocomo_params, Some(cocomo::EMBEDDED));
        assert_eq!(config.jobs, Some(4));

        let mut options = Options::default();
        config.apply(&mut options).unwrap();
        assert_eq!(options.format, Format::Markdown);
        assert_eq!(options.sort, "smells");
        assert!(options.by_file);
        assert!(options.uloc);
        assert!(options.smells);
        assert!(options.functions);
        assert!(options.authors);
        assert_eq!(options.top_functions, 5);
        assert_eq!(options.top, 50);
        assert_eq!(options.jobs, 4);
        assert!(!options.complexity);
        assert_eq!(options.min_gen_line_length, 200);
        assert_eq!(options.exclude, ["*/generated/*"]);
        assert_eq!(options.fail_if.len(), 2);
        assert_eq!(options.fail_if[1].limit, 6000.0);
    }

    #[test]
    fn arrays_span_several_lines_and_keep_comments_out() {
        let config = parsed("[gate]\nfail_if = [\n  \"code>1\",  # first\n  \"smells > 0\",\n]\n");
        assert_eq!(config.fail_if, ["code>1", "smells > 0"]);
    }

    #[test]
    fn strings_keep_hashes_and_escapes() {
        let config = parsed("[scope]\nexclude = [\"src/#1/*\", \"a b\"]\n");
        assert_eq!(
            config.exclude,
            Some(vec!["src/#1/*".to_string(), "a b".to_string()])
        );
        let escaped = parsed("[scope]\nexclude = [\"a\\\"b\"]\n");
        assert_eq!(escaped.exclude, Some(vec!["a\"b".to_string()]));
    }

    #[test]
    fn empty_and_comment_only_files_are_fine() {
        assert_eq!(parsed("").roots, Vec::<String>::new());
        assert!(parsed("# nothing here\n\n").fail_if.is_empty());
    }

    #[test]
    fn mistakes_name_the_line_and_the_key() {
        assert!(refused("[nope]\nx = 1\n").contains("unknown section [nope]"));
        assert!(refused("[scope]\nnope = 1\n").contains("unknown key `nope`"));
        assert!(refused("[scope]\ngitignore = yes\n").contains("needs true or false"));
        assert!(refused("[scope]\nexclude = \"x\"\n").contains("needs an array"));
        assert!(refused("[report]\nsort = \"nope\"\n").contains("unknown sort key"));
        assert!(refused("[report]\nformat = \"xml\"\n").contains("unknown format"));
        assert!(refused("gitignore = true\n").contains("under a [section]"));
        assert!(refused("[scope]\nexclude = [\"unterminated\n").contains("unterminated"));
        let missing = load(Some(Path::new("does-not-exist.toml")), Path::new(".")).unwrap_err();
        assert!(missing.contains("config file not found"));
    }

    #[test]
    fn gates_are_checked_when_the_config_is_applied() {
        let config = parsed("[gate]\nfail_if = [\"nope\"]\n");
        let error = config.apply(&mut Options::default()).unwrap_err();
        assert!(error.contains("METRIC OP VALUE"), "{}", error);
        let good = parsed("[gate]\nfail_if = [\"smells > 0\"]\n");
        let mut options = Options::default();
        good.apply(&mut options).unwrap();
        assert_eq!(options.fail_if.len(), 1);
    }

    #[test]
    fn the_working_directory_config_is_found_by_name() {
        let scratch = std::env::temp_dir().join(format!(
            "rust-loc-config-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&scratch).unwrap();
        assert!(load(None, &scratch).unwrap().is_none());
        std::fs::write(scratch.join(CONFIG_FILE_NAME), "[report]\nsmells = true\n").unwrap();
        let config = load(None, &scratch).unwrap().unwrap();
        assert_eq!(config.smells, Some(true));
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
