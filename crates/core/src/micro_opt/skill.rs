//! Bestdori-compatible skill effect family.
//!
//! The standard [`crate::model::preparation::SkillDefinition`] reduces skills
//! to `duration + score_up` for team search. Micro-optimisation instead needs
//! the full frontend effect family because judgement upgrades, life
//! conditions and perfect-count growth all change which offset maximises the
//! final score.

use crate::model::schema::Attribute;

/// Activation condition of an effect, mapped to a judgement-rank threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectCondition {
    Perfect,
    Great,
    Good,
    Bad,
    Miss,
    None,
}

impl EffectCondition {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "perfect" => Some(Self::Perfect),
            "great" => Some(Self::Great),
            "good" => Some(Self::Good),
            "bad" => Some(Self::Bad),
            "miss" => Some(Self::Miss),
            "none" => Some(Self::None),
            _ => None,
        }
    }

    /// Notes with `judgement <= threshold(self)` satisfy the condition. The
    /// frontend maps `none` to `Infinity`.
    pub fn threshold(self) -> f64 {
        match self {
            Self::Perfect => 0.0,
            Self::Great => 1.0,
            Self::Good => 2.0,
            Self::Bad => 3.0,
            Self::Miss => 4.0,
            Self::None => f64::INFINITY,
        }
    }
}

/// Effect carrying a per-level percentage value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NumericEffect {
    pub condition: EffectCondition,
    /// Percentage already divided by 100, e.g. `0.10` for a 10% score up.
    pub value: f64,
}

/// Effect with only an activation condition.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConditionEffect {
    pub condition: EffectCondition,
}

/// Effect gated on the current life value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LifeConditionEffect {
    pub condition: EffectCondition,
    pub condition_life: f64,
    pub value: f64,
}

/// Trigger-time (once per activation) life recovery.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OnceLifeCondition {
    Always,
    UnderLife(f64),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OnceLifeEffect {
    pub value: f64,
    pub condition: OnceLifeCondition,
}

/// One team card's skill resolved at its equipped level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MicroSkill {
    pub skill_id: u32,
    pub skill_level: u8,
    pub duration: f64,
    pub effects: MicroSkillEffects,
}

/// The full effect family read from Bestdori's
/// `activationEffect.activateEffectTypes`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MicroSkillEffects {
    pub score: Option<NumericEffect>,
    pub judge: Option<ConditionEffect>,
    pub damage: Option<ConditionEffect>,
    pub never_die: bool,
    pub score_over_life: Option<LifeConditionEffect>,
    pub score_under_life: Option<LifeConditionEffect>,
    pub score_continued_note_judge: Option<NumericEffect>,
    pub score_rate_up_with_perfect: Option<ConditionEffect>,
    pub score_under_great_half: Option<NumericEffect>,
    pub once_life: Option<OnceLifeEffect>,
    pub unification_band_id: Option<u32>,
    pub unification_attribute: Option<Attribute>,
    /// Bestdori stores the unification value as percent; already divided.
    pub unification_value: f64,
}

impl MicroSkill {
    /// Unification bonus fraction for a team, mirroring the frontend's
    /// `bandId match || attribute match` resolution.
    pub fn unification_bonus(
        &self,
        team_band_id: Option<u32>,
        team_attribute: Option<Attribute>,
    ) -> f64 {
        let band_hit = self
            .effects
            .unification_band_id
            .is_some_and(|band_id| team_band_id == Some(band_id));
        let attr_hit = self
            .effects
            .unification_attribute
            .is_some_and(|attribute| team_attribute == Some(attribute));
        if band_hit || attr_hit {
            self.effects.unification_value
        } else {
            0.0
        }
    }
}
