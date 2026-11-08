//  RibbonRail.swift
//  The filter surface: one jewel per category, each with its ribbon directly beneath it,
//  every ribbon's width set by its own jewel's state.
//
//  Gestures, per the mechanic:
//    TAP        select — expands this ribbon. Width only; never changes what matches.
//    HOLD       arrange — the jewels jiggle and can be dragged to RE-ORDER. A rail jewel
//               always returns to the rail (re-ordering is not removal — wandering off
//               the band cannot demote it); a carousel jewel promotes by dropping on
//               the band (room permitting) and re-orders within the carousel otherwise.
//    TAP A ROW  bind — pins that dimension to the entity, or releases the pin.
//
//  NO PULL-DOWN LOCK (27 Aug). A jewel used to lock as a filter dimension on a short
//  drag down, with a haptic at the threshold. It is gone: the gesture lived on top of
//  the tap and the hold in the same few pixels, and a selector whose meaning depends on
//  how far your finger travelled is a selector you cannot aim. `locked` stays a real
//  input to the face — `LifeQuery` still holds it and the profile rail still sets it —
//  it simply cannot be set from this rail any more.
//
//  The rail carries as many categories as fit; the rest wait in a vertical CAROUSEL under
//  the rightmost jewel, revealed while arranging.
//
//  Entity ribbons are FACETED: they list only entities that appear in the currently
//  matching commitments, so the intersection is visible and you can never bind your way
//  into an empty result.

import SwiftUI
import UIKit

struct RibbonRail: View {
    @EnvironmentObject private var shell: ShellState    // draws the dragged jewel above all
    @ObservedObject var query: LifeQuery
    /// The orb rides IN the row as its leading circle (0-ANSWER-G S31: seven equal
    /// circles). Its glow follows the feed's current item.
    var orbID: String? = nil
    var onOrb: (() -> Void)? = nil
    /// Arrange mode. Owned by the screen so a tap anywhere OUTSIDE the rail can leave it.
    @Binding var jiggle: Bool

    // ARRANGEMENT. The rail carries `order`; the overflow waits in `carousel`. Neither is
    // touched until a drag is released.
    // THINGS lost its jewel (1 Aug): the market it carried moved to the LIFE screen's
    // second face (blue disc, second tap), and a Things ribbon without the market listed
    // nothing — Thing has no EntityKind. The category survives in the grammar as a
    // participant dimension only. The carousel stays as the mechanism; it starts empty.
    // PROJECTS is DEFERRED (1 Aug): hidden from the rail, not removed from the grammar —
    // restore by adding `.projects` back here. The `pacificOpenProject` deep link still
    // opens ProjectDetailView; only the jewel is gone.
    // THE FIVE-JEWEL RAIL (11 Aug order): SPACES · EVENTS · PEOPLE · LISTINGS ·
    // POSTS. Places left the rail (the map is its own ground); JOB folded into
    // LISTINGS (a job is a listing posture, resolver-retarget). Key versioned so
    // the new roster beats an older persisted arrangement.
    @State private var order: [EventCategory] = RibbonRail.storedOrder()
    @State private var carousel: [EventCategory] = []

    static let defaultOrder: [EventCategory] = [.groups, .events, .people, .trade, .posts]
    private static let orderKey = "life.rail.order.v2"

    static func storedOrder() -> [EventCategory] {
        let stored = (UserDefaults.standard.stringArray(forKey: orderKey) ?? [])
            .compactMap(EventCategory.init(rawValue:))
            .filter { defaultOrder.contains($0) }
        return stored.count == defaultOrder.count ? stored
                                                  : stored + defaultOrder.filter { !stored.contains($0) }
    }

    static func store(_ order: [EventCategory]) {
        UserDefaults.standard.set(order.map(\.rawValue), forKey: orderKey)
    }
    /// The rail's frame in GLOBAL space — the jewel band and the zone basis.
    @State private var railFrame: CGRect = .zero

    // The live drag. Held in a REFERENCE type, not @State: gesture callbacks run several
    // times between renders, and a @State write is not visible to a read in the same call.
    // That made `begin` re-run every event and reset `session.targetIndex`, so a drag could never
    // progress. A published property's stored value updates immediately, so the gesture
    // sees its own writes — and the view still re-renders because it observes the object.
    @StateObject private var session = DragSession()

    private let jewelSize: CGFloat = RibbonWidth.contracted   // 46 (seven equal circles)
    /// The selectors start after the orb's slot — every zone/centre computation offsets by it.
    private var jewelOriginX: CGFloat { railInset + jewelSize + railGap }
    private let railInset: CGFloat = DesignTokens.Rail.inset
    private let railGap: CGFloat = DesignTokens.Rail.gap
    private let carouselGap: CGFloat = 12
    private let carouselDrop: CGFloat = 28
    /// The small deadzone straddling every zone boundary, as a fraction of a zone.
    private let deadzone: CGFloat = 0.2
    /// Vertical slack around the jewel band so it is forgiving to aim at.
    private let bandSlack: CGFloat = 14

    // MARK: Arrangement geometry

    private var railWidth: CGFloat { max(0, railFrame.width - railInset * 2 - (jewelSize + railGap)) }
    /// The jewel band's top — their 8pt top padding below the rail's own origin.
    private var bandTop: CGFloat { railFrame.minY + 8 }
    private var carouselTop: CGFloat { bandTop + jewelSize + carouselDrop }

    /// How many contracted jewels fit the rail: n·Ø + (n−1)·gap ≤ railWidth. Derived, so it
    /// stays right on any device rather than being a guessed constant.
    /// What actually FITS: n·Ø + (n−1)·gap ≤ railWidth. Reserving room for a full 168pt
    /// pill instead gave capacity 4 — with 4 already in the rail nothing could ever be
    /// promoted. The selected column adapts down instead, which is what bounds the rail.
    private var capacity: Int {
        guard railWidth > 0 else { return 5 }
        return max(1, Int((railWidth + railGap) / (jewelSize + railGap)))
    }
    private var hasRoom: Bool { order.count < capacity }
    /// True when the dragged jewel would land in the rail if released now. A jewel that
    /// STARTED in the rail always lands back in it — arranging re-orders, it never
    /// removes — so only a carousel jewel's landing depends on where the finger is.
    private var willLand: Bool { session.fromRail || (session.overRail && hasRoom) }

    /// The rail's actual pitch while arranging — jewels are circles with the slack shared
    /// between them, so this places each one exactly.
    private var railPitch: CGFloat {
        let n = max(1, displayOrder.count)
        guard n > 1 else { return jewelSize }
        return (railWidth - CGFloat(n) * jewelSize) / CGFloat(n - 1) + jewelSize
    }

    /// Which jewel is under a point — the container gesture's hit test, since it no longer
    /// learns the category from whichever view was touched.
    private func categoryAt(_ p: CGPoint) -> (EventCategory, Bool)? {
        if inBand(p.y) {
            let n = displayOrder.count
            guard n > 0, railPitch > 0 else { return nil }
            let i = Int(((p.x - railFrame.minX - jewelOriginX) / railPitch).rounded())
            guard i >= 0, i < n else { return nil }
            return (displayOrder[i], true)
        }
        let cx = railFrame.minX + carouselX
        guard p.x >= cx - 8, p.x <= cx + jewelSize + 8 else { return nil }
        let i = Int(floor((p.y - carouselTop) / (jewelSize + carouselGap)))
        guard i >= 0, i < displayCarousel.count else { return nil }
        return (displayCarousel[i], false)
    }

    /// A jewel's centre in global space, from the same geometry the hit test uses.
    private func centre(of c: EventCategory, fromRail rail: Bool) -> CGPoint {
        if rail, let i = displayOrder.firstIndex(of: c) {
            return CGPoint(x: railFrame.minX + jewelOriginX + CGFloat(i) * railPitch + jewelSize / 2,
                           y: bandTop + jewelSize / 2)
        }
        let i = displayCarousel.firstIndex(of: c) ?? 0
        return CGPoint(x: railFrame.minX + carouselX + jewelSize / 2,
                       y: carouselTop + CGFloat(i) * (jewelSize + carouselGap) + jewelSize / 2)
    }

    private func inBand(_ y: CGFloat) -> Bool {
        guard railFrame.height > 0 else { return false }
        return y >= bandTop - bandSlack && y <= bandTop + jewelSize + bandSlack
    }

    /// The rail AS RENDERED: the dragged jewel takes its prospective slot (drawn invisible)
    /// so the row opens a gap exactly where it would land.
    private var displayOrder: [EventCategory] {
        guard let c = session.category else { return order }
        var arr = order
        arr.removeAll { $0 == c }
        if willLand { arr.insert(c, at: min(max(0, session.targetIndex), arr.count)) }
        return arr
    }

    /// The carousel AS RENDERED — covers both a demotion from the rail and a plain reorder.
    private var displayCarousel: [EventCategory] {
        guard let c = session.category else { return carousel }
        var arr = carousel
        arr.removeAll { $0 == c }
        if !willLand { arr.insert(c, at: min(max(0, session.carouselIndex), arr.count)) }
        return arr
    }

    // MARK: Body

    var body: some View {
        ZStack(alignment: .topLeading) {
            // BOUNDED to the screen: no horizontal scroll. Flexible spacers absorb any
            // slack, so the row of seven equal circles always ends at the right inset.
            Group {
                HStack(alignment: .top, spacing: 0) {
                    FeedOrb(currentID: orbID, size: jewelSize) { onOrb?() }
                    Spacer(minLength: railGap)
                    ForEach(Array(displayOrder.enumerated()), id: \.element) { index, cat in
                        jewel(cat, index: index)
                        if index < displayOrder.count - 1 { Spacer(minLength: railGap) }
                    }
                }
                .padding(.horizontal, railInset)
                .padding(.top, 8)
            }

            // The overflow categories fan DOWN from under the rightmost jewel.
            if jiggle { overflowCarousel }
        }
        .background(
            GeometryReader { p in
                Color.clear
                    .allowsHitTesting(false)   // measurement only — never eat the background's touches
                    .onAppear { railFrame = p.frame(in: .global) }
                    // Only adopt a MEANINGFUL change: sub-pixel jitter during an animation
                    // would otherwise write state every frame and re-render mid-drag.
                    .onChange(of: p.frame(in: .global)) { _, f in
                        let moved = abs(f.minX - railFrame.minX) > 0.5 || abs(f.minY - railFrame.minY) > 0.5
                            || abs(f.width - railFrame.width) > 0.5 || abs(f.height - railFrame.height) > 0.5
                        if moved { railFrame = f }
                    }
            }
        )
        // The rail owns arranging drags; at rest the jewels keep their own tap/lock/hold.
        // SIMULTANEOUS, and never masked. Two earlier attempts failed here: a plain
        // `.gesture` lost to the jewels' own minimumDistance-0 drags (a child outranks a
        // parent), and masking the subviews out while arranging flipped the mask MID-TOUCH
        // — the jewel's gesture was switched off under the finger while this one, disabled
        // when the touch began, could never adopt it, so hold-then-drag reached nobody.
        // Simultaneous means this tracks from touch-down regardless, so it is already live
        // the instant the long press turns `jiggle` on. It no-ops at rest via its own guard.
        .simultaneousGesture(arrangeDrag)
        .animation(.spring(response: 0.36, dampingFraction: 0.78), value: query.maximised)
        .animation(.spring(response: 0.36, dampingFraction: 0.78), value: query.locked)
        .onChange(of: order) { _, o in Self.store(o) }
    }

    /// The categories that don't fit the rail, as a vertical carousel under its right edge.
    private var overflowCarousel: some View {
        VStack(spacing: carouselGap) {
            ForEach(Array(displayCarousel.enumerated()), id: \.element) { i, cat in
                jewel(cat, index: i)
                    // The stagger rides the TRANSITION, not a blanket `.animation(value:)`
                    // — that modifier swallows the jewel's repeatForever wobble.
                    .transition(.scale(scale: 0.1, anchor: .top).combined(with: .opacity)
                        .animation(.spring(response: 0.34, dampingFraction: 0.72)
                                    .delay(Double(i) * 0.06)))
            }
        }
        // Anchored to the LAST rail jewel, not the screen edge: their HStack is
        // left-aligned with a fixed gap, so the final jewel stops short of the trailing
        // edge and a trailing-aligned carousel would hang ~50pt to its right.
        .frame(maxWidth: .infinity, alignment: .leading)
        .offset(x: carouselX, y: 8 + jewelSize + carouselDrop)
    }

    /// The x of the last rail jewel's leading edge — where the carousel hangs from.
    private var carouselX: CGFloat {
        railInset + CGFloat(max(0, displayOrder.count - 1)) * railPitch
    }

    /// The ONE drag that does all the arranging. It lives on the rail CONTAINER, not on the
    /// jewels: a jewel's view moves (and is rebuilt) the instant a reorder lands, which
    /// CANCELS an in-flight gesture — and a cancelled DragGesture never calls `onEnded`, so
    /// `dragEnded()` never ran, `session.category` stayed set and the rail wedged after exactly one
    /// reorder. The container never moves, so nothing can cancel it.
    private var arrangeDrag: some Gesture {
        DragGesture(minimumDistance: 0, coordinateSpace: .global)
            .onChanged { v in
                guard jiggle else { return }
                if session.category == nil {
                    // Only begin once the finger has genuinely moved, and pick the jewel
                    // from where the press STARTED, not where it is now.
                    guard abs(v.translation.width) > 2 || abs(v.translation.height) > 2,
                          let (cat, hitFromRail) = categoryAt(v.startLocation) else { return }
                    begin(cat, fromRail: hitFromRail, at: v.startLocation)
                }
                dragChanged(at: v.location)
            }
            .onEnded { _ in
                guard jiggle else { return }
                let entered = session.justEntered
                session.justEntered = false
                if session.category != nil {
                    dragEnded()
                } else if !entered {
                    withAnim { jiggle = false }   // a press that never dragged = leave arrange
                }
                // else: this IS the press that entered arrange — it stays open.
            }
    }

    private func begin(_ c: EventCategory, fromRail rail: Bool, at finger: CGPoint) {
        let mid = centre(of: c, fromRail: rail)
        session.grab = CGSize(width: finger.x - mid.x, height: finger.y - mid.y)
        shell.jewelDrag.begin(glyph: c.glyph, color: JewelFace.opal,
                              size: jewelSize, at: mid)
        session.category = c
        session.fromRail = rail
        session.targetIndex = order.firstIndex(of: c) ?? max(0, order.count - 1)
        session.carouselIndex = carousel.firstIndex(of: c) ?? carousel.count
    }

    /// S32 amended (10 Aug): CIRCLE RADIUS STAYS CONSTANT — the unmaximised jewels
    /// never shrink. The maximised capsule is a modest ×1.45 of the circle (half the
    /// previous full-slack width); the flexible gaps absorb the difference, edges
    /// anchored.
    private func jewelWidth(_ c: EventCategory) -> CGFloat {
        guard !jiggle, query.maximised == c else { return jewelSize }
        return jewelSize * 1.45
    }

    private func jewel(_ cat: EventCategory, index: Int) -> some View {
        Jewel(category: cat,
              width: jewelWidth(cat),
              band: jewelSize,
              selected: query.maximised == cat,
              locked: query.isLocked(cat),
              jiggle: jiggle,
              index: index,
              hidden: session.category == cat,
              onTap: { tap(cat) },
              onHold: { hold() })
    }

    // MARK: Gestures

    private func tap(_ c: EventCategory) {
        if jiggle {
            withAnim { jiggle = false }
        } else {
            // Single-page grammar: a tap MAXIMISES the jewel (opens its config page);
            // tapping the maximised jewel minimises back to the feed.
            withAnim { query.maximise(c) }
        }
    }

    private func hold() {
        guard !jiggle else { return }
        UIImpactFeedbackGenerator(style: .rigid).impactOccurred()
        session.justEntered = true      // this gesture's release must not read as leave-arrange
        withAnimation(.spring(response: 0.3, dampingFraction: 0.6)) { jiggle = true }
    }

    /// The whole drag, for a jewel from EITHER container.
    ///
    /// The decision inputs are the finger's absolute global position and fixed geometry —
    /// never `order`, `carousel` or the rendered index. A crossing therefore cannot feed
    /// back into the decision that produced it.
    private func dragChanged(at p: CGPoint) {
        guard jiggle, session.category != nil else { return }
        let origin = session.fromRail    // captured once, in `begin`
        // The jewel's centre, not the finger: it keeps the spot it was grabbed by. Writing
        // to the shell's model re-renders only that overlay, never the rail.
        shell.jewelDrag.point = CGPoint(x: p.x - session.grab.width,
                                        y: p.y - session.grab.height)

        let now = inBand(p.y)
        // The band threshold only matters to a CAROUSEL jewel (promotion). A rail jewel
        // lands back in the rail wherever the finger wanders, so crossing the band means
        // nothing for it — no state flip, and no haptic promising a drop that can't change.
        if !origin, now != session.overRail {
            withAnimation(.spring(response: 0.28, dampingFraction: 0.8)) { session.overRail = now }
            UIImpactFeedbackGenerator(style: .soft).impactOccurred()   // felt at the threshold
        }

        // Decide from the LOCAL values, never by reading the @State back. `session.fromRail`
        // was written on the first event; reading @State back here returns the pre-write
        // value, which routed every rail drag down the carousel branch (so the rail never
        // reordered, and nothing could be promoted). Re-ordering is not removal: a
        // rail-origin jewel is ALWAYS landing.
        let landing = origin || (now && hasRoom)

        // Same mechanics both ways — static zones, a deadzone straddling every boundary,
        // one slot per step — just along the axis the jewel is currently over.
        if landing {
            let pitch = railFrame.width / CGFloat(max(1, displayOrder.count))
            if let next = stepped(session.targetIndex,
                                  toward: zoneIndex(p.x, origin: railFrame.minX, pitch: pitch),
                                  last: max(0, displayOrder.count - 1)) {
                withAnimation(.spring(response: 0.25, dampingFraction: 0.85)) { session.targetIndex = next }
            }
        } else {
            guard railFrame.height > 0 else { return }
            if let next = stepped(session.carouselIndex,
                                  toward: zoneIndex(p.y, origin: carouselTop, pitch: jewelSize + carouselGap),
                                  last: max(0, displayCarousel.count - 1)) {
                withAnimation(.spring(response: 0.25, dampingFraction: 0.85)) { session.carouselIndex = next }
            }
        }
    }

    /// Which zone a coordinate falls in, or nil while inside the deadzone straddling a
    /// boundary. Pure geometry — no array, no current index.
    private func zoneIndex(_ v: CGFloat, origin: CGFloat, pitch: CGFloat) -> Int? {
        guard pitch > 0 else { return nil }
        let z = (v - origin) / pitch
        guard abs(z - z.rounded()) >= deadzone / 2 else { return nil }
        return Int(floor(z))
    }

    /// The index one slot along toward `raw`, clamped — or nil if it shouldn't move. Returns
    /// rather than mutating: a @State passed `inout` is copy-in/copy-out, so the write-back
    /// would land AFTER `withAnimation` returned and the move would snap.
    private func stepped(_ index: Int, toward raw: Int?, last: Int) -> Int? {
        guard let raw, raw != index else { return nil }
        let next = min(max(0, index + (raw > index ? 1 : -1)), last)
        return next == index ? nil : next
    }

    /// Commit ONCE, on release. The floating copy then FLIES to the slot the jewel landed
    /// in and fades there, rather than vanishing at the finger — the teleport.
    private func dragEnded() {
        guard let c = session.category else { return }
        let landed = willLand

        withAnimation(.spring(response: 0.34, dampingFraction: 0.8)) {
            order.removeAll { $0 == c }
            carousel.removeAll { $0 == c }
            if landed {
                order.insert(c, at: min(max(0, session.targetIndex), order.count))
            } else {
                // Only a carousel-origin jewel can end up here (a rail jewel always
                // lands — see `willLand`): this is its re-order within the carousel,
                // or a promotion attempt that found no room.
                carousel.insert(c, at: min(max(0, session.carouselIndex), carousel.count))
            }
            session.category = nil        // the real jewel takes its slot again
            session.overRail = false
        }

        // Now that the arrays are settled, `restCentre` is the jewel's true resting place.
        let rest = restCentre(c, landed: landed)
        withAnimation(.spring(response: 0.3, dampingFraction: 0.86)) {
            shell.jewelDrag.point = rest
            shell.jewelDrag.fading = true
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.32) {
            shell.jewelDrag.visible = false
        }
        UIImpactFeedbackGenerator(style: .light).impactOccurred()
    }

    /// Where a jewel comes to rest after the commit — read from the SETTLED arrays.
    private func restCentre(_ c: EventCategory, landed: Bool) -> CGPoint {
        if landed, let i = order.firstIndex(of: c) {
            return CGPoint(x: railFrame.minX + jewelOriginX + CGFloat(i) * railPitch + jewelSize / 2,
                           y: bandTop + jewelSize / 2)
        }
        let i = carousel.firstIndex(of: c) ?? 0
        return CGPoint(x: railFrame.minX + carouselX + jewelSize / 2,
                       y: carouselTop + CGFloat(i) * (jewelSize + carouselGap) + jewelSize / 2)
    }

    private func withAnim(_ body: () -> Void) {
        withAnimation(.spring(response: 0.36, dampingFraction: 0.78), body)
    }
}

// MARK: - The dragged jewel

/// Everything about the in-flight drag. A class so the gesture's own writes are visible
/// to its own reads between renders; @Published so the rail still re-renders on each
/// meaningful change (which slot it would take, which side of the band it is on).
@MainActor final class DragSession: ObservableObject {
    @Published var category: EventCategory?
    @Published var fromRail = false
    @Published var targetIndex = 0        // prospective slot in the RAIL
    @Published var carouselIndex = 0      // prospective slot in the CAROUSEL
    @Published var overRail = false
    /// True for the single gesture that turned arrange mode ON. That press never drags, so
    /// without this the container's "a press that never dragged leaves arrange" rule fired
    /// on the very press that entered — arrange opened and shut in one motion. Not
    /// @Published: it decides nothing that is drawn.
    var justEntered = false
    /// Finger-minus-jewel-centre at pick-up. Without it the floating copy centres itself on
    /// the finger, so the jewel visibly jumps the moment it is picked up.
    var grab: CGSize = .zero
}



// MARK: - The jewel

/// The jewel's face alone — shared by the rail, the carousel and the floating drag.
/// 0-ANSWER-G S29: icon OUTLINE only, dim opal — no fills, no tints. S32: the
/// maximised jewel WIDENS into a capsule; the compressed neighbours stay circles.
/// S33: LOCKED reads as a filled opal jewel (it is filtering the feed).
struct JewelFace: View {
    let category: EventCategory
    var width: CGFloat = RibbonWidth.contracted
    var band: CGFloat = RibbonWidth.contracted
    var expanded = false
    var locked = false

    static let opal = DesignTokens.Rail.opal

    private var stroke: Double { expanded ? 0.95 : (locked ? 0.8 : 0.4) }
    private var icon: Double { expanded ? 0.95 : (locked ? 0.85 : 0.55) }

    var body: some View {
        Capsule(style: .circular)
            .strokeBorder(Self.opal.opacity(stroke), lineWidth: DesignTokens.Rail.stroke)
            .background(Capsule(style: .circular).fill(Self.opal.opacity(locked ? 0.16 : 0)))
            .overlay(
                Image(systemName: category.glyph)
                    .font(.system(size: min(width, band) * DesignTokens.Rail.glyphRatio, weight: .regular))
                    .foregroundStyle(Self.opal.opacity(icon))
            )
            .frame(width: width, height: min(band, width))
            .frame(height: band)   // centred in the band whatever the diameter
    }
}

struct Jewel: View {
    let category: EventCategory
    let width: CGFloat
    let band: CGFloat
    let selected: Bool
    let locked: Bool
    let jiggle: Bool
    let index: Int
    /// True while THIS jewel is the one under the finger — it holds its slot open as a gap
    /// while the floating copy does the travelling.
    let hidden: Bool
    let onTap: () -> Void
    let onHold: () -> Void

    @State private var rot: Double = 0         // jiggle wobble angle
    /// True from the moment a hold fires until the finger lifts, so the release isn't also
    /// read as a tap (which would immediately leave arrange mode).
    @State private var handledByHold = false

    var body: some View {
        JewelFace(category: category,
                  width: width,
                  band: band,
                  expanded: selected && !jiggle,
                  locked: locked)
            .opacity(hidden ? 0 : 1)
            .rotationEffect(.degrees(rot))
            .contentShape(Capsule(style: .circular))
            .gesture(drag)
            .simultaneousGesture(
                LongPressGesture(minimumDuration: 0.45).onEnded { _ in
                    handledByHold = true
                    onHold()
                }
            )
            // Started on APPEAR as well as on change — a jewel promoted into the rail
            // mid-arrange is newly created, so `onChange` never fires for it and it would
            // otherwise sit still among wobbling neighbours.
            .onAppear { if jiggle { startWobble() } }
            .onChange(of: jiggle) { _, on in on ? startWobble() : stopWobble() }
            // The jewels are gesture-driven, not Buttons, so without this the whole rail is
            // invisible to XCUITest — and a selector nothing can drive is a selector nothing
            // can regression-test.
            .accessibilityElement(children: .ignore)
            .accessibilityIdentifier("jewel-\(category.rawValue)")
            .accessibilityLabel(category.title)
            .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
    }

    private func startWobble() {
        let dir: Double = index.isMultiple(of: 2) ? 1 : -1
        rot = -1.6 * dir
        withAnimation(.easeInOut(duration: 0.14).repeatForever(autoreverses: true)) {
            rot = 1.6 * dir
        }
    }

    private func stopWobble() { withAnimation(.easeOut(duration: 0.12)) { rot = 0 } }

    /// TAP, and only tap. `minimumDistance: 0` still, because this gesture's whole job is
    /// now to tell a STILL press from a travelling finger: a still press is the tap, and
    /// anything that moved belongs to the hold (arranging) or to the rail's own swipes,
    /// which read the mode strip underneath. It stays a DragGesture rather than becoming
    /// `.onTapGesture` so that "moved, therefore not a tap" keeps being decided here,
    /// where the hold latch is, instead of by two gestures racing.
    private var drag: some Gesture {
        DragGesture(minimumDistance: 0, coordinateSpace: .global)
            .onEnded { v in
                defer { handledByHold = false }
                guard !jiggle else { return }        // the rail owns gestures while arranging
                let moved = abs(v.translation.width) > 3 || abs(v.translation.height) > 3
                if !moved, !handledByHold { onTap() }
            }
    }
}

// MARK: - An entity ribbon

private struct EntityRibbon: View {
    let entities: [ParticipantVM]
    let width: CGFloat
    let boundID: String?
    let onSelect: (String) -> Void

    private var tier: RibbonTier { RibbonTier(width: width) }

    var body: some View {
        ScrollView(showsIndicators: false) {
            LazyVStack(alignment: .leading, spacing: tier == .tick ? 10 : 12) {
                ForEach(entities) { e in
                    row(e)
                        .contentShape(Rectangle())
                        .onTapGesture { onSelect(e.id) }
                }
            }
            .padding(.vertical, 12)
        }
        .frame(width: width)
    }

    @ViewBuilder
    private func row(_ e: ParticipantVM) -> some View {
        let isBound = e.id == boundID
        switch tier {
        case .tick:
            Circle()
                .fill(e.tint)
                .frame(width: isBound ? 12 : 8, height: isBound ? 12 : 8)
                .overlay(Circle().strokeBorder(Theme.accent, lineWidth: isBound ? 1.5 : 0))
        case .compact, .full:
            HStack(spacing: 7) {
                Circle().fill(e.tint).frame(width: 7, height: 7)
                Text(e.name)
                    .font(.system(size: 13, weight: isBound ? .semibold : .regular))
                    .foregroundStyle(isBound ? .primary : .secondary)
                    .lineLimit(1)
            }
        }
    }
}

// MARK: - Glyphs

extension EventCategory {
    var glyph: String {
        switch self {
        case .events:   return "calendar"
        case .people:   return "person.fill"
        case .groups:   return "square.grid.2x2.fill"
        case .places:   return "mappin.circle.fill"
        case .projects: return "square.grid.2x2.fill"
        case .things:   return "shippingbox.fill"
        case .trade:    return "arrow.left.arrow.right"
        case .job:      return "briefcase.fill"
        case .posts:    return "photo.fill"
        case .channels: return "number"          // a channel is a #
        }
    }
}

// MARK: - Preview

#Preview("Ribbon rail — seeded") {
    struct Harness: View {
        @StateObject private var query = LifeQuery()
        @State private var jiggle = false
        var body: some View {
            VStack(spacing: 0) {
                RibbonRail(query: query, jiggle: $jiggle)
                Spacer()
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Theme.bg)
            .preferredColorScheme(.dark)
        }
    }
    return Harness()
}
