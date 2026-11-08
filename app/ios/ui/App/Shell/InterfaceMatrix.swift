//  InterfaceMatrix.swift
//  The shell's disc row as ONE explicit table — the interface matrix. The three discs
//  never come and go; they change IDENTITY with the interface context, and every
//  identity they can wear is declared here, in one place, instead of being scattered
//  through ternaries at each call site:
//
//                    RED (note)              GOLD (chat)               BLUE (life)
//   rest(.notes)     Note ✎ · active         Chat → chat               Life → life
//   rest(.chat)      Note ✎                  Chat · active             Life → life
//   rest(.life)      Note ✎                  Chat → chat               Life · active · HOME
//   composing        ✕ cancel                ↑ share (own gate) or —   → commit (own gate)
//   settings         ✕ close · active        (parked off-screen)       (parked off-screen)
//
//  `composing` is no longer the NOTE's row: it is the row for ANY `New` surface
//  (Design/New.swift), and the cell it carries supplies the labels and the two gates.
//  A kind with no hand-off leaves GOLD glyphless and disabled rather than removing it —
//  the discs are persistent identities that only ever MOVE, which is what makes them
//  slide. See `NewCell.hasShare`.
//
//  The blue disc's SECOND tap — rest(.life) — is the HOME gesture (10 Aug order):
//  LIFE minimises whatever jewel owns the ground and returns to the Feed. (The
//  axial market flip this tap used to trigger was removed 10 Aug.)
//
//  The matrix owns WHAT each disc is (label · glyph · state · intent); the shell keeps
//  WHERE it is and HOW it moves (offsets, springs, bloom) — choreography is not identity.
//  A cell's action is a `DiscIntent`, not a closure: the matrix stays a value table and
//  `RootShell.perform(_:)` is the one router that gives intents effect.
//
//  `DiscSlot` is also the home of the channel colours. The slot colour IS the channel
//  identity — the disc at rest, the compose glyph's ground, and the interior a channel's
//  screen dresses itself in (the EVENTS page reads `DiscSlot.life.blockColor` as its
//  accent), so a channel can never drift off its disc.

import SwiftUI

/// The three disc positions, left to right. The raw value is the shell's bloom id.
enum DiscSlot: Int, CaseIterable {
    case note = 0, chat = 1, life = 2

    /// The channel's block colour — constant across every CONTEXT, which is what
    /// lets a disc SLIDE between identities rather than cross-fading — but keyed
    /// by the THEME: the skin's berry/gold/blue on Kenjin's bedrock, CMY inks on
    /// Pacific's paper (note→cyan, chat→magenta, life→yellow), the same duality
    /// the one-pager's inverted mode wears.
    var blockColor: Color {
        switch self {
        case .note: return Theme.dyn(light: 0x00B7D6, dark: 0xD8434A)   // berry → cyan
        case .chat: return Theme.dyn(light: 0xE5178F, dark: 0xF2C744)   // gold  → magenta
        case .life: return Theme.dyn(light: 0xEFC400, dark: 0x546CAC)   // blue  → yellow
        }
    }
}

/// Everything a disc can DO, named. The matrix assigns these; only the shell runs them.
enum DiscIntent {
    case show(ShellScreen)   // swap the shown screen (a rest-context tap)
    case beginNew(NewKind)   // NOTE at rest — raise a `New` surface (.note from here)
    case newCancel           // the surface's ✕
    case newShare            // the surface's ↑
    case newCommit           // the surface's →
    case dismissSettings     // NOTE while Settings is up
    case openNotesPage       // NOTE held at rest — the Notes PAGE (a destination)
    case advanceChatScope    // GOLD while CHAT is already up — CHATS ⇄ SPACES
}

/// CHAT's scope, and the whole of it: two states, no third. The four-chip row
/// (All · Chats · Groups · Channels) was REMOVED — "All" was a non-answer that
/// showed everything and taught nothing, and Groups/Channels split one idea in
/// two. A hosted room and a plain group chat are both a SPACE; the only real
/// question is whether you are looking at people or at rooms.
///
/// The GOLD disc is now the only control. Arriving at CHAT lands on CHATS; the
/// next tap advances to SPACES, and the one after that comes back — so the disc
/// is a two-stop rotary, not a toggle you have to find.
enum ChatScope: String, CaseIterable {
    case chats = "Chats", spaces = "Rooms"

    var next: ChatScope { self == .chats ? .spaces : .chats }

    /// What the SURFACE calls itself — its own name, singular, which is what an empty
    /// one wears. The raw value is the DISC's label ("where the next tap goes"), and the
    /// two are not the same string: the disc says Rooms, the empty screen says ROOMS,
    /// and the chats side is CHAT rather than CHATS because that is the tab's name.
    var word: String { self == .chats ? "CHAT" : "ROOMS" }

    /// CHAT is where you TALK TO PEOPLE — a DM and a group chat are both that, and the
    /// group chat belongs there (Ralph, 18 Aug). ROOMS is not a second inbox: it is a way
    /// to reach the rooms hosted by your Spaces without going through each Space's page.
    ///
    /// So the line is HOSTING, not directness. It used to be `isDirect`, which put every
    /// group chat in the other lane purely for having more than two people in it.
    func admits(hostedRoom: Bool) -> Bool { self == .chats ? !hostedRoom : hostedRoom }
}

/// One cell of the matrix: a disc's identity in one context.
struct DiscSpec {
    let label: String            // accessibility name (spoken, not shown)
    let glyph: String            // SF Symbol — the white compose glyph when `showsGlyph`
    var showsGlyph = false
    var isActive = false
    var enabled = true
    let tap: DiscIntent
    var hold: DiscIntent? = nil
    var holdLabel = "More"
}

/// The persistent screens the disc row swaps between (bottom-row order).
enum ShellScreen { case notes, chat, life }

enum InterfaceMatrix {
    /// The contexts the row can be in. A context is derived, never stored — the shell
    /// computes it from its own state each render, so the row can never disagree with
    /// the screen it chromes. (Inside an immersive conversation the row is faded out
    /// entirely, so that state needs no row here.)
    enum Context: Equatable {
        /// Rest carries the CHAT scope as well as the screen, because the GOLD
        /// disc's identity genuinely depends on it: on CHAT its tap advances the
        /// scope rather than navigating, and its spoken label has to say so.
        case rest(ShellScreen, ChatScope)
        /// A `New` surface is up and the three discs ARE its buttons. The cell carries
        /// the labels and the two gates, because `hasContent` was doing three jobs at
        /// once — gate GOLD, gate BLUE, and stand in for the labels — which is fine for
        /// one surface (the note) and wrong for seven.
        case composing(NewCell)
        case settings
    }

    /// THE matrix: context × slot → identity. Every cell in the table above, verbatim.
    static func spec(_ slot: DiscSlot, in context: Context) -> DiscSpec {
        switch (context, slot) {

        // Rest — NOTE · CHAT · LIFE. Each disc lights up when its screen is showing;
        // NOTE's tap is an ACTION (compose), its hold is the destination.
        case (.rest(let screen, _), .note):
            return DiscSpec(label: "Note", glyph: "square.and.pencil",
                            isActive: screen == .notes,
                            tap: .beginNew(.note),
                            hold: .openNotesPage, holdLabel: "Your notes")
        // GOLD. Off CHAT it is a destination; on CHAT it is the scope control,
        // and its label names WHERE THE NEXT TAP GOES, not where you are — the
        // same contract NOTE's hold has.
        case (.rest(let screen, let scope), .chat):
            let onChat = screen == .chat
            return DiscSpec(label: onChat ? scope.next.rawValue : "Chat",
                            glyph: "bubble.left.and.bubble.right",
                            isActive: onChat,
                            tap: onChat ? .advanceChatScope : .show(.chat))
        case (.rest(let screen, _), .life):
            return DiscSpec(label: "Life", glyph: "tree",
                            isActive: screen == .life,
                            tap: .show(.life))

        // Composing — the discs ARE the surface's buttons: ✕ · Share · Commit. Share and
        // Commit light up (and enable) on their OWN gates, which genuinely come apart:
        // New Space's gold opens the member list, answerable before the Space has a name.
        case (.composing(let c), .note):
            return DiscSpec(label: c.cancelLabel, glyph: "xmark", showsGlyph: true,
                            isActive: true,
                            tap: .newCancel)

        // GOLD with nothing to hand off does NOT disappear — the three discs never come
        // and go, they only move, and that persistence is what makes them slide rather
        // than cross-fade. It keeps its rest identity, glyphless and disabled, which the
        // row already renders at 0.3 opacity: a disc that is not part of this action,
        // which is the truth. (`ComposeDiscs` may omit its gold; the shell may not.)
        case (.composing(let c), .chat):
            return DiscSpec(label: c.hasShare ? c.shareLabel : "Chat",
                            glyph: c.hasShare ? c.shareGlyph : "bubble.left.and.bubble.right",
                            showsGlyph: c.hasShare,
                            isActive: c.hasShare && c.canShare,
                            enabled: c.hasShare && c.canShare,
                            tap: .newShare)

        case (.composing(let c), .life):
            return DiscSpec(label: c.commitLabel, glyph: c.commitGlyph, showsGlyph: true,
                            isActive: c.canCommit, enabled: c.canCommit,
                            tap: .newCommit)

        // Settings — NOTE centres and becomes the dismiss button; the other two are
        // parked off-screen (the shell's choreography), disabled so a stray tap on a
        // disc mid-flight does nothing.
        case (.settings, .note):
            return DiscSpec(label: "Close", glyph: "xmark", showsGlyph: true,
                            isActive: true,
                            tap: .dismissSettings)
        case (.settings, .chat):
            return DiscSpec(label: "Chat", glyph: "bubble.left.and.bubble.right",
                            enabled: false,
                            tap: .show(.chat))
        case (.settings, .life):
            return DiscSpec(label: "Life", glyph: "tree",
                            enabled: false,
                            tap: .show(.life))
        }
    }
}
