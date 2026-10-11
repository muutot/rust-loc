//! Maintainability Index and the Halstead volume behind it.
//!
//! The score is the classic Oman/Hagemeister formula, without the comment
//! weight, scaled to 0..100:
//!
//! ```text
//! MI = (171 - 5.2 ln V - 0.23 G - 16.2 ln LOC) * 100 / 171
//! ```
//!
//! `V` is the Halstead program volume, `G` the cyclomatic complexity and
//! `LOC` the source lines of code. The bands match the ones
//! `rust-code-analysis` reports, so the number can be compared with a peer
//! tool rather than only with itself.

/// `0..=100`, higher is easier to maintain. An empty file scores 100.
pub fn maintainability_index(volume: f64, complexity: u32, code_lines: u32) -> f64 {
    if code_lines == 0 {
        return 100.0;
    }
    let volume_term = if volume > 0.0 { 5.2 * volume.ln() } else { 0.0 };
    let raw =
        171.0 - volume_term - 0.23 * f64::from(complexity) - 16.2 * f64::from(code_lines).ln();
    (raw * 100.0 / 171.0).clamp(0.0, 100.0)
}

/// The band a score falls into, the same five names `rust-code-analysis` uses.
pub fn band(index: f64) -> &'static str {
    if index >= 80.0 {
        "very high"
    } else if index >= 60.0 {
        "high"
    } else if index >= 40.0 {
        "moderate"
    } else if index >= 20.0 {
        "low"
    } else {
        "very low"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_file_is_perfectly_maintainable() {
        assert_eq!(maintainability_index(0.0, 0, 0), 100.0);
    }

    #[test]
    fn more_complexity_lowers_the_score() {
        let easy = maintainability_index(1000.0, 5, 100);
        let hard = maintainability_index(1000.0, 50, 100);
        assert!(hard < easy, "{hard} should be below {easy}");
    }

    #[test]
    fn more_volume_lowers_the_score() {
        let small = maintainability_index(1000.0, 10, 100);
        let large = maintainability_index(100_000.0, 10, 100);
        assert!(large < small, "{large} should be below {small}");
    }

    #[test]
    fn the_scale_is_clamped_to_zero_hundred() {
        assert_eq!(maintainability_index(f64::MAX, u32::MAX, 1), 0.0);
        assert_eq!(maintainability_index(1.0, 0, 1), 100.0);
    }

    #[test]
    fn bands_step_at_the_documented_scores() {
        assert_eq!(band(100.0), "very high");
        assert_eq!(band(80.0), "very high");
        assert_eq!(band(79.9), "high");
        assert_eq!(band(60.0), "high");
        assert_eq!(band(40.0), "moderate");
        assert_eq!(band(20.0), "low");
        assert_eq!(band(19.9), "very low");
        assert_eq!(band(0.0), "very low");
    }
}
