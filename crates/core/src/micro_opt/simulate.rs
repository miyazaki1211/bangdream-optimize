//! Deterministic single-run score simulation.
//!
//! This is a faithful port of Bestdori song optimizer's `k()` function: given
//! the hit events, a skill order and per-event frame offsets, it replays the
//! whole song once and returns the total score plus the skill windows that
//! actually fired. Ordering, rounding and float expression shapes mirror the
//! frontend so results stay bit-compatible.

use super::events::{MicroHitEvent, MicroHitKind};
use super::judge::{
    combo_mod, judge_in, life_penalty, Judgement, JudgementWindowSet, GREAT, JUDGEMENT_ACCURACY,
    MISS, PERFECT,
};
use super::skill::{MicroSkill, OnceLifeCondition};
use crate::model::schema::Attribute;

/// Seconds a queued skill waits after the previous skill's window closes.
pub const SKILL_QUEUE_GAP: f64 = 0.75;

/// Everything a simulation needs, already resolved for one skill order.
pub(crate) struct Simulation<'a> {
    pub events: &'a [MicroHitEvent],
    pub play_level: i32,
    pub band_power: i64,
    /// Six entries: the five team cards in `skill_order` order plus the
    /// looping sixth slot.
    pub ordered_skills: &'a [MicroSkill],
    pub team_band_id: Option<u32>,
    pub team_attribute: Option<Attribute>,
    /// Judgement windows the run is scored against (default: Bestdori's
    /// inclusive table).
    pub judgement_windows: &'a JudgementWindowSet,
    /// Extra seconds appended to each skill window's end when deciding
    /// coverage (0.0 = Bestdori real-time semantics `(start, end]`;
    /// `1.0/60.0` = frame-boundary-inclusive engines whose effective end
    /// covers one further frame). Does not move queued-window starts.
    pub end_grace: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct WindowInfo {
    pub start: f64,
    pub end: f64,
    pub trigger_index: usize,
    pub trigger_time: f64,
    pub covered_notes: u32,
}

pub(crate) struct SimOutcome {
    pub score: i64,
    /// Final (post judge-upgrade) judgement per event index.
    pub judgements: Vec<Judgement>,
    /// One entry per skill slot; `None` when the slot never activated.
    pub windows: Vec<Option<WindowInfo>>,
}

struct SkillRun {
    start: Option<f64>,
    end: Option<f64>,
    activated_once: bool,
    failed_continue: bool,
    perfect_count: i64,
    covered_notes: u32,
    trigger_index: usize,
    trigger_time: f64,
}

impl SkillRun {
    fn new() -> Self {
        Self {
            start: None,
            end: None,
            activated_once: false,
            failed_continue: false,
            perfect_count: 0,
            covered_notes: 0,
            trigger_index: usize::MAX,
            trigger_time: 0.0,
        }
    }
}

/// `o || value` JS truthiness: the unification value replaces the default
/// whenever it is non-zero.
fn or_value(unification: f64, value: f64) -> f64 {
    if unification != 0.0 {
        unification
    } else {
        value
    }
}

/// Applies the effect family of an active skill to one hit. Mirrors the
/// frontend's `C()` helper including its early-return paths: returning early
/// skips the trailing rate-up/score branches exactly like the original.
#[allow(clippy::too_many_arguments)]
fn apply_effects(
    def: &MicroSkill,
    kind: MicroHitKind,
    judgement: &mut Judgement,
    penalty: &mut f64,
    skill_mod: &mut f64,
    life: f64,
    run: &mut SkillRun,
    team_band_id: Option<u32>,
    team_attribute: Option<Attribute>,
) {
    let effects = &def.effects;
    let mut unification = def.unification_bonus(team_band_id, team_attribute);

    if let Some(effect) = &effects.judge {
        if (*judgement as f64) <= effect.condition.threshold() {
            *judgement = PERFECT;
        }
    }
    *penalty = life_penalty(kind, *judgement);
    if let Some(effect) = &effects.damage {
        if (*judgement as f64) <= effect.condition.threshold() {
            *penalty = 0.0;
        }
    }
    if effects.never_die {
        *penalty = (*penalty).min(life - 5.0);
    }

    if let Some(effect) = &effects.score_continued_note_judge {
        if !run.failed_continue {
            if (*judgement as f64) <= effect.condition.threshold() {
                *skill_mod += or_value(unification, effect.value);
                return;
            }
            run.failed_continue = true;
        }
        unification = 0.0;
    }

    if let Some(effect) = &effects.score_over_life {
        if (*judgement as f64) <= effect.condition.threshold() && life >= effect.condition_life {
            *skill_mod += or_value(unification, effect.value);
            return;
        }
        unification = 0.0;
    }

    if let Some(effect) = &effects.score_under_life {
        if (*judgement as f64) <= effect.condition.threshold() && life < effect.condition_life {
            *skill_mod += or_value(unification, effect.value);
            return;
        }
        unification = 0.0;
    }

    if let Some(effect) = &effects.score_rate_up_with_perfect {
        if (*judgement as f64) <= effect.condition.threshold() {
            run.perfect_count += 1;
        }
    }

    if let Some(effect) = &effects.score {
        if (*judgement as f64) <= effect.condition.threshold() {
            *skill_mod += or_value(unification, effect.value);
            *skill_mod += 0.005 * run.perfect_count.min(100) as f64;
        }
    }

    if let Some(effect) = &effects.score_under_great_half {
        if (*judgement as f64) <= effect.condition.threshold() {
            *skill_mod += or_value(unification, effect.value);
        } else {
            *skill_mod = 0.5;
        }
    }
}

/// Runs one full simulation. `offsets` holds one frame offset per event
/// (0.0 = natural timing).
pub(crate) fn simulate(sim: &Simulation<'_>, offsets: &[f64]) -> SimOutcome {
    let events = sim.events;
    let n = events.len();
    debug_assert_eq!(offsets.len(), n);

    let note_coefficient = (3.0 + 0.03 * (sim.play_level - 5) as f64) / n as f64;

    let mut times = vec![0.0f64; n];
    for (idx, event) in events.iter().enumerate() {
        times[idx] = event.time + offsets[idx] / 60.0;
    }
    let mut hit_order: Vec<u32> = (0..n as u32).collect();
    hit_order.sort_by(|&a, &b| {
        times[a as usize]
            .partial_cmp(&times[b as usize])
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.cmp(&b))
    });

    let mut runs: Vec<SkillRun> = sim.ordered_skills.iter().map(|_| SkillRun::new()).collect();
    let mut life = 1000.0f64;
    let mut combo = 0i32;
    let mut next_slot = 0usize;
    let mut score_total = 0i64;
    let mut judgements = vec![MISS; n];

    for &hit in &hit_order {
        let idx = hit as usize;
        let event = &events[idx];
        let hit_time = times[idx];

        let hit_combo_mod = combo_mod(combo);
        let mut judgement = judge_in(event.kind, offsets[idx], sim.judgement_windows);
        let mut skill_mod = 1.0f64;
        let mut penalty;

        let active = runs.iter().position(|run| match (run.start, run.end) {
            (Some(start), Some(end)) => {
                hit_time > start && hit_time <= end + sim.end_grace
            }
            _ => false,
        });

        match active {
            Some(slot) => {
                let def = &sim.ordered_skills[slot];
                if !runs[slot].activated_once {
                    runs[slot].activated_once = true;
                    if let Some(once) = &def.effects.once_life {
                        let allowed = match once.condition {
                            OnceLifeCondition::Always => true,
                            OnceLifeCondition::UnderLife(condition_life) => life < condition_life,
                        };
                        if allowed {
                            life = (life + once.value).min(2000.0);
                        }
                    }
                }
                penalty = life_penalty(event.kind, judgement);
                apply_effects(
                    def,
                    event.kind,
                    &mut judgement,
                    &mut penalty,
                    &mut skill_mod,
                    life,
                    &mut runs[slot],
                    sim.team_band_id,
                    sim.team_attribute,
                );
                runs[slot].covered_notes += 1;
            }
            None => {
                penalty = life_penalty(event.kind, judgement);
            }
        }

        life = (life - penalty).max(0.0);

        if event.is_skill {
            if judgement <= GREAT && next_slot < runs.len() {
                let start = match active {
                    Some(slot_index) => runs[slot_index].end.unwrap_or(hit_time) + SKILL_QUEUE_GAP,
                    None => hit_time,
                };
                let duration = sim.ordered_skills[next_slot].duration;
                let slot = &mut runs[next_slot];
                slot.start = Some(start);
                slot.end = Some(start + duration);
                slot.trigger_index = idx;
                slot.trigger_time = hit_time;
            }
            next_slot += 1;
        }

        if judgement <= GREAT {
            combo += 1;
        } else {
            combo = 0;
        }

        let accuracy = JUDGEMENT_ACCURACY[judgement as usize];
        let base = ((sim.band_power as f64 * note_coefficient) * hit_combo_mod) * accuracy;
        let note_score = (base.floor() * skill_mod).floor() as i64;
        score_total += note_score;
        judgements[idx] = judgement;
    }

    let windows = runs
        .into_iter()
        .map(|run| {
            run.start.map(|start| WindowInfo {
                start,
                end: run.end.unwrap_or(start),
                trigger_index: run.trigger_index,
                trigger_time: run.trigger_time,
                covered_notes: run.covered_notes,
            })
        })
        .collect();

    SimOutcome {
        score: score_total,
        judgements,
        windows,
    }
}
