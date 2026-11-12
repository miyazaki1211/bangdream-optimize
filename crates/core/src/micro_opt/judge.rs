//! Judgement windows, combo multipliers and life penalties.
//!
//! The default window table replicates the game's measured PERFECT windows
//! (converted to frames of 1/60 s; ms values are approximate):
//!
//! - tap / flick / directional flick / straight slide start / long start:
//!   PERFECT −41.667 ms ~ +41.667 ms (−2.5 ~ +2.5 frames, inclusive).
//! - straight slide end: PERFECT −58.333 ms ~ +58.333 ms
//!   (−3.5 ~ +3.5 frames, **exclusive**).
//! - shifting (lane-moving) slide start and end:
//!   PERFECT −58.333 ms ~ +208.333 ms (−3.5 ~ +12.5 frames, inclusive).
//!
//! The GREAT/GOOD/BAD rows keep Bestdori's song optimizer values, as do the
//! longEnd/longFlick windows (longStart PERFECT is narrowed to ±2.5 like the
//! tap family, its lower ranks stay ±6.5/±7.5/±8.5). Callers can override
//! any kind through [`JudgementWindowSet::from_kind_overrides`].

use super::events::{MicroHitKind, MICRO_HIT_KINDS};

pub type Judgement = u8;

pub const PERFECT: Judgement = 0;
pub const GREAT: Judgement = 1;
pub const GOOD: Judgement = 2;
pub const BAD: Judgement = 3;
pub const MISS: Judgement = 4;

/// Accuracy multipliers indexed by judgement rank.
///
/// `perfect = 1.1` is Bestdori's empirical calibration of the in-game note
/// score; the value is kept as-is so results stay bit-compatible.
pub const JUDGEMENT_ACCURACY: [f64; 5] = [1.1, 0.8, 0.5, 0.0, 0.0];

pub fn judgement_name(judgement: Judgement) -> &'static str {
    match judgement {
        PERFECT => "perfect",
        GREAT => "great",
        GOOD => "good",
        BAD => "bad",
        _ => "miss",
    }
}

/// One inclusive-or-open frame interval.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JudgementWindow {
    pub min: f64,
    pub max: f64,
    /// `true`: `min <= offset <= max` hits the window; `false`: the bounds
    /// are excluded.
    pub inclusive: bool,
}

impl JudgementWindow {
    pub const fn inclusive(min: f64, max: f64) -> Self {
        Self {
            min,
            max,
            inclusive: true,
        }
    }

    pub const fn exclusive(min: f64, max: f64) -> Self {
        Self {
            min,
            max,
            inclusive: false,
        }
    }

    fn contains(&self, offset: f64) -> bool {
        if self.inclusive {
            offset >= self.min && offset <= self.max
        } else {
            offset > self.min && offset < self.max
        }
    }
}

/// Judgement window for `(kind, rank)` under the default (game-truth) table.
/// Kinds whose table only has a single window (`slideTick`, `slideFlick`)
/// return `None` for the missing ranks, matching the frontend's sparse table
/// lookup.
pub fn window(kind: MicroHitKind, rank: Judgement) -> Option<JudgementWindow> {
    if rank > BAD {
        return None;
    }
    match kind {
        MicroHitKind::Tap
        | MicroHitKind::Flick
        | MicroHitKind::DirectionalFlick
        | MicroHitKind::SlideStraightStart
        | MicroHitKind::LongStart => match rank {
            PERFECT => Some(JudgementWindow::inclusive(-2.5, 2.5)),
            GREAT => Some(JudgementWindow::inclusive(-6.5, 6.5)),
            GOOD => Some(JudgementWindow::inclusive(-7.5, 7.5)),
            _ => Some(JudgementWindow::inclusive(-8.5, 8.5)),
        },
        MicroHitKind::LongEnd | MicroHitKind::LongFlick => match rank {
            PERFECT => Some(JudgementWindow::inclusive(-4.5, 4.5)),
            GREAT => Some(JudgementWindow::inclusive(-7.5, 7.5)),
            GOOD => Some(JudgementWindow::inclusive(-8.5, 8.5)),
            _ => Some(JudgementWindow::inclusive(-9.5, 9.5)),
        },
        MicroHitKind::SlideStraightEnd => match rank {
            PERFECT => Some(JudgementWindow::exclusive(-3.5, 3.5)),
            GREAT => Some(JudgementWindow::inclusive(-6.5, 12.5)),
            GOOD => Some(JudgementWindow::inclusive(-7.5, 12.5)),
            _ => Some(JudgementWindow::inclusive(-8.5, 12.5)),
        },
        MicroHitKind::SlideShiftStart | MicroHitKind::SlideShiftEnd => match rank {
            PERFECT => Some(JudgementWindow::inclusive(-3.5, 12.5)),
            GREAT => Some(JudgementWindow::inclusive(-6.5, 12.5)),
            GOOD => Some(JudgementWindow::inclusive(-7.5, 12.5)),
            _ => Some(JudgementWindow::inclusive(-8.5, 12.5)),
        },
        MicroHitKind::SlideTick => match rank {
            PERFECT => Some(JudgementWindow::inclusive(0.0, 12.5)),
            _ => None,
        },
        MicroHitKind::SlideFlick => match rank {
            PERFECT => Some(JudgementWindow::inclusive(0.0, 6.5)),
            _ => None,
        },
    }
}

/// Complete judgement window set: per kind, the frame interval for each rank
/// (perfect, great, good, bad). Kinds whose table has a single window
/// (`slideTick`, `slideFlick`) store it in the PERFECT slot with the
/// remaining ranks empty, mirroring the sparse lookup.
#[derive(Debug, Clone, PartialEq)]
pub struct JudgementWindowSet {
    windows: [[Option<JudgementWindow>; 4]; 12],
}

impl JudgementWindowSet {
    /// The game-truth default table (see the module docs).
    pub fn game_default() -> Self {
        let mut windows = [[None; 4]; 12];
        for (kind_index, kind) in MICRO_HIT_KINDS.iter().enumerate() {
            for (rank_index, rank) in [PERFECT, GREAT, GOOD, BAD].into_iter().enumerate() {
                windows[kind_index][rank_index] = window(*kind, rank);
            }
        }
        Self { windows }
    }

    /// Window for `(kind, rank)`; `None` when the kind has no window for the
    /// rank (single-window kinds beyond PERFECT).
    pub fn window(&self, kind: MicroHitKind, rank: Judgement) -> Option<JudgementWindow> {
        self.windows[kind as usize][rank as usize]
    }

    /// Outermost search bounds of a kind: the boundary of its BAD window, or
    /// the single window for slide ticks/flicks.
    pub fn outer_bounds(&self, kind: MicroHitKind) -> (f64, f64) {
        self.window(kind, BAD)
            .or_else(|| self.window(kind, PERFECT))
            .expect("kind has at least one window")
            .into_bounds()
    }

    /// Builds a set from an optional per-kind override map (kind name ->
    /// ordered rank windows). Kinds absent from the map keep their default
    /// windows. Every present kind must list exactly the number of windows
    /// its table supports (4, or 1 for `slideTick`/`slideFlick`) with
    /// `min <= max` per pair.
    pub fn from_kind_overrides(
        overrides: &std::collections::BTreeMap<String, Vec<JudgementWindowInput>>,
    ) -> Result<Self, InvalidJudgementWindows> {
        let mut set = Self::game_default();
        for (name, pairs) in overrides {
            let kind = parse_kind(name).ok_or_else(|| InvalidJudgementWindows {
                kind: name.clone(),
                reason: format!("unknown note kind {name:?}"),
            })?;
            let expected = if matches!(kind, MicroHitKind::SlideTick | MicroHitKind::SlideFlick) {
                1
            } else {
                4
            };
            if pairs.len() != expected {
                return Err(InvalidJudgementWindows {
                    kind: name.clone(),
                    reason: format!("expected {expected} windows, got {}", pairs.len()),
                });
            }
            for (index, pair) in pairs.iter().enumerate() {
                let (low, high, inclusive) = (pair.min, pair.max, pair.inclusive);
                if !low.is_finite() || !high.is_finite() || low > high {
                    return Err(InvalidJudgementWindows {
                        kind: name.clone(),
                        reason: format!("window {index} must satisfy min <= max"),
                    });
                }
                set.windows[kind as usize][index] = Some(JudgementWindow {
                    min: low,
                    max: high,
                    inclusive,
                });
            }
        }
        Ok(set)
    }
}

/// A per-kind window override failed validation.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("invalid judgement windows for {kind}: {reason}")]
pub struct InvalidJudgementWindows {
    pub kind: String,
    pub reason: String,
}

/// Wire representation of one window override: `[min, max]` for an inclusive
/// interval or `[min, max, inclusive]` with `inclusive = false` for an open
/// interval.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JudgementWindowInput {
    pub min: f64,
    pub max: f64,
    pub inclusive: bool,
}

impl JudgementWindowInput {
    pub const fn inclusive(min: f64, max: f64) -> Self {
        Self {
            min,
            max,
            inclusive: true,
        }
    }
}

impl<'de> serde::Deserialize<'de> for JudgementWindowInput {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = JudgementWindowInput;
            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("[min, max] or [min, max, inclusive]")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                let min = seq
                    .next_element::<f64>()?
                    .ok_or_else(|| serde::de::Error::invalid_length(0, &self))?;
                let max = seq
                    .next_element::<f64>()?
                    .ok_or_else(|| serde::de::Error::invalid_length(1, &self))?;
                let inclusive = seq.next_element::<bool>()?.unwrap_or(true);
                Ok(JudgementWindowInput {
                    min,
                    max,
                    inclusive,
                })
            }
        }
        deserializer.deserialize_seq(Visitor)
    }
}

fn parse_kind(name: &str) -> Option<MicroHitKind> {
    MICRO_HIT_KINDS
        .iter()
        .find(|kind| kind.as_str() == name)
        .copied()
}

impl JudgementWindow {
    fn into_bounds(self) -> (f64, f64) {
        (self.min, self.max)
    }
}

impl Default for JudgementWindowSet {
    fn default() -> Self {
        Self::game_default()
    }
}

/// Judgement of a hit shifted by `offset` frames against the default
/// (game-truth) windows. Ranks are probed in order `perfect..=bad`; the
/// first window containing the offset wins and anything else is a miss.
pub fn judge(kind: MicroHitKind, offset: f64) -> Judgement {
    judge_in(kind, offset, &JudgementWindowSet::game_default())
}

/// [`judge`] against a custom window set (e.g. Bestdori's legacy table).
pub fn judge_in(kind: MicroHitKind, offset: f64, windows: &JudgementWindowSet) -> Judgement {
    for rank in PERFECT..=BAD {
        if let Some(bound) = windows.window(kind, rank) {
            if bound.contains(offset) {
                return rank;
            }
        }
    }
    MISS
}

/// Default search range of a kind: the boundary of its BAD window (or the
/// single window for slide ticks/flicks) under the default table. These are
/// the values Bestdori's auto-optimisation panel ships with.
pub fn default_search_bounds(kind: MicroHitKind) -> (f64, f64) {
    window(kind, BAD)
        .or_else(|| window(kind, PERFECT))
        .expect("kind has at least one window")
        .into_bounds()
}

/// Life penalty of a judged hit. Most kinds pay `50` on BAD and `100` on MISS;
/// slide ticks only pay `20` on MISS.
pub fn life_penalty(kind: MicroHitKind, judgement: Judgement) -> f64 {
    if kind == MicroHitKind::SlideTick {
        if judgement >= MISS {
            20.0
        } else {
            0.0
        }
    } else {
        match judgement {
            PERFECT | GREAT | GOOD => 0.0,
            BAD => 50.0,
            _ => 100.0,
        }
    }
}

/// Combo multiplier applied by note position in the current combo chain.
pub fn combo_mod(combo: i32) -> f64 {
    if combo < 20 {
        1.0
    } else if combo < 50 {
        1.01
    } else if combo < 100 {
        1.02
    } else if combo < 150 {
        1.03
    } else if combo < 200 {
        1.04
    } else if combo < 250 {
        1.05
    } else if combo < 300 {
        1.06
    } else if combo < 400 {
        1.07
    } else if combo < 500 {
        1.08
    } else if combo < 600 {
        1.09
    } else if combo < 700 {
        1.1
    } else {
        1.11
    }
}
