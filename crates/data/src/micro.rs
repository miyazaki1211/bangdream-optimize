//! Micro-optimisation data layer: Bestdori JSON parsing and input resolution.
//!
//! The standard chart parser collapses notes into `Node | Skill`, which is
//! enough for team search but destroys the note shape micro-optimisation
//! needs. This module re-reads the raw Bestdori chart JSON retaining all ten
//! hit kinds, parses the full skill effect family, and assembles the
//! calculation input for [`bangdream_optimize_core::micro_opt`].

use bangdream_optimize_core::floor_team_stat;
use bangdream_optimize_core::micro_opt::{
    judgement_name, micro_optimize, MicroHitEvent, MicroHitKind, MicroOptimizeInput,
    MicroOptimizeResult, MicroSearchConfig, MicroSkill,
};
use bangdream_optimize_core::model::preparation::{AreaItemPercent, PreparedCard};
use bangdream_optimize_core::model::schema::{Attribute, SongSelection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use crate::utils::{get_array, get_f64};
use crate::DataError;

/// Semantic version of the ported scoring rules. Bump when the constants in
/// `core::micro_opt` deliberately diverge from Bestdori's frontend.
pub const MICRO_SEMANTICS_VERSION: &str = "bestdori-compatible-v1";

/// Expands a raw Bestdori chart into judgeable hit events.
///
/// BPM handling, note expansion and the final ordering (ascending time,
/// skill-flagged hits first on ties) replicate Bestdori's optimizer. Note
/// kinds decide which judgement window applies to each micro-offset.
pub fn micro_events_from_bestdori(chart_data: &Value) -> Result<Vec<MicroHitEvent>, DataError> {
    let (events, _pairs) = micro_events_with_pairs_from_bestdori(chart_data)?;
    Ok(events)
}

/// [`micro_events_from_bestdori`] that additionally returns ordered
/// physical-dependency pairs in sorted-event index space: the
/// `(start, end)` of every Long/Slide connection, plus same-lane
/// consecutive notes close enough for offsets to cross. The search uses
/// these to enforce that the second hit never lands before (or within one
/// frame of) the first.
pub fn micro_events_with_pairs_from_bestdori(
    chart_data: &Value,
) -> Result<(Vec<MicroHitEvent>, Vec<(u32, u32)>), DataError> {
    let notes = chart_data.as_array().ok_or(DataError::InvalidField {
        field: "chart",
        value: "expected array".to_owned(),
    })?;

    let mut bpm_points = notes
        .iter()
        .filter(|note| note.get("type").and_then(Value::as_str) == Some("BPM"))
        .map(|note| {
            Ok(crate::chart::BpmPoint {
                beat: get_f64(note, "beat")?,
                bpm: get_f64(note, "bpm")?,
                time: 0.0,
            })
        })
        .collect::<Result<Vec<_>, DataError>>()?;
    bpm_points.sort_by(|a, b| a.beat.total_cmp(&b.beat));
    crate::chart::compute_bpm_times(&mut bpm_points)?;

    let mut events = Vec::new();
    // (start, end) emission indices of every Long/Slide connection.
    let mut pair_ids: Vec<(u32, u32)> = Vec::new();
    // Lane per emitted event, parallel to `events`.
    let mut lanes: Vec<i64> = Vec::new();
    for note in notes {
        let note_type = note.get("type").and_then(Value::as_str);
        let skill_flag =
            |value: &Value| value.get("skill").and_then(Value::as_bool).unwrap_or(false);
        match note_type {
            Some("Single") => {
                lanes.push(
                    note.get("lane")
                        .and_then(Value::as_f64)
                        .map(|lane| lane as i64)
                        .unwrap_or(-(events.len() as i64) - 1),
                );
                events.push(MicroHitEvent {
                    kind: if note.get("flick").and_then(Value::as_bool).unwrap_or(false) {
                        MicroHitKind::Flick
                    } else {
                        MicroHitKind::Tap
                    },
                    time: crate::chart::time_for_beat(&bpm_points, get_f64(note, "beat")?)?,
                    is_skill: skill_flag(note),
                });
            }
            Some("Directional") => {
                lanes.push(
                    note.get("lane")
                        .and_then(Value::as_f64)
                        .map(|lane| lane as i64)
                        .unwrap_or(-(events.len() as i64) - 1),
                );
                events.push(MicroHitEvent {
                    kind: MicroHitKind::DirectionalFlick,
                    time: crate::chart::time_for_beat(&bpm_points, get_f64(note, "beat")?)?,
                    is_skill: false,
                });
            }
            Some("Long") => {
                let connections = get_array(note, "connections")?;
                if connections.len() < 2 {
                    return Err(DataError::InvalidField {
                        field: "chart.long.connections",
                        value: "expected at least two connections".to_owned(),
                    });
                }
                let start_id = events.len() as u32;
                lanes.push(
                    connections[0]
                        .get("lane")
                        .and_then(Value::as_f64)
                        .map(|lane| lane as i64)
                        .unwrap_or(-(events.len() as i64) - 1),
                );
                events.push(MicroHitEvent {
                    kind: MicroHitKind::LongStart,
                    time: crate::chart::time_for_beat(
                        &bpm_points,
                        get_f64(&connections[0], "beat")?,
                    )?,
                    is_skill: skill_flag(&connections[0]),
                });
                lanes.push(
                    connections[1]
                        .get("lane")
                        .and_then(Value::as_f64)
                        .map(|lane| lane as i64)
                        .unwrap_or(-(events.len() as i64) - 1),
                );
                events.push(MicroHitEvent {
                    kind: if connections[1]
                        .get("flick")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        MicroHitKind::LongFlick
                    } else {
                        MicroHitKind::LongEnd
                    },
                    time: crate::chart::time_for_beat(
                        &bpm_points,
                        get_f64(&connections[1], "beat")?,
                    )?,
                    is_skill: skill_flag(&connections[1]),
                });
                pair_ids.push((start_id, start_id + 1));
            }
            Some("Slide") => {
                let connections = get_array(note, "connections")?;
                let last_index = connections.len().saturating_sub(1);
                let shifts_lanes = connections.windows(2).any(|pair| {
                    get_f64(&pair[0], "lane").ok() != get_f64(&pair[1], "lane").ok()
                });
                let (kind_start, kind_end) = if shifts_lanes {
                    (MicroHitKind::SlideShiftStart, MicroHitKind::SlideShiftEnd)
                } else {
                    (
                        MicroHitKind::SlideStraightStart,
                        MicroHitKind::SlideStraightEnd,
                    )
                };
                let mut slide_start_id: Option<u32> = None;
                let mut slide_end_id: Option<u32> = None;
                for (index, connection) in connections.iter().enumerate() {
                    if connection
                        .get("hidden")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        continue;
                    }
                    let emission_id = events.len() as u32;
                    lanes.push(
                        connection
                            .get("lane")
                            .and_then(Value::as_f64)
                            .map(|lane| lane as i64)
                            .unwrap_or(-(events.len() as i64) - 1),
                    );
                    let kind = if index == 0 {
                        kind_start
                    } else if index == last_index {
                        if connection
                            .get("flick")
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                        {
                            MicroHitKind::SlideFlick
                        } else {
                            kind_end
                        }
                    } else {
                        MicroHitKind::SlideTick
                    };
                    events.push(MicroHitEvent {
                        kind,
                        time: crate::chart::time_for_beat(
                            &bpm_points,
                            get_f64(connection, "beat")?,
                        )?,
                        is_skill: matches!(
                            kind,
                            MicroHitKind::SlideStraightStart
                                | MicroHitKind::SlideShiftStart
                                | MicroHitKind::SlideStraightEnd
                                | MicroHitKind::SlideShiftEnd
                                | MicroHitKind::SlideFlick
                        ) && skill_flag(connection),
                    });
                    if index == 0 {
                        slide_start_id = Some(emission_id);
                    } else if index == last_index {
                        slide_end_id = Some(emission_id);
                    }
                }
                if let (Some(start), Some(end)) = (slide_start_id, slide_end_id) {
                    pair_ids.push((start, end));
                }
            }
            _ => {}
        }
    }

    let mut indexed: Vec<(MicroHitEvent, u32, i64)> = events
        .into_iter()
        .zip(lanes)
        .enumerate()
        .map(|(id, (e, lane))| (e, id as u32, lane))
        .collect();
    indexed.sort_by(|a, b| {
        a.0.time
            .partial_cmp(&b.0.time)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.is_skill.cmp(&b.0.is_skill))
    });
    let mut position_of = vec![0u32; indexed.len()];
    for (position, (_, id, _)) in indexed.iter().enumerate() {
        position_of[*id as usize] = position as u32;
    }
    let events: Vec<MicroHitEvent> = indexed.iter().map(|(e, _, _)| *e).collect();
    let mut pairs = pair_ids
        .into_iter()
        .map(|(start, end)| (position_of[start as usize], position_of[end as usize]))
        .collect::<Vec<(u32, u32)>>();

    const LANE_PAIR_MAX_GAP_SECONDS: f64 = 24.0 / 60.0;
    let mut by_lane: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
    for (index, (_, _, lane)) in indexed.iter().enumerate() {
        by_lane.entry(*lane).or_default().push(index);
    }
    for (_, indices) in by_lane {
        for window in indices.windows(2) {
            let (earlier, later) = (window[0], window[1]);
            if events[later].time - events[earlier].time <= LANE_PAIR_MAX_GAP_SECONDS {
                pairs.push((earlier as u32, later as u32));
            }
        }
    }
    Ok((events, pairs))
}

fn parse_condition(value: &Value, field: &'static str) -> Result<EffectCondition, DataError> {
    let condition = value.as_str().ok_or(DataError::InvalidField {
        field,
        value: value.to_string(),
    })?;
    bangdream_optimize_core::micro_opt::skill::EffectCondition::parse(condition).ok_or(
        DataError::InvalidField {
            field,
            value: condition.to_owned(),
        },
    )
}

use bangdream_optimize_core::micro_opt::skill::EffectCondition;

/// Nearest-lower-level numeric fallback for Bestdori's `null` level slots.
fn value_at_level(values: &[Value], level: u8) -> f64 {
    let mut index = level.saturating_sub(1) as usize;
    loop {
        if let Some(value) = values.get(index).and_then(Value::as_f64) {
            return value;
        }
        if index == 0 {
            return 0.0;
        }
        index -= 1;
    }
}

fn effect_value_at_level(effect: &Value, field: &'static str, level: u8) -> Result<f64, DataError> {
    let values = effect
        .get("activateEffectValue")
        .and_then(Value::as_array)
        .ok_or(DataError::MissingField { field })?;
    Ok(value_at_level(values, level) / 100.0)
}

fn condition_life(effect: &Value, field: &'static str) -> Result<f64, DataError> {
    effect
        .get("activateConditionLife")
        .and_then(Value::as_f64)
        .ok_or(DataError::MissingField { field })
}

/// Parses the full Bestdori skill effect family at one equipped level
/// (1-5). Unknown effect keys are ignored, matching the frontend which only
/// reads the keys it knows.
pub fn micro_skill_from_bestdori(
    skill_id: u32,
    skill: &Value,
    skill_level: u8,
) -> Result<MicroSkill, DataError> {
    let durations = get_array(skill, "duration")?;
    let raw_duration = durations
        .get(skill_level.saturating_sub(1) as usize)
        .and_then(Value::as_f64)
        .ok_or(DataError::InvalidField {
            field: "skill.duration",
            value: format!("skill {skill_id} level {skill_level} out of range"),
        })?;

    let mut effects = bangdream_optimize_core::micro_opt::skill::MicroSkillEffects::default();
    if let Some(activation) = skill.get("activationEffect") {
        if let Some(types) = activation
            .get("activateEffectTypes")
            .and_then(Value::as_object)
        {
            for (key, effect) in types {
                match key.as_str() {
                    "score" | "score_continued_note_judge" | "score_under_great_half" => {
                        let parsed = bangdream_optimize_core::micro_opt::skill::NumericEffect {
                            condition: parse_condition(
                                effect
                                    .get("activateCondition")
                                    .unwrap_or(&Value::String("none".to_owned())),
                                "activateCondition",
                            )?,
                            value: effect_value_at_level(
                                effect,
                                "activateEffectValue",
                                skill_level,
                            )?,
                        };
                        match key.as_str() {
                            "score" => effects.score = Some(parsed),
                            "score_continued_note_judge" => {
                                effects.score_continued_note_judge = Some(parsed)
                            }
                            _ => effects.score_under_great_half = Some(parsed),
                        }
                    }
                    "judge" | "damage" | "score_rate_up_with_perfect" => {
                        let parsed = bangdream_optimize_core::micro_opt::skill::ConditionEffect {
                            condition: parse_condition(
                                effect
                                    .get("activateCondition")
                                    .unwrap_or(&Value::String("none".to_owned())),
                                "activateCondition",
                            )?,
                        };
                        match key.as_str() {
                            "judge" => effects.judge = Some(parsed),
                            "damage" => effects.damage = Some(parsed),
                            _ => effects.score_rate_up_with_perfect = Some(parsed),
                        }
                    }
                    "score_over_life" | "score_under_life" => {
                        let parsed =
                            bangdream_optimize_core::micro_opt::skill::LifeConditionEffect {
                                condition: parse_condition(
                                    effect
                                        .get("activateCondition")
                                        .unwrap_or(&Value::String("none".to_owned())),
                                    "activateCondition",
                                )?,
                                condition_life: condition_life(effect, "activateConditionLife")?,
                                value: effect_value_at_level(
                                    effect,
                                    "activateEffectValue",
                                    skill_level,
                                )?,
                            };
                        if key == "score_over_life" {
                            effects.score_over_life = Some(parsed);
                        } else {
                            effects.score_under_life = Some(parsed);
                        }
                    }
                    "never_die" => effects.never_die = true,
                    _ => {}
                }
            }
        }

        effects.unification_band_id = activation
            .get("unificationActivateConditionBandId")
            .and_then(Value::as_u64)
            .map(|value| value as u32);
        if let Some(attribute) = activation
            .get("unificationActivateConditionType")
            .and_then(Value::as_str)
        {
            effects.unification_attribute = Some(crate::utils::parse_attribute(attribute)?);
        }
        if let Some(value) = activation
            .get("unificationActivateEffectValue")
            .and_then(Value::as_f64)
        {
            effects.unification_value = value / 100.0;
        }

        if let Some(once) = activation.get("onceEffect") {
            if once.get("onceEffectType").and_then(Value::as_str) == Some("life") {
                let values = once
                    .get("onceEffectValue")
                    .and_then(Value::as_array)
                    .ok_or(DataError::MissingField {
                        field: "onceEffect.onceEffectValue",
                    })?;
                // Same nearest-lower fallback as activateEffectValue.
                let value = value_at_level(values, skill_level);
                let condition = match once
                    .get("onceEffectConditionLifeType")
                    .and_then(Value::as_str)
                {
                    None => bangdream_optimize_core::micro_opt::skill::OnceLifeCondition::Always,
                    Some("under_life") => {
                        let life = once
                            .get("onceEffectConditionLife")
                            .and_then(Value::as_f64)
                            .ok_or(DataError::MissingField {
                                field: "onceEffect.onceEffectConditionLife",
                            })?;
                        bangdream_optimize_core::micro_opt::skill::OnceLifeCondition::UnderLife(
                            life,
                        )
                    }
                    Some(other) => {
                        return Err(DataError::InvalidField {
                            field: "onceEffect.onceEffectConditionLifeType",
                            value: other.to_owned(),
                        })
                    }
                };
                effects.once_life =
                    Some(bangdream_optimize_core::micro_opt::skill::OnceLifeEffect {
                        value,
                        condition,
                    });
            }
        }
    }

    Ok(MicroSkill {
        skill_id,
        skill_level,
        duration: raw_duration,
        effects,
    })
}

/// Band power under the same selection model as `/v1/calc-result`
/// (`maximize`): pick the best `SelectedAreaItems` combination (one band
/// key + one attribute key + one magazine) for this fixed team and floor the
/// sum once, exactly like the maximize search does for its winning team.
pub fn calc_parity_band_power(cards: &[PreparedCard], percent: &AreaItemPercent) -> i64 {
    bangdream_optimize_core::model::preparation::area_item_combinations(percent)
        .iter()
        .map(|combo| {
            floor_team_stat(cards.iter().map(|card| {
                card.add_up_stat(
                    percent,
                    &combo.band,
                    &combo.attribute,
                    combo.magazine.as_str(),
                )
            })) as i64
        })
        .max()
        .unwrap_or_else(|| {
            floor_team_stat(
                cards
                    .iter()
                    .map(|card| card.stat.sum() + card.event_add_stat.sum()),
            ) as i64
        })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MicroSongInfo {
    pub song_id: u32,
    pub difficulty: u8,
    pub play_level: i32,
    pub note_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MicroTeamCard {
    pub card_id: u32,
    pub character_id: u32,
    pub band_id: u32,
    pub rarity: u8,
    pub attribute: Attribute,
    pub level: u8,
    pub skill_level: u8,
    pub limit_break_rank: u8,
    pub episodes: [bool; 2],
    pub training: bool,
    pub skill_id: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MicroUnification {
    pub band_id: u32,
    pub attribute: Attribute,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MicroTeamInfo {
    pub cards: Vec<MicroTeamCard>,
    pub band_power: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unification: Option<MicroUnification>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MicroNoteOffset {
    pub index: u32,
    pub kind: MicroHitKind,
    pub time: f64,
    pub offset: f64,
    pub judgement: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MicroSkillWindow {
    pub slot: usize,
    pub card_index: usize,
    pub card_id: u32,
    pub trigger_index: u32,
    pub trigger_time: f64,
    pub start: f64,
    pub end: f64,
    pub queued: bool,
    pub covered_notes: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MicroOutcomeMeta {
    pub simulations: u64,
    pub total_offset: f64,
    pub semantics: String,
    /// Whether the run used custom judgement windows.
    pub custom_judgement_windows: bool,
}

/// Serializable response of a micro-optimisation run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MicroOptimizeOutcome {
    pub event_id: u32,
    /// Raw Bestdori event type label (`story`, `challenge`, ...).
    pub event_type: String,
    pub song: MicroSongInfo,
    pub team: MicroTeamInfo,
    pub baseline_score: i64,
    pub max_score: i64,
    pub score_gain: i64,
    /// Six firing slots as team card IDs; the looping sixth slot repeats the
    /// card at team position 0.
    pub skill_order: [u32; 6],
    pub optimizations: Vec<MicroNoteOffset>,
    pub skill_windows: Vec<MicroSkillWindow>,
    pub meta: MicroOutcomeMeta,
}

/// Assembles the outcome from a finished search plus resolved context.
#[allow(clippy::too_many_arguments)]
pub fn build_micro_outcome(
    event_id: u32,
    event_type: String,
    song: SongSelection,
    events: &[MicroHitEvent],
    play_level: i32,
    prepared_team: &[PreparedCard],
    skill_ids: [u32; 5],
    band_power: i64,
    result: &MicroOptimizeResult,
    custom_judgement_windows: bool,
) -> MicroOptimizeOutcome {
    let optimizations = result
        .optimization
        .iter()
        .map(|(index, offset)| {
            let event = &events[*index as usize];
            MicroNoteOffset {
                index: *index,
                kind: event.kind,
                time: event.time,
                offset: *offset,
                judgement: judgement_name(result.judgements[*index as usize]).to_owned(),
            }
        })
        .collect();
    let skill_windows = result
        .skill_windows
        .iter()
        .map(|window| MicroSkillWindow {
            slot: window.slot,
            card_index: window.card_index,
            card_id: prepared_team[window.card_index].card_id,
            trigger_index: window.trigger_index,
            trigger_time: window.trigger_time,
            start: window.start,
            end: window.end,
            queued: window.queued,
            covered_notes: window.covered_notes,
        })
        .collect();

    MicroOptimizeOutcome {
        event_id,
        event_type,
        song: MicroSongInfo {
            song_id: song.song_id,
            difficulty: song.difficulty,
            play_level,
            note_count: events.len(),
        },
        team: MicroTeamInfo {
            cards: prepared_team
                .iter()
                .zip(skill_ids)
                .map(|(card, skill_id)| MicroTeamCard {
                    card_id: card.card_id,
                    character_id: card.character_id,
                    band_id: card.band_id,
                    rarity: card.rarity,
                    attribute: card.attribute,
                    level: card.level,
                    skill_level: card.skill_level,
                    limit_break_rank: card.limit_break_rank,
                    episodes: card.episodes,
                    training: card.training,
                    skill_id,
                })
                .collect(),
            band_power,
            unification: result_unification(prepared_team),
        },
        baseline_score: result.baseline_score,
        max_score: result.max_score,
        score_gain: result.score_gain,
        skill_order: result
            .skill_order
            .map(|card_index| prepared_team[card_index].card_id),
        optimizations,
        skill_windows,
        meta: MicroOutcomeMeta {
            simulations: result.simulations,
            total_offset: result.total_offset,
            semantics: MICRO_SEMANTICS_VERSION.to_owned(),
            custom_judgement_windows,
        },
    }
}

fn result_unification(prepared_team: &[PreparedCard]) -> Option<MicroUnification> {
    let first = prepared_team.first()?;
    let same_band = prepared_team
        .iter()
        .all(|card| card.band_id == first.band_id);
    let same_attribute = prepared_team
        .iter()
        .all(|card| card.attribute == first.attribute);
    (same_band && same_attribute).then_some(MicroUnification {
        band_id: first.band_id,
        attribute: first.attribute,
    })
}

/// Runs the kernel on fully resolved inputs. Exposed for callers that
/// already hold parsed pieces (and for tests).
pub fn run_micro_optimize(input: &MicroOptimizeInput) -> Result<MicroOptimizeResult, DataError> {
    micro_optimize(input, &MicroSearchConfig::default()).map_err(|error| DataError::InvalidField {
        field: "microOptimize",
        value: error.to_string(),
    })
}
