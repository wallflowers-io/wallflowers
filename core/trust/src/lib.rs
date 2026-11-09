//! trust — whose key is whose.
//!
//! The checks that bind a public key to a person, held apart from the engine so they
//! can be audited alone. Pure: no I/O, no state directory, no network, no unsafe.

#![forbid(unsafe_code)]
