//! Micro-optimisation search: the port of Bestdori's `S()` auto-optimizer.
//!
//! Strategy (kept identical to the frontend, including iteration order, so
//! results are deterministic and bit-compatible):
//!
//! 1. Enumerate the fixed 120-order list of five-card skill permutations
//!    (or a single caller-supplied order), appending the looping slot 0.
//! 2. Deduplicate orders by their `(skill id, skill level)` sequence.
//! 3. Per unique order, greedily sweep the skill-flagged events in chart
//!    order. For every candidate frame offset of the skill note, evaluate a
//!    full simulation plus "boundary shift" children that pull the nearest
//!    non-skill note just inside the candidate skill window.
//! 4. Candidates win by strictly higher score, then strictly smaller total
//!    absolute offset.

use std::collections::{BTreeMap, HashMap};

use super::events::{MicroHitEvent, MicroHitKind};
use super::judge::JudgementWindowSet;
use super::simulate::{simulate, Simulation};
use super::skill::MicroSkill;

/// The exact permutation order Bestdori's frontend enumerates. The order is
/// part of the contract: ties are resolved in favour of the first candidate,
/// so reordering this table can change the reported optimisation map.
const SKILL_ORDERS: [[usize; 5]; 120] = [
    [0, 1, 2, 3, 4],
    [1, 0, 2, 3, 4],
    [2, 0, 1, 3, 4],
    [0, 2, 1, 3, 4],
    [1, 2, 0, 3, 4],
    [2, 1, 0, 3, 4],
    [2, 1, 3, 0, 4],
    [1, 2, 3, 0, 4],
    [3, 2, 1, 0, 4],
    [2, 3, 1, 0, 4],
    [1, 3, 2, 0, 4],
    [3, 1, 2, 0, 4],
    [3, 0, 2, 1, 4],
    [0, 3, 2, 1, 4],
    [2, 3, 0, 1, 4],
    [3, 2, 0, 1, 4],
    [0, 2, 3, 1, 4],
    [2, 0, 3, 1, 4],
    [1, 0, 3, 2, 4],
    [0, 1, 3, 2, 4],
    [3, 1, 0, 2, 4],
    [1, 3, 0, 2, 4],
    [0, 3, 1, 2, 4],
    [3, 0, 1, 2, 4],
    [4, 0, 1, 2, 3],
    [0, 4, 1, 2, 3],
    [1, 4, 0, 2, 3],
    [4, 1, 0, 2, 3],
    [0, 1, 4, 2, 3],
    [1, 0, 4, 2, 3],
    [1, 0, 2, 4, 3],
    [0, 1, 2, 4, 3],
    [2, 1, 0, 4, 3],
    [1, 2, 0, 4, 3],
    [0, 2, 1, 4, 3],
    [2, 0, 1, 4, 3],
    [2, 4, 1, 0, 3],
    [4, 2, 1, 0, 3],
    [1, 2, 4, 0, 3],
    [2, 1, 4, 0, 3],
    [4, 1, 2, 0, 3],
    [1, 4, 2, 0, 3],
    [0, 4, 2, 1, 3],
    [4, 0, 2, 1, 3],
    [2, 0, 4, 1, 3],
    [0, 2, 4, 1, 3],
    [4, 2, 0, 1, 3],
    [2, 4, 0, 1, 3],
    [3, 4, 0, 1, 2],
    [4, 3, 0, 1, 2],
    [0, 3, 4, 1, 2],
    [3, 0, 4, 1, 2],
    [4, 0, 3, 1, 2],
    [0, 4, 3, 1, 2],
    [0, 4, 1, 3, 2],
    [4, 0, 1, 3, 2],
    [1, 0, 4, 3, 2],
    [0, 1, 4, 3, 2],
    [4, 1, 0, 3, 2],
    [1, 4, 0, 3, 2],
    [1, 3, 0, 4, 2],
    [3, 1, 0, 4, 2],
    [0, 1, 3, 4, 2],
    [1, 0, 3, 4, 2],
    [3, 0, 1, 4, 2],
    [0, 3, 1, 4, 2],
    [4, 3, 1, 0, 2],
    [3, 4, 1, 0, 2],
    [1, 4, 3, 0, 2],
    [4, 1, 3, 0, 2],
    [3, 1, 4, 0, 2],
    [1, 3, 4, 0, 2],
    [2, 3, 4, 0, 1],
    [3, 2, 4, 0, 1],
    [4, 2, 3, 0, 1],
    [2, 4, 3, 0, 1],
    [3, 4, 2, 0, 1],
    [4, 3, 2, 0, 1],
    [4, 3, 0, 2, 1],
    [3, 4, 0, 2, 1],
    [0, 4, 3, 2, 1],
    [4, 0, 3, 2, 1],
    [3, 0, 4, 2, 1],
    [0, 3, 4, 2, 1],
    [0, 2, 4, 3, 1],
    [2, 0, 4, 3, 1],
    [4, 0, 2, 3, 1],
    [0, 4, 2, 3, 1],
    [2, 4, 0, 3, 1],
    [4, 2, 0, 3, 1],
    [3, 2, 0, 4, 1],
    [2, 3, 0, 4, 1],
    [0, 3, 2, 4, 1],
    [3, 0, 2, 4, 1],
    [2, 0, 3, 4, 1],
    [0, 2, 3, 4, 1],
    [1, 2, 3, 4, 0],
    [2, 1, 3, 4, 0],
    [3, 1, 2, 4, 0],
    [1, 3, 2, 4, 0],
    [2, 3, 1, 4, 0],
    [3, 2, 1, 4, 0],
    [3, 2, 4, 1, 0],
    [2, 3, 4, 1, 0],
    [4, 3, 2, 1, 0],
    [3, 4, 2, 1, 0],
    [2, 4, 3, 1, 0],
    [4, 2, 3, 1, 0],
    [4, 1, 3, 2, 0],
    [1, 4, 3, 2, 0],
    [3, 4, 1, 2, 0],
    [4, 3, 1, 2, 0],
    [1, 3, 4, 2, 0],
    [3, 1, 4, 2, 0],
    [2, 1, 4, 3, 0],
    [1, 2, 4, 3, 0],
    [4, 2, 1, 3, 0],
    [2, 4, 1, 3, 0],
    [1, 4, 2, 3, 0],
    [4, 1, 2, 3, 0],
];

#[derive(Debug, Clone, Copy, PartialEq)]
struct Candidate {
    offset: f64,
    time: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Shift {
    index: u32,
    offset: f64,
}

pub(crate) struct SearchOutcome {
    pub order: [usize; 6],
    pub optimization: BTreeMap<u32, f64>,
    /// Best score across all evaluated skill orders with every offset at
    /// zero: the strongest natural-timing result before micro-optimisation.
    pub baseline_score: i64,
    pub score: i64,
    pub total: f64,
    pub simulations: u64,
}

pub(crate) struct SearchContext<'a> {
    pub events: &'a [MicroHitEvent],
    pub play_level: i32,
    pub band_power: i64,
    pub skills: &'a [MicroSkill],
    pub team_band_id: Option<u32>,
    pub team_attribute: Option<crate::model::schema::Attribute>,
    pub search_bounds: &'a dyn Fn(MicroHitKind) -> (f64, f64),
    pub step: f64,
    pub restrictions: &'a BTreeMap<u32, (f64, f64)>,
    pub fixed_order: Option<[usize; 5]>,
    /// Judgement windows passed through to every simulation.
    pub judgement_windows: &'a JudgementWindowSet,
    /// Extra seconds appended to window ends during coverage (see
    /// `MicroOptimizeInput::skill_window_end_grace`). Boundary-shift
    /// children target the effective end.
    pub end_grace: f64,
    /// `(start, end)` event-index pairs of Long/Slide connections; the
    /// search rejects offset maps that invert or over-compress them.
    pub pairs: &'a [(u32, u32)],
}

fn better(score: i64, total: f64, incumbent_score: i64, incumbent_total: f64) -> bool {
    score > incumbent_score || (score == incumbent_score && total < incumbent_total)
}

struct Incumbent {
    score: i64,
    total: f64,
    optimization: BTreeMap<u32, f64>,
}

fn time_key(time: f64) -> u64 {
    debug_assert!(time.is_finite());
    time.to_bits()
}

pub(crate) fn search(context: &SearchContext<'_>) -> SearchOutcome {
    let events = context.events;
    let n = events.len();

    const MIN_PAIR_GAP_FRAMES: f64 = 1.0;
    let hit_time = |map: &BTreeMap<u32, f64>, index: u32| -> f64 {
        events[index as usize].time + map.get(&index).copied().unwrap_or(0.0) / 60.0
    };
    let pair_ok = |map: &BTreeMap<u32, f64>, touched: u32| -> bool {
        context
            .pairs
            .iter()
            .filter(|&&(start, end)| start == touched || end == touched)
            .all(|&(start, end)| {
                hit_time(map, end) >= hit_time(map, start) + MIN_PAIR_GAP_FRAMES / 60.0
            })
    };

    // Candidate offsets per event, enumerated once and shared by every order.
    let candidates: Vec<Vec<Candidate>> = events
        .iter()
        .enumerate()
        .map(|(index, event)| {
            let bounds = context
                .restrictions
                .get(&(index as u32))
                .copied()
                .unwrap_or_else(|| (context.search_bounds)(event.kind));
            // Bestdori truncates both bounds toward zero onto the step grid.
            let low = (bounds.0 / context.step).trunc() * context.step;
            let high = (bounds.1 / context.step).trunc() * context.step;
            let mut list = Vec::new();
            let mut offset = low;
            while offset <= high {
                if event.is_skill || offset != 0.0 {
                    list.push(Candidate {
                        offset,
                        time: event.time + offset / 60.0,
                    });
                }
                offset += context.step;
            }
            list
        })
        .collect();

    let skill_indices: Vec<u32> = events
        .iter()
        .enumerate()
        .filter(|(_, event)| event.is_skill)
        .map(|(index, _)| index as u32)
        .collect();

    let mut simulations: u64 = 0;
    let mut evaluate = |ordered: &[MicroSkill], optimization: &BTreeMap<u32, f64>| -> i64 {
        let mut offsets = vec![0.0f64; n];
        for (index, offset) in optimization {
            offsets[*index as usize] = *offset;
        }
        let sim = Simulation {
            events,
            play_level: context.play_level,
            band_power: context.band_power,
            ordered_skills: ordered,
            team_band_id: context.team_band_id,
            team_attribute: context.team_attribute,
            judgement_windows: context.judgement_windows,
            end_grace: context.end_grace,
        };
        simulations += 1;
        simulate(&sim, &offsets).score
    };

    let mut start_cache: HashMap<u64, Vec<Shift>> = HashMap::new();
    let mut end_cache: HashMap<u64, Vec<Shift>> = HashMap::new();

    let build_start_list = |c_time: f64| -> Vec<Shift> {
        let mut list = Vec::new();
        for (index, event) in events.iter().enumerate() {
            if event.time > c_time {
                break;
            }
            if !event.is_skill {
                for candidate in &candidates[index] {
                    if candidate.time > c_time {
                        list.push(Shift {
                            index: index as u32,
                            offset: candidate.offset,
                        });
                        break;
                    }
                }
            }
        }
        list
    };
    let build_end_list = |f_end: f64| -> Vec<Shift> {
        let mut list = Vec::new();
        for (index, event) in events.iter().enumerate().rev() {
            if event.time < f_end {
                break;
            }
            if !event.is_skill {
                for candidate in candidates[index].iter().rev() {
                    if candidate.time <= f_end {
                        list.push(Shift {
                            index: index as u32,
                            offset: candidate.offset,
                        });
                        break;
                    }
                }
            }
        }
        list
    };

    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut best = SearchOutcome {
        order: [0, 1, 2, 3, 4, 0],
        optimization: BTreeMap::new(),
        baseline_score: i64::MIN,
        score: i64::MIN,
        total: 0.0,
        simulations: 0,
    };

    let orders: Vec<[usize; 5]> = match context.fixed_order {
        Some(order) => vec![order],
        None => SKILL_ORDERS.to_vec(),
    };
    let mut full_orders: Vec<[usize; 6]> = Vec::with_capacity(orders.len() * 5);
    for permutation in &orders {
        for sixth in 0..5usize {
            full_orders.push([
                permutation[0],
                permutation[1],
                permutation[2],
                permutation[3],
                permutation[4],
                sixth,
            ]);
        }
    }

    for order in full_orders {
        let key = order
            .map(|card_index| {
                let skill = &context.skills[card_index];
                format!("{}@{}", skill.skill_id, skill.skill_level)
            })
            .join(",");
        if !seen.insert(key) {
            continue;
        }

        let ordered: Vec<MicroSkill> = order
            .map(|card_index| context.skills[card_index].clone())
            .to_vec();

        let natural_score = evaluate(&ordered, &BTreeMap::new());
        if natural_score > best.baseline_score {
            best.baseline_score = natural_score;
        }
        let mut incumbent = Incumbent {
            score: natural_score,
            total: 0.0,
            optimization: BTreeMap::new(),
        };

        for (ordinal, &skill_index) in skill_indices.iter().enumerate() {
            let base_optimization = incumbent.optimization.clone();
            let base_total = incumbent.total;
            let duration = ordered[ordinal].duration;

            for candidate in &candidates[skill_index as usize] {
                let mut current = Incumbent {
                    score: 0,
                    total: base_total + candidate.offset.abs(),
                    optimization: base_optimization.clone(),
                };
                if candidate.offset != 0.0 {
                    current.optimization.insert(skill_index, candidate.offset);
                    if !pair_ok(&current.optimization, skill_index) {
                        continue;
                    }
                }
                current.score = evaluate(&ordered, &current.optimization);

                let c_time = candidate.time;
                let f_end = c_time + duration + context.end_grace;
                let start_list = start_cache
                    .entry(time_key(c_time))
                    .or_insert_with(|| build_start_list(c_time));
                let end_list = end_cache
                    .entry(time_key(f_end))
                    .or_insert_with(|| build_end_list(f_end));

                for shift in start_list.iter().chain(end_list.iter()) {
                    let mut child = Incumbent {
                        score: 0,
                        total: current.total + shift.offset.abs(),
                        optimization: current.optimization.clone(),
                    };
                    child.optimization.insert(shift.index, shift.offset);
                    if !pair_ok(&child.optimization, shift.index) {
                        continue;
                    }
                    child.score = evaluate(&ordered, &child.optimization);
                    if better(child.score, child.total, current.score, current.total) {
                        current = child;
                    }
                }

                if better(
                    current.score,
                    current.total,
                    incumbent.score,
                    incumbent.total,
                ) {
                    incumbent = current;
                }
            }
        }

        if better(incumbent.score, incumbent.total, best.score, best.total) {
            best.order = order;
            best.optimization = incumbent.optimization;
            best.score = incumbent.score;
            best.total = incumbent.total;
        }
    }

    best.simulations = simulations;
    best
}
