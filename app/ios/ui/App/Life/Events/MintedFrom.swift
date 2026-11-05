//  MintedFrom.swift
//  Which listing a minted Event came FROM — the device-local half of the
//  listing→event identity. Keyed by the SOURCE KEY the feed rows already carry
//  ("ra:<id>" / "art:<ident>" — the sources' stable ids, never their URLs), so
//  re-tapping MINT on a listing opens the event this device already made instead
//  of minting a twin.
//
//  DEVICE-LOCAL deliberately: the cross-connection half — announcing "I minted
//  ra:166016 → object X" so connections JOIN the canonical event instead of
//  minting their own (two MLS groups never merge; see object.rs doctrine) — is
//  the Phase 2 contact op.

import Foundation

enum MintedFrom {
    private static let key = "pacific.events.mintedFrom"

    /// The Event this device minted from `sourceKey`, if any — validated by the
    /// caller against the live fold (a deleted event must not trap the listing).
    static func objectId(for sourceKey: String) -> String? {
        (UserDefaults.standard.dictionary(forKey: key) as? [String: String])?[sourceKey]
    }

    static func record(_ objectId: String, from sourceKey: String) {
        var map = (UserDefaults.standard.dictionary(forKey: key) as? [String: String]) ?? [:]
        map[sourceKey] = objectId
        UserDefaults.standard.set(map, forKey: key)
    }
}
