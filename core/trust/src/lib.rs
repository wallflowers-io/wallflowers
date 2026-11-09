//! trust — whose key is whose.
//!
//! The checks that bind a public key to a person, held apart from the engine so they
//! can be audited alone. Pure: no I/O, no state directory, no network, no unsafe.
//!
//! - [`sig`]: the one strict Ed25519 verify.
//! - [`qr`]: the in-person QR's contact bundle, and the binding of its key package to
//!   its signer.

#![forbid(unsafe_code)]

pub mod cbor;
pub mod qr;
pub mod sig;

/// A refusal. Callers classify by variant; the text is the reason, unchanged from
/// the engine's.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The bundle does not encode, decode, or is a version this build does not read,
    /// or its key package speaks for someone else.
    #[error("{0}")]
    Bundle(String),
    /// A public key that is not one, or a signature that does not verify.
    #[error("{0}")]
    Signature(String),
}
