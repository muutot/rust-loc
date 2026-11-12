//! Which files change together.
//!
//! Two files that keep landing in the same commits are coupled: a change to
//! one usually demands a change to the other, which is often a sign they
//! should move or merge together. The analysis walks the same
//! `git log --no-merges --numstat` output the churn ranking reads and counts
//! how often every pair of files shares a commit. Commits that touch more
//! than [`MAX_FILES_PER_COMMIT`] files are release-style sweeps that would
//! couple everything to everything, so they pair nothing.

use std::collections::HashMap;
use std::path::Path;

use crate::scan;

/// Commits a coupling analysis reads at most.
pub const DEFAULT_MAX_COMMITS: usize = 1000;

/// Commits touching more files than this pair nothing: they are sweeps, not
/// feature changes.
pub const MAX_FILES_PER_COMMIT: usize = 50;

/// Two files and how many commits they shared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair {
    /// The path that sorts first.
    pub a: String,
    /// The path that sorts second.
    pub b: String,
    pub commits: u64,
}

/// The pairs of a history walk, strongest first.
#[derive(Debug, Clone, Default)]
pub struct Coupling {
    pub pairs: Vec<Pair>,
    /// How many commits the walk examined.
    pub commits: usize,
}

impl Coupling {
    /// The `n` strongest pairs, strongest first.
    pub fn top(&self, n: usize) -> &[Pair] {
        &self.pairs[..n.min(self.pairs.len())]
    }

    /// Walk the history of the repository `root` lives in and count the file
    /// pairs. `None` when git is missing or the root is not a repository.
    pub fn collect(root: &Path, max_commits: usize) -> Option<Coupling> {
        let text = scan::run_git(
            root,
            &[
                "log",
                "--no-merges",
                "--numstat",
                "--format=%H",
                "-n",
                &max_commits.to_string(),
            ],
        )?;
        Some(parse(&text))
    }
}

/// Count the pairs a `git log --format=%H --numstat` transcript shares.
pub fn parse(text: &str) -> Coupling {
    let mut counts: HashMap<(&str, &str), u64> = HashMap::new();
    let mut files: Vec<&str> = Vec::new();
    let mut commits = 0usize;
    for raw in text.lines() {
        let line = raw.trim_end_matches('\r');
        if crate::git::is_commit_header(line) {
            pair_commit(&mut files, &mut counts);
            commits += 1;
            continue;
        }
        let mut fields = line.split('\t');
        let (Some(_), Some(_), Some(path)) = (fields.next(), fields.next(), fields.next()) else {
            continue;
        };
        if !files.contains(&path) {
            files.push(path);
        }
    }
    pair_commit(&mut files, &mut counts);

    let mut pairs: Vec<Pair> = counts
        .into_iter()
        .map(|((a, b), commits)| Pair {
            a: a.to_string(),
            b: b.to_string(),
            commits,
        })
        .collect();
    pairs.sort_by(|left, right| {
        right
            .commits
            .cmp(&left.commits)
            .then(left.a.cmp(&right.a))
            .then(left.b.cmp(&right.b))
    });
    Coupling { pairs, commits }
}

/// Fold one commit's file list into the pair counts and reset for the next.
fn pair_commit<'t>(files: &mut Vec<&'t str>, counts: &mut HashMap<(&'t str, &'t str), u64>) {
    if files.len() > MAX_FILES_PER_COMMIT {
        files.clear();
        return;
    }
    for first in 0..files.len() {
        for second in first + 1..files.len() {
            let (a, b) = if files[first] <= files[second] {
                (files[first], files[second])
            } else {
                (files[second], files[first])
            };
            *counts.entry((a, b)).or_insert(0) += 1;
        }
    }
    files.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD_A: &str = "1111111111111111111111111111111111111111";
    const HEAD_B: &str = "2222222222222222222222222222222222222222";
    const HEAD_C: &str = "3333333333333333333333333333333333333333";

    #[test]
    fn pairs_count_the_commits_they_share() {
        let text = format!(
            "{HEAD_A}\n1\t0\tsrc/a.rs\n2\t0\tsrc/b.rs\n\
             {HEAD_B}\n3\t1\tsrc/a.rs\n4\t0\tsrc/b.rs\n5\t0\tsrc/c.rs\n\
             {HEAD_C}\n1\t0\tsrc/c.rs\n"
        );
        let coupling = parse(&text);
        assert_eq!(coupling.commits, 3);
        let a_b = coupling
            .pairs
            .iter()
            .find(|pair| pair.a == "src/a.rs" && pair.b == "src/b.rs")
            .expect("a and b share two commits");
        assert_eq!(a_b.commits, 2);
        let b_c = coupling
            .pairs
            .iter()
            .find(|pair| pair.a == "src/b.rs" && pair.b == "src/c.rs")
            .expect("b and c share one commit");
        assert_eq!(b_c.commits, 1);
        assert_eq!(coupling.pairs.len(), 3);
    }

    #[test]
    fn the_strongest_pairs_come_first() {
        let text = format!(
            "{HEAD_A}\n1\t0\tsrc/a.rs\n1\t0\tsrc/b.rs\n\
             {HEAD_B}\n1\t0\tsrc/c.rs\n1\t0\tsrc/d.rs\n\
             {HEAD_C}\n1\t0\tsrc/c.rs\n1\t0\tsrc/d.rs\n"
        );
        let coupling = parse(&text);
        assert_eq!(coupling.top(1)[0].a, "src/c.rs");
        assert_eq!(coupling.top(1)[0].commits, 2);
    }

    #[test]
    fn a_commit_touching_every_file_pairs_nothing() {
        let mut text = format!("{HEAD_A}\n");
        for index in 0..MAX_FILES_PER_COMMIT + 1 {
            text.push_str(&format!("1\t0\tsrc/f{}.rs\n", index));
        }
        text.push_str(&format!("{HEAD_B}\n1\t0\tsrc/a.rs\n1\t0\tsrc/b.rs\n"));
        let coupling = parse(&text);
        assert_eq!(coupling.commits, 2);
        assert_eq!(coupling.pairs.len(), 1, "only the small commit pairs");
    }

    #[test]
    fn a_file_repeated_in_one_commit_pairs_once() {
        let text = format!("{HEAD_A}\n1\t0\tsrc/a.rs\n2\t0\tsrc/a.rs\n1\t0\tsrc/b.rs\n");
        let coupling = parse(&text);
        assert_eq!(coupling.pairs.len(), 1);
        assert_eq!(coupling.pairs[0].commits, 1);
    }

    #[test]
    fn an_empty_history_has_no_pairs() {
        let coupling = parse("");
        assert_eq!(coupling.commits, 0);
        assert!(coupling.pairs.is_empty());
        assert!(coupling.top(5).is_empty());
    }
}
