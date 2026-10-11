//! Basic COCOMO cost estimation (Boehm, 1981), the same model `scc` reports.
//!
//! Only the business code line count feeds the model: `effort = a * KLOC^b`
//! person-months, `schedule = c * effort^d` months, `people = effort /
//! schedule`, and the dollar figure is effort times the average loaded cost of
//! one person-month.

/// Average cost of one developer month, in US dollars.
pub const COST_PER_PERSON_MONTH: f64 = 2452.67;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectType {
    Organic,
    SemiDetached,
    Embedded,
    Custom,
}

impl ProjectType {
    pub fn parse(value: &str) -> Option<ProjectType> {
        match value.to_ascii_lowercase().as_str() {
            "organic" => Some(ProjectType::Organic),
            "semi-detached" | "semidetached" | "semi_detached" => Some(ProjectType::SemiDetached),
            "embedded" => Some(ProjectType::Embedded),
            "custom" => Some(ProjectType::Custom),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ProjectType::Organic => "organic",
            ProjectType::SemiDetached => "semi-detached",
            ProjectType::Embedded => "embedded",
            ProjectType::Custom => "custom",
        }
    }

    /// The textbook coefficients; `custom` has none of its own.
    pub fn params(self) -> Option<Params> {
        match self {
            ProjectType::Organic => Some(ORGANIC),
            ProjectType::SemiDetached => Some(SEMI_DETACHED),
            ProjectType::Embedded => Some(EMBEDDED),
            ProjectType::Custom => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Params {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
}

pub const ORGANIC: Params = Params {
    a: 2.4,
    b: 1.05,
    c: 2.5,
    d: 0.38,
};

pub const SEMI_DETACHED: Params = Params {
    a: 3.0,
    b: 1.12,
    c: 2.5,
    d: 0.35,
};

pub const EMBEDDED: Params = Params {
    a: 3.6,
    b: 1.2,
    c: 2.5,
    d: 0.32,
};

impl Params {
    /// Parse "a,b,c,d" as handed to --cocomo-params.
    pub fn parse(value: &str) -> Option<Params> {
        let parts: Vec<&str> = value.split(',').map(|part| part.trim()).collect();
        if parts.len() != 4 {
            return None;
        }
        let number = |text: &str| text.parse::<f64>().ok().filter(|value| *value > 0.0);
        Some(Params {
            a: number(parts[0])?,
            b: number(parts[1])?,
            c: number(parts[2])?,
            d: number(parts[3])?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Estimate {
    /// Person-months of development effort.
    pub effort: f64,
    /// Calendar months of schedule.
    pub schedule: f64,
    /// Average staff size needed over the schedule.
    pub people: f64,
    /// Development cost in US dollars.
    pub cost: f64,
}

/// Estimate effort, schedule, staffing and cost for a business line count.
pub fn estimate(code_lines: u32, params: Params) -> Estimate {
    let kloc = code_lines as f64 / 1000.0;
    let effort = params.a * kloc.powf(params.b);
    let schedule = if effort > 0.0 {
        params.c * effort.powf(params.d)
    } else {
        0.0
    };
    let people = if schedule > 0.0 {
        effort / schedule
    } else {
        0.0
    };
    Estimate {
        effort,
        schedule,
        people,
        cost: effort * COST_PER_PERSON_MONTH,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_types_parse_and_map_to_coefficients() {
        assert_eq!(ProjectType::parse("Organic"), Some(ProjectType::Organic));
        assert_eq!(
            ProjectType::parse("semi-detached"),
            Some(ProjectType::SemiDetached)
        );
        assert_eq!(
            ProjectType::parse("semidetached"),
            Some(ProjectType::SemiDetached)
        );
        assert_eq!(ProjectType::parse("embedded"), Some(ProjectType::Embedded));
        assert_eq!(ProjectType::parse("custom"), Some(ProjectType::Custom));
        assert_eq!(ProjectType::parse("nope"), None);
        assert_eq!(ProjectType::Organic.params(), Some(ORGANIC));
        assert_eq!(ProjectType::SemiDetached.params(), Some(SEMI_DETACHED));
        assert_eq!(ProjectType::Embedded.params(), Some(EMBEDDED));
        assert_eq!(ProjectType::Custom.params(), None);
        assert_eq!(ProjectType::Custom.name(), "custom");
    }

    #[test]
    fn params_parse_four_positive_numbers() {
        assert_eq!(
            Params::parse("2.4, 1.05,2.5,0.38"),
            Some(Params {
                a: 2.4,
                b: 1.05,
                c: 2.5,
                d: 0.38
            })
        );
        for bad in ["2.4,1.05,2.5", "2.4,1.05,2.5,0", "x,1,1,1", ""] {
            assert_eq!(Params::parse(bad), None, "{}", bad);
        }
    }

    #[test]
    fn a_thousand_lines_matches_the_textbook_numbers() {
        let result = estimate(1000, ORGANIC);
        assert!((result.effort - 2.4).abs() < 1e-9, "{}", result.effort);
        assert!(
            (result.schedule - 3.4868).abs() < 1e-3,
            "{}",
            result.schedule
        );
        assert!((result.people - 0.6883).abs() < 1e-3, "{}", result.people);
        assert!((result.cost - 5886.41).abs() < 0.01, "{}", result.cost);
    }

    #[test]
    fn embedded_costs_more_than_organic() {
        let organic = estimate(50_000, ORGANIC);
        let embedded = estimate(50_000, EMBEDDED);
        assert!(embedded.effort > organic.effort);
        assert!(embedded.people > organic.people);
        assert!(embedded.cost > organic.cost);
    }

    #[test]
    fn no_code_means_no_estimate() {
        let result = estimate(0, ORGANIC);
        assert_eq!(
            result,
            Estimate {
                effort: 0.0,
                schedule: 0.0,
                people: 0.0,
                cost: 0.0
            }
        );
    }
}
