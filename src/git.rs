//! Git history: which files change the most.
//!
//! `--hotspots` ranks files by complexity times how often they change, the
//! way `scc --hotspots` does. Nothing here runs unless the flag is given,
//! because walking the history costs a subprocess and a few milliseconds.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::scan;

/// Lines added and removed, and the commits that touched one file.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Churn {
    pub added: u64,
    pub deleted: u64,
    pub commits: u64,
}

impl Churn {
    /// Lines that moved in or out of the file.
    pub fn lines(&self) -> u64 {
        self.added + self.deleted
    }
}

/// Parse `git log --no-merges --numstat --format=%H` output.
///
/// Every commit contributes a bare object-id header followed by one
/// `added<TAB>deleted<TAB>path` line per file it touched. The header is what
/// makes a commit count possible; an empty `--format=` prints no separator at
/// all, so `%H` is the format to ask git for.
pub fn parse_numstat(text: &str) -> HashMap<String, Churn> {
    let mut churn: HashMap<String, Churn> = HashMap::new();
    let mut touched: Vec<String> = Vec::new();
    let close_commit = |touched: &mut Vec<String>, churn: &mut HashMap<String, Churn>| {
        for path in touched.drain(..) {
            churn.entry(path).or_default().commits += 1;
        }
    };
    for raw in text.lines() {
        let line = raw.trim_end_matches('\r');
        if is_commit_header(line) {
            close_commit(&mut touched, &mut churn);
            continue;
        }
        let mut fields = line.split('\t');
        let (Some(added), Some(deleted), Some(path)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let entry = churn.entry(path.to_string()).or_default();
        entry.added += added.trim().parse::<u64>().unwrap_or(0);
        entry.deleted += deleted.trim().parse::<u64>().unwrap_or(0);
        if !touched.iter().any(|seen| seen == path) {
            touched.push(path.to_string());
        }
    }
    close_commit(&mut touched, &mut churn);
    churn
}

/// True for the `%H` line git prints ahead of each commit's file stats: a
/// bare object id and nothing else. Numstat rows always carry a tab.
fn is_commit_header(line: &str) -> bool {
    matches!(line.len(), 40 | 64) && line.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Churn for every path the history mentions, plus the repository root.
/// `None` means git is missing or the root is not a repository.
pub fn churn(root: &Path) -> Option<(PathBuf, HashMap<String, Churn>)> {
    let toplevel = scan::run_git(root, &["rev-parse", "--show-toplevel"])?;
    let toplevel = PathBuf::from(toplevel.trim());
    if !toplevel.is_dir() {
        return None;
    }
    let log = scan::run_git(
        &toplevel,
        &["log", "--no-merges", "--numstat", "--format=%H"],
    )?;
    Some((toplevel, parse_numstat(&log)))
}

/// Path of `path` below the repository root, with forward slashes.
pub fn repo_relative(path: &Path, toplevel: &Path) -> Option<String> {
    let rest = path.strip_prefix(toplevel).ok()?;
    Some(scan::posix(rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD_A: &str = "1111111111111111111111111111111111111111";
    const HEAD_B: &str = "2222222222222222222222222222222222222222";

    #[test]
    fn numstat_totals_lines_and_commits() {
        let text = format!("{HEAD_A}\n12\t3\tsrc/a.rs\n1\t0\tsrc/b.rs\n{HEAD_B}\n4\t4\tsrc/a.rs\n");
        let churn = parse_numstat(&text);
        let a = churn["src/a.rs"];
        assert_eq!(a.added, 16);
        assert_eq!(a.deleted, 7);
        assert_eq!(a.commits, 2);
        assert_eq!(a.lines(), 23);
        assert_eq!(churn["src/b.rs"].commits, 1);
    }

    #[test]
    fn a_commit_that_touches_a_file_twice_counts_once() {
        let text = format!("{HEAD_A}\n1\t1\tsrc/a.rs\n2\t2\tsrc/a.rs\n");
        let churn = parse_numstat(&text);
        assert_eq!(churn["src/a.rs"].commits, 1);
        assert_eq!(churn["src/a.rs"].added, 3);
    }

    #[test]
    fn binary_files_and_junk_lines_are_ignored() {
        let text = format!("{HEAD_A}\n-\t-\tlogo.png\nnot a numstat line\n7\t1\tsrc/a.rs\n");
        let churn = parse_numstat(&text);
        assert_eq!(churn["logo.png"].lines(), 0);
        assert_eq!(churn["logo.png"].commits, 1);
        assert_eq!(churn["src/a.rs"].added, 7);
    }

    #[test]
    fn a_bare_header_with_no_files_closes_cleanly() {
        let text = format!("{HEAD_A}\n{HEAD_B}\n1\t0\tsrc/a.rs\n");
        let churn = parse_numstat(&text);
        assert_eq!(churn.len(), 1);
        assert_eq!(churn["src/a.rs"].commits, 1);
    }

    #[test]
    fn repo_relative_strips_the_root() {
        let root = Path::new("/repo");
        assert_eq!(
            repo_relative(Path::new("/repo/src/a.rs"), root).unwrap(),
            "src/a.rs"
        );
        assert!(repo_relative(Path::new("/elsewhere/a.rs"), root).is_none());
    }
}
