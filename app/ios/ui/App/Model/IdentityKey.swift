//  IdentityKey.swift
//  THE one place the app knows how an identity is written down.
//
//  An identity is an Ed25519 PUBLIC KEY — 32 bytes — and nothing else. It has
//  been written three ways:
//
//      ed25519:<64 hex>   the honest rendering (12 Aug)
//      space1<64 hex>     the legacy rendering — still inside shipped folds,
//                         so it is read forever and never emitted again
//      <64 hex>           bare, as the wire and the directory carry it
//
//  WHY THIS FILE EXISTS. Before it, the app carried SIX hand-rolled
//  `hasPrefix("space1") ? dropFirst(6) : self` strips and FOUR hand-rolled
//  `"space1" + hex` concatenations, scattered across CoreService, OrgWizard,
//  GroupDetailView, EntityDetail, ObjectProjection and DevLinks. Every one of
//  them silently fails the moment an id arrives in a different rendering: the
//  prefix check misses, the id is never normalised, and a string comparison
//  that decides whether two peers are the same person quietly answers "no".
//  That is a data bug wearing a formatting bug's clothes.
//
//  THE RULE: compare `hex(_:)`, never raw strings. Render with `display(_:)`.
//  Mirrors `pacific-core/src/identity.rs` (`parse_identity_key`,
//  `IDENTITY_KEY_PREFIX`, `LEGACY_SPACE_PREFIX`) — read wide, write narrow.

import Foundation

enum IdentityKey {

    /// The rendering this app emits.
    static let prefix = "ed25519:"

    /// The pre-12-Aug rendering. Read forever; never written.
    static let legacyPrefix = "space1"

    /// Any rendering → the bare 64-hex the wire and the directory speak.
    /// THE comparison form: two ids name the same person iff their `hex` match.
    static func hex(_ id: String) -> String {
        if id.hasPrefix(prefix) { return String(id.dropFirst(prefix.count)) }
        if id.hasPrefix(legacyPrefix) { return String(id.dropFirst(legacyPrefix.count)) }
        return id
    }

    /// Bare hex (or any rendering) → the canonical written form.
    static func display(_ id: String) -> String { prefix + hex(id) }

    /// Do two renderings name the same identity?
    static func same(_ a: String, _ b: String) -> Bool { hex(a) == hex(b) }

    /// The short form for a label — first 8 hex, the eye's version of the
    /// fingerprint. Never a truncated *rendering*, which would compare wrong.
    static func short(_ id: String) -> String { String(hex(id).prefix(8)) }
}
