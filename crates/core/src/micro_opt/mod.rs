//! Bestdori-compatible theoretical maximum score with micro-operations.
//!
//! This module is a self-contained, IO-free calculation kernel. Given a chart
//! expanded into judgeable hit events, the resolved band power and the five
//! team card skills, it searches per-note frame offsets (early/late clicks
//! that keep every skill trigger within at least a GREAT judgement — either
//! PERFECT or GREAT fires the skill, skill notes are never required to be
//! GREAT) so the skill windows cover as many high-value notes as possible,
//! and returns the theoretical maximum score together with the winning
//! offset map.
//!
//! Semantics are pinned to the Bestdori song optimizer frontend (see
//! `docs/micro-optimize-design-draft.md`); the golden regression fixture
//! keeps the port bit-compatible.

pub mod error;
pub mod events;
pub mod judge;
pub mod search;
pub mod simulate;
pub mod skill;

use std::collections::BTreeMap;

use serde::Serialize;

use crate::model::schema::Attribute;
pub use error::MicroOptError;
pub use events::{MicroHitEvent, MicroHitKind, MICRO_HIT_KINDS};
pub use judge::{
    judge_in, judgement_name, InvalidJudgementWindows, Judgement, JudgementWindowInput,
    JudgementWindowSet,
};
pub use skill::MicroSkill;

/// Per-kind frame search bounds. Defaults mirror Bestdori's auto-optimisation
/// panel: the boundary of each kind's BAD judgement window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SearchOffsetTable(pub [(f64, f64); 12]);

impl Default for SearchOffsetTable {
    fn default() -> Self {
        let mut table = [(0.0, 0.0); 12];
        for (slot, kind) in MICRO_HIT_KINDS.iter().enumerate() {
            table[slot] = judge::default_search_bounds(*kind);
        }
        Self(table)
    }
}

impl SearchOffsetTable {
    pub fn bounds(self, kind: MicroHitKind) -> (f64, f64) {
        self.0[MICRO_HIT_KINDS
            .iter()
            .position(|&k| k == kind)
            .expect("kind table")]
    }
}

/// Search configuration. The default matches Bestdori's frontend defaults:
/// any skill order, default search bounds, 0.5-frame steps, no restrictions.
#[derive(Debug, Clone, PartialEq)]
pub struct MicroSearchConfig {
    pub search_offset: SearchOffsetTable,
    pub step: f64,
    pub restrictions: BTreeMap<u32, (f64, f64)>,
    /// `Some(order)` restricts the search to this five-card order (the
    /// frontend's "current setting" mode); `None` enumerates all 120.
    pub fixed_order: Option<[usize; 5]>,
}

impl Default for MicroSearchConfig {
    fn default() -> Self {
        Self {
            search_offset: SearchOffsetTable::default(),
            step: 0.5,
            restrictions: BTreeMap::new(),
            fixed_order: None,
        }
    }
}

/// Fully resolved calculation input. All game data lookups happen before this
/// point; the kernel performs no IO.
#[derive(Debug, Clone, PartialEq)]
pub struct MicroOptimizeInput {
    /// Sorted hit events (ascending time, skill-flagged first on ties).
    pub events: Vec<MicroHitEvent>,
    /// Chart play level, e.g. 25 for Expert 25.
    pub play_level: i32,
    /// Team band power already resolved (cards, potentials, area items and
    /// the event-type gated event bonus).
    pub band_power: i64,
    /// One skill per team card, resolved at its equipped level.
    pub skills: Vec<MicroSkill>,
    /// Team unification context for band/attribute unified skills.
    pub team_band_id: Option<u32>,
    pub team_attribute: Option<Attribute>,
    /// Judgement windows used by every simulation and by the default search
    /// range derivation; defaults to Bestdori's inclusive table.
    pub judgement_windows: JudgementWindowSet,
    /// Extra seconds appended to each skill window's end when deciding
    /// coverage. `0.0` (default) keeps Bestdori real-time semantics
    /// `(trigger, trigger + duration]`; `1.0 / 60.0` matches
    /// frame-boundary-inclusive engines (calc-result / Ideal60 style),
    /// letting boundary notes up to one frame past the end stay covered.
    /// Does not affect queued-window starts or reported window bounds.
    pub skill_window_end_grace: f64,
    /// Ordered physical-dependency pairs `(first, second)` of event
    /// indices: every Long/Slide connection head→tail, plus same-lane
    /// consecutive notes close enough for offsets to cross (the game feeds
    /// same-lane inputs to the earliest unhit note). The search rejects any
    /// offset map whose second hit lands before — or within one frame of —
    /// its first hit. Bestdori's frontend has no such constraints; this is
    /// a deliberate divergence. Empty = unchecked.
    pub connection_pairs: Vec<(u32, u32)>,
}

/// A skill window that actually fired in the winning run.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillWindow {
    pub slot: usize,
    /// Team card position (0-4) whose skill occupies this slot.
    pub card_index: usize,
    pub trigger_index: u32,
    pub trigger_time: f64,
    pub start: f64,
    pub end: f64,
    /// `true` when the skill queued behind an active window instead of
    /// starting at its trigger note.
    pub queued: bool,
    pub covered_notes: u32,
}

/// Final result of the micro-optimisation search.
#[derive(Debug, Clone, PartialEq)]
pub struct MicroOptimizeResult {
    /// Highest natural-timing score across all evaluated skill orders
    /// (every offset at zero): the best score achievable without
    /// micro-operations.
    pub baseline_score: i64,
    /// Theoretical maximum score after micro-optimisation (best skill order
    /// plus the winning offset map).
    pub max_score: i64,
    pub score_gain: i64,
    /// Six slots: the five team cards in firing order plus the looping slot.
    pub skill_order: [usize; 6],
    /// Non-zero frame offsets by event index, ascending.
    pub optimization: BTreeMap<u32, f64>,
    /// Sum of absolute offsets of the winning map (tie-break minimised).
    pub total_offset: f64,
    pub skill_windows: Vec<SkillWindow>,
    /// Final judgement per event index of the winning run.
    pub judgements: Vec<Judgement>,
    pub simulations: u64,
}

/// Runs the full micro-optimisation search.
///
/// The offset unit is frames of 1/60 s. Deterministic: identical inputs
/// always yield an identical result.
pub fn micro_optimize(
    input: &MicroOptimizeInput,
    config: &MicroSearchConfig,
) -> Result<MicroOptimizeResult, MicroOptError> {
    if input.events.is_empty() {
        return Err(MicroOptError::EmptyChart);
    }
    if input.skills.len() != 5 {
        return Err(MicroOptError::InvalidTeamSize {
            count: input.skills.len(),
        });
    }
    let skill_note_count = input.events.iter().filter(|event| event.is_skill).count();
    if skill_note_count > 6 {
        return Err(MicroOptError::TooManySkillNotes {
            count: skill_note_count,
        });
    }
    if config.step <= 0.0 {
        return Err(MicroOptError::InvalidStep { step: config.step });
    }

    let default_offset_table = config.search_offset == SearchOffsetTable::default();
    let bounds = |kind: MicroHitKind| {
        if default_offset_table {
            input.judgement_windows.outer_bounds(kind)
        } else {
            config.search_offset.bounds(kind)
        }
    };
    let context = search::SearchContext {
        events: &input.events,
        play_level: input.play_level,
        band_power: input.band_power,
        skills: &input.skills,
        team_band_id: input.team_band_id,
        team_attribute: input.team_attribute,
        judgement_windows: &input.judgement_windows,
        search_bounds: &bounds,
        step: config.step,
        restrictions: &config.restrictions,
        fixed_order: config.fixed_order,
        end_grace: input.skill_window_end_grace,
        pairs: &input.connection_pairs,
    };
    let outcome = search::search(&context);

    // Replay the winner once with full instrumentation for the response.
    let ordered: Vec<MicroSkill> = outcome
        .order
        .map(|card_index| input.skills[card_index].clone())
        .to_vec();
    let mut offsets = vec![0.0f64; input.events.len()];
    for (index, offset) in &outcome.optimization {
        offsets[*index as usize] = *offset;
    }
    let sim = simulate::Simulation {
        events: &input.events,
        play_level: input.play_level,
        band_power: input.band_power,
        ordered_skills: &ordered,
        team_band_id: input.team_band_id,
        team_attribute: input.team_attribute,
        judgement_windows: &input.judgement_windows,
        end_grace: input.skill_window_end_grace,
    };
    let final_run = simulate::simulate(&sim, &offsets);
    debug_assert_eq!(final_run.score, outcome.score);

    let baseline = outcome.baseline_score;

    let skill_windows = final_run
        .windows
        .iter()
        .enumerate()
        .filter_map(|(slot, window)| {
            window.map(|window| SkillWindow {
                slot,
                card_index: outcome.order[slot],
                trigger_index: window.trigger_index as u32,
                trigger_time: window.trigger_time,
                start: window.start,
                end: window.end,
                queued: window.start - window.trigger_time > 1.0e-9,
                covered_notes: window.covered_notes,
            })
        })
        .collect();

    Ok(MicroOptimizeResult {
        baseline_score: baseline,
        score_gain: outcome.score - baseline,
        max_score: outcome.score,
        skill_order: outcome.order,
        optimization: outcome.optimization,
        total_offset: outcome.total,
        skill_windows,
        judgements: final_run.judgements,
        simulations: outcome.simulations,
    })
}

/// Replayed result of one fixed arrangement.
#[derive(Debug, Clone, PartialEq)]
pub struct ArrangementEvaluation {
    pub score: i64,
    pub skill_windows: Vec<SkillWindow>,
    pub judgements: Vec<Judgement>,
}

/// Deterministically replays one fixed arrangement — a six-slot skill order
/// (team card positions, sixth slot repeating any card) plus a sparse
/// per-event frame-offset map — through the same simulator the search uses.
///
/// No searching happens; this exists to score hand-crafted plans (e.g.
/// boundary-sitting offsets the greedy cannot propose) against search
/// results.
pub fn evaluate_arrangement(
    input: &MicroOptimizeInput,
    skill_order: [usize; 6],
    optimization: &BTreeMap<u32, f64>,
) -> Result<ArrangementEvaluation, MicroOptError> {
    if input.events.is_empty() {
        return Err(MicroOptError::EmptyChart);
    }
    if input.skills.len() != 5 {
        return Err(MicroOptError::InvalidTeamSize {
            count: input.skills.len(),
        });
    }
    if skill_order.iter().any(|&card_index| card_index >= 5) {
        return Err(MicroOptError::InvalidArrangementIndex {
            index: *skill_order.iter().max().unwrap(),
        });
    }
    let mut offsets = vec![0.0f64; input.events.len()];
    for (index, offset) in optimization {
        let idx = *index as usize;
        if idx >= offsets.len() {
            return Err(MicroOptError::InvalidArrangementIndex { index: idx });
        }
        offsets[idx] = *offset;
    }
    let ordered: Vec<MicroSkill> = skill_order
        .map(|card_index| input.skills[card_index].clone())
        .to_vec();
    let sim = simulate::Simulation {
        events: &input.events,
        play_level: input.play_level,
        band_power: input.band_power,
        ordered_skills: &ordered,
        team_band_id: input.team_band_id,
        team_attribute: input.team_attribute,
        judgement_windows: &input.judgement_windows,
        end_grace: input.skill_window_end_grace,
    };
    let run = simulate::simulate(&sim, &offsets);
    let skill_windows = run
        .windows
        .iter()
        .enumerate()
        .filter_map(|(slot, window)| {
            window.map(|window| SkillWindow {
                slot,
                card_index: skill_order[slot],
                trigger_index: window.trigger_index as u32,
                trigger_time: window.trigger_time,
                start: window.start,
                end: window.end,
                queued: window.start - window.trigger_time > 1.0e-9,
                covered_notes: window.covered_notes,
            })
        })
        .collect();
    Ok(ArrangementEvaluation {
        score: run.score,
        skill_windows,
        judgements: run.judgements,
    })
}
