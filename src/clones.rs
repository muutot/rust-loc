//! Copy-paste detection over normalised code lines.
//!
//! `--duplicates` answers the question a line count cannot: which parts of the
//! code are the same lines written twice. The method is the classic one used by
//! CPD and Sonar: normalise every code line, slide a window of N lines over each
//! file, and grow the windows that hash the same into the longest block they
//! share.
//!
//! Normalisation drops blank lines, comment-only lines and — unless
//! `--include-tests` says otherwise — test lines (the same lexer that classifies
//! the report decides which they are), collapses runs of whitespace and drops
//! lines with almost nothing on them, so `}` and `});` do not produce thousands
//! of "clones".

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::lexer;
use crate::scan;

/// Window size in lines unless `--dup-lines` says otherwise.
pub const DEFAULT_WINDOW: usize = 6;

/// How many sites of one clone the text report prints before "and N more".
pub const SITES_SHOWN: usize = 4;

/// One place a clone was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Site {
    pub path: String,
    /// 1-based line in the original file.
    pub line: u32,
}

/// One group of sites that share `lines` lines of normalised code.
#[derive(Debug, Clone)]
pub struct Cluster {
    pub lines: u32,
    pub sites: Vec<Site>,
}

/// What the scan found.
#[derive(Debug, Default, Clone)]
pub struct Dupes {
    pub window: usize,
    /// Groups of two or more sites, biggest first.
    pub clusters: Vec<Cluster>,
    /// Lines that belong to at least one clone, over the whole scan.
    pub duplicated_lines: u32,
    /// Normalised code lines the windows were built from.
    pub code_lines: u32,
}

impl Dupes {
    /// Share of the compared code that is a copy of something else.
    pub fn percent(&self) -> f64 {
        if self.code_lines == 0 {
            return 0.0;
        }
        f64::from(self.duplicated_lines) / f64::from(self.code_lines) * 100.0
    }
}

/// The normalised view of one file: the lines that take part in windows, and
/// where each came from. Kept per file rather than per window, which is what
/// keeps the peak memory at roughly the size of the scanned text.
struct FileLines {
    path: String,
    text: Vec<String>,
    numbers: Vec<u32>,
}

/// A run of lines that two or more sites share, before it is turned into a
/// `Cluster`. `sites` are `(file index, first normalised line)` pairs.
struct Block {
    sites: Vec<(u32, u32)>,
    lines: u32,
}

/// Find the clones across `files`, in parallel, then group them.
pub fn detect(
    files: &[PathBuf],
    cwd: &Path,
    window: usize,
    include_tests: bool,
    workers: usize,
) -> Dupes {
    let window = window.max(2);
    let prepared = prepare(files, cwd, include_tests, workers);
    let hashes: Vec<Vec<u64>> = prepared
        .iter()
        .map(|file| window_hashes(&file.text, window))
        .collect();

    // Every window that appears in more than one place is a seed; growing the
    // seeds gives the longest block each group of sites shares.
    let mut groups: HashMap<u64, Vec<(u32, u32)>> = HashMap::new();
    for (index, file) in hashes.iter().enumerate() {
        for (start, hash) in file.iter().enumerate() {
            groups
                .entry(*hash)
                .or_default()
                .push((index as u32, start as u32));
        }
    }
    let mut blocks: Vec<Block> = Vec::new();
    for sites in groups.values() {
        if sites.len() >= 2 {
            grow(sites, &hashes, window, &mut blocks);
        }
    }

    let kept = longest_blocks(blocks);
    let mut duplicated: Vec<Vec<bool>> = prepared
        .iter()
        .map(|file| vec![false; file.text.len()])
        .collect();
    let mut clusters: Vec<Cluster> = Vec::with_capacity(kept.len());
    for block in &kept {
        for (file, start) in &block.sites {
            let file = *file as usize;
            for flag in duplicated[file]
                .iter_mut()
                .take((*start + block.lines) as usize)
                .skip(*start as usize)
            {
                *flag = true;
            }
        }
        clusters.push(Cluster {
            lines: block.lines,
            sites: block
                .sites
                .iter()
                .map(|(file, start)| Site {
                    path: prepared[*file as usize].path.clone(),
                    line: prepared[*file as usize].numbers[*start as usize],
                })
                .collect(),
        });
    }

    clusters.sort_by(|left, right| {
        right
            .lines
            .cmp(&left.lines)
            .then(right.sites.len().cmp(&left.sites.len()))
            .then(left.sites[0].path.cmp(&right.sites[0].path))
            .then(left.sites[0].line.cmp(&right.sites[0].line))
    });

    Dupes {
        window,
        clusters,
        duplicated_lines: duplicated
            .iter()
            .map(|flags| flags.iter().filter(|flag| **flag).count() as u32)
            .sum(),
        code_lines: prepared.iter().map(|file| file.text.len() as u32).sum(),
    }
}

/// FNV-1a hash of every `window`-line slice of `text`.
fn window_hashes(text: &[String], window: usize) -> Vec<u64> {
    if text.len() < window {
        return Vec::new();
    }
    (0..=text.len() - window)
        .map(|start| hash_window(&text[start..start + window]))
        .collect()
}

/// Grow `sites`, which all share the same window, into the longest block they
/// share. When the sites stop matching together, the ones that still match are
/// grown on their own, so a coincidence does not shorten a real clone.
fn grow(sites: &[(u32, u32)], hashes: &[Vec<u64>], window: usize, out: &mut Vec<Block>) {
    let mut depth = 0usize;
    loop {
        let next: Vec<Option<u64>> = sites
            .iter()
            .map(|(file, start)| {
                hashes[*file as usize]
                    .get(*start as usize + depth + 1)
                    .copied()
            })
            .collect();
        let head = next[0];
        if head.is_some() && next.iter().all(|hash| *hash == head) {
            depth += 1;
            continue;
        }
        out.push(Block {
            sites: sites.to_vec(),
            lines: (window + depth) as u32,
        });
        let mut buckets: HashMap<u64, Vec<(u32, u32)>> = HashMap::new();
        for (site, hash) in sites.iter().zip(next.iter()) {
            if let Some(hash) = hash {
                buckets.entry(*hash).or_default().push(*site);
            }
        }
        for bucket in buckets.values() {
            if bucket.len() >= 2 {
                grow(bucket, hashes, window, out);
            }
        }
        return;
    }
}

/// Keep the blocks that are not already covered by a longer one, so a region
/// copied once is reported once instead of once per window position.
fn longest_blocks(blocks: Vec<Block>) -> Vec<Block> {
    let mut by_sites: HashMap<Vec<u32>, Vec<Block>> = HashMap::new();
    for block in blocks {
        let key: Vec<u32> = block.sites.iter().map(|(file, _)| *file).collect();
        by_sites.entry(key).or_default().push(block);
    }
    let mut kept: Vec<Block> = Vec::new();
    for list in by_sites.values_mut() {
        list.sort_by_key(|block| block.sites[0].1);
        let mut ends: Vec<u32> = vec![0; list[0].sites.len()];
        for block in list.drain(..) {
            let beyond = block
                .sites
                .iter()
                .enumerate()
                .any(|(index, (_, start))| start + block.lines > ends[index]);
            if beyond {
                for (index, (_, start)) in block.sites.iter().enumerate() {
                    ends[index] = ends[index].max(start + block.lines);
                }
                kept.push(block);
            }
        }
    }
    kept
}

/// FNV-1a over the normalised window, which is all a group key needs to be.
fn hash_window(lines: &[String]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for line in lines {
        for byte in line.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash ^= u64::from(b'\n');
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Read and normalise every file, in parallel.
fn prepare(files: &[PathBuf], cwd: &Path, include_tests: bool, workers: usize) -> Vec<FileLines> {
    let chunk = chunk_size(files.len(), workers);
    let mut prepared: Vec<Vec<FileLines>> = Vec::new();
    std::thread::scope(|scope| {
        let handles: Vec<_> = files
            .chunks(chunk)
            .map(|slice| {
                scope.spawn(move || {
                    slice
                        .iter()
                        .map(|path| normalise(path, cwd, include_tests))
                        .collect()
                })
            })
            .collect();
        for handle in handles {
            if let Ok(partial) = handle.join() {
                prepared.push(partial);
            }
        }
    });
    prepared.into_iter().flatten().collect()
}

/// One file, reduced to its participating lines.
fn normalise(path: &Path, cwd: &Path, include_tests: bool) -> FileLines {
    let mut file = FileLines {
        path: scan::relative(path, cwd),
        text: Vec::new(),
        numbers: Vec::new(),
    };
    let Some(text) = scan::read_text(path) else {
        return file;
    };
    let analysis = lexer::analyze(&text);
    for (index, raw) in text.lines().enumerate() {
        if index >= analysis.total || !analysis.code[index] {
            continue;
        }
        if !include_tests && analysis.is_test[index] {
            continue;
        }
        let line = normalise_line(raw);
        if line
            .chars()
            .filter(|character| character.is_alphanumeric())
            .count()
            < 2
        {
            continue;
        }
        file.text.push(line);
        file.numbers.push(index as u32 + 1);
    }
    file
}

/// Collapse runs of whitespace, so indentation and alignment do not hide a
/// clone. Comments are already out of the way: `analyze` marked the line as
/// code, and a line that is only a comment never gets here.
fn normalise_line(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut previous_space = false;
    for character in raw.chars() {
        if character.is_whitespace() {
            previous_space = true;
            continue;
        }
        if previous_space && !out.is_empty() {
            out.push(' ');
        }
        previous_space = false;
        out.push(character);
    }
    out
}

fn chunk_size(total: usize, workers: usize) -> usize {
    if total == 0 {
        return 1;
    }
    total.div_ceil(workers.max(1)).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_is_zero_without_code() {
        assert_eq!(Dupes::default().percent(), 0.0);
    }

    #[test]
    fn percent_is_duplicated_over_code() {
        let dupes = Dupes {
            window: 6,
            clusters: Vec::new(),
            duplicated_lines: 30,
            code_lines: 120,
        };
        assert!((dupes.percent() - 25.0).abs() < 1e-9);
    }

    #[test]
    fn window_hashes_need_a_full_window() {
        let text: Vec<String> = vec!["a".into(), "b".into()];
        assert!(window_hashes(&text, 6).is_empty());
        assert_eq!(window_hashes(&text, 2).len(), 1);
    }

    #[test]
    fn equal_windows_hash_the_same() {
        let one: Vec<String> = vec!["let x = 1;".into(), "y += 2;".into()];
        let two: Vec<String> = vec!["let x = 1;".into(), "y += 2;".into()];
        assert_eq!(hash_window(&one), hash_window(&two));
        assert_ne!(hash_window(&one), hash_window(&two[..1]));
    }

    /// The trio shares three lines; the pair keeps matching for a fourth, so
    /// the seed splits instead of dragging the real clone down to three.
    #[test]
    fn grow_extends_a_clone_and_splits_a_partial_one() {
        let hashes = vec![
            vec![10u64, 11, 12],
            vec![10u64, 11, 12],
            vec![10u64, 11, 20],
        ];
        let mut blocks = Vec::new();
        grow(&[(0, 0), (1, 0), (2, 0)], &hashes, 2, &mut blocks);
        let mut sizes: Vec<(u32, usize)> = longest_blocks(blocks)
            .iter()
            .map(|block| (block.lines, block.sites.len()))
            .collect();
        sizes.sort_unstable();
        assert_eq!(sizes, vec![(3, 3), (4, 2)]);
    }
}
