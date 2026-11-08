//  PublicPlace.swift
//  A Place anyone can join by scanning a static QR — a gate, a bench, an allotment.
//
//  A PublicPlace is not a new kind of object — it is a Place in a different access
//  posture (`Access::Public` in pacific-core), exactly as a Thing's market posture is
//  state on the Thing. One kind, one type_id, one list.
//
//  # The doorbell
//
//      pacific://place-join?id=<place object id>&db=<doorbell, 64 hex>
//
//  The doorbell is a 32-byte capability living in the PLACE's log — both the mailbox
//  address a stranger knocks at and the key that seals what they leave. It replaced the
//  host's personal contact bundle, which was wrong on a public sign in two ways:
//
//    * `intro_tag` is ONE PER DEVICE — the key to a person's entire inbound pairing
//      mailbox. Printing it on a gate published that to everyone who walked past, for
//      every pairing they would ever do, unrotatably.
//    * It forced every joiner into a 1:1 connection with whoever printed the poster. You
//      should not have to befriend the person who put up the sign.
//
//  The doorbell is public by construction — anyone with the sign can read who is
//  arriving, which for an allotment gate is the roster of an allotment. It is not buying
//  secrecy. It buys REVOCATION: rotate it and every printed sign goes inert, which is
//  also the only way to cut off an ex-member (removing someone from the group does not
//  retract the doorbell they already folded out of the log).
//
//  # Knocking
//
//  Scanning deposits YOUR OWN signed contact bundle in that mailbox. A bundle is
//  self-contained — key package, identity, signature — so any member who picks it up can
//  admit you with `group_add_member` on their next pass. No pairing, and no waiting for
//  prekeys to be stocked, so admission lands on the first sync rather than after retries.
//
//  # Who answers
//
//  ANY member, not the founder. The doorbell is in the Place's log, so everyone who is in
//  folds the same key, and `add_member_core` relaxes its owner-only rule exactly at a
//  public door. That is what keeps an allotment joinable while any plot-holder's app is
//  running rather than one specific person's.
//
//  Over the relay that needs someone foregrounded (or a push wake); over the BLE mesh it
//  works with phones asleep in pockets — which is the common case here, because the person
//  knocking is physically standing at the gate.

import Foundation

enum PublicPlace {

    /// A scanned place sign, taken apart.
    struct JoinLink: Equatable {
        /// The Place's object id — stable across reprints.
        let placeID: String
        /// The doorbell capability, 64 hex chars.
        let doorbell: String
    }

    /// Parse a scanned string as a place sign, or nil if it is something else (an ordinary
    /// pairing QR, a URL, junk).
    ///
    /// Strict about both parts: an id with no doorbell cannot be knocked at, and a
    /// doorbell with no id names no door.
    static func joinLink(from raw: String) -> JoinLink? {
        let s = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let comps = URLComponents(string: s),
              comps.scheme == "pacific",
              comps.host == "place-join" || comps.path == "place-join",
              let id = comps.queryItems?.first(where: { $0.name == "id" })?.value,
              let db = comps.queryItems?.first(where: { $0.name == "db" })?.value,
              !id.isEmpty, db.count == 64
        else { return nil }
        return JoinLink(placeID: id, doorbell: db)
    }

    /// Knock: leave our bundle at the door.
    ///
    /// Returns true once the knock is published. YOU ARE NOT A MEMBER YET — a member of
    /// the place admits you when they next answer. Callers must say so rather than
    /// implying the join already landed.
    @MainActor
    @discardableResult
    static func knock(_ link: JoinLink, core: CoreService) async -> Bool {
        await core.knockAtPlace(placeID: link.placeID, doorbell: link.doorbell)
    }
}
