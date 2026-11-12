//! Application service for the Bestdori-compatible micro-optimisation.

use bangdream_optimize_core::SongSelection;
use bangdream_optimize_data::{
    DataError, MicroOptimizeInputBuilder, MicroOptimizeOutcome, PlayerConfigStore,
};
use std::sync::Arc;

#[derive(Clone)]
pub struct MicroOptimizeService {
    player_store: Arc<dyn PlayerConfigStore>,
    calculator: Arc<dyn MicroOptimizeInputBuilder>,
}

impl std::fmt::Debug for MicroOptimizeService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("MicroOptimizeService").finish()
    }
}

impl MicroOptimizeService {
    pub fn new(
        player_store: Arc<dyn PlayerConfigStore>,
        calculator: Arc<dyn MicroOptimizeInputBuilder>,
    ) -> Self {
        Self {
            player_store,
            calculator,
        }
    }

    /// Loads the player profile from the store, then delegates to the data
    /// layer for resolution and the core kernel for the search.
    pub async fn optimize_for_player(
        &self,
        player_id: i64,
        event_id: u32,
        song: SongSelection,
        team: [u32; 5],
        judgement_windows: Option<bangdream_optimize_core::micro_opt::JudgementWindowSet>,
    ) -> Result<MicroOptimizeOutcome, DataError> {
        let player = self
            .player_store
            .get_player_config(player_id)
            .await?
            .ok_or(DataError::PlayerNotFound { player_id })?;
        self.calculator
            .micro_optimize(player, event_id, song, team, judgement_windows)
            .await
    }
}
