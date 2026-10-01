//! MEDIA — shape, budget, validation and compression for every media slot.
//!
//! Dependency-free on purpose. This crate is compiled both into the phone (via
//! `pacific-core`) and into a wasm shell for the browser demos, and the moment
//! it grows a dependency that only builds for one of those, the demos stop
//! exercising the real thing and start exercising a copy of it.

pub mod anim;
pub mod bc1;
pub mod media;
pub mod preprocess;
pub mod pyramid;

pub use media::{Delivery, MediaError, MediaKind, MediaRef, Slot};
pub use preprocess::{preprocess, Discard, Orientation, Preprocessed, SourceFacts, CONTAINER_MIME};

use std::collections::BTreeMap;

/// The delta argument vocabulary. Mirrors `pacific_core::coordinator::ArgVal`
/// exactly; it lives here so media can flatten itself onto the wire without
/// this crate depending on the coordinator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArgVal {
    Int(i64),
    Text(String),
}

pub type Args = BTreeMap<String, ArgVal>;
