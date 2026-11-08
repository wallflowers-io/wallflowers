//! The record the resolver hands back that the engine does not define: the episode provenance
//! row `ArcResolver::episode_index` returns and the FFI serializes.

/// One episode's provenance row: which entities the observation produced facts about.
///
/// `episode_id` is the caller-stable id the episode was written under (a note's UUID), so a
/// consumer inverts this into `entity -> [source object]` without any id translation table.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct EpisodeProvenance {
    pub episode_id: String,
    /// The observation kind the episode was recorded under ("note", "message", ...).
    pub source: String,
    /// Event time of the observation, epoch ms.
    pub occurred_at: i64,
    /// Entities reachable as endpoints of this episode's facts, deduped, first-seen order.
    pub entities: Vec<String>,
}
