//  ObjectIndex.swift
//  THE index of what this device holds: every object, by kind, from the core's one
//  sweep (`objects(kind:)`), and every person with their avatar (`people()`). Two
//  FFI reads per refresh (plus a photo only when it changed); every reader filters
//  here, never by asking the core again.
//
//  A value held on CoreService (`core.index`), so a view that observes CoreService
//  redraws when it changes. Identities are compared by `IdentityKey.hex`.

import SwiftUI
import PacificCore

struct ObjectIndex: Equatable {
    private(set) var rows: [ObjectRow] = []
    /// identity hex → person; this device's own identity included (`isMe`).
    private(set) var people: [String: PacificCore.PersonRow] = [:]
    /// identity hex → the avatar photo's bytes, at the `photoAt` they were read for.
    private(set) var photos: [String: (at: UInt64, data: Data?)] = [:]

    /// The comments role, as the core's ICD-pinned table names it.
    static let commentsPart = commentsRole()

    // MARK: objects

    /// Every object of one kind.
    func objects(_ kind: ObjectKind) -> [ObjectRow] {
        let name = objectKindName(kind: kind)
        return rows.filter { $0.kind == name }
    }

    func object(_ id: String) -> ObjectRow? { rows.first { $0.id == id } }

    /// A Forum that is its parent's comments section: reached from its object, never
    /// listed as a conversation.
    func isCommentsSection(_ forumId: String) -> Bool {
        object(forumId)?.parentRole == Self.commentsPart
    }

    /// The object's comments section, by the Forum's own back-edge; nil until held.
    func commentsForum(of objectId: String) -> String? {
        rows.first { $0.parent == objectId && $0.parentRole == Self.commentsPart }?.id
    }

    // MARK: people

    func person(_ identity: String) -> PacificCore.PersonRow? { people[IdentityKey.hex(identity)] }

    /// A person's name, or nil when this device knows none.
    func name(_ identity: String) -> String? {
        person(identity).map(\.name).flatMap { $0.isEmpty ? nil : $0 }
    }

    /// Every other person's name by identity hex, for labelling rosters and authors.
    var names: [String: String] {
        people.values.reduce(into: [:]) { out, p in if !p.isMe && !p.name.isEmpty { out[p.identity] = p.name } }
    }

    /// A person's avatar photo bytes, or nil.
    func photo(_ identity: String) -> Data? { photos[IdentityKey.hex(identity)]?.data ?? nil }

    /// A person's chosen avatar colour, or nil to keep the neutral disc.
    func color(_ identity: String) -> Color? {
        person(identity).flatMap { $0.color.isEmpty ? nil : Color(hex: $0.color) }
    }

    // MARK: refresh

    /// The photos to fetch: each person never read, or whose `photoAt` moved since.
    func stalePhotos(in fresh: [PacificCore.PersonRow]) -> [String] {
        fresh.filter { photos[$0.identity]?.at != $0.photoAt }.map(\.identity)
    }

    /// Fold one refresh in. `fetched` holds the photos read for `stalePhotos`.
    mutating func apply(rows: [ObjectRow], people fresh: [PacificCore.PersonRow], fetched: [String: PersonPhoto?]) {
        self.rows = rows
        people = Dictionary(fresh.map { ($0.identity, $0) }, uniquingKeysWith: { a, _ in a })
        for p in fresh {
            if let got = fetched[p.identity] {
                photos[p.identity] = (p.photoAt, got.flatMap { Data(base64Encoded: $0.photo) })
            }
        }
        photos = photos.filter { people[$0.key] != nil }
    }

    static func == (a: ObjectIndex, b: ObjectIndex) -> Bool {
        a.rows == b.rows && a.people == b.people
            && a.photos.mapValues(\.at) == b.photos.mapValues(\.at)
    }
}
