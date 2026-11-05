//  ShellSurface.swift
//  What the shell can host UNDER its disc row.
//
//  THE ONE LINE THAT MATTERS. A surface presented ABOVE the shell — a sheet, a
//  fullScreenCover — covers the three discs and therefore cannot borrow them, so
//  it has to draw a replica of the row and the replica drifts. A surface hosted
//  under the row gets the real circles: they slide out of NOTE · CHAT · LIFE into
//  ✕ · ↑ · →, and slide home when it leaves. That choreography is the difference
//  between a modal that belongs to this app and one that merely runs in it.
//
//  `NewSession` had that privilege alone. This protocol is the same privilege,
//  named — so the detail modal (`Design/Detail/`) inherits the presentation, the
//  ground, the glass, the exit curves and the disc row without re-rolling any of
//  it, and without becoming a mint to get them (a `New` is DEFINED BY WHAT IT
//  MINTS, and a detail modal mints nothing).
//
//  WHAT A SURFACE OWES THE SHELL is only what the row and the departure need:
//  what the three discs say and whether they are live (`cell`), what GOLD and BLUE
//  DO, whether closing would destroy something typed, and a publisher so the row
//  re-asks when the surface changes under it. Everything else — the body, the
//  fields, what committing means — is the surface's own business, which is why
//  two very different modals can sit in the same slot.

import SwiftUI
import Combine

@MainActor
protocol ShellSurface: AnyObject {
    /// What the three discs ARE while this is up — labels, glyphs, and the two
    /// gates. The shell hands this to `InterfaceMatrix.Context.composing`.
    var cell: NewCell { get }

    /// Non-nil once the surface is on its way out — still MOUNTED so the
    /// departure can be seen. The shell sets it through `beginExit`.
    var exit: NewExit? { get }

    /// The noun the discard warning names: "Discard this note?" / "…this event?".
    /// One kind's word was fine when there was one kind.
    var discardNoun: String { get }

    /// Would ✕ destroy something the person typed? Only then does the shell warn —
    /// a warning over an empty surface trains people to dismiss warnings.
    var needsDiscardWarning: Bool { get }

    /// GOLD. Universally "who is this for" on a mint; a surface may mean something
    /// else by it, but it is always the surface's own second action.
    func gold()

    /// BLUE. Returns how the surface should LEAVE, or nil when the action did not
    /// happen — in which case the surface stays up with its content intact, which
    /// is the only honest response to a commit that failed.
    func performCommit() async -> NewExit?

    /// State the outcome; the surface plays its own departure curves from it.
    func beginExit(_ exit: NewExit)

    /// A nested ObservableObject does not propagate to the shell on its own, and
    /// the disc row reads its gates THROUGH the shell — so without this seam BLUE
    /// keeps whatever cell it was handed at open and never wakes up. (That was a
    /// real bug on the note: the gate was right, the row just never re-asked.)
    var changes: ObservableObjectPublisher { get }
}
