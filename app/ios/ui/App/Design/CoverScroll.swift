//  CoverScroll.swift
//  The scroll for a pushed screen whose first element is a WALL — a cover photo,
//  a gradient ground — that must reach the PHYSICAL TOP of the glass, behind the
//  status bar and the back button. Without this, the wall stops at the safe area
//  and the screen shows a seam: black above, ground below (the 2 Aug screenshot).
//
//  Two things make it work, and both are the component's contract:
//   • the scroll ignores the top safe area, so content genuinely starts at the
//     top edge of the glass;
//   • the navigation bar's own background is HIDDEN — with content beneath it
//     from the start, the system would otherwise paint the bar's scrolled
//     appearance over the wall, which reads as the very seam this removes.
//
//  The wall itself should size with the glass, not with the safe area — inset
//  arithmetic under `ignoresSafeArea` is exactly the thing SwiftUI makes
//  unreliable (an expanded subtree reports zero insets). House rule:
//
//      wall.containerRelativeFrame(.vertical, alignment: .top) { h, _ in h / 3 }
//
//  — a third of the glass, every device, no inset ever read.
//
//  The collapse seam (additive, both defaulted — the enabling seam for the
//  Profile pages' collapsing headers): the host may hand in bindings and the
//  scroll reports into them. `offset` is the raw contentOffset.y, every change.
//  `collapsed` is the DISCIPLINED header state — HideOnScroll's rules
//  (Design/Signal.swift) verbatim: an 8pt movement threshold so jitter and
//  rubber-banding don't count, never collapsed within the first 24pt, and a
//  0.35s deaf window after each flip so the reflow a collapse causes can't
//  immediately flip it back. Writes are UNANIMATED on purpose: the collapse
//  curve belongs to the consumer (ProfileGeometry.spring, the HeadlineCard
//  contract — one spring, owned where the state is read).

import SwiftUI

struct CoverScroll<Content: View>: View {
    /// Raw scroll offset out — contentOffset.y, negative while rubber-banding.
    var offset: Binding<CGFloat>? = nil
    /// Disciplined collapse state out — drives a collapsing header's `collapsed`.
    var collapsed: Binding<Bool>? = nil
    @ViewBuilder var content: Content

    @State private var last: CGFloat = 0
    @State private var deafUntil = Date.distantPast

    var body: some View {
        ScrollView {
            content
        }
        .scrollIndicators(.hidden)
        .onScrollGeometryChange(for: CGFloat.self) { $0.contentOffset.y } action: { _, y in
            offset?.wrappedValue = y
            track(y)
        }
        .ignoresSafeArea(edges: .top)
        .glassTopBar()
    }

    /// HideOnScroll's threshold/deaf-window discipline, reporting a header state
    /// instead of hiding a search field.
    private func track(_ y: CGFloat) {
        guard let collapsed else { return }
        defer { last = y }
        guard Date() >= deafUntil else { return }
        let dy = y - last
        guard abs(dy) > 8 else { return }
        // Only collapse once you are actually into the page, never at the top.
        let wants = dy > 0 && y > 24
        guard wants != collapsed.wrappedValue else { return }
        deafUntil = Date().addingTimeInterval(0.35)
        collapsed.wrappedValue = wants
    }
}
