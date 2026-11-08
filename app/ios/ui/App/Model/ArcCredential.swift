//  ArcCredential.swift
//  The membership credential minter — how this device proves to an Arc that it is a member,
//  without anything but the core ever touching the identity key.
//
//  Membership IS the {user, Arc} MLS tether (ICD §10), the same pair-scan key exchange as
//  user↔user, so a member proves it by SIGNING with the identity key that established the
//  tether (checked by the membership plane's `verify_member`). The credential is minted fresh
//  per request:
//
//      Authorization: <space_id>:<ts_ms>:<sig_hex>
//      sig = Ed25519("pacific-arc-member:v1\n<arc_space_id>\n<space_id>\n<ts_ms>")
//
//  It is short-lived by construction (the Arc enforces a ±5min window), scoped to ONE Arc by
//  the audience inside the signed payload, and leaves nothing at rest to steal. A bearer key is
//  the wrong shape for membership: long-lived, transferable, and unrelated to the tether that
//  established it.
//
//  WHY A REGISTERED CLOSURE. The whole credential — payload, signature, encoding — is assembled
//  in Rust (`arc_member_credential`), so the format stays pinned to the verifier that reads it
//  and no general signing primitive crosses the FFI. `CoreService` owns the core, so it
//  registers the minter once the identity has loaded; every caller (today `ArcMembership`)
//  holds only the narrow capability "mint a membership credential for THIS Arc". A caller is
//  never handed the core, so nothing that asks for a credential can sign anything else.

import Foundation
import os

/// Membership diagnostics go through the unified log, not `print`. A `print` lands only on the
/// stdout of a process you are already attached to, so it vanishes the moment the app is
/// relaunched by anything but the debugger — exactly when a failed mint is most likely to be
/// the one you needed to see. Capture with:
/// `log stream --debug --predicate 'subsystem == "network.pacific" AND category == "membership"'`.
let membershipLog = Logger(subsystem: "network.pacific", category: "membership")

/// The seam through which the app mints membership credentials.
enum ArcCredential {
    /// `(arcSpaceID) -> credential`. Set once by `CoreService` after the identity loads, because
    /// minting needs a key to sign with. Nil means this device cannot mint yet, and a caller
    /// then reports membership as unproven rather than sending a request it cannot authenticate.
    static var provider: (@Sendable (String) async throws -> String)?
}
