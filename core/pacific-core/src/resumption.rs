//! What `Node::resume` hands back (resumption.md §9, as amended A6–A7).
//!
//! Every object the spine names has an outcome, whatever happened to it — the
//! compliant-or-not doctrine: a shorter list with nothing said is the failure.

use crate::head::ChainVerdict;

/// The head as the platform fetched it, and the position the Arc declared beside it.
#[derive(Debug, Clone)]
pub struct HeadInput {
    pub blob: Vec<u8>,
    pub declared_position: u64,
}

#[derive(Debug, Clone)]
pub struct Resumed {
    /// `None` when no head was supplied: nothing claims completeness without one.
    pub verdict: Option<ChainVerdict>,
    /// The self record's id, hex. `None` when the chain is empty.
    pub spine: Option<String>,
    /// The spine first (A6.5), then every object the spine names and has not left.
    pub objects: Vec<ObjectOutcome>,
}

#[derive(Debug, Clone)]
pub struct ObjectOutcome {
    pub object: String,
    pub kind: String,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Joined through a pool leaf; readable from its join epoch.
    Joined { from_epoch: u64 },
    /// This device already holds the object; nothing was done.
    AlreadyHeld,
    /// Every pool leaf the way-in lists is leased or evicted.
    PoolExhausted,
    /// The spine names the object but holds no `Pool` entry for it (A6.11).
    WayInMissing,
    /// Kept for the wire; not produced under A1 — an entry that will not open
    /// names no object, and ends the walk instead (A8).
    WayInUnreadable(String),
    /// The object was not joined: a leaf was taken and its Welcome or key package
    /// would not join (A7.2), or the way-in's route was refused before any leaf was
    /// taken (CA-5). The reason says which.
    JoinFailed(String),
}
