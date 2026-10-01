//! projection — the seam where a reduced GroupObject content unit may be handed to
//! a store beside the directory. The union LodeDB route (`lode_route`, the `lode`
//! feature) implements it; a Node with no projector installed skips it. Apart from
//! `lode_route` so a build without lodedb (the Door, O-36) still has the seam.

/// The §3 `source` classifier for a row (which producer authored the content).
/// Mirrors the Swift metadata builder's `source` vocabulary value-for-value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeltaSource {
    Note,
    Forum,
    Project,
    Topic,
    Entity,
    System,
}

impl DeltaSource {
    /// The stable on-row string. MUST match the Swift builder's `source` values.
    pub fn as_str(self) -> &'static str {
        match self {
            DeltaSource::Note => "note",
            DeltaSource::Forum => "forum",
            DeltaSource::Project => "project",
            DeltaSource::Topic => "topic",
            DeltaSource::Entity => "entity",
            DeltaSource::System => "system",
        }
    }
}

/// One reduced GroupObject content unit handed to a [`ContentProjector`] hook.
pub struct ProjectedContent<'a> {
    pub source: DeltaSource,
    /// The enclosing GroupObject id (the §3 `groupObjectId` scope key).
    pub group_object_id: &'a str,
    /// The content unit's own id (a delta id / message ref / item id).
    pub object_id: &'a str,
    pub text: &'a str,
}

/// The opt-in seam the delta reduce/append path (`node.rs`) calls to project a
/// reduced GroupObject content unit into a store. A node with no projector
/// installed skips it entirely. Fails loud — a projection error propagates rather
/// than silently dropping content out of the index.
pub trait ContentProjector: Send + Sync {
    fn project(&self, content: ProjectedContent<'_>) -> Result<u64, crate::CoreError>;
}
