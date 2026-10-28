//! A1a AT THE INTERFACE — one assertion, in a binary of its own.
//!
//! `docs/the-build.html` §02 A1a pins WHERE the type goes: "Lift `PerLeafIdentity`
//! out of `m24_leaf_pool.rs:46-101` into `mls.rs`". `docs/execution.html` §04 says
//! why an interface has to be pinned in advance: "If a signature is not named in
//! advance the suite will not compile and the discipline degenerates into
//! negotiation."
//!
//! This is a whole test binary for a type name because a missing name is a COMPILE
//! error and a compile error takes its binary with it. Kept alone, the cost of
//! disagreeing about the name is this file and nothing else —
//! `gate_a1a_pacific_builder_admits_two_leaves.rs` goes on proving the behaviour
//! either way.
//!
//! What it pins: the type EXISTS, is PUBLIC, lives in `pacific_core::mls` (not in
//! a test file, not in a private module), and is an `IdentityProvider` that a
//! client builder can take (`Clone`, as every mls-rs client config requires).
//!
//! What it deliberately does NOT pin: construction. `identity()` returning
//! credential ‖ signature_key and `valid_successor` comparing credentials only are
//! `m24_leaf_pool`'s assertions, made against the type directly; re-making them
//! here would be a second copy that can drift.
//!
//! TODAY: does not compile — `pacific_core::mls::PerLeafIdentity` does not exist;
//! the type is a private experiment inside `tests/m24_leaf_pool.rs`.
//! AFTER: passes.
//!
//! HOW THIS COULD BE FAKED. Re-export a `PerLeafIdentity` from `mls.rs` that is
//! not what `build_client` uses. Closed by the companion binary, which builds
//! every client through `mls::build_client_sqlite` and never names this type.

use mls_rs_core::identity::IdentityProvider;

/// Instantiating this with a type is the assertion.
fn pins_an_identity_provider<P: IdentityProvider + Clone>() {}

#[test]
fn per_leaf_identity_is_public_in_pacific_core_mls() {
    pins_an_identity_provider::<pacific_core::mls::PerLeafIdentity>();
}
