//! Hit-event kinds retained for micro-optimisation judging.
//!
//! The standard [`crate::Chart`] collapses every note into `Node | Skill`
//! because scoring does not need the note shape. Micro-optimisation instead
//! shifts individual hits inside per-kind judgement windows, so the original
//! Bestdori note shape must survive parsing.

use serde::{Deserialize, Serialize};

/// Judgeable hit kinds, mirroring Bestdori's song optimizer event types.
///
/// Green slides split into straight (every connection stays on one lane) and
/// shifting (any lane change) variants because the game judges their start
/// and end notes in different windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MicroHitKind {
    Tap,
    Flick,
    DirectionalFlick,
    LongStart,
    LongEnd,
    LongFlick,
    SlideStraightStart,
    SlideShiftStart,
    SlideTick,
    SlideStraightEnd,
    SlideShiftEnd,
    SlideFlick,
}

impl MicroHitKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tap => "tap",
            Self::Flick => "flick",
            Self::DirectionalFlick => "directionalFlick",
            Self::LongStart => "longStart",
            Self::LongEnd => "longEnd",
            Self::LongFlick => "longFlick",
            Self::SlideStraightStart => "slideStraightStart",
            Self::SlideShiftStart => "slideShiftStart",
            Self::SlideTick => "slideTick",
            Self::SlideStraightEnd => "slideStraightEnd",
            Self::SlideShiftEnd => "slideShiftEnd",
            Self::SlideFlick => "slideFlick",
        }
    }
}

pub const MICRO_HIT_KINDS: [MicroHitKind; 12] = [
    MicroHitKind::Tap,
    MicroHitKind::Flick,
    MicroHitKind::DirectionalFlick,
    MicroHitKind::LongStart,
    MicroHitKind::LongEnd,
    MicroHitKind::LongFlick,
    MicroHitKind::SlideStraightStart,
    MicroHitKind::SlideShiftStart,
    MicroHitKind::SlideTick,
    MicroHitKind::SlideStraightEnd,
    MicroHitKind::SlideShiftEnd,
    MicroHitKind::SlideFlick,
];

/// One judgeable hit inside a chart.
///
/// `time` is derived from the chart beat through the BPM timeline, in seconds.
/// `is_skill` marks the notes the game binds the skill button to; in Bestdori
/// semantics only `Single`, `Long` connections and non-tick `Slide`
/// connections can carry the flag.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MicroHitEvent {
    pub kind: MicroHitKind,
    pub time: f64,
    pub is_skill: bool,
}

/// Sorts events exactly like Bestdori's optimizer: ascending time with
/// non-skill hits first on exact ties (the frontend's `!!t.skill - !!e.skill`
/// tie-break sorts the skill flag last).
pub fn sort_events(events: &mut [MicroHitEvent]) {
    events.sort_by(|a, b| {
        a.time
            .partial_cmp(&b.time)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.is_skill.cmp(&b.is_skill))
    });
}
