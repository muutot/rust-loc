//! Persisted UI settings, shared by the native front end and the web front
//! end: a tiny key=value file under the user's profile, no serialisation
//! dependency.
//!
//! The file is written wherever a front end last saved, so switching between
//! the two keeps the scan roots, filters and language ticks the reader chose.

use std::path::{Path, PathBuf};

use crate::i18n;

/// Persisted UI state; a tiny key=value file, no serialisation dependency.
#[derive(Clone)]
pub struct Settings {
    pub roots: String,
    pub gitignore: bool,
    pub default_ignores: bool,
    pub include_tests: bool,
    pub complexity: bool,
    pub uloc: bool,
    pub exclude_generated: bool,
    pub cocomo: bool,
    pub authors: bool,
    pub exclude: String,
    /// Extra directory names a scan skips, one per entry. A list rather than a
    /// comma separated field, so a reader sees every name and removes the one
    /// they mean.
    pub extra_dirs: Vec<String>,
    /// The extensions a scan skips, kept as typed so a restart restores them.
    pub exclude_ext: String,
    /// `--fail-if` gate specs the web page evaluates, comma separated. Each
    /// spec fails when the condition holds: `code>120000,fn_complexity>60`.
    pub fail_if: String,
    /// The per-file size cap in megabytes, kept as typed for the same reason.
    pub max_file_size: String,
    pub view: usize,
    /// The pane the window opens on. The tables keep the index they had, so a
    /// file written before the panes existed cannot land on the wrong one.
    pub group: usize,
    pub top: usize,
    pub format: usize,
    pub lang: usize,
    pub filter: String,
    pub sort_column: i32,
    pub sort_descending: bool,
    pub sources: String,
    /// The settings page the screen opens on.
    pub settings_page: usize,
    pub window: Option<(i32, i32, i32, i32)>,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            roots: String::new(),
            gitignore: true,
            default_ignores: true,
            include_tests: false,
            complexity: true,
            uloc: false,
            exclude_generated: false,
            cocomo: false,
            authors: false,
            exclude: String::new(),
            extra_dirs: Vec::new(),
            exclude_ext: String::new(),
            fail_if: String::new(),
            max_file_size: String::new(),
            view: 0,
            // The code pane: the panes are Overview, Authors, Code, Settings.
            group: 2,
            top: 1,
            format: 0,
            lang: i18n::system_lang().index(),
            filter: String::new(),
            sort_column: 3,
            sort_descending: true,
            sources: crate::sources::Sources::RUST.slug(),
            settings_page: 0,
            window: None,
        }
    }
}

pub fn settings_path() -> PathBuf {
    let base = std::env::var("APPDATA")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(|home| format!("{home}/.config"))
        })
        .or_else(|| {
            std::env::var("USERPROFILE")
                .ok()
                .map(|home| format!("{home}\\AppData\\Roaming"))
        })
        .unwrap_or_else(|| ".".to_string());
    Path::new(&base).join("rust-loc").join("ui.ini")
}

/// The languages the check list offers, in the order it draws them: the two
/// with a reader of their own first, then the rest of the table.
pub const SOURCE_LANGS: [crate::sources::Lang; 32] = crate::sources::Lang::ALL;

/// The selection a saved setting names. The value is a comma separated list of
/// language slugs, the way `--sources` spells one; a bare 0, 1 or 2 is a
/// setting written by an older build and is read as the choice it used to be.
pub fn sources_of(slugs: &str) -> crate::sources::Sources {
    match slugs.trim() {
        "0" => crate::sources::Sources::RUST,
        "1" => crate::sources::Sources::PYTHON,
        "2" => crate::sources::Sources::RUST.union(crate::sources::Sources::PYTHON),
        // An empty or unreadable value reads as the Rust default rather than as
        // a scan of nothing at all.
        other => crate::sources::Sources::parse(other)
            .filter(|selection| !selection.is_empty())
            .unwrap_or(crate::sources::Sources::RUST),
    }
}

/// The value a selection is saved as: one slug per ticked language.
pub fn sources_slugs(selection: crate::sources::Sources) -> String {
    selection.slug()
}

/// Paths pasted into the roots box, one per `;` or line.
pub fn parse_paths(text: &str) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    for part in text.split([';', '\r', '\n']) {
        let token = part.trim().trim_matches('"').trim();
        if !token.is_empty() && !result.iter().any(|seen| seen == token) {
            result.push(token.to_string());
        }
    }
    result
}

/// Globs typed into the exclude box, split on the usual separators.
pub fn parse_list(text: &str) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    for part in text.split([',', ';', ' ', '\t', '\r', '\n']) {
        let token = part.trim();
        if !token.is_empty() && !result.iter().any(|seen| seen == token) {
            result.push(token.to_string());
        }
    }
    result
}

/// One history file per root set, so switching projects does not splice two
/// series together. The name is an FNV-1a hash of the lowercased roots.
pub fn history_path(roots: &[String]) -> PathBuf {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for root in roots {
        for byte in root.trim().to_lowercase().bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash ^= u64::from(b'/');
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    if roots.is_empty() {
        return settings_path().with_file_name("history.tsv");
    }
    settings_path().with_file_name(format!("history-{hash:016x}.tsv"))
}

pub fn load_settings() -> Settings {
    let mut settings = Settings::default();
    let Ok(text) = std::fs::read_to_string(settings_path()) else {
        return settings;
    };
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let flag = value.trim() == "1";
        match key.trim() {
            "roots" => settings.roots = value.trim().to_string(),
            "gitignore" => settings.gitignore = flag,
            "default_ignores" => settings.default_ignores = flag,
            "include_tests" => settings.include_tests = flag,
            "complexity" => settings.complexity = flag,
            "uloc" => settings.uloc = flag,
            "exclude_generated" => settings.exclude_generated = flag,
            "cocomo" => settings.cocomo = flag,
            "authors" => settings.authors = flag,
            "exclude" => settings.exclude = value.trim().to_string(),
            "extra_dirs" => {
                settings.extra_dirs = value
                    .trim()
                    .split(',')
                    .map(|name| name.trim().to_string())
                    .filter(|name| !name.is_empty())
                    .collect();
            }
            "view" => settings.view = value.trim().parse().unwrap_or(0),
            "group" => settings.group = value.trim().parse().unwrap_or(2),
            "lang" => settings.lang = value.trim().parse().unwrap_or(settings.lang),
            "top" => settings.top = value.trim().parse().unwrap_or(1),
            "format" => settings.format = value.trim().parse().unwrap_or(0),
            "filter" => settings.filter = value.trim().to_string(),
            "sort_column" => settings.sort_column = value.trim().parse().unwrap_or(3),
            "sort_descending" => settings.sort_descending = flag,
            "sources" => settings.sources = value.trim().to_string(),
            "settings_page" => settings.settings_page = value.trim().parse().unwrap_or(0),
            "exclude_ext" => settings.exclude_ext = value.trim().to_string(),
            "fail_if" => settings.fail_if = value.trim().to_string(),
            "max_file_size" => settings.max_file_size = value.trim().to_string(),
            "window" => {
                let parts: Vec<i32> = value
                    .trim()
                    .split(',')
                    .filter_map(|part| part.trim().parse().ok())
                    .collect();
                if parts.len() == 4 {
                    settings.window = Some((parts[0], parts[1], parts[2], parts[3]));
                }
            }
            _ => {}
        }
    }
    settings
}

pub fn save_settings(settings: &Settings) {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut text = String::new();
    text.push_str(&format!("roots={}\n", settings.roots));
    text.push_str(&format!("gitignore={}\n", flag(settings.gitignore)));
    text.push_str(&format!(
        "default_ignores={}\n",
        flag(settings.default_ignores)
    ));
    text.push_str(&format!("include_tests={}\n", flag(settings.include_tests)));
    text.push_str(&format!("complexity={}\n", flag(settings.complexity)));
    text.push_str(&format!("uloc={}\n", flag(settings.uloc)));
    text.push_str(&format!(
        "exclude_generated={}\n",
        flag(settings.exclude_generated)
    ));
    text.push_str(&format!("cocomo={}\n", flag(settings.cocomo)));
    text.push_str(&format!("authors={}\n", flag(settings.authors)));
    text.push_str(&format!("exclude={}\n", settings.exclude));
    text.push_str(&format!("extra_dirs={}\n", settings.extra_dirs.join(",")));
    text.push_str(&format!("view={}\n", settings.view));
    text.push_str(&format!("group={}\n", settings.group));
    text.push_str(&format!(
        "sources={}\n",
        sources_slugs(sources_of(&settings.sources))
    ));
    text.push_str(&format!("lang={}\n", settings.lang));
    text.push_str(&format!("settings_page={}\n", settings.settings_page));
    text.push_str(&format!("exclude_ext={}\n", settings.exclude_ext));
    text.push_str(&format!("fail_if={}\n", settings.fail_if));
    text.push_str(&format!("max_file_size={}\n", settings.max_file_size));
    text.push_str(&format!("top={}\n", settings.top));
    text.push_str(&format!("format={}\n", settings.format));
    text.push_str(&format!("filter={}\n", settings.filter));
    text.push_str(&format!("sort_column={}\n", settings.sort_column));
    text.push_str(&format!(
        "sort_descending={}\n",
        flag(settings.sort_descending)
    ));
    if let Some((x, y, width, height)) = settings.window {
        text.push_str(&format!("window={x},{y},{width},{height}\n"));
    }
    let _ = std::fs::write(path, text);
}

fn flag(value: bool) -> u8 {
    if value {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_split_on_semicolons_and_lines() {
        let parsed = parse_paths("a;\r\nb\r\n\"c d\"");
        assert_eq!(parsed, vec!["a", "b", "c d"]);
    }

    #[test]
    fn lists_drop_repeats() {
        let parsed = parse_list("a, b; a\tc");
        assert_eq!(parsed, vec!["a", "b", "c"]);
    }

    #[test]
    fn history_name_follows_the_roots() {
        let one = history_path(&["B:/x".into()]);
        let again = history_path(&["b:/x".into()]);
        let other = history_path(&["B:/y".into()]);
        assert_eq!(one, again);
        assert_ne!(one, other);
        assert!(one.to_string_lossy().contains("rust-loc"));
    }
}
