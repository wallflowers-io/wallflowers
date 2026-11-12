//  KindModels.swift
//  The maximised detail's data contract — the PIE half of the module.
//
//  PURPOSE. One component renders every NON-EGOIC GroupObject a card stands for:
//  an event, a listing, a post, a place. They are not profiles and must not borrow
//  the profile's IA — a profile answers "who is this", and none of these is a who.
//  What they each have is the same four things:
//
//      MEDIA    what it looks like
//      ROWS     the values its ops carry, in reading order
//      AUTHOR   whose it is — a person or a Space, and a door to their profile
//      COMMENTS what people said about it, and a way to say something
//
//  What differs between kinds is only which rows a binder answers, so a kind with
//  more to say says it in its rows, never in a new slot here.
//
//  THE OWNER'S PAGE IS ALSO WHERE THE OBJECT GROWS. The creator takes three fields;
//  every other field an op carries is written here, by the owner, on the row that
//  shows it. A row's `edit` is what makes it writable — the binder sets it only for
//  the owner — and an unset row is shown only to the person who can set it.
//
//  Honest-by-default, as everywhere: an unknown value is "", never a fabricated
//  stand-in. A page whose source cannot answer shows less, never more.

import SwiftUI

// MARK: - Page values

/// The media block, the title, and the kind's rows under them.
struct KindHeadline {
    let title: String
    /// The owner writes the title where it is printed, not in a second copy below it.
    var titleEdit: KindEdit? = nil
    /// The picture's REAL bytes, decoded from the fold. nil = there is none, and
    /// the slot shows the stripes rather than implying one is loading.
    var photo: Data? = nil
    /// A REMOTE picture, for the one case with no fold behind it: a world listing
    /// (RA, ArtRabbit) read straight off its source.
    var photoURL: URL? = nil
    /// A Place's media: the area patch around its own point. Empty = none.
    var area: [ProfileGeoCell] = []
    /// The pictures after the first, in order: an event's photos, a listing's, a
    /// post's assets.
    var gallery: [KindPicture] = []
    /// A document read in place — a Post's PDF.
    var document: KindDocument? = nil
    /// The kind's values, in the order they should read.
    var rows: [KindRow] = []
    /// What the owner can add here. Empty for everyone else.
    var adds: [KindAdd] = []
    /// A way INTO the object, past its page — a Place's POV.
    var inside: KindInside? = nil
}

/// A screen the page opens over itself; `view` is handed the close.
struct KindInside {
    let label: String
    let glyph: String
    let id: String
    let view: (@escaping () -> Void) -> AnyView
}

struct KindPicture: Identifiable {
    let id: Int
    let data: Data
    var alt: String = ""
}

struct KindDocument {
    let name: String
    let data: Data
}

/// One value on the page. A VIEWER sees `value` alone — no caption, because a value
/// that needs one is the wrong value — and nothing at all when it is empty. The
/// OWNER sees every row that has an `edit`, labelled, in the creator's row grammar,
/// so an unset field is a place to write rather than a hole.
struct KindRow: Identifiable {
    let label: String
    let value: String
    /// Opens in the browser.
    var link: URL? = nil
    /// A door to another object's page — an event's venue Place.
    var place: String? = nil
    /// The author wrote it in markdown (`descriptorFormat`, `bodyFormat`).
    var markdown = false
    var edit: KindEdit? = nil

    var id: String { edit.map { "\($0.op).\($0.arg)" } ?? label }
}

/// How the owner writes a row: the op that replaces whole, its ICD arg, and the arg's
/// current wire value, which the editor opens on.
struct KindEdit {
    let op: String
    let arg: String
    let input: KindInput
    let current: String
}

enum KindInput: Equatable {
    case line
    case multiline
    /// The ICD's words for the arg (`CoreService.vocabulary`).
    case choice([String])
    case toggle
    /// Epoch ms, as text on the wire of `current`.
    case date
}

enum KindAdd: String, Identifiable {
    case picture, document
    var id: String { rawValue }
}

/// Whose it is. A door, not a credit line: tapping it opens their profile, which
/// is the one place the egoic IA belongs.
struct KindAuthor {
    let id: String
    let name: String
    let initials: String
    /// Person or Space — decides which page the door opens.
    let isSpace: Bool
    var photo: Data? = nil
    var color: Color? = nil
}

/// One comment on the object's comments section. Flat on purpose: the threaded
/// projection exists (`ForumThreadRow.depth`) and this page deliberately does not
/// use it — a nested tree is a room's IA, and this is a detail with comments on it.
struct KindComment: Identifiable {
    let id: String
    /// The author's identity hex — their profile's door.
    let authorID: String
    let author: String
    let initials: String
    let body: String
    let whenMs: Int64
    var mine: Bool = false
    var photo: Data? = nil
    var color: Color? = nil
}

// MARK: - The binder seam

/// Everything a detail page asks of the world. The module declares it; the host
/// injects the concrete (PIE⊗CI — the arrow is module → protocol ← host), exactly
/// as `ProfileSource` works for the egoic half.
///
/// Async because the real binders fold/read off-main; each answer is the CURRENT
/// truth, nil where the world cannot honestly answer.
@MainActor
protocol KindSource {
    /// The media block and rows. nil = the object is gone or unreadable, and the
    /// page shows nothing rather than a fabricated shell.
    func headline() async -> KindHeadline?
    /// Whose it is. nil = the fold does not name an author — no credit line is
    /// better than a guessed one.
    func author() async -> KindAuthor?
    /// What has been said, oldest first.
    func comments() async -> [KindComment]
    /// Whether this device holds the object's comments section — the section is on
    /// the page exactly when it does, said-in or not.
    func commentsHeld() async -> Bool
    /// Say something. false = it did not land, and the surface keeps the text.
    func post(_ text: String) async -> Bool
    /// Hand this object outward — GOLD. Absent (the default) means this object has
    /// nowhere to go, and the disc stays glyphless and dead rather than lying.
    func shareTargets() async -> [KindShareTarget]
    func share(to target: KindShareTarget) async -> Bool
    /// The owner writes one row. false = it did not land.
    func edit(_ edit: KindEdit, to value: String) async -> Bool
    /// The owner adds a picture, or sets the document.
    func addPicture(_ data: Data) async -> Bool
    func setDocument(_ data: Data, name: String) async -> Bool
}

/// Somewhere an object can be handed to — a Space you are actually in.
struct KindShareTarget: Identifiable, Hashable {
    let id: String
    let name: String
}

extension KindSource {
    func shareTargets() async -> [KindShareTarget] { [] }
    func share(to target: KindShareTarget) async -> Bool { false }
    func commentsHeld() async -> Bool { false }
    func edit(_ edit: KindEdit, to value: String) async -> Bool { false }
    func addPicture(_ data: Data) async -> Bool { false }
    func setDocument(_ data: Data, name: String) async -> Bool { false }
}
