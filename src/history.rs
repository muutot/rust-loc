//! Scan history: one row per scan and the sparkline that reads it.
//!
//! The file is plain text with a header and a tab separated row per scan, in
//! the same spirit as a baseline snapshot: it reads well in a diff, it can be
//! committed, and writing or parsing it needs no dependencies. `--history`
//! and the GUI both append to it, and the report draws the newest values as a
//! sparkline so a team can watch the tree grow between commits.

use std::path::{Path, PathBuf};

use crate::report::Report;

/// First line of a history file.
pub const HEADER: &str = "# rust-loc history v1";

/// Rows kept; the oldest fall off the top so the file stays small.
pub const LIMIT: usize = 120;

/// Characters a text sparkline draws; the series is trimmed to the newest.
pub const SPARK_WIDTH: usize = 32;

/// The levels a sparkline climbs, lowest first.
const LEVELS: [char; 8] = [
    '\u{2581}', '\u{2582}', '\u{2583}', '\u{2584}', '\u{2585}', '\u{2586}', '\u{2587}', '\u{2588}',
];

/// The counters one scan contributes to the history.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Sample {
    /// Unix seconds; `stamp` renders what the file stores.
    pub when: u64,
    pub files: u32,
    pub lines: u32,
    pub blank: u32,
    pub comment: u32,
    pub code: u32,
    pub complexity: u32,
    pub smells: u32,
}

impl Sample {
    /// Reduce a finished scan to the row the history keeps.
    pub fn of(report: &Report, when: u64) -> Sample {
        Sample {
            when,
            files: report.business.len() as u32,
            lines: report.summary.lines,
            blank: report.summary.blank,
            comment: report.summary.comment,
            code: report.summary.code,
            complexity: report.summary.complexity,
            smells: report.summary.smells.total(),
        }
    }

    /// The counters without the timestamp, so a caller can tell whether
    /// anything changed since the previous scan.
    pub fn counters(self) -> [u32; 7] {
        [
            self.files,
            self.lines,
            self.blank,
            self.comment,
            self.code,
            self.complexity,
            self.smells,
        ]
    }

    pub fn value(self, trend: Trend) -> u32 {
        match trend {
            Trend::Files => self.files,
            Trend::Lines => self.lines,
            Trend::Blank => self.blank,
            Trend::Comment => self.comment,
            Trend::Code => self.code,
            Trend::Complexity => self.complexity,
            Trend::Smells => self.smells,
        }
    }
}

/// A counter that can be followed across scans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trend {
    Files,
    Lines,
    Blank,
    Comment,
    Code,
    Complexity,
    Smells,
}

impl Trend {
    /// Every trend, in the order the report lists them.
    pub fn all() -> &'static [Trend] {
        &[
            Trend::Files,
            Trend::Lines,
            Trend::Blank,
            Trend::Comment,
            Trend::Code,
            Trend::Complexity,
            Trend::Smells,
        ]
    }

    pub fn parse(text: &str) -> Option<Trend> {
        let name = text.trim().to_ascii_lowercase().replace('-', "_");
        Trend::all()
            .iter()
            .copied()
            .find(|trend| trend.as_str() == name)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Trend::Files => "files",
            Trend::Lines => "lines",
            Trend::Blank => "blank",
            Trend::Comment => "comment",
            Trend::Code => "code",
            Trend::Complexity => "complexity",
            Trend::Smells => "smells",
        }
    }

    /// Column heading, e.g. "Complexity (est.)".
    pub fn label(self) -> &'static str {
        match self {
            Trend::Files => "Files",
            Trend::Lines => "Lines",
            Trend::Blank => "Blank",
            Trend::Comment => "Comment",
            Trend::Code => "Code",
            Trend::Complexity => "Complexity (est.)",
            Trend::Smells => "Smells",
        }
    }
}

/// Every recorded scan, oldest first.
#[derive(Debug, Clone)]
pub struct History {
    pub path: PathBuf,
    pub samples: Vec<Sample>,
}

impl History {
    /// Load a history file; a missing file is an empty history, not an error,
    /// so the first `--history` run creates it.
    pub fn load(path: &Path) -> Result<History, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => parse(&text, path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(History {
                path: path.to_path_buf(),
                samples: Vec::new(),
            }),
            Err(error) => Err(format!("cannot read {}: {}", path.display(), error)),
        }
    }

    /// Append a sample, dropping the oldest rows past `LIMIT`.
    pub fn record(&mut self, sample: Sample) {
        self.samples.push(sample);
        if self.samples.len() > LIMIT {
            let drop = self.samples.len() - LIMIT;
            self.samples.drain(..drop);
        }
    }

    pub fn latest(&self) -> Option<Sample> {
        self.samples.last().copied()
    }

    pub fn previous(&self) -> Option<Sample> {
        if self.samples.len() < 2 {
            return None;
        }
        Some(self.samples[self.samples.len() - 2])
    }

    /// The newest `width` values of one counter, oldest first.
    pub fn series(&self, trend: Trend, width: usize) -> Vec<u32> {
        let start = self.samples.len().saturating_sub(width);
        self.samples[start..]
            .iter()
            .map(|sample| sample.value(trend))
            .collect()
    }

    /// "2026-10-08 09:12 -> 2026-10-11 12:04", or one stamp alone.
    pub fn span(&self) -> String {
        match (self.samples.first(), self.samples.last()) {
            (Some(first), Some(last)) if first.when != last.when => {
                format!("{} -> {}", stamp(first.when), stamp(last.when))
            }
            (Some(only), _) => stamp(only.when),
            _ => String::new(),
        }
    }

    /// Text form: header, column line, one row per scan.
    pub fn render(&self) -> String {
        let mut out = String::from(HEADER);
        out.push('\n');
        out.push_str("# when\tfiles\tlines\tblank\tcomment\tcode\tcomplexity\tsmells\n");
        for sample in &self.samples {
            out.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                sample.when,
                sample.files,
                sample.lines,
                sample.blank,
                sample.comment,
                sample.code,
                sample.complexity,
                sample.smells
            ));
        }
        out
    }

    /// Write the history, creating the parent directory when needed.
    pub fn save(&self) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("cannot create {}: {}", parent.display(), error))?;
            }
        }
        std::fs::write(&self.path, self.render())
            .map_err(|error| format!("cannot write {}: {}", self.path.display(), error))
    }
}

fn broken(path: &Path, line: usize, message: &str) -> String {
    format!("{}:{}: {}", path.display(), line, message)
}

/// Parse a history written by `render`; every complaint names the line.
pub fn parse(text: &str, path: &Path) -> Result<History, String> {
    let text = text.trim_start_matches('\u{feff}');
    let mut samples: Vec<Sample> = Vec::new();
    let mut header_seen = false;
    for (index, raw) in text.lines().enumerate() {
        let number = index + 1;
        let line = raw.trim_end_matches('\r');
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            if !header_seen && !line.is_empty() {
                if line != HEADER {
                    return Err(broken(path, number, "not a rust-loc history file"));
                }
                header_seen = true;
            }
            continue;
        }
        if !header_seen {
            return Err(broken(
                path,
                number,
                "missing the # rust-loc history v1 header",
            ));
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() != 8 {
            return Err(broken(
                path,
                number,
                "expected 8 tab separated fields: when, files, lines, blank, comment, code, complexity, smells",
            ));
        }
        let mut numbers = [0u64; 8];
        for (slot, field) in numbers.iter_mut().zip(fields.iter()) {
            *slot = field
                .trim()
                .parse::<u64>()
                .map_err(|_| broken(path, number, &format!("not a number: {}", field)))?;
        }
        samples.push(Sample {
            when: numbers[0],
            files: numbers[1] as u32,
            lines: numbers[2] as u32,
            blank: numbers[3] as u32,
            comment: numbers[4] as u32,
            code: numbers[5] as u32,
            complexity: numbers[6] as u32,
            smells: numbers[7] as u32,
        });
    }
    if !header_seen {
        return Err(broken(path, 1, "not a rust-loc history file"));
    }
    Ok(History {
        path: path.to_path_buf(),
        samples,
    })
}

/// "▁▂▃▄▅▆▇█" for a series, scaled to its own range so the shape reads. A
/// series that never moves draws a flat line through the middle.
pub fn sparkline(values: &[u32]) -> String {
    let (min, max) = match (values.iter().min(), values.iter().max()) {
        (Some(min), Some(max)) => (*min, *max),
        _ => return String::new(),
    };
    let span = max - min;
    let top = (LEVELS.len() - 1) as u64;
    values
        .iter()
        .map(|value| {
            let level = if span == 0 {
                top / 2
            } else {
                u64::from(*value - min) * top / u64::from(span)
            };
            LEVELS[level as usize]
        })
        .collect()
}

/// Unix seconds for "now", or 0 when the clock sits before the epoch.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// "2026-10-11 12:04" for a unix stamp, in UTC.
pub fn stamp(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        year,
        month,
        day,
        rest / 3600,
        (rest % 3600) / 60
    )
}

/// Howard Hinnant's civil-from-days, so the date needs no dependency.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = (shifted - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(when: u64, code: u32) -> Sample {
        Sample {
            when,
            files: 3,
            lines: code + 10,
            blank: 2,
            comment: 8,
            code,
            complexity: 5,
            smells: 1,
        }
    }

    fn history() -> History {
        History {
            path: PathBuf::from("h.txt"),
            samples: Vec::new(),
        }
    }

    #[test]
    fn render_round_trips_through_parse() {
        let mut history = history();
        history.record(sample(0, 10));
        history.record(sample(86_400, 14));
        let parsed = parse(&history.render(), Path::new("h.txt")).unwrap();
        assert_eq!(parsed.samples, history.samples);
        assert_eq!(parsed.series(Trend::Code, SPARK_WIDTH), [10, 14]);
    }

    #[test]
    fn the_oldest_rows_fall_off_past_the_limit() {
        let mut history = history();
        for index in 0..(LIMIT + 5) {
            history.record(sample(index as u64, index as u32));
        }
        assert_eq!(history.samples.len(), LIMIT);
        assert_eq!(history.samples[0].code, 5);
        assert_eq!(history.latest().unwrap().code, (LIMIT + 4) as u32);
    }

    #[test]
    fn only_the_newest_values_are_drawn() {
        let mut history = history();
        for index in 0..5u32 {
            history.record(sample(u64::from(index), index));
        }
        assert_eq!(history.series(Trend::Code, 2), [3, 4]);
        assert_eq!(history.series(Trend::Code, 0), [] as [u32; 0]);
    }

    #[test]
    fn a_missing_file_is_an_empty_history() {
        let path = std::env::temp_dir().join(format!(
            "rust-loc-history-missing-{}.txt",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let history = History::load(&path).unwrap();
        assert!(history.samples.is_empty());
        assert!(history.latest().is_none());
        assert_eq!(history.span(), "");
    }

    #[test]
    fn save_and_load_agree() {
        let path = std::env::temp_dir().join(format!(
            "rust-loc-history-save-{}-{}.txt",
            std::process::id(),
            now()
        ));
        let mut history = History {
            path: path.clone(),
            samples: Vec::new(),
        };
        history.record(sample(0, 10));
        history.record(sample(86_400, 14));
        history.save().unwrap();
        let loaded = History::load(&path).unwrap();
        assert_eq!(loaded.samples, history.samples);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn sparkline_climbs_from_the_minimum_to_the_maximum() {
        let art = sparkline(&[0, 1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(art.chars().count(), 8);
        assert_eq!(art.chars().next().unwrap(), LEVELS[0]);
        assert_eq!(art.chars().last().unwrap(), LEVELS[7]);
    }

    #[test]
    fn a_flat_series_draws_a_flat_line() {
        let art = sparkline(&[4, 4, 4]);
        assert_eq!(art.chars().collect::<Vec<char>>(), vec![LEVELS[3]; 3]);
        assert_eq!(sparkline(&[]), "");
    }

    #[test]
    fn stamps_render_as_utc_minutes() {
        assert_eq!(stamp(0), "1970-01-01 00:00");
        assert_eq!(stamp(1_700_000_000), "2023-11-14 22:13");
    }

    #[test]
    fn the_span_covers_the_recorded_scans() {
        let mut history = history();
        assert_eq!(history.span(), "");
        history.record(sample(0, 1));
        assert_eq!(history.span(), "1970-01-01 00:00");
        history.record(sample(86_400, 2));
        assert_eq!(history.span(), "1970-01-01 00:00 -> 1970-01-02 00:00");
        assert_eq!(history.previous().unwrap().code, 1);
    }

    #[test]
    fn trends_parse_and_round_trip() {
        for trend in Trend::all() {
            assert_eq!(Trend::parse(trend.as_str()), Some(*trend));
            assert!(!trend.label().is_empty());
        }
        assert_eq!(Trend::parse("Complexity"), Some(Trend::Complexity));
        assert_eq!(Trend::parse("nope"), None);
    }

    #[test]
    fn a_row_without_the_header_is_rejected() {
        let text = "0\t1\t2\t3\t4\t5\t6\t7\n";
        let error = parse(text, Path::new("h.txt")).unwrap_err();
        assert!(error.contains("h.txt:1"), "{}", error);
        assert!(
            error.contains("missing the # rust-loc history v1 header"),
            "{}",
            error
        );
    }

    #[test]
    fn a_broken_row_names_the_line_and_the_problem() {
        let short = format!("{}\n0\t1\t2\n", HEADER);
        let error = parse(&short, Path::new("h.txt")).unwrap_err();
        assert!(error.contains("h.txt:2"), "{}", error);
        assert!(error.contains("8 tab separated fields"), "{}", error);

        let text = format!("{}\n0\t1\t2\t3\t4\t5\t6\tnope\n", HEADER);
        let error = parse(&text, Path::new("h.txt")).unwrap_err();
        assert!(error.contains("not a number: nope"), "{}", error);
    }

    #[test]
    fn a_file_that_is_not_a_history_is_rejected() {
        let error = parse("# something else\n", Path::new("h.txt")).unwrap_err();
        assert!(error.contains("not a rust-loc history file"), "{}", error);
        let error = parse("", Path::new("h.txt")).unwrap_err();
        assert!(error.contains("not a rust-loc history file"), "{}", error);
    }
}
