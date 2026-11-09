//  New.swift
//  THE universal `New` surface — one component behind every `+` in the app.
//
//  Derived from the NOTE card (Notes/NoteComposeModal.swift), which is where all of
//  this already worked for exactly one kind. A single tap on the RED circle raises it,
//  and the thing that makes it feel unlike every other modal in the app is that it has
//  NO BUTTONS: its three actions ARE the shell's own RED · GOLD · BLUE circles, which
//  slide out of their neutral arrangement into ✕ · ↑ · → and slide home again when it
//  leaves.
//
//  Design/ComposeSurface.swift said why nothing else could have that, in its own header:
//
//      "The NOTE card does not use this: its buttons are the shell's ACTUAL discs,
//       which slide into this arrangement and back out again, so it gets the
//       choreography as well as the look. A modal presented ABOVE the shell (a sheet,
//       a full-screen cover) covers those discs and cannot borrow them, so it hosts
//       this row instead."
//
//  So the fix is not a restyle. It is one structural move — present every mint as a
//  shell overlay UNDER the disc row, and generalise `composing: Bool` into
//  `compose: NewSession?` — after which the ground, the glass, the ambient, the flash,
//  the exit curves and the haptics are already shared and already generic.
//
//  ─────────────────────────────────────────────────────────────────────────────
//  THE INVARIANT
//
//  `NewKind` is anchored to the core's `ObjectKind`, NOT to the LIFE rail. A `New` is
//  DEFINED BY WHAT IT MINTS, and its `rawValue` is the core's own wire name, so
//  `core.mint(kind:draft:)` takes it verbatim and a rename on either side fails at the
//  boundary rather than minting into the void.
//
//  That is not a style preference. The `+` used to be dressed by `LifeFilter.mintType`
//  — a rail-shaped vocabulary — and it minted whatever the wizard behind it happened to
//  call, which on three of LIFE's five jewels was the WRONG SPECIES: EVENTS minted a
//  device-local outline row (plus a Google Calendar insert) while the real Event(29)
//  mint sat unreachable, and PEOPLE minted a local row while the list above it was built
//  from real Contact(25) connections. Anchoring the kind to the object type makes that
//  category of bug unrepresentable.

import SwiftUI
import Combine      // ObservableObjectPublisher — the ShellSurface seam
import PacificCore   // MintDraft — the core's own draft, crossed once in `NewDraft.mint(for:)`

// MARK: - Kind

/// Every mint the app can perform, named by WHAT IT MINTS.
///
/// The raw value is `ObjectKind::name()` in the core (pacific-core/src/object.rs), which
/// `pacific_core::mint::kind_from_name` parses on the other side of the FFI. `.note` is
/// the one case with no object: it is genuinely device-local (NotesStore), and saying so
/// here is better than pretending otherwise.
enum NewKind: String, CaseIterable {
    case note                    // device-local — NotesStore, no object
    case space      = "group"    // ObjectKind::Group   = 18
    case event                   // ObjectKind::Event   = 29
    case listing    = "thing"    // ObjectKind::Thing   = 27
    case place                   // ObjectKind::Place   = 28
    case project                 // ObjectKind::Project = 22
    case channel    = "forum"    // ObjectKind::Forum   = 19 — minted INTO a Space
    case post                    // ObjectKind::Post    = 30

    /// The core kind this mints, or nil when the mint is device-local. Passed straight
    /// to `CoreService.mint(kind:draft:)`.
    var coreKind: String? { self == .note ? nil : rawValue }

    /// The wizard's title — "New Space", "New Event".
    var noun: String {
        switch self {
        case .note:    return "Note"
        case .space:   return "Space"
        case .event:   return "Event"
        case .listing: return "Listing"
        case .place:   return "Place"
        case .project: return "Project"
        case .channel: return "Channel"
        case .post:    return "Post"
        }
    }

    var glyph: String {
        switch self {
        case .note:    return "square.and.pencil"
        case .space:   return "person.3.fill"
        case .event:   return "calendar"
        case .listing: return "shippingbox.fill"
        case .place:   return "mappin.circle.fill"
        case .project: return "square.grid.2x2.fill"
        case .channel: return "number"
        case .post:    return "text.bubble.fill"
        }
    }

    /// What BLUE says. Six phrasings for one action used to live as literals at six call
    /// sites ("Save Place", "Save Event", "Save listing", "Create Space", "Constitute",
    /// "Create"); collected here so the variation is a table.
    var commitLabel: String {
        switch self {
        case .note:    return "Send"
        case .space:   return "Create Space"
        case .channel: return "Create Channel"
        default:       return "Save \(noun)"
        }
    }

    /// The surface's colour — what the glass is cast in, what the ambient's fourth blob
    /// becomes, and what the edge flashes on a successful commit.
    ///
    /// Every kind takes its slot in the ACTIVE Van Gogh palette, the note included (Ralph,
    /// 5 Nov: one rule, so the creator's cards are one family), so a repalette moves every
    /// mint surface at once, and the slots match `LifeFilter.colorIndex` — a jewel and the
    /// surface its `+` opens cannot disagree.
    var gloss: (hi: Color, lo: Color) {
        let base = PaletteStore.base(colorIndex)
        return (base.lightened(0.16), base.darkened(0.32))
    }

    private var colorIndex: Int {
        switch self {
        case .event:   return 0
        case .place:   return 2
        case .space:   return 3
        case .project: return 4
        case .listing: return 5
        case .channel: return 3      // a Channel is part of its Space — same jewel
        case .post:    return 6
        case .note:    return 1      // the one slot no jewel holds
        }
    }
}

// MARK: - The disc cell

/// What the three circles ARE, right now, for the surface that is up.
///
/// This replaces `InterfaceMatrix.Context.composing(hasContent: Bool)`. The note's
/// `hasContent` was doing three jobs at once — gate GOLD, gate BLUE, and stand in for
/// the labels — which is fine for one surface and wrong for seven.
///
/// Equatable so the matrix context stays Equatable and the row still derives itself
/// purely from shell state each render, never stored.
struct NewCell: Equatable {
    var canCommit: Bool
    /// GOLD's own gate. It is NOT `canCommit`: New Space's gold opens the member list,
    /// which is answerable before the Space has a name and is exactly what you want to
    /// do first (ComposeSurface.swift found this out first).
    var canShare = false
    /// Whether this surface has a GOLD action AT ALL.
    ///
    /// `ComposeDiscs` omits the disc entirely when there is nothing to hand off — "a
    /// button that can never fire is worse than an absent one". The shell CANNOT: the
    /// three circles never come and go, they only move and change identity, and that
    /// persistence is exactly what makes them slide rather than cross-fade. So an absent
    /// share does not remove GOLD — it leaves GOLD with NO compose identity: glyphless,
    /// disabled, at the row's existing 0.3 inactive opacity. It reads as a disc that is
    /// not part of this action, which is the truth, and it costs no new mechanism.
    var hasShare = false

    var commitGlyph = "chevron.right"
    var commitLabel = "Save"
    var shareGlyph = "chevron.up"
    var shareLabel = "Share"
    var cancelLabel = "Cancel"

    /// The note's cell, verbatim — the behaviour the generalisation must preserve
    /// exactly. Named so a regression shows up as a diff here rather than as a feel
    /// that changed.
    static func note(hasContent: Bool) -> NewCell {
        NewCell(canCommit: hasContent,
                canShare: hasContent,
                hasShare: true,
                commitLabel: "Send")
    }
}

// MARK: - Outcome

/// How a `New` left, which is how the surface plays its departure — each outcome gets
/// its own edge colour, its own exit curve and its own (or no) haptic, so the surface
/// says what happened without a word of copy.
///
/// This is `ShellState.NoteExit` renamed and nothing else: the three outcomes were
/// already general. A minted Space is `.saved`; one you added people to on the way out
/// is `.shared`; a binned draft is `.cancelled`.
enum NewExit {
    case saved      // tint edge, opens out past the screen edges as it fades, success haptic
    case shared     // azure edge, leaves upward after what it handed off, medium impact
    case cancelled  // no colour, no haptic — discarding is the plain fade it always was
}

// MARK: - The payload

/// What is being written. One struct, not seven, because the SHELL holds it and the
/// fields are overwhelmingly shared — the same judgement `EntityWizard` already made
/// with one flat bag of `@State` for five types, restated as a value.
///
/// Mirrors `pacific_core::mint::MintDraft` field for field, and `mint()` below is the
/// only place that crossing happens.
struct NewDraft: Equatable {
    var name = ""
    var descriptor = ""

    // Kind-specific. All optional; a kind that does not use a field never reads it.
    // There is deliberately NO `note` slot: a Space's purpose IS its descriptor (the
    // core folds it into the ContactCard's `note`), and the Place/Event "Note" fields
    // are their `descriptor` too. A separate slot is what let those fields be typed
    // into and silently dropped.
    var date: Date? = nil                 // event
    var endDate: Date? = nil              // event

    /// An event's opening default: TOMORROW, at whichever of 08:00 · 12:00 · 18:00 · 22:00
    /// sits nearest the current time (Ralph, 18 Aug). Nearest is measured ROUND THE CLOCK,
    /// so 23:30 defaults to 22:00 rather than crossing eight hours to 08:00 — the slot
    /// closest on the dial is the one closest to the part of the day you are in.
    ///
    /// A default, not a placeholder: it is written into the draft, so an event minted
    /// without touching WHEN has a real time rather than the zero sentinel `isAnswered`
    /// reads as unanswered.
    static func defaultEventStart(from now: Date = Date(),
                                  calendar: Calendar = .current) -> Date {
        let slots = [8, 12, 18, 22]
        let mins = calendar.component(.hour, from: now) * 60 + calendar.component(.minute, from: now)
        let best = slots.min { a, b in
            func gap(_ h: Int) -> Int {
                let d = abs(h * 60 - mins)
                return min(d, 24 * 60 - d)          // round the dial, not along a line
            }
            return gap(a) < gap(b)
        } ?? 12
        let tomorrow = calendar.date(byAdding: .day, value: 1, to: now) ?? now
        return calendar.date(bySettingHour: best, minute: 0, second: 0, of: tomorrow) ?? tomorrow
    }
    var venue = ""                        // event — the profile's venue NAME
    var recurrence = ""                   // event — RFC 5545 RRULE
    var lineup: [String] = []             // event — billing order
    var shape = ""                        // space — individual|team|organisation|community
    var category = ""                     // listing — artifact|skill|job
    var posture = ""                       // listing — has|wants|offers|buying|selling
    var reach = ""                        // listing — private|network
    var placed = false                    // place — what we ASSERT about lat/lng
    var lat = 0.0
    var lng = 0.0
    var body = ""                         // note
    var icon = Data()                     // media — the round face
    var banner = Data()                   // media — the wide wall
    var price = ""                        // listing — only on an ACTIVE posture
    var place = ""                        // a Place REFERENCE, not a coordinate
    var form = ""                         // Post: article | link | image
    var link = ""                         // Post: a link post's destination

    // WHO IT IS FOR, and what stands outside it (#76). Part of the draft, so an edit to
    // either is an edit, and the commit reads them with the rest.
    var audience: Audience = .onlyMe
    var outside = ""                      // the exterior's description
    var door: DoorChoice = .review        // how an outsider gets in

    /// Read/write the slot a declared field binds to. One total switch, so a new
    /// `MintKey` cannot be half-wired.
    subscript(key: MintKey) -> String {
        get {
            switch key {
            case .name:       return name
            case .descriptor: return descriptor
            case .shape:      return shape
            case .category:   return category
            case .price:      return price
            case .stance:     return posture
            case .recurrence: return recurrence
            case .place:      return place
            case .form:       return form
            case .link:       return link
            // Not text: an instant, a fix, and two byte blobs.
            case .start, .fix, .icon, .banner: return ""
            }
        }
        set {
            switch key {
            case .name:       name = newValue
            case .descriptor: descriptor = newValue
            case .shape:      shape = newValue
            case .category:   category = newValue
            case .price:      price = newValue
            case .stance:     posture = newValue
            case .recurrence: recurrence = newValue
            case .place:      place = newValue
            case .form:       form = newValue
            case .link:       link = newValue
            case .start, .fix, .icon, .banner: break
            }
        }
    }

    /// Is this field answered? Mirrors `MintDraft::is_answered` in the core.
    ///
    /// `.start` is answered by any NON-ZERO instant. 0 is the sentinel
    /// `event.setProfile` used to receive when the picker was never touched — `req_int`
    /// accepts it, so the event was minted in **1970**. The gate has to be here.
    func isAnswered(_ key: MintKey) -> Bool {
        switch key {
        case .start:  return date != nil
        case .fix:    return placed
        case .icon:   return !icon.isEmpty
        case .banner: return !banner.isEmpty
        default:      return !self[key].trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        }
    }

    /// BLUE's gate: every REQUIRED field the type declares must be answered. One rule
    /// applied to the declaration, rather than a per-kind condition in the UI that can
    /// disagree with what the reducer demands.
    ///
    /// The note is the one surface with no declaration — it is device-local and mints no
    /// object — so its content is its gate.
    func canCommit(_ fields: [MintFieldRow]) -> Bool {
        guard !fields.isEmpty else { return hasBody }
        return fields.allSatisfy { !$0.required || isAnswered($0.key) }
    }

    /// Anything typed at all — what RED warns about before discarding.
    func hasContent(_ fields: [MintFieldRow]) -> Bool {
        hasBody || fields.contains { isAnswered($0.key) }
    }

    private var hasBody: Bool {
        !body.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            || !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    /// The one crossing into the core's own draft.
    ///
    /// A straight field-for-field copy — no per-kind logic. `card` is deliberately EMPTY:
    /// a Space's purpose reaches the wire as the ContactCard's `note`, and the core builds
    /// that from `descriptor` (`mint::card_with_note`). The UI used to hand-build that
    /// JSON, which is precisely how the declared field and the delta drifted apart.
    /// Built FROM THE DECLARATION, one assignment per declared field — not a positional
    /// copy of all eighteen slots.
    ///
    /// The declaration is the core's (`pacific_core::mint`), read here by the same static
    /// lookup `NewSession` uses. Two things follow. A field the kind does NOT declare
    /// cannot reach the wire, so a slot left over in the draft — a venue typed on a
    /// previous kind, a lineup from a resumed session — can no longer ride along into an
    /// object that never asked for it. And a field ADDED to the declaration needs one case
    /// here and nothing else: the switch is total, so it fails to compile until it is
    /// wired, which is the same discipline `subscript(key:)` already holds.
    func mint(for kind: NewKind) -> MintDraft {
        var d = MintDraft(name: "", descriptor: "", shape: "", card: "", category: "",
                          startMs: 0, endMs: 0, venue: "", recurrence: "", lineup: "",
                          form: "", link: "",
                          placed: false, lat: 0, lng: 0, status: "",
                          icon: Data(), banner: Data(), price: "", stance: "", place: "")
        for field in kind.coreKind.map({ mintFields(kind: $0) }) ?? [] {
            switch field.key {
            case .name:       d.name = name.trimmingCharacters(in: .whitespacesAndNewlines)
            case .descriptor: d.descriptor = descriptor.trimmingCharacters(in: .whitespacesAndNewlines)
            case .shape:      d.shape = shape
            case .category:   d.category = category
            case .price:      d.price = price
            // STANCE rides `thing.setPosture` after the mint (`afterMint`), not op 0 —
            // carried here so the caller has it without reaching around the draft.
            case .stance:     d.stance = posture
            case .recurrence: d.recurrence = recurrence
            case .place:      d.place = place
            // A Post's shape, and the destination only a `link` carries. The fold
            // refuses a link on any other form, so a stale value from a previous
            // kind cannot ride along — the declaration is what gates it here.
            case .form:       d.form = form
            case .link:       d.link = link
            case .icon:       d.icon = icon
            case .banner:     d.banner = banner
            // WHEN carries the whole span: one declared field, two slots on the wire.
            case .start:
                d.startMs = date.map { Int64($0.timeIntervalSince1970 * 1000) } ?? 0
                d.endMs = endDate.map { Int64($0.timeIntervalSince1970 * 1000) } ?? 0
            // HERE is the assertion plus its coordinate — `placed` is what we CLAIM about
            // lat/lng, so it travels with them or not at all.
            case .fix:
                d.placed = placed
                d.lat = lat
                d.lng = lng
            }
        }
        // Declared by no `MintKey` yet, and set only by a caller that pre-fills the draft
        // (`NewSession(kind:draft:)`) — never by a control on this surface. They stay
        // outside the loop rather than being silently dropped by it.
        d.venue = venue
        d.lineup = lineup.joined(separator: "\n")
        // `card` and `status` are the core's to build: a Space's purpose reaches the wire
        // as the ContactCard's `note`, which `mint::card_with_note` composes from
        // `descriptor`. The UI used to hand-build that JSON, which is precisely how the
        // declared field and the delta drifted apart.
        return d
    }
}

// MARK: - Audience — WHO this object is for

/// The universal audience dial, reached from the GOLD circle on every surface.
///
/// Five options spanning TWO mechanisms, and it is worth being explicit about which is
/// which rather than pretending it is one facet:
///
///   Only Me      MEMBERSHIP — a GroupObject with no additional members. The default at
///                mint, so it needs no op at all.
///   People       MEMBERSHIP — selected connections join the roster.
///   Spaces       MEMBERSHIP — selected Spaces you belong to.
///   Connections  VISIBILITY — every direct peer may learn it exists (n-1). Never relayed.
///   Network      VISIBILITY — the full discovery reach, relayable (n-2 today).
///
/// The last two are `pacific_core::visibility::Visibility`, whose wire values these raw
/// values match. The first three are the roster.
///
/// PERSISTENCE: Only Me, Connections and Network are written at mint as the object's
/// `base.setVisibility` (ICD 2.6.0 puts it on every kind with an exterior), and at
/// Connections or Network its exterior and door with it (`CoreService.setOutside`). The
/// per-delta flag (Ralph, 18 Aug) is separate and still to come. People and Spaces are the
/// roster, and still need their selection step.
enum Audience: String, CaseIterable, Identifiable {
    case onlyMe      = "private"
    case people      = "people"
    case spaces      = "spaces"
    case connections = "connections"
    case network     = "network"

    var id: String { rawValue }

    var title: String {
        switch self {
        case .onlyMe:      return "Only Me"
        case .people:      return "People"
        case .spaces:      return "Spaces"
        case .connections: return "Connections"
        case .network:     return "Network"
        }
    }

    /// True when this option is a `Visibility` level the core already names. The other
    /// three are roster choices and need a selection step.
    var isVisibilityLevel: Bool {
        self == .onlyMe || self == .connections || self == .network
    }

    /// The two that need you to pick WHO before they mean anything.
    var needsSelection: Bool { self == .people || self == .spaces }
}

/// Which face of the surface is showing. The card does not go away and is not covered —
/// it TURNS, the way the disc row changes identity rather than swapping views.
enum NewFace { case fields, visibility }

// MARK: - The session

/// ONE mint in flight. The shell holds at most one; `compose != nil` IS `composing`,
/// which is what keeps `buttonRow` untouched by all of this — it still asks one Bool
/// whether to spread the circles.
///
/// A reference type because the surface EDITS it and the disc row ACTS on it, and
/// neither owns the other — the same reason `NoteDraft` is one.
@MainActor
final class NewSession: ObservableObject, Identifiable {
    let id = UUID()
    let kind: NewKind

    /// Republishing this re-derives the disc cell, so BLUE lights up the moment the
    /// surface becomes committable — the contract `hasContent` had.
    @Published var draft: NewDraft

    /// Non-nil once the surface is on its way out — still MOUNTED so the departure can
    /// be seen. Set by `ShellState.endNew`, read by the surface.
    @Published fileprivate(set) var exit: NewExit?

    /// True while a commit is in flight. BLUE goes dead for the duration, so a double
    /// tap cannot mint twice — the old wizards each carried their own `minting` flag for
    /// this and two of them did not have one.
    @Published fileprivate(set) var committing = false

    /// Where this mints INTO, when the kind does not imply it: the Space a Channel
    /// belongs to. nil = the user's own graph, which is what LIFE's `+` always means.
    let parent: String?

    /// WHAT THIS KIND REQUIRES TO EXIST — declared by the type, in `pacific_core::mint`,
    /// and read once here. The surface renders these in order and BLUE gates on them, so
    /// the UI cannot ask for a field the mint drops, nor let you commit without an arg
    /// the reducer demands. Empty for `.note`, which mints no object.
    let fields: [MintFieldRow]

    /// Which face is showing — the fields, or the audience dial. GOLD turns it.
    @Published var face: NewFace = .fields

    /// WHO this is for. Universal across every kind (Ralph, 18 Aug), which settles the
    /// open question about what GOLD means: it is privacy/sharing, everywhere. Held in the
    /// draft, so it publishes with it.
    var audience: Audience {
        get { draft.audience }
        set { draft.audience = newValue }
    }

    /// The draft as it opened, seeded. What `edited` measures against.
    private var opening: NewDraft

    /// Has the person changed anything — a field, a choice, a date, the audience? On the
    /// creator the first edit chooses the kind.
    var edited: Bool { draft != opening || audience != .onlyMe }

    /// What moved from the opening, by name — for the log that says why a kind locked.
    var edits: [String] {
        let now = Mirror(reflecting: draft).children, then = Mirror(reflecting: opening).children
        var moved = zip(now, then).compactMap { a, b in
            String(describing: a.value) == String(describing: b.value) ? nil : a.label
        }
        return moved
    }

    /// A change the DEVICE makes, not the person — the Place's fix landing. It moves the
    /// opening with it, so it never reads as an edit.
    func absorb(_ change: (inout NewDraft) -> Void) {
        change(&draft)
        change(&opening)
    }

    /// BLUE. Returns the outcome so the shell can play the right departure; `nil` means
    /// the commit FAILED and the surface stays up with its content intact.
    let commit: (NewDraft) async -> NewExit?

    init(kind: NewKind,
         draft: NewDraft = NewDraft(),
         parent: String? = nil,
         commit: @escaping (NewDraft) async -> NewExit?) {
        self.kind = kind
        // Read the type's declaration ONCE, at construction. A pure static lookup in the
        // core — no node, no state — so it is safe to call while building a view.
        self.fields = kind.coreKind.map { mintFields(kind: $0) } ?? []
        // Seed every declared field that has a sensible opening answer, so a REQUIRED
        // field is never silently unanswered behind a control that looks filled in. The
        // date picker showed "now" and wrote nothing; that is exactly the 1970 bug.
        var seeded = draft
        for field in fields where !seeded.isAnswered(field.key) {
            switch field.input {
            case .choice: seeded[field.key] = field.choices.first ?? ""
            // An EVENT opens at its own default — tomorrow, at the nearest of
            // 08:00 · 12:00 · 18:00 · 22:00 — because "now" is never when a night is.
            // Any other kind's date still opens at now.
            case .date:   seeded.date = kind == .event ? NewDraft.defaultEventStart() : Date()
            default:      break
            }
        }
        self.draft = seeded
        self.opening = seeded
        self.parent = parent
        self.commit = commit
    }

    /// The row's identity, derived — never stored, so the circles cannot disagree with
    /// the surface they are the buttons for.
    var cell: NewCell {
        // GOLD is ALWAYS live and always means the same thing — who this is for. It is
        // not gated on the commit rule: the audience is answerable before the object has
        // a name, and is often the first thing you know about it.
        NewCell(canCommit: draft.canCommit(fields) && !committing,
                canShare: !committing,
                hasShare: true,
                commitLabel: kind.commitLabel,
                // The UP CHEVRON the NOTE's gold always wore — not an eye (Ralph,
                // 18 Aug). The chevron is the shell's own vocabulary for "this goes
                // outward", which is exactly what deciding an audience is; an eye is a
                // borrowed icon that means "view".
                shareGlyph: face == .visibility ? "chevron.down" : "chevron.up",
                shareLabel: face == .visibility ? "Back" : "Visibility")
    }

    /// GOLD. Turns the card between its two faces.
    func toggleVisibility() {
        face = face == .fields ? .visibility : .fields
    }

    /// RED destroys something that cannot be recovered, so a surface with content in it
    /// warns first — the note's rule, and the right rule for all seven.
    var needsDiscardWarning: Bool { draft.hasContent(fields) }

    fileprivate func setExit(_ e: NewExit?) { exit = e }
    fileprivate func setCommitting(_ v: Bool) { committing = v }

    /// Run BLUE. Guards re-entry itself rather than trusting every caller to.
    fileprivate func runCommit() async -> NewExit? {
        guard !committing, draft.canCommit(fields) else { return nil }
        committing = true
        let outcome = await commit(draft)
        committing = false
        return outcome
    }
}

// MARK: - The surface

/// THE universal `New` modal. One implementation; the kind supplies the fields.
///
/// Structurally identical to `NoteComposeModal` — because it IS `NoteComposeModal`,
/// generalised. Same ground, same glass, same three departures, same measured travel,
/// same keyboard release on exit, and the same bottom clearance that stops it ABOVE the
/// disc row so those circles can be its buttons.
///
/// It carries no buttons. That sentence is the whole component.
struct NewSurface: View {
    @ObservedObject var session: NewSession
    /// Dismiss without keeping. Only ever called from here for an EMPTY surface (tapping
    /// the ground); a surface with content leaves by RED, which warns first.
    var onCancel: () -> Void
    /// RED's universal creator, when this surface is it: every kind's card side by side,
    /// and the carousel that picks between them. nil is the single-kind surface every
    /// other `+` raises. `session` is then the creator's card in view.
    var creator: NewCreator? = nil

    /// The card's own height, so "leaves upward" clears the screen on any device rather
    /// than travelling some guessed number of points.
    @State private var travel: CGFloat = 900
    /// The card's live frame, reported every layout so the ground's bloom can stay
    /// anchored to it while the keyboard moves it.
    @State private var cardFrame: CGRect = .zero

    private var exit: NewExit? { session.exit }

    /// VISIBILITY's cyan. Deliberately not the ambient's azure (`0x49A6F2`, which already
    /// means "handed off") — this is a state the surface is IN, not an outcome.
    static let visibilityCyan = Color(hex: 0x22D3EE)

    /// Each outcome leaves on its own curve. These were already general — only the names
    /// said "note".
    private var cardExit: Animation {
        switch exit {
        case .saved:  return Motion.noteExpand
        case .shared: return Motion.noteSlideOut
        default:      return Motion.noteFade
        }
    }

    var body: some View {
        ZStack {
            // The shared compose ground — the same deep-slate scrim and drifting jewel
            // light, now cast in THIS kind's colour. `EntityWizard` used to re-roll this
            // inline with its own spring; adopting the real one was a deletion.
            // TAP OFF THE CANVAS TO DISCARD — one behaviour for every kind (Ralph,
            // 18 Aug). It routes to the SAME action RED does, so an empty surface just
            // goes and one with something in it asks first. Before this the tap was
            // gated on `!needsDiscardWarning`, which meant that on a surface with
            // content it silently did nothing — the note's original rule, never
            // inherited by the newitem modals and not what is wanted anyway.
            ComposeGround(tint: session.tint, opaque: true, canvas: cardFrame,
                          onTapOutside: onCancel)
                // The ground goes FIRST and fast — the screen underneath is back before
                // the card in front of it has finished leaving.
                .opacity(exit == nil ? 1 : 0)
                .animation(Motion.noteGroundOut, value: exit)

            Group {
                if let creator {
                    CreatorPages(creator: creator)
                } else {
                    CardTablet(id: "new-tablet-\(session.kind.rawValue)") { height in
                        NewCard(session: session, minHeight: height).padding(.horizontal, 16)
                    }
                    .onAppear { NSLog("[creator] single card %@", session.kind.rawValue) }
                }
            }
                // The departure, in three flavours. SAVED opens out past the screen edges
                // as it fades — the thing going INTO the app, not off it. SHARED goes up,
                // after what it handed off. CANCELLED just fades. All of them run with the
                // edge flash and the circles sliding home, a beat behind the ground.
                .scaleEffect(exit == .saved ? 1.3 : 1)
                .opacity(exit == nil ? 1 : 0)
                .offset(y: exit == .shared ? -travel : 0)
                .animation(cardExit, value: exit)
                // Up to the status bar, over the top row's buttons (Ralph, 5 Nov): while
                // a New is up, the discs are the only controls. The tablet runs on under the
                // status bar, blurred, and keeps its own space below it (`CardTablet.lead`).
                // No stop above the disc row: the tablet runs on under it and the keyboard,
                // blurred behind them, and keeps its own clearance (`CardTablet`).
                .background(GeometryReader { geo in
                    Color.clear.onAppear { travel = geo.size.height * 1.4 }
                })
        }
        .coordinateSpace(name: "newSurface")
        .onPreferenceChange(CardFrameKey.self) { cardFrame = $0 }
        .allowsHitTesting(exit == nil)   // a leaving surface takes no more taps
    }
}

extension NewSession {
    /// The surface's colour. On the VISIBILITY face the gold glow swaps to CYAN
    /// (Ralph, 18 Aug) — the same card, turned, wearing the audience dial's own colour so
    /// you can see at a glance which face you are on.
    var tint: Color { face == .visibility ? NewSurface.visibilityCyan : kind.gloss.hi }
}

/// ONE session's glass card — the fields, the header, the visibility face. `NewSurface`
/// holds the ground and the departure around it.
struct NewCard: View {
    @ObservedObject var session: NewSession
    /// One of the creator's cards: no header, and nothing focused on open.
    var creator = false
    /// The card in view. A card that leaves lets go of the keyboard; only the card in
    /// view reports its frame to the ground.
    var active = true
    /// At least this tall: the page its tablet shows, so a short card still fills it.
    var minHeight: CGFloat = 0

    @FocusState private var focus: FocusTarget?
    /// Focus is keyed by the DECLARED field, not by a hand-kept case list — so a new
    /// declared field is focusable without touching this, and the in-field title knows
    /// exactly which box is active.
    private enum FocusTarget: Hashable {
        case key(MintKey)
        case body            // the note's editor, which has no declared field
    }

    /// PLACES mints from a COORDINATE, so the surface reads the live fix while it is up.
    /// `PlaceLocator` rather than the shared LocationService because only it surfaces
    /// `horizontalAccuracy`, which is what sets the geohash precision (PlaceFix.swift) —
    /// the same reason `EntityWizard` used it. Constructed for every kind and STARTED
    /// only for `.place`, once it is being edited; an idle locator holds no CoreLocation
    /// session.
    @StateObject private var locator = PlaceLocator()

    /// WHICH media slot is asking for the camera, if any. Keyed by the declared field so
    /// the capture knows where to put what it takes — a single Bool could not say.
    ///
    /// TODO(media): wire this to the capture surface (the camera, not a library picker)
    /// and store the result in `draft.icon` / `draft.banner`. The slots are the design;
    /// the core declares both fields `Carriage::Deferred` until the per-kind media ops
    /// (`group.setCover` 9 · `event.setMedia` 4 · `thing.setPhoto` 3) are authored at mint.
    @State private var camera: MintKey? = nil

    private var exit: NewExit? { session.exit }
    private var kind: NewKind { session.kind }
    private var tint: Color { session.tint }

    /// The tint for kept, azure for handed off. Discarding gets NO colour — it goes the
    /// way it always went, and a draft thrown away is not an event worth marking.
    private var flash: Color? {
        switch exit {
        case .saved:  return tint
        case .shared: return Color(hex: 0x49A6F2)   // the ambient's azure
        default:      return nil
        }
    }

    var body: some View {
        card
        .onAppear {
            // NOTHING IS SELECTED when a newitem modal opens (Ralph, 18 Aug), so the
            // keyboard stays down until you choose a field. The surface is a form — a
            // MEDIA row, switches, WHERE — not a single box to type in; opening straight
            // onto the keyboard hides half of it and presumes the first field is the one
            // you wanted.
            //
            // The NOTE is the exception and keeps its autofocus: it IS a single box to
            // type in, and it has raised the keyboard on open since it was built.
            // As the universal creator nothing is focused for ANY kind: the picker is the
            // first question, and the keyboard would cover it.
            focus = kind == .note && !creator ? .key(.name) : nil
        }
        // Location is asked for when the person starts on the Place (Ralph, 5 Nov) — a
        // field chosen, anything changed, or the WHERE row tapped — never because a Place
        // card is showing or slid past.
        .onChange(of: focus) { _, f in if f != nil { locate() } }
        .onChange(of: session.edited) { _, edited in if edited { locate() } }
        .onChange(of: active) { _, now in if !now { focus = nil } }
        // The fix lands ASYNCHRONOUSLY, after the surface is already up — so the draft
        // takes it whenever it arrives rather than being read once at commit. A Place
        // saved before it lands is minted UNPLACED, which is honest: refusing to save
        // because GPS is slow indoors would lose the user's typing for a reason they
        // cannot act on.
        // Flipping STANCE can strand KIND on a value it no longer offers — pick NEED ▸
        // JOB, then flip to OFFER, and the picker is holding "job" against a list that
        // has SKILL and THING. The capability swaps with the stance; THING is offered on
        // both sides and is left alone.
        .onChange(of: session.draft.category) { _, kind in
            // KIND decides STANCE's vocabulary, so a stance the new kind does not offer
            // has to move: HAVE and OFFER are the same side of the same trade, so they
            // swap. WANT/NEED is `wants` either way and needs no clamp.
            let stance = session.draft.posture
            if kind == "skill", stance == "offers" { session.draft.posture = "has" }
            if kind != "skill", stance == "has" { session.draft.posture = "offers" }
        }
        .onChange(of: locator.state) { _, s in
            guard kind == .place, let fix = s.fix else { return }
            session.absorb { d in
                d.lat = fix.coordinate.latitude
                d.lng = fix.coordinate.longitude
                d.placed = true
            }
        }
        // Let go of the keyboard the INSTANT the surface starts leaving, so it slides
        // down alongside the departure instead of blinking out once everything has gone.
        .onChange(of: exit) { _, leaving in if leaving != nil { focus = nil } }
        // A container of its own: without one (the scroll view inside the card was it) the
        // identifier lands on every field and overwrites theirs.
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("new-surface-\(kind.rawValue)")
    }

    // MARK: The glass card

    /// The NOTE has no header — its title field IS its header — and nor does the creator,
    /// whose dial names the kind.
    private var showsHeader: Bool { !creator && kind != .note }

    private var card: some View {
        VStack(spacing: 0) {
            // The NOTE surface has no header — its title field IS its header, and a
            // "New Note" bar above the thing you are already typing into would be the
            // component telling you what you can plainly see.
            if session.face == .visibility {
                visibilityHeader
            } else if showsHeader {
                header
            }
            // NOTHING SCROLLS INSIDE THE CARD (Ralph, 8 Nov, #60): the card is a tablet, at
            // its full height, and its `CardTablet` scrolls it whole.
            //
            // THE EXPLICIT STACK. `fields` is a ViewBuilder that yields SEVERAL views, so a
            // bare `fields` became a TupleView that SwiftUI laid out on top of itself: the
            // note's title rendered UNDER its body, and every newitem modal's rows sat on
            // each other instead of stacking.
            //
            // `.leading` because a label-over-well field row is left-aligned and a
            // centred stack would make the labels wander with their content's width.
            VStack(alignment: .leading, spacing: session.face == .visibility ? 0 : rowGap) {
                if session.face == .visibility { visibilityOptions } else { fields }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 20)
            // Without a header (the note, the creator) the fields take its top margin.
            .padding(.top, showsHeader ? 4 : 24)
            .padding(.bottom, 12)
        }
        .frame(maxWidth: .infinity, minHeight: minHeight, alignment: .top)
        // Report where the card actually IS. In the ground's own space, so the bloom can
        // use it directly — the ground and the card are siblings in the same ZStack, and
        // both ignore the safe area, so `.named("newSurface")` is common ground.
        .background(GeometryReader { proxy in
            Color.clear.preference(key: CardFrameKey.self,
                                   value: active ? proxy.frame(in: .named("newSurface")) : .zero)
        })
        .animation(Motion.screenIn, value: session.face)
        // The shared glass. `flash` swaps the resting edge for the outcome's colour.
        // Opaque for every kind (Ralph, 5 Nov): the note's see-through ground made it the
        // odd card out beside the others.
        .composeGlass(tint: tint, flash: flash, opaque: true)
    }

    /// The modal's title, in the SAME type as the "NOTE" / "CHAT" word a screen shows
    /// when nothing has been created yet (NotesView.emptyState: 26pt semibold, tracking 2,
    /// secondary). So a newitem modal reads as the same surface as the empty screen it
    /// opened from, rather than as a dialog dropped on top of one.
    ///
    /// The kind's glyph disc that used to sit top-left is GONE (Ralph, 18 Aug): the word
    /// says what this is, and a coloured badge beside it said it twice.
    private var header: some View {
        Text(kind.noun.uppercased())
            .font(.system(size: 26, weight: .semibold))
            .tracking(2)
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 20)
            .padding(.top, 26)
            .padding(.bottom, 18)
            .accessibilityAddTraits(.isHeader)
    }

    // MARK: The VISIBILITY face — one component, every GroupObject

    private var visibilityHeader: some View {
        Text("VISIBILITY")
            .font(.system(size: 26, weight: .semibold))
            .tracking(2)
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 20)
            .padding(.top, 26)
            .padding(.bottom, 18)
            .accessibilityAddTraits(.isHeader)
    }

    /// The five options, as one list. Common to NOTE and every GroupObject — the same
    /// component, because "who is this for" is the same question whatever you are making.
    ///
    /// The two that need a selection (People, Spaces) say so instead of pretending a tap
    /// finished the job.
    @ViewBuilder private var visibilityOptions: some View {
        ForEach(Audience.allCases) { option in
            Button {
                session.audience = option
            } label: {
                HStack(spacing: 14) {
                    // A filled ring for the choice, an empty one for the rest. No tick
                    // glyph: the circles are this app's own vocabulary for state.
                    Circle()
                        .strokeBorder(session.audience == option ? tint : Theme.hairline,
                                      lineWidth: session.audience == option ? 6 : 1)
                        .frame(width: 20, height: 20)
                    Text(option.title)
                        .font(.body)
                        .foregroundStyle(Theme.textPrimary)
                    Spacer(minLength: 0)
                    if option.needsSelection && session.audience == option {
                        Text("Choose")
                            .font(.system(.footnote, design: .monospaced))
                            .foregroundStyle(Theme.textTertiary)
                    }
                }
                .padding(.horizontal, 20)
                .frame(height: CircleButton.discSize)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("visibility-\(option.rawValue)")
            .accessibilityAddTraits(session.audience == option ? .isSelected : [])
        }
        // WHAT STANDS OUTSIDE (#76). Once it reaches past you, an outsider holds its name, a
        // description and a way in, and nothing from inside: the owner writes it here. (Its
        // picture is by reference, and comes with the blob store's client, #56.)
        if session.audience == .connections || session.audience == .network {
            outsideFields
        }
    }

    @ViewBuilder private var outsideFields: some View {
        Text("OUTSIDE")
            .font(.system(.footnote, design: .monospaced))
            .tracking(2)
            .foregroundStyle(Theme.textTertiary)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 20)
            .padding(.top, 18)
            .accessibilityAddTraits(.isHeader)
        TextField("What outsiders see", text: $session.draft.outside, axis: .vertical)
            .lineLimit(2...4)
            .font(.body)
            .padding(.horizontal, 20)
            .accessibilityIdentifier("outside-description")
        ForEach(DoorChoice.allCases) { door in
            Button {
                session.draft.door = door
            } label: {
                HStack(spacing: 14) {
                    Circle()
                        .strokeBorder(session.draft.door == door ? tint : Theme.hairline,
                                      lineWidth: session.draft.door == door ? 6 : 1)
                        .frame(width: 20, height: 20)
                    Text(door.title)
                        .font(.body)
                        .foregroundStyle(Theme.textPrimary)
                    Spacer(minLength: 0)
                }
                .padding(.horizontal, 20)
                .frame(height: CircleButton.discSize)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("door-\(door == .none ? "none" : door.rawValue)")
            .accessibilityAddTraits(session.draft.door == door ? .isSelected : [])
        }
    }

    /// The gap between rows. The NOTE flows title→body as one piece of writing, so it
    /// takes none; a newitem modal is a stack of labelled rows and needs the air.
    private var rowGap: CGFloat { kind == .note ? 0 : 18 }


    // MARK: Fields — RENDERED FROM THE TYPE'S DECLARATION

    /// The kind's own initialisation fields, in the order it asks for them.
    ///
    /// There is no `switch kind` here any more, and that is the point. The UI used to
    /// decide which fields a mint showed while the reducers separately decided which args
    /// had to be present — two statements of one fact, and they had drifted in four
    /// places: an Event minted with `startMs` 0 (an event in 1970) because the picker
    /// was never touched; a "Note" on the Place and Event surfaces that went nowhere;
    /// a Project "Objective" that went nowhere. Every one of them was a field you could
    /// type into and have silently dropped.
    ///
    /// Now the type declares (`pacific_core::mint::fields`) and this renders. A field
    /// that does not reach the wire fails `fields_carry_to_the_wire` in the core, so it
    /// cannot reach a screen.
    ///
    /// The three-field ceiling still holds — it is enforced in the declaration, where the
    /// judgement belongs, rather than by whoever last edited this view.
    @ViewBuilder private var fields: some View {
        if session.fields.isEmpty {
            noteFields          // `.note` mints no object, so it has no declaration
        } else {
            // MEDIA is the one row that pairs TWO declared fields side by side — a round
            // icon and a wide banner — so it is composed here rather than being two rows.
            // Everything else is one declared field, one row, in declaration order.
            ForEach(rows, id: \.id) { row in
                switch row.shape {
                case .single(let f):
                    // A text row carries NO label above it: the title sits CENTRED INSIDE
                    // the box and clears when you tap in (Ralph, 18 Aug). Everything else
                    // — the switches, WHERE, HERE — keeps its caption.
                    if f.input == .line || f.input == .multiline {
                        control(for: f)
                    } else if f.input == .date || f.input == .recurrence {
                        InlineFieldRow(label: f.label) { control(for: f) }
                    } else if f.input == .choice || f.input == .money {
                        // KIND and PRICE read as inline rows too — same component, so a
                        // listing's switches sit on the same line as their captions.
                        InlineFieldRow(label: f.label) { control(for: f) }
                    } else if f.input == .place {
                        // WHERE reads from the RIGHT of its selector (Ralph, 18 Aug).
                        InlineFieldRow(label: f.label, titleSide: .trailing) { control(for: f) }
                    } else {
                        labelledRow(f.label) { control(for: f) }
                    }
                case .media(let icon, let banner):
                    // MEDIA needs no title — two slots and a camera say what they are.
                    mediaRow(icon: icon, banner: banner)
                }
            }
        }
    }

    /// One rendered row. The declaration is a flat list; the only grouping the design asks
    /// for is Icon+Banner sharing the MEDIA row, so that is the only pairing here.
    private struct Row: Identifiable {
        enum Shape {
            case single(MintFieldRow)
            case media(MintFieldRow, MintFieldRow)
        }
        let id: Int
        let shape: Shape
    }

    private var rows: [Row] {
        var out: [Row] = []
        var pendingIcon: MintFieldRow?
        for (i, f) in session.fields.enumerated() {
            switch f.key {
            case .icon:
                pendingIcon = f
            case .banner:
                if let icon = pendingIcon {
                    out.append(Row(id: i, shape: .media(icon, f)))
                    pendingIcon = nil
                } else {
                    out.append(Row(id: i, shape: .single(f)))
                }
            default:
                // An icon declared without a banner after it still gets its own row —
                // never dropped just because it was not paired.
                if let icon = pendingIcon {
                    out.append(Row(id: i - 1, shape: .single(icon)))
                    pendingIcon = nil
                }
                out.append(Row(id: i, shape: .single(f)))
            }
        }
        if let icon = pendingIcon { out.append(Row(id: session.fields.count, shape: .single(icon))) }
        return out
    }

    /// One control per declared input. The kind never appears here — only the shape of
    /// the field, which is what makes this total and drift-proof.
    @ViewBuilder private func control(for field: MintFieldRow) -> some View {
        switch field.input {
        case .line:
            entryBox(field) {
                TextField("", text: binding(field.key))
                    .focused($focus, equals: .key(field.key))
                    .submitLabel(.next)
            }

        case .multiline:
            entryBox(field, minHeight: CircleButton.discSize * 1.8) {
                TextField("", text: binding(field.key), axis: .vertical)
                    .lineLimit(2...5)
                    .focused($focus, equals: .key(field.key))
            }

        case .choice:
            // The vocabulary is the REDUCER'S OWN, carried through the declaration — so
            // the picker cannot offer a value the fold will reject. What this layer may
            // do is NARROW it and RENAME it: offering fewer of the reducer's own values
            // is always safe, and the wire word is not always the word to show.
            Switch(options: offered(field), word: word, selection: binding(field.key))

        case .date:
            DatePicker("", selection: Binding(get: { session.draft.date ?? Date() },
                                              set: { session.draft.date = $0 }))
                .labelsHidden()
                .datePickerStyle(.compact)

        case .fix:
            fixReadout

        // Declared, shown, NOT yet persisted — the core says so (`Carriage::Deferred`)
        // and the control says so too. A live-looking control that drops what you give
        // it is the bug this whole declaration exists to prevent.
        case .recurrence:
            // NOT deferred, and never was. `mint.rs:311` declares Recurrence with
            // `opt(...)` — carriage `Op0` — and `set_profile_args` carries it in the
            // event's own OP 0 delta, where the core parses it (`Recurrence::parse`)
            // and rejects anything it cannot expand. Rendering it as a deferred row
            // meant the one field the core was ready for could not be set at all.
            Picker("", selection: binding(field.key)) {
                ForEach(Self.recurrences, id: \.rule) { Text($0.label).tag($0.rule) }
            }
            .labelsHidden()
            .pickerStyle(.menu)
            .tint(Theme.textPrimary)
            // ONE LINE, at full size. The row squeezes the menu to whatever the caption
            // leaves, and "One Time Thing" wrapped to two. `fixedSize` lets it take its
            // natural width instead — deliberately not `minimumScaleFactor`, which would
            // fit it by shrinking the type and make this one control read smaller than
            // every other value on the card.
            .fixedSize()
            .lineLimit(1)

        case .money, .place:
            deferredRow(field)

        // Handled by `mediaRow`; reached only if a media field is declared unpaired.
        case .icon:
            mediaSlot(circular: true, field: field)
        case .banner:
            mediaSlot(circular: false, field: field)
        }
    }

    // MARK: The entry box — the DM composer's own shape

    /// Text rows wear the CHAT composer's slot (Design/Signal.swift): corner radius EXACTLY
    /// the action-disc radius, `Theme.surface2`, a hairline. Reused rather than re-drawn, so
    /// the place you type a message and the place you name a Space are the same object.
    ///
    /// The field's TITLE lives INSIDE the box, centred, and clears the moment you tap in
    /// (Ralph, 18 Aug). So it is a placeholder rather than a caption — which is why these
    /// rows carry no label above them, and why they do not go through `FieldRow`: that
    /// wraps its content in a SECOND well at radius 12, and two nested wells fight.
    ///
    /// It returns once the box is empty and unfocused, so an emptied field never becomes
    /// an anonymous blank.
    private func entryBox<C: View>(_ field: MintFieldRow,
                                   minHeight: CGFloat = CircleButton.discSize,
                                   @ViewBuilder _ content: () -> C) -> some View {
        let slot = RoundedRectangle(cornerRadius: CircleButton.discRadius, style: .continuous)
        let showTitle = session.draft[field.key].isEmpty && focus != .key(field.key)
        return content()
            .textFieldStyle(.plain)
            .padding(.horizontal, 20)
            .padding(.vertical, 12)
            .frame(maxWidth: .infinity, minHeight: minHeight, alignment: .leading)
            .background(Theme.surface2, in: slot)
            .overlay(slot.strokeBorder(Theme.hairline, lineWidth: 1))
            .accessibilityIdentifier("new-field-\(fieldID(field.key))")
            .overlay {
                // Centred in the box, in the same tracked monospaced caption the field
                // captions used — so the title reads as a label sitting in its own well
                // rather than as text you typed. Never hit-tests: the tap must reach the
                // field under it, which is what clears the title in the first place.
                if showTitle {
                    Text(field.label.uppercased())
                        .font(.system(.caption, design: .monospaced))
                        .tracking(1.5)
                        .foregroundStyle(Theme.textTertiary)
                        .allowsHitTesting(false)
                        .transition(.opacity)
                }
            }
            .animation(Motion.screenIn, value: showTitle)

    }

    /// What RECURRENCE offers, and the RRULE each answer IS. "" is not a missing value —
    /// it is the core's own way of saying the event happens once (`event.rs`: "an RFC 5545
    /// RRULE when this event REPEATS, "" when it happens once"), and `non_empty` drops it
    /// from the delta rather than sending a rule that would fail to parse.
    ///
    /// Both rules round-trip through `Recurrence::to_rrule` unchanged, so what is stored is
    /// exactly what is offered here.
    static let recurrences: [(label: String, rule: String)] = [
        ("One Time Thing", ""),
        ("Weekly",         "FREQ=WEEKLY"),
        ("Monthly",        "FREQ=MONTHLY"),
    ]


    /// A stable id per declared field, for the tests that drive this surface.
    private func fieldID(_ key: MintKey) -> String {
        switch key {
        case .name: return "name";             case .descriptor: return "describe"
        case .shape: return "shape";           case .category: return "category"
        case .start: return "when";            case .fix: return "here"
        case .icon: return "icon";             case .banner: return "banner"
        case .price: return "price";           case .recurrence: return "recurrence"
        case .stance: return "stance"
        case .place: return "where"
        case .form: return "form";             case .link: return "link"
        }
    }

    /// A row that keeps its caption above the control — the switches, WHERE, HERE. Same
    /// caption type as `FieldRow`, but WITHOUT that view's own filled well: a segmented
    /// picker or a date wheel inside a well reads as a control trapped in a box.
    private func labelledRow<C: View>(_ label: String,
                                      @ViewBuilder _ content: () -> C) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(label.uppercased())
                .font(.system(.caption2, design: .monospaced))
                .tracking(1.5)
                .foregroundStyle(Theme.textTertiary)
            // The caption stays OUTSIDE — the canvas belongs to the control, exactly as
            // it does on the inline rows.
            content().composeCard()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    // MARK: MEDIA — a round face, a wide wall, and the camera

    /// Two slots side by side — a round icon and a wide banner. Each carries its OWN
    /// tappable camera (Ralph, 18 Aug): the standalone camera circle is gone, because a
    /// third control could not say WHICH slot it was going to fill.
    ///
    /// The icon is a circle because that is the shape it will be worn in; the banner is a
    /// rectangle for the same reason.
    private func mediaRow(icon: MintFieldRow, banner: MintFieldRow) -> some View {
        HStack(spacing: 12) {
            mediaSlot(circular: true, field: icon)
                .frame(width: CircleButton.discSize * 1.5, height: CircleButton.discSize * 1.5)
            mediaSlot(circular: false, field: banner)
                .frame(maxWidth: .infinity)
                .frame(height: CircleButton.discSize * 1.5)
        }
    }

    /// An empty media slot with its camera in it. The whole slot is the button — a small
    /// glyph inside a large empty shape reads as decoration, and the target should be the
    /// thing you are filling.
    private func mediaSlot(circular: Bool, field: MintFieldRow) -> some View {
        Button(action: { camera = field.key }) {
            ZStack {
                if circular {
                    Circle().fill(Theme.surface2)
                        .overlay(Circle().strokeBorder(Theme.hairline, lineWidth: 1))
                } else {
                    let r = RoundedRectangle(cornerRadius: 14, style: .continuous)
                    r.fill(Theme.surface2).overlay(r.strokeBorder(Theme.hairline, lineWidth: 1))
                }
                Image(systemName: "camera.fill")
                    .font(.system(size: 17, weight: .semibold))
                    .foregroundStyle(Theme.textTertiary)
            }
        }
        .buttonStyle(.plain)
        .accessibilityLabel(field.label)
        .accessibilityIdentifier("new-media-\(field.key == .icon ? "icon" : "banner")")
    }

    /// A row the type declares and does not yet persist. It reads as inert on purpose.
    private func deferredRow(_ field: MintFieldRow) -> some View {
        // No slot of its own any more: the ROW is the card now, and a dimmed box drawn
        // inside a card is a box in a box. The em-dash and the tertiary colour are what
        // say inert — which is what they always said; the slot underneath them was only
        // ever standing in for a row canvas that did not exist yet.
        Text("—")
            .font(.system(.footnote, design: .monospaced))
            .foregroundStyle(Theme.textTertiary)
            .frame(maxWidth: .infinity, alignment: .leading)
            .accessibilityLabel("\(field.label), not yet available")
    }

    /// The choices this field actually offers, narrowed from the declaration.
    ///
    /// KIND decides STANCE: a THING is OFFERED or WANTED, a SKILL is HAD or NEEDED. The
    /// declaration cannot say that — a `Choice` vocabulary is static per kind — so the
    /// dependency lives here. It only ever REMOVES values the reducer already accepts;
    /// nothing is invented, and the order is stated rather than inherited so THING is
    /// first whatever the declaration happens to list.
    private func offered(_ field: MintFieldRow) -> [String] {
        switch field.key {
        case .category: return ["artifact", "skill"]
        case .stance:   return session.draft.category == "skill"
                            ? ["has", "wants"]        // HAVE · NEED
                            : ["offers", "wants"]     // OFFER · WANT
        default:        return field.choices
        }
    }

    /// The word for a wire value. `wants` is the one that reads two ways — WANT of a
    /// thing, NEED of a skill — which is why this takes the draft into account rather
    /// than being a flat table.
    private func word(_ value: String) -> String {
        switch value {
        case "artifact": return "Thing"
        case "offers":   return "Offer"
        case "has":      return "Have"
        case "wants":    return session.draft.category == "skill" ? "Need" : "Want"
        default:         return value.capitalized
        }
    }

    private func binding(_ key: MintKey) -> Binding<String> {
        Binding(get: { session.draft[key] }, set: { session.draft[key] = $0 })
    }

    /// The live fix, as a READOUT — never a picker. A Place is evidence of having been
    /// somewhere, so the coordinate is the one the device reports, not one you scrolled
    /// a map to.
    ///
    /// Each state says what is actually true. "Unplaced" is a real outcome, not an
    /// error: the Place is still minted, listed but not pinned, and a later
    /// `setLocation` places it. The `±N m` carries the accuracy because that is what
    /// sets the geohash precision — a cell must never undercut its own error bar.
    private var fixReadout: some View {
        let (text, live): (String, Bool) = {
            switch locator.state {
            case .idle:              return ("Tap to locate", false)
            case .requesting:        return ("Locating…", false)
            case .searching:         return ("Searching…", false)
            case .denied:            return ("Unplaced — location off", false)
            case .located(let fix):  return ("\(fix.geohash) · ±\(Int(fix.accuracy.rounded())) m", true)
            }
        }()
        return Button(action: locate) {
            Text(text)
                .font(.system(.footnote, design: .monospaced))
                .foregroundStyle(live ? Theme.textPrimary : Theme.textTertiary)
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("new-fix")
    }

    /// Start the Place's fix, once. Every kind has a locator; only a Place uses it.
    private func locate() {
        guard kind == .place, locator.state == .idle else { return }
        locator.start()
    }

    /// The note keeps its bare title + body editor — no labelled wells. It is the one
    /// surface where the content IS the form.
    ///
    /// The title used to render UNDER the body. Not a z-order problem and not this view's
    /// fault: `card` handed these straight to a `ScrollView`, whose content is ONE view, so
    /// three siblings became a `TupleView` laid out on top of itself. The fix is the
    /// explicit `VStack` in `card` — which is the same fix as the newitem modals' broken
    /// stacking, because it was the same bug.
    @ViewBuilder private var noteFields: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            TextField("Title", text: $session.draft.name)
                .font(.title2.weight(.bold))
                .foregroundStyle(Theme.textPrimary)
                .focused($focus, equals: .key(.name))
                .submitLabel(.next)
                .onSubmit { focus = .body }

            // TODO(reminder): a BELL here turns the note into a REMINDER — the note gains a
            // when, and the when is what wakes you. Sits top-right of the title row so it
            // reads as a property of the note rather than a fourth action (the three
            // circles are the actions, and this is not one of them).
            //
            // Needs, in order: a `Reminder` slot on the draft (an instant + a fired flag);
            // NotesStore persistence for it; and the wake path — `WakeTests`/`WakeStore`
            // already own scheduling, so this is a caller, not a new mechanism. Whether a
            // reminder is still a NOTE or becomes its own primitive is Ralph's call, and
            // the bell should not ship until that is answered: minting the wrong species
            // is the exact bug the `New` component was built to make unrepresentable.
            Button(action: {}) {
                Image(systemName: "bell")
                    .font(.system(size: 17, weight: .semibold))
                    .foregroundStyle(Theme.textTertiary)
            }
            .disabled(true)
            .accessibilityLabel("Remind me")
            .accessibilityIdentifier("note-reminder-bell")
        }

        TextEditor(text: $session.draft.body)
            .font(.body)
            .foregroundStyle(Theme.textPrimary)
            .scrollContentBackground(.hidden)    // let the glass show behind the editor
            .focused($focus, equals: .body)
            .frame(maxWidth: .infinity, minHeight: 260)
            .padding(.horizontal, -4)   // the editor's own text inset, so its words line up with the title's
            .padding(.top, 14)
            .overlay(alignment: .topLeading) {
                // TextEditor has no native placeholder on iOS 17 — supply our own.
                if session.draft.body.isEmpty {
                    Text("Write a note…")
                        .font(.body)
                        .foregroundStyle(Theme.textTertiary)
                        .padding(.horizontal, 1)
                        .padding(.top, 22)
                        .allowsHitTesting(false)
                }
            }
    }
}

// MARK: - Shell hooks
//
// The two entry points the shell calls. They live here rather than in ShellState so the
// component owns its own lifecycle and the shell keeps only the one optional.

extension NewSession: ShellSurface {
    /// Run BLUE and report the outcome. `nil` = the commit failed; the surface stays up.
    @MainActor
    func performCommit() async -> NewExit? { await runCommit() }

    @MainActor
    func beginExit(_ e: NewExit) { setExit(e) }

    // The rest of `ShellSurface` is what this session already was — named, so a
    // second kind of surface can sit in the shell's slot beside it.

    /// GOLD: the audience dial, on every kind (Ralph, 18 Aug).
    @MainActor
    func gold() { toggleVisibility() }

    /// "Discard this note?" · "Discard this space?"
    nonisolated var discardNoun: String { kind.noun.lowercased() }

    nonisolated var changes: ObservableObjectPublisher { objectWillChange }
}

// MARK: - The creator

/// RED's universal creator: one session per kind, the kind in view, and the lock.
///
/// Every kind's session exists from the start, each with its own opening draft, so the
/// cards between two kinds are real cards as they slide past. Nothing carries between
/// them: the first edit to the card in view CHOOSES the kind (Ralph, 5 Nov), and from
/// then the carousel is gone and the cards stop moving.
@MainActor
final class NewCreator: ObservableObject {
    let kinds: [NewKind]
    let sessions: [NewKind: NewSession]
    /// The kind in view. Set when the pager comes to rest.
    @Published var active: NewKind
    /// Latched by the first edit to the card in view.
    @Published private(set) var locked = false
    private var sinks: [AnyCancellable] = []

    init(kinds: [NewKind], make: (NewKind) -> NewSession) {
        self.kinds = kinds
        sessions = Dictionary(uniqueKeysWithValues: kinds.map { ($0, make($0)) })
        active = kinds.first ?? .note
        for (kind, s) in sessions {
            // `$draft` publishes the NEW value (the audience with it), before it is stored, so
            // the edit is judged on what was sent rather than on `s.edited`.
            sinks.append(s.$draft.sink { [weak self, weak s] _ in
                DispatchQueue.main.async {
                    guard let self, let s, !self.locked, kind == self.active, s.edited else { return }
                    NSLog("[creator] locked on %@ by %@", kind.rawValue, s.edits.joined(separator: ","))
                    withAnimation(Motion.screenIn) { self.locked = true }
                }
            })
        }
    }

    var session: NewSession { sessions[active]! }
}

/// The creator's cards, one per kind, under the dial that picks between them — two of
/// SwiftUI's own scroll views, coupled.
///
/// The cards page (`.paging`); the dial snaps item by item (`.viewAligned`). Whichever
/// one the person is touching leads with Apple's physics, and the other follows it point
/// for point, its own snapping off (`Unsnapped`), so a drag on either turns both. Neither
/// wraps on its own, so both hold the kinds in LAPS and, at rest, jump unseen to the same
/// kind in the middle lap: past Post comes Note (Ralph, 5 Nov).
private struct CreatorPages: View {
    @ObservedObject var creator: NewCreator
    @ObservedObject private var inView: NewSession
    @State private var cards: ScrollPosition
    @State private var dial: ScrollPosition
    /// The cards' position, in cards, as last laid out.
    @State private var at: CGFloat
    @State private var cardWidth: CGFloat = 1
    /// Which scroll view the person is touching; the other follows it.
    @State private var lead: Lead = .cards
    private enum Lead { case cards, dial }
    /// The dial has once stood where the cards are. Until then it neither leads nor moves
    /// them: its first report, before its position applies, reads the leading margin (two
    /// items), and the touch that opened the creator can still be on it — on a cold first
    /// launch that pair carried the cards two kinds along, Event to Place.
    @State private var dialAligned = false
    /// The tablet in view has scrolled up: the dial makes way (Ralph, 8 Nov, #60).
    @State private var dialHidden = false
    /// Where the tablet turned last: the furthest it went in the direction it is going.
    @State private var turned: CGFloat = 0
    /// Travel past the turn that hides or brings back the dial.
    private static let dialSlack: CGFloat = 8

    private static let laps = 5

    init(creator: NewCreator) {
        self.creator = creator
        _inView = ObservedObject(wrappedValue: creator.session)
        let start = KindRing.home(creator.kinds.count, laps: Self.laps)
            + (creator.kinds.firstIndex(of: creator.active) ?? 0)
        _cards = State(initialValue: ScrollPosition(id: start))
        _dial = State(initialValue: ScrollPosition(id: start))
        _at = State(initialValue: CGFloat(start))
    }

    private var ids: [Int] { Array(0..<(creator.kinds.count * Self.laps)) }

    /// The dial and the paging go once the kind is chosen, and on VISIBILITY, which
    /// is the object's audience rather than what the object is.
    private var choosing: Bool { !creator.locked && inView.face == .fields }

    var body: some View {
        // THE DIAL FLOATS OVER THE TABLETS, it takes no space from them: going and coming
        // it moves, and the tablet under it keeps its frame (Ralph, 8 Nov — a dial that gave
        // its space to the page as it went made the tablet's height jitter under a slow drag).
        // The cards rest below it, on the tablet's top margin.
        ZStack(alignment: .top) {
            ScrollView(.horizontal) {
                LazyHStack(spacing: 0) {
                    ForEach(ids, id: \.self) { i in
                        let k = KindRing.kind(at: i, in: creator.kinds)
                        CardTablet(id: "new-tablet-\(k.rawValue)", top: KindDial.slot,
                                   scrolled: { y in if k == creator.active { tabletAt(y) } }) { height in
                            NewCard(session: creator.sessions[k]!, creator: true,
                                    active: k == creator.active, minHeight: height)
                                .padding(.horizontal, 16)
                        }
                        .containerRelativeFrame(.horizontal)
                    }
                }
                .scrollTargetLayout()
            }
            .scrollTargetBehavior(lead == .dial ? AnyScrollTargetBehavior(Unsnapped())
                                                : AnyScrollTargetBehavior(.paging))
            .scrollIndicators(.hidden)
            .scrollPosition($cards)
            .scrollDisabled(!choosing)
            .onScrollGeometryChange(for: CGSize.self) { g in
                CGSize(width: g.contentOffset.x, height: g.containerSize.width)
            } action: { _, g in
                cardWidth = max(g.height, 1)
                at = g.width / cardWidth
                if lead == .cards { dial.scrollTo(x: at * KindDial.item) }
            }
            .onScrollPhaseChange { _, phase in
                if phase == .interacting { lead = .cards }
                if phase == .idle, lead != .dial { rest() }
            }

            // Above the card it picks (Ralph, 8 Nov).
            if choosing && !dialHidden {
                KindDial(kinds: creator.kinds, ids: ids, position: $dial,
                         selected: Int(at.rounded()), following: lead == .cards,
                         touched: { if dialAligned { lead = .dial } },
                         moved: { f in
                             if !dialAligned {
                                 // Not yet where the cards are: put it there. A new dial
                                 // starts at its content's origin, two items in, and the
                                 // cards at rest send it nowhere.
                                 dialAligned = abs(f - at) < 0.5
                                 if !dialAligned { dial.scrollTo(x: at * KindDial.item) }
                             } else if lead == .dial {
                                 cards.scrollTo(x: f * cardWidth)
                             }
                         },
                         rested: { if lead == .dial { rest() } },
                         pick: pick)
                    .padding(.top, CardTablet<EmptyView>.lead)
                    .transition(.move(edge: .top).combined(with: .opacity))
                    // A dial that comes back (after VISIBILITY, or scrolled away) aligns
                    // again before it leads.
                    .onAppear { dial.scrollTo(x: at * KindDial.item) }
                    .onDisappear { dialAligned = false }
            }
        }
        // One tick as each kind takes the centre, as a picker does. The kind, not the
        // index, so the unseen lap jump is silent.
        .sensoryFeedback(.selection, trigger: KindRing.kind(at: Int(at.rounded()), in: creator.kinds))
        // Another kind in view: its tablet is at its own top, and the dial is back.
        .onChange(of: creator.active) { _, _ in setDial(hidden: false, at: 0) }
    }

    /// The tablet in view is at `y` from its top. Up past the slack hides the dial, down
    /// past it (or the top) brings it back, as Safari's bar does: a drag or a flick alike,
    /// since the dial floats and its going moves nothing under it.
    private func tabletAt(_ y: CGFloat) {
        if y <= 0 { return setDial(hidden: false, at: 0) }
        if dialHidden {
            if y > turned { turned = y } else if turned - y > Self.dialSlack { setDial(hidden: false, at: y) }
        } else {
            if y < turned { turned = y } else if y - turned > Self.dialSlack { setDial(hidden: true, at: y) }
        }
    }

    private func setDial(hidden: Bool, at y: CGFloat) {
        turned = y
        guard hidden != dialHidden else { return }
        withAnimation(Motion.screenIn) { dialHidden = hidden }
    }

    /// A tap on the dial: the cards scroll to it, so the ones between cross on the way, and
    /// the dial follows them as it does a swipe.
    private func pick(_ i: Int) {
        lead = .cards
        withAnimation(Motion.screenIn) { cards.scrollTo(id: i) }
    }

    /// At rest: the kind in view is the card that is showing — not one that crossed on the
    /// way, so Place is never asked for a location as it passes. Off the middle lap, both
    /// jump unseen to the same kind in it.
    private func rest() {
        let n = creator.kinds.count
        let i = Int(at.rounded())
        let k = KindRing.kind(at: i, in: creator.kinds)
        if k != creator.active {
            NSLog("[creator] in view %@ (card %d)", k.rawValue, i)
            creator.active = k
        }
        let home = KindRing.home(n, laps: Self.laps)
        if !(home..<(home + n)).contains(i) {
            let j = home + ((i % n) + n) % n
            var t = Transaction()
            t.disablesAnimations = true
            withTransaction(t) {
                cards.scrollTo(id: j)
                dial.scrollTo(id: j)
            }
        }
        lead = .cards
    }
}

/// A card as a TABLET (Ralph, 8 Nov, #60): nothing scrolls inside it; the card itself
/// scrolls, whole and at its full height. It runs on under the status bar, the disc row
/// and the keyboard, blurred behind them all (the keyboard is glass; the bars get a blur
/// of their own), and its ends scroll clear of them. `card` is given the height a
/// short card fills, the page between its top margin and the row, measured with the
/// keyboard down: LOCKED, so neither the keyboard nor the dial resizes a card under the
/// finger (Ralph, 8 Nov).
///
/// THE BARS ARE TRACKED THROUGH THE SAFE AREA, not the keyboard's notifications. Readers:
/// the page as it shows, which stops at the status bar and above the keyboard (or the home
/// indicator), and the page the tablet runs under, which does not. Their differences are
/// how far the bars reach, and SwiftUI moves them in the keyboard's own transaction, so the
/// margins and the blurs ride with it exactly. (A reader that ignores the safe area reads
/// an inset of nothing.)
private struct CardTablet<Card: View>: View {
    let id: String
    /// Where the card rests below the status bar's space: below the dial, in the creator.
    var top: CGFloat = gap
    /// The tablet was scrolled: its offset from the top, reported for the person's own
    /// movement only (`push`), and always at the top.
    var scrolled: (CGFloat) -> Void = { _ in }
    @ViewBuilder let card: (CGFloat) -> Card

    /// What is moving the tablet. A finger, either way; a fling, the way it was thrown;
    /// nothing else — the settle at either end, the bounce back from it, is not the person
    /// scrolling, and read as a scroll back it brought the dial straight back.
    private enum Push { case finger, up, down, none }
    @State private var push = Push.none

    /// Below the status bar, before anything rests: the dial, or a card with no dial.
    static var lead: CGFloat { 8 }
    /// Above the card: the same gap as the dial keeps from the notch (Ralph, 8 Nov).
    static var gap: CGFloat { 20 }
    /// Under the card: the disc row, the surface's buttons.
    private static var row: CGFloat { ShellMetrics.bottomBarHeight + 6 }

    var body: some View {
        GeometryReader { shown in
            GeometryReader { page in
                GeometryReader { whole in
                    let ceiling = max(shown.frame(in: .global).minY - whole.frame(in: .global).minY, 0)
                    let bottom = whole.frame(in: .global).maxY - shown.frame(in: .global).maxY
                    tablet(page: page.size.height, ceiling: ceiling, floor: max(bottom, 0))
                }
                .ignoresSafeArea(.container, edges: [.top, .bottom])
            }
            .ignoresSafeArea(.keyboard, edges: .bottom)
        }
    }

    /// `page`: the tablet's height with the keyboard down, which the card's height is held
    /// to. `ceiling`: the status bar. `floor`: what the row stands on, the keyboard when it
    /// is up, else the home indicator.
    private func tablet(page: CGFloat, ceiling: CGFloat, floor: CGFloat) -> some View {
        ScrollView(.vertical) {
            card(max(page - Self.lead - top - Self.row, 0))
        }
        .scrollIndicators(.hidden)
        .contentMargins(.top, ceiling + Self.lead + top, for: .scrollContent)
        .contentMargins(.bottom, Self.row + floor, for: .scrollContent)
        .onScrollGeometryChange(for: CGFloat.self) { g in
            let travel = max(g.contentSize.height + g.contentInsets.top + g.contentInsets.bottom - g.containerSize.height, 0)
            return min(max(g.contentOffset.y + g.contentInsets.top, 0), travel)
        } action: { old, y in
            let d = y - old
            if y <= 0 || push == .finger || (push == .up && d > 0) || (push == .down && d < 0) {
                scrolled(y)
            }
        }
        .onScrollPhaseChange { _, phase, context in
            switch phase {
            case .interacting: push = .finger
            case .decelerating: push = (context.velocity?.dy ?? 0) >= 0 ? .up : .down
            default: push = .none
            }
        }
        .overlay(alignment: .top) {
            // The blur behind the status bar, as behind the row: the card passes under it.
            Rectangle().fill(.ultraThinMaterial)
                .mask(LinearGradient(stops: [.init(color: .black, location: 0.65),
                                             .init(color: .clear, location: 1)],
                                     startPoint: .top, endPoint: .bottom))
                .frame(height: ceiling + Self.lead)
                .allowsHitTesting(false)
        }
        .overlay(alignment: .bottom) {
            // The blur behind the row, soft at its top edge: the card passes under the
            // circles seen through frosted glass, not through a hole.
            Rectangle().fill(.ultraThinMaterial)
                .mask(LinearGradient(stops: [.init(color: .clear, location: 0),
                                             .init(color: .black, location: 0.35)],
                                     startPoint: .top, endPoint: .bottom))
                .frame(height: Self.row + floor)
                .allowsHitTesting(false)
        }
        .accessibilityIdentifier(id)
    }
}

/// The lap arithmetic the cards and the dial share.
private enum KindRing {
    /// Index `i` in the laps → its kind.
    static func kind(at i: Int, in kinds: [NewKind]) -> NewKind {
        let n = kinds.count
        return kinds[((i % n) + n) % n]
    }

    /// The first index of the middle lap, where both scroll views rest.
    static func home(_ n: Int, laps: Int) -> Int { n * (laps / 2) }
}

/// No snapping: a scroll view's behaviour while it follows the other. SwiftUI applies a
/// target behaviour to programmatic scrolls too, so a paging follower was rounded back to
/// its page on every frame and only caught up when the leader stopped.
private struct Unsnapped: ScrollTargetBehavior {
    func updateTarget(_ target: inout ScrollTarget, context: TargetContext) {}
}

/// The kind selector: every kind's glyph over its noun in one Liquid Glass capsule, the
/// kind in view centred between its two neighbours. The ORDER is a circle — past Post
/// comes Note — but the icons slide flat (Ralph, 5 Nov). A view-aligned scroll view, so a
/// tap is a button and a drag is the scroll view's own.
private struct KindDial: View {
    let kinds: [NewKind]
    let ids: [Int]
    @Binding var position: ScrollPosition
    /// The index the cards are showing.
    let selected: Int
    /// The cards lead; the dial tracks them unsnapped.
    let following: Bool
    /// The person has started touching the dial.
    let touched: () -> Void
    /// The dial's position in items, every layout. `scrollTo(x:)` and `contentOffset` do
    /// not share an origin: the offset reads the leading margin short, so the position is
    /// offset plus margin, while `scrollTo(x: i · item)` centres item `i`.
    let moved: (CGFloat) -> Void
    /// The dial came to rest.
    let rested: () -> Void
    let pick: (Int) -> Void

    static let item: CGFloat = 68
    private static let height: CGFloat = 62
    /// What the cards rest below: the dial, and the gap it keeps from them, the same gap it
    /// keeps from the notch.
    static var slot: CGFloat { height + CardTablet<EmptyView>.gap }
    /// The kind in view, its neighbours, and one more each side, fading — so it reads as
    /// a set that goes on (Ralph, 5 Nov). The scroll view clips the rest.
    private static let visible: CGFloat = 5

    var body: some View {
        ZStack {
            // The selection: a tint INSIDE the glass. A second glass shape here is what
            // turned the whole dial flat — glass cannot refract glass.
            Capsule()
                .fill(Brand.accent.opacity(0.16))
                .frame(width: Self.item, height: Self.height - 12)
            ScrollView(.horizontal) {
                LazyHStack(spacing: 0) {
                    ForEach(ids, id: \.self) { i in
                        let k = KindRing.kind(at: i, in: kinds)
                        Button { pick(i) } label: {
                            VStack(spacing: 3) {
                                Image(systemName: k.glyph).font(.system(size: 19, weight: .semibold))
                                Text(k.noun).font(.system(size: 11, weight: .medium))
                            }
                            .foregroundStyle(i == selected ? Brand.accent : Theme.textSecondary)
                            .frame(width: Self.item, height: Self.height)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .accessibilityIdentifier("new-kind-\(k.rawValue)")
                        .accessibilityAddTraits(i == selected ? .isSelected : [])
                    }
                }
                .scrollTargetLayout()
            }
            .scrollTargetBehavior(following ? AnyScrollTargetBehavior(Unsnapped())
                                            : AnyScrollTargetBehavior(.viewAligned))
            .scrollIndicators(.hidden)
            .contentMargins(.horizontal, Self.item * (Self.visible - 1) / 2, for: .scrollContent)
            .scrollPosition($position, anchor: .center)
            .mask {
                let edge = 1 / Self.visible
                LinearGradient(stops: [.init(color: .clear, location: 0),
                                       .init(color: .black, location: edge),
                                       .init(color: .black, location: 1 - edge),
                                       .init(color: .clear, location: 1)],
                               startPoint: .leading, endPoint: .trailing)
            }
            .onScrollGeometryChange(for: CGFloat.self) { g in
                (g.contentOffset.x + g.contentInsets.leading) / Self.item
            } action: { _, f in moved(f) }
            .onScrollPhaseChange { _, phase in
                if phase == .interacting { touched() }
                if phase == .idle { rested() }
            }
            .frame(width: Self.item * Self.visible)
        }
        .frame(height: Self.height)
        .padding(.horizontal, 10)
        .dialGlass()
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("new-kinds")
    }
}

private extension View {
    /// The system's Liquid Glass where there is one (iOS 26), ONE shape and unclipped, so
    /// its rim and highlights survive; a material below it.
    @ViewBuilder func dialGlass() -> some View {
        if #available(iOS 26.0, *) {
            glassEffect(.regular.interactive(), in: Capsule())
        } else {
            background(.ultraThinMaterial, in: Capsule())
        }
    }
}

/// The card's frame, reported up to the surface so the ground's bloom can anchor to it.
///
/// A preference rather than a binding because the card is BELOW the ground in the ZStack
/// and must not have to know what reads it — and because the value changes on every
/// layout, which is exactly what a preference is for.
private struct CardFrameKey: PreferenceKey {
    static var defaultValue: CGRect = .zero
    static func reduce(value: inout CGRect, nextValue: () -> CGRect) {
        let next = nextValue()
        if !next.isEmpty { value = next }
    }
}
