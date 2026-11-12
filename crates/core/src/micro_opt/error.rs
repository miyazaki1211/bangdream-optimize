//! Errors of the micro-optimisation calculation kernel.

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum MicroOptError {
    #[error("chart must contain at least one judgeable hit event")]
    EmptyChart,
    #[error("team must contain exactly five skills, got {count}")]
    InvalidTeamSize { count: usize },
    #[error("chart has {count} skill-flagged hits, at most six are supported")]
    TooManySkillNotes { count: usize },
    #[error("search step must be positive, got {step}")]
    InvalidStep { step: f64 },
    #[error("search bounds for {kind} are empty after stepping by {step}")]
    EmptySearchRange { kind: &'static str, step: f64 },
    #[error("arrangement references index {index} outside the chart or team")]
    InvalidArrangementIndex { index: usize },
}
