//! When the authors were active, month by month.
//!
//! `--authors` also walks the commit log asking for the author and the date
//! of every commit, then folds those into a matrix: one row per author, one
//! column per month, every cell a commit count. The grid shows who is driving
//! the project now and who has gone quiet, which a blame share alone cannot
//! say.

use std::collections::BTreeMap;
use std::path::Path;

use crate::scan;

/// Months the matrix keeps, the most recent ones.
pub const MONTHS_SHOWN: usize = 12;

/// Authors the matrix keeps, the most active ones.
pub const AUTHORS_SHOWN: usize = 12;

/// One author's commits per month, aligned with [`Activity::months`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityRow {
    pub name: String,
    pub commits: Vec<u32>,
}

/// The month axis plus one row per author.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Activity {
    /// Months ascending, `YYYY-MM`.
    pub months: Vec<String>,
    /// Authors by total commits, strongest first.
    pub rows: Vec<ActivityRow>,
}

impl Activity {
    /// Walk the commit history of the repository `root` lives in. `None` when
    /// git is missing or the root is not a repository.
    pub fn collect(root: &Path) -> Option<Activity> {
        let text = scan::run_git(
            root,
            &[
                "log",
                "--no-merges",
                "--format=%an%x09%ad",
                "--date=short",
            ],
        )?;
        Some(parse(&text))
    }
}

/// Fold a `%an<TAB>YYYY-MM-DD` transcript into the matrix.
pub fn parse(text: &str) -> Activity {
    let mut counts: BTreeMap<(String, String), u32> = BTreeMap::new();
    for raw in text.lines() {
        let line = raw.trim_end_matches('\r');
        let Some((name, date)) = line.split_once('\t') else {
            continue;
        };
        if name.is_empty() || date.len() < 7 {
            continue;
        }
        *counts
            .entry((name.to_string(), date[..7].to_string()))
            .or_insert(0) += 1;
    }
    if counts.is_empty() {
        return Activity::default();
    }

    let mut months: Vec<String> = counts
        .keys()
        .map(|(_, month)| month.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let cut = months.len().saturating_sub(MONTHS_SHOWN);
    months.drain(..cut);

    let mut totals: BTreeMap<String, u32> = BTreeMap::new();
    for ((name, _), count) in &counts {
        *totals.entry(name.clone()).or_insert(0) += *count;
    }
    let mut ranked: Vec<(String, u32)> = totals.into_iter().collect();
    ranked.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
    ranked.truncate(AUTHORS_SHOWN);

    let rows = ranked
        .into_iter()
        .map(|(name, _)| ActivityRow {
            commits: months
                .iter()
                .map(|month| {
                    counts
                        .get(&(name.clone(), month.clone()))
                        .copied()
                        .unwrap_or(0)
                })
                .collect(),
            name,
        })
        .collect();
    Activity { months, rows }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commits_land_in_their_author_and_month() {
        let text = "\
Alice\t2026-01-05
Bob\t2026-01-07
Alice\t2026-02-14
Alice\t2026-01-20
";
        let activity = parse(text);
        assert_eq!(activity.months, vec!["2026-01", "2026-02"]);
        let alice = &activity.rows[0];
        assert_eq!(alice.name, "Alice");
        assert_eq!(alice.commits, vec![2, 1]);
        let bob = &activity.rows[1];
        assert_eq!(bob.commits, vec![1, 0]);
    }

    #[test]
    fn only_the_most_recent_months_survive() {
        let mut text = String::new();
        for offset in 0..15 {
            let (year, month) = (2025 + offset / 12, offset % 12 + 1);
            text.push_str(&format!("Alice\t{year}-{month:02}-01\n"));
        }
        let activity = parse(&text);
        assert_eq!(activity.months.len(), MONTHS_SHOWN);
        assert_eq!(activity.months[0], "2025-04");
        assert_eq!(activity.months[MONTHS_SHOWN - 1], "2026-03");
    }

    #[test]
    fn only_the_most_active_authors_survive() {
        let mut text = String::new();
        for author in 0..20 {
            for _ in 0..=author {
                text.push_str(&format!("user{}\t2026-01-01\n", author));
            }
        }
        let activity = parse(&text);
        assert_eq!(activity.rows.len(), AUTHORS_SHOWN);
        assert_eq!(activity.rows[0].name, "user19");
        assert_eq!(activity.rows[0].commits, vec![20]);
    }

    #[test]
    fn junk_lines_are_ignored() {
        let activity = parse("not a commit line\n\nAlice\t\n");
        assert!(activity.months.is_empty());
        assert!(activity.rows.is_empty());
    }
}
