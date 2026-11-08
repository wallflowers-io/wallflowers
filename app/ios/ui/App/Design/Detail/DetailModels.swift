//  DetailModels.swift
//  The detail modal's data contract — the PIE half of the module.
//
//  PURPOSE. One component renders every NON-EGOIC GroupObject: an event, a
//  listing, a post. They are not profiles and must not borrow the profile's IA —
//  a profile answers "who is this", and none of these three is a who. What they
//  each have is the same three things, which is the whole of this contract:
//
//      MEDIA    what it looks like, and the few facts printed on it
//      AUTHOR   whose it is — a person or a Space, and a door to their profile
//      COMMENTS what people said about it, and a way to say something
//
//  Anything a particular kind has BEYOND those three is that kind's business and
//  does not belong here. The temptation is to add a slot per kind until the
//  contract is the union of three screens; the discipline is that a kind with
//  more to say says it in its `facts`.
//
//  Honest-by-default, as everywhere: an unknown value is nil, never a fabricated
//  "" / 0 / stand-in. A modal whose source cannot answer shows less, never more.

import SwiftUI

// MARK: - Card values

/// The media block and the handful of facts under it.
struct DetailHeadline {
    let title: String
    /// The picture's REAL bytes, decoded from the fold. nil = there is none, and
    /// the slot shows the stripes rather than implying one is loading.
    var photo: Data? = nil
    /// A REMOTE picture, for the one case with no fold behind it: a world listing
    /// (RA, ArtRabbit) read straight off its source.
    var photoURL: URL? = nil
    /// The lines printed under the title — "Sat 2 Aug · 13:00 – 18:00", "Weekly on
    /// Thursday", a price, a venue. TERSE VALUES, in the order they should read;
    /// the component does not label them, because a value that needs a caption is
    /// the wrong value.
    var facts: [String] = []
}

/// Whose it is. A door, not a credit line: tapping it opens their profile, which
/// is the one place the egoic IA belongs.
struct DetailAuthor {
    let id: String
    let name: String
    let initials: String
    /// Person or Space — decides which page the door opens.
    let isSpace: Bool
    var photo: Data? = nil
    var color: Color? = nil
}

/// One comment on the object's own log. Flat on purpose: the threaded projection
/// exists (`ForumThreadRow.depth`) and this modal deliberately does not use it —
/// a nested tree is a room's IA, and this is a detail screen with comments on it.
struct DetailComment: Identifiable {
    let id: String
    let author: String
    let initials: String
    let body: String
    let whenMs: Int64
    var mine: Bool = false
    var color: Color? = nil
}

// MARK: - The binder seam

/// Everything a detail modal asks of the world. The module declares it; the host
/// injects the concrete (PIE⊗CI — the arrow is module → protocol ← host), exactly
/// as `ProfileSource` works for the egoic half.
///
/// Async because the real binders fold/read off-main; each answer is the CURRENT
/// truth, nil where the world cannot honestly answer.
@MainActor
protocol DetailSource {
    /// The media block. nil = the object is gone or unreadable, and the modal
    /// shows nothing rather than a fabricated shell.
    func headline() async -> DetailHeadline?
    /// Whose it is. nil = the fold does not name an author — no credit line is
    /// better than a guessed one.
    func author() async -> DetailAuthor?
    /// What has been said, oldest first.
    func comments() async -> [DetailComment]
    /// Say something. false = it did not land, and the surface keeps the text.
    func post(_ text: String) async -> Bool
    /// Hand this object outward — GOLD. Absent (the default) means this object has
    /// nowhere to go, and the disc stays glyphless and dead rather than lying.
    func shareTargets() async -> [DetailShareTarget]
    func share(to target: DetailShareTarget) async -> Bool
}

/// Somewhere an object can be handed to — a Space you are actually in.
struct DetailShareTarget: Identifiable, Hashable {
    let id: String
    let name: String
}

extension DetailSource {
    func shareTargets() async -> [DetailShareTarget] { [] }
    func share(to target: DetailShareTarget) async -> Bool { false }
}
