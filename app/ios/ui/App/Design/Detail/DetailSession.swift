//  DetailSession.swift
//  The detail modal's session — a `ShellSurface`, so the three discs become its
//  buttons exactly as they do for a mint.
//
//  WHAT THE DISCS MEAN HERE, and why each is the inherited meaning rather than a
//  new one:
//
//    RED ✕    close. The same disc that cancels a mint, doing the same job — and
//             it warns first for the same reason, when there is a typed comment
//             about to be thrown away.
//    GOLD ↑   hand it outward. On a mint GOLD is the audience dial; the chevron
//             has meant "this goes outward" in this shell since the note, and
//             what going outward means for a thing you did not make is sharing it
//             into a Space. Nowhere to send it ⇒ the disc stays glyphless and
//             dead, which is the row's own honest state, not a missing button.
//    BLUE →   say something. A mint's BLUE commits what you typed; so does this.
//
//  The session holds the LOADED STATE as well as the draft, because the surface
//  must survive a reload without flashing — and because the disc row reads its
//  gates through `cell`, which has to know whether there is a comment to send.

import SwiftUI
import Combine

/// What BLUE does. Taken as a value for the same reason `NewSession` takes its
/// commit as a closure: the disc always COMMITS, and what committing means is the
/// surface's business, not the shell's.
///
/// `.comment` is the ordinary case — a GroupObject has a log, so BLUE says the
/// thing you typed. `.action` is for a subject with no log to speak into: a world
/// listing nobody has minted commits by BECOMING one, and there is no comment
/// field on that surface at all, because there is nowhere for a comment to go.
enum DetailPrimary {
    case comment
    case action(label: String, run: () async -> NewExit?)
}

@MainActor
final class DetailSession: ObservableObject, Identifiable, ShellSurface {
    let id = UUID()
    /// The word the discard warning names — "event", "listing", "post". Given by
    /// the host, because only the host knows what it opened.
    let noun: String
    let primary: DetailPrimary
    private let source: any DetailSource

    /// Whether this surface has anything to type INTO. A `.action` surface has no
    /// log behind it, so it shows no field rather than one that goes nowhere.
    var takesComment: Bool { if case .comment = primary { true } else { false } }

    /// What the world answered. All nil/empty until the first load lands; the
    /// surface renders what it has and never a placeholder for what it hasn't.
    @Published private(set) var headline: DetailHeadline?
    @Published private(set) var author: DetailAuthor?
    @Published private(set) var comments: [DetailComment] = []
    @Published private(set) var targets: [DetailShareTarget] = []

    /// The comment being typed. This is the only thing on the surface a person can
    /// destroy by closing, so it is the only thing the discard warning is about.
    @Published var draft = ""
    /// Sending — BLUE goes dead for the duration so a double tap cannot post twice.
    @Published private(set) var sending = false
    /// The share face: GOLD turns the card over to the Space list and back, the
    /// same turn the mint's audience dial makes.
    @Published var sharing = false

    @Published private(set) var exit: NewExit?
    /// An owner's write in flight — its row goes still so it cannot be written twice.
    @Published private(set) var saving: String?

    /// The card's square, in screen space, that the page grows out of and shrinks back
    /// into. nil = it was not opened from a card, and it rises in place.
    let origin: CGRect?

    init(noun: String, source: any DetailSource, primary: DetailPrimary = .comment,
         origin: CGRect? = nil) {
        self.noun = noun
        self.source = source
        self.primary = primary
        self.origin = origin
    }

    // MARK: The owner's writes

    /// Each lands through its op and is READ BACK from the fold, never mirrored, so
    /// the page cannot show a value the log does not hold.
    func edit(_ edit: DetailEdit, to value: String) async {
        guard saving == nil, value != edit.current else { return }
        saving = edit.op + "." + edit.arg
        if await source.edit(edit, to: value) { headline = await source.headline() }
        saving = nil
    }

    func addPicture(_ data: Data) async {
        guard saving == nil else { return }
        saving = DetailAdd.picture.rawValue
        if await source.addPicture(data) { headline = await source.headline() }
        saving = nil
    }

    func setDocument(_ data: Data, name: String) async {
        guard saving == nil else { return }
        saving = DetailAdd.document.rawValue
        if await source.setDocument(data, name: name) { headline = await source.headline() }
        saving = nil
    }

    // MARK: Loading

    /// Ask the world. Runs on appear and again after a post lands, so a comment
    /// you just made is read back from the fold rather than mirrored into a local
    /// array that could then disagree with it.
    func load() async {
        headline = await source.headline()
        author = await source.author()
        comments = await source.comments()
        targets = await source.shareTargets()
    }

    // MARK: ShellSurface

    /// The three discs. BLUE lights the moment there is something to say; GOLD has
    /// an identity at all only if this object has somewhere to go.
    nonisolated var cell: NewCell {
        MainActor.assumeIsolated {
            let live: Bool
            let label: String
            switch primary {
            case .comment:
                live = !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !sending
                label = "Send"
            case .action(let l, _):
                live = !sending
                label = l
            }
            return NewCell(canCommit: live,
                           canShare: !targets.isEmpty,
                           hasShare: !targets.isEmpty,
                           commitGlyph: "chevron.right",
                           commitLabel: label,
                           shareGlyph: "chevron.up",
                           shareLabel: "Share",
                           cancelLabel: "Close")
        }
    }

    nonisolated var discardNoun: String {
        MainActor.assumeIsolated { noun }
    }

    /// Only a typed comment is worth warning about. Closing a detail modal
    /// destroys nothing else — you are looking at something, not making it.
    nonisolated var needsDiscardWarning: Bool {
        MainActor.assumeIsolated {
            !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        }
    }

    nonisolated var changes: ObservableObjectPublisher {
        MainActor.assumeIsolated { objectWillChange }
    }

    func gold() {
        withAnimation(.spring(response: 0.4, dampingFraction: 0.86)) { sharing.toggle() }
    }

    /// BLUE.
    ///
    /// COMMENTING LEAVES THE SURFACE UP, deliberately. A comment is not why you
    /// opened the thing, so saying one should not take the thing away — the field
    /// clears, the log is re-read, and nil tells the shell to play no departure.
    /// An `.action` surface is the opposite: its commit IS the reason it was open,
    /// so it reports a real exit and goes.
    func performCommit() async -> NewExit? {
        switch primary {
        case .comment:
            let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !text.isEmpty, !sending else { return nil }
            sending = true
            let ok = await source.post(text)
            sending = false
            guard ok else { return nil }      // the text stays; nothing was said
            draft = ""
            comments = await source.comments()
            return nil
        case .action(_, let run):
            guard !sending else { return nil }
            sending = true
            let outcome = await run()
            sending = false
            return outcome
        }
    }

    func beginExit(_ exit: NewExit) { self.exit = exit }

    // MARK: Sharing

    func share(to target: DetailShareTarget) async {
        guard await source.share(to: target) else { return }
        withAnimation(.spring(response: 0.4, dampingFraction: 0.86)) { sharing = false }
    }
}
