//  ConvergenceView.swift
//  The CONVERGENCE deck — a fork of the NOTE compose surface (NoteComposeModal):
//  same material-glass card, same drifting-jewel ambient, same colour-coded
//  directional departure — but the card is a READ-ONLY binary question instead of
//  an editable note, and it leaves LEFT (reject) or RIGHT (accept) instead of
//  up/scale. The symmetry is deliberate: you write into this glass (notes); the
//  graph asks its questions on the same glass.
//
//  v1 is self-contained: swipe or tap ✗/✓ (the buttons keep it drivable by
//  XCUITest, where raw swipes are TCC-blocked). The shell disc-row morph is a
//  later pass. The bottom field is the optional metadata input — the note body
//  editor, repurposed.

import SwiftUI

// MARK: - UI facets of the contract's LifeType (kept out of the pure data file)

extension LifeType {
    var label: String {
        switch self {
        case .person: "Person"; case .place: "Place"; case .event: "Event"
        case .project: "Project"; case .plant: "Plant"; case .animal: "Animal"
        case .thing: "Thing"; case .group: "Group"
        case .inventory: "Inventory"; case .skill: "Skill"
        case .trade: "Trade"; case .job: "Job"
        }
    }
    var glyph: String {
        switch self {
        case .person: "person.fill"; case .place: "mappin.circle.fill"
        case .event: "calendar"; case .project: "square.grid.2x2.fill"
        case .plant: "leaf.fill"; case .animal: "pawprint.fill"
        case .thing: "shippingbox.fill"; case .group: "person.3.fill"
        // The market 2×2 wears the same glyphs as its face's selectors (MarketFaceCategory).
        case .inventory: "shippingbox.fill"; case .skill: "wrench.and.screwdriver.fill"
        case .trade: "arrow.left.arrow.right"; case .job: "hammer.fill"
        }
    }
    /// A jewel hue per type — the card's ambient + edge tint. The colour is itself
    /// a signal of what kind of thing you're being asked about.
    var tint: Color {
        switch self {
        case .person: Color(hex: 0x7A6FF0)   // indigo
        case .place:  Color(hex: 0x49A6F2)   // azure
        case .event:  Color(hex: 0xF0669E)   // rose
        case .project: Color(hex: 0xE8C06A)  // gold
        case .group:  Color(hex: 0x33C08C)   // emerald
        case .plant:  Color(hex: 0x8EC16E)   // leaf
        case .animal: Color(hex: 0xE0894B)   // amber
        case .thing:  Color(hex: 0x8A93A3)   // steel
        // The market 2×2, toned to its face's selector tints.
        case .inventory: Color(hex: 0x2FB5AE)  // teal
        case .skill:  Color(hex: 0x9B59D0)     // purple
        case .trade:  Color(hex: 0x2E70B8)     // exchange blue
        case .job:    Color(hex: 0xA07048)     // umber
        }
    }
}

// MARK: - Deck model (the fork of NoteDraft)

/// The batch being reviewed + the metadata being typed for the current card.
/// `decide` records the answer (apply instruction AND classifier training row) and
/// advances. The queue is the source of truth for what's on screen.
@MainActor
final class ConvergenceDeck: ObservableObject {
    @Published private(set) var pending: [Proposal]
    /// The optional bottom field — the NoteDraft.body analogue.
    @Published var metadata: String = ""
    /// Everything decided this session — feeds apply + classifier training.
    @Published private(set) var decisions: [ProposalDecision] = []

    init(_ proposals: [Proposal]) { pending = proposals }

    /// Called as each card leaves, BEFORE it is dropped from `pending`. This is where a
    /// swipe becomes an act — accept mints a GroupObject. Set by the presenting view; the
    /// deck itself stays ignorant of what a decision means.
    var onDecide: ((Proposal, ProposalDecision.Decision) -> Void)?

    var current: Proposal? { pending.first }
    /// Up to two cards peeking behind the front one.
    var peek: [Proposal] { Array(pending.dropFirst().prefix(2)) }
    var remaining: Int { pending.count }
    var isEmpty: Bool { pending.isEmpty }

    func decide(_ decision: ProposalDecision.Decision) {
        guard let p = current else { return }
        let meta = metadata.trimmingCharacters(in: .whitespacesAndNewlines)
        decisions.append(.init(proposalId: p.id, decision: decision,
                               metadata: meta, decidedAt: Int(Date().timeIntervalSince1970 * 1000)))
        onDecide?(p, decision)
        metadata = ""
        pending.removeFirst()
    }
}

// MARK: - The deck screen

struct ConvergenceView: View {
    @StateObject private var deck: ConvergenceDeck
    var onDone: () -> Void = {}

    /// Live drag of the FRONT card; also driven programmatically by the ✗/✓ buttons
    /// so a tap leaves on the same curve as a swipe.
    @State private var drag: CGSize = .zero
    @State private var leaving = false
    @FocusState private var metaFocused: Bool

    private let flingThreshold: CGFloat = 110

    init(_ proposals: [Proposal],
         onDecide: ((Proposal, ProposalDecision.Decision) -> Void)? = nil,
         onDone: @escaping () -> Void = {}) {
        let deck = ConvergenceDeck(proposals)
        deck.onDecide = onDecide
        _deck = StateObject(wrappedValue: deck)
        self.onDone = onDone
    }

    /// The front card's tint drives the whole surface's ambient.
    private var tint: Color { deck.current?.subject.type.tint ?? Theme.textSecondary }

    var body: some View {
        ZStack {
            ZStack { Theme.bg.opacity(0.72); ConvergenceAmbient(tint: tint) }
                .ignoresSafeArea()

            if deck.isEmpty {
                doneState
            } else {
                VStack(spacing: 0) {
                    header
                    Spacer(minLength: 8)
                    deckStack
                    Spacer(minLength: 8)
                    controls
                }
                .padding(.horizontal, 20)
                .padding(.top, 56)
                .padding(.bottom, 18)
            }
        }
        .animation(.spring(response: 0.4, dampingFraction: 0.8), value: deck.remaining)
    }

    // MARK: header — batch progress

    private var header: some View {
        HStack {
            Text("Review").font(.title2.weight(.bold)).foregroundStyle(Theme.textPrimary)
            Spacer()
            Text("\(deck.remaining) left")
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(Theme.textSecondary)
            Button { onDone() } label: {
                Image(systemName: "xmark").font(.subheadline.weight(.bold))
                    .foregroundStyle(Theme.textSecondary).padding(8)
            }
            .accessibilityIdentifier("convergence-close")
        }
    }

    // MARK: the stacked cards

    private var deckStack: some View {
        ZStack {
            // Peek cards behind, deepest first.
            ForEach(Array(deck.peek.enumerated().reversed()), id: \.element.id) { i, p in
                ConvergenceCard(proposal: p)
                    .scaleEffect(1 - CGFloat(i + 1) * 0.05)
                    .offset(y: CGFloat(i + 1) * 14)
                    .opacity(0.5)
            }
            if let p = deck.current {
                ConvergenceCard(proposal: p, dragProgress: dragProgress)
                    .offset(drag)
                    .rotationEffect(.degrees(Double(drag.width / 22)))
                    .gesture(swipe)
                    .id(p.id)
                    .transition(.identity)
            }
        }
        .frame(maxHeight: 460)
    }

    /// -1 (full left / reject) … +1 (full right / accept).
    private var dragProgress: CGFloat {
        max(-1, min(1, drag.width / flingThreshold))
    }

    private var swipe: some Gesture {
        DragGesture()
            .onChanged { v in if !leaving { drag = v.translation } }
            .onEnded { v in
                if abs(v.translation.width) >= flingThreshold {
                    commit(v.translation.width > 0 ? .accept : .reject)
                } else {
                    withAnimation(.spring(response: 0.3, dampingFraction: 0.7)) { drag = .zero }
                }
            }
    }

    // MARK: controls — the metadata field + the two deliberate-tap buttons

    private var controls: some View {
        VStack(spacing: 14) {
            HStack(spacing: 10) {
                Image(systemName: "text.append")
                    .font(.footnote).foregroundStyle(Theme.textTertiary)
                TextField("Add a note…", text: $deck.metadata, axis: .vertical)
                    .font(.subheadline).foregroundStyle(Theme.textPrimary)
                    .focused($metaFocused).lineLimit(1...3)
                    .accessibilityIdentifier("convergence-metadata")
            }
            .padding(.horizontal, 14).padding(.vertical, 10)
            .background(Theme.surface2.opacity(0.7), in: Capsule())
            .overlay(Capsule().strokeBorder(Theme.hairline, lineWidth: 1))

            HStack(spacing: 40) {
                circleButton(system: "xmark", color: Color(hex: 0xF0669E), id: "convergence-reject") {
                    commit(.reject)
                }
                circleButton(system: "checkmark", color: tint, id: "convergence-accept") {
                    commit(.accept)
                }
            }
        }
    }

    private func circleButton(system: String, color: Color, id: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: system).font(.title2.weight(.bold)).foregroundStyle(color)
                .frame(width: 62, height: 62)
                .background(Theme.surface.opacity(0.9), in: Circle())
                .overlay(Circle().strokeBorder(color.opacity(0.5), lineWidth: 1.5))
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier(id)
    }

    // MARK: commit — fly the front card off, then record + advance

    private func commit(_ decision: ProposalDecision.Decision) {
        guard !leaving, deck.current != nil else { return }
        leaving = true
        metaFocused = false
        let dir: CGFloat = decision == .accept ? 1 : -1
        withAnimation(.easeIn(duration: 0.26)) {
            drag = CGSize(width: dir * 900, height: drag.height)
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.26) {
            deck.decide(decision)
            drag = .zero
            leaving = false
        }
    }

    private var doneState: some View {
        VStack(spacing: 12) {
            Image(systemName: "checkmark.seal.fill").font(.system(size: 44)).foregroundStyle(Theme.textSecondary)
            Text("All caught up").font(.headline).foregroundStyle(Theme.textPrimary)
            if !deck.decisions.isEmpty {
                Text("\(deck.decisions.count) reviewed")
                    .font(.subheadline).foregroundStyle(Theme.textSecondary)
            }
            Button("Done", action: onDone)
                .font(.subheadline.weight(.semibold)).foregroundStyle(Theme.textPrimary)
                .padding(.horizontal, 22).padding(.vertical, 10)
                .background(Theme.surface2, in: Capsule()).padding(.top, 4)
        }
    }

}

// MARK: - One card (forked glass, read-only binary question)

struct ConvergenceCard: View {
    let proposal: Proposal
    /// -1…+1 live drag, for the reject/accept edge flash on the FRONT card.
    var dragProgress: CGFloat = 0

    private var type: LifeType { proposal.subject.type }
    private var accepting: Bool { dragProgress > 0 }
    private var flashColor: Color { accepting ? type.tint : Color(hex: 0xF0669E) }
    private var flashOpacity: Double { Double(min(1, abs(dragProgress))) * 0.9 }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            typeChip
            question
            if proposal.kind == .merge, let t = proposal.target { compare(t) }
            quote
            Spacer(minLength: 0)
            footer
        }
        .padding(22)
        .frame(maxWidth: .infinity, alignment: .leading)
        .frame(height: 440)
        // The compose card's skin; its edge takes the live reject/accept flash.
        .composeGlass(tint: type.tint, flash: flashOpacity > 0 ? flashColor.opacity(flashOpacity) : nil)
    }

    private var typeChip: some View {
        HStack(spacing: 6) {
            Image(systemName: type.glyph).font(.caption.weight(.bold))
            Text(type.label.uppercased()).font(.caption2.weight(.heavy)).tracking(1.1)
        }
        .foregroundStyle(type.tint)
        .padding(.horizontal, 10).padding(.vertical, 5)
        .background(type.tint.opacity(0.14), in: Capsule())
    }

    @ViewBuilder private var question: some View {
        switch proposal.kind {
        case .mint:
            VStack(alignment: .leading, spacing: 4) {
                Text(proposal.subject.name).font(.title.weight(.bold)).foregroundStyle(Theme.textPrimary)
                Text("Add as a new \(type.label)?").font(.headline).foregroundStyle(Theme.textSecondary)
            }
        case .merge:
            Text("Same \(type.label)?").font(.title.weight(.bold)).foregroundStyle(Theme.textPrimary)
        }
    }

    private func compare(_ t: Proposal.Target) -> some View {
        HStack(spacing: 12) {
            side(title: proposal.subject.name, sub: "this \(proposal.provenance.source.rawValue)", hard: false)
            Image(systemName: "arrow.left.arrow.right").font(.footnote.weight(.bold)).foregroundStyle(Theme.textTertiary)
            side(title: t.name, sub: t.summary, hard: t.tier == .hard)
        }
        .padding(12)
        .background(Theme.surface.opacity(0.6), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
    }

    private func side(title: String, sub: String, hard: Bool) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 5) {
                Text(title).font(.subheadline.weight(.bold)).foregroundStyle(Theme.textPrimary).lineLimit(1)
                if hard {
                    Text("LIVE").font(.system(size: 8, weight: .black)).tracking(0.6)
                        .foregroundStyle(type.tint)
                        .padding(.horizontal, 4).padding(.vertical, 1)
                        .background(type.tint.opacity(0.18), in: Capsule())
                }
            }
            Text(sub).font(.caption2).foregroundStyle(Theme.textSecondary).lineLimit(2)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// The source span — the context that makes the question answerable, with the
    /// mention emphasized.
    private var quote: some View {
        highlighted(proposal.subject.snippet, mark: proposal.subject.name)
            .font(.callout).foregroundStyle(Theme.textSecondary)
            .padding(.leading, 12)
            .overlay(alignment: .leading) {
                Capsule().fill(type.tint.opacity(0.5)).frame(width: 3)
            }
    }

    private var footer: some View {
        VStack(alignment: .leading, spacing: 6) {
            if !proposal.rationale.isEmpty {
                Label(proposal.rationale, systemImage: "sparkles")
                    .font(.caption2).foregroundStyle(Theme.textTertiary).lineLimit(2)
            }
            Text(proposal.provenance.label).font(.caption2.weight(.medium)).foregroundStyle(Theme.textTertiary)
        }
    }

    private func highlighted(_ s: String, mark: String) -> Text {
        guard let r = s.range(of: mark, options: .caseInsensitive) else { return Text(s) }
        return Text(String(s[..<r.lowerBound]))
            + Text(String(s[r])).foregroundColor(Theme.textPrimary).bold()
            + Text(String(s[r.upperBound...]))
    }
}

// MARK: - Ambient (forked NoteAmbient, biased to the card's type tint)

private struct ConvergenceAmbient: View {
    var tint: Color
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        Group {
            if reduceMotion { blobs(at: 0) }
            else { TimelineView(.animation) { ctx in blobs(at: ctx.date.timeIntervalSinceReferenceDate) } }
        }
        .blur(radius: 56)
        .animation(.easeInOut(duration: 0.6), value: tint)
    }

    @ViewBuilder private func blobs(at s: Double) -> some View {
        ZStack {
            drift(tint, speed: 0.12, s, phase: 0)                 // the type's hue leads
            drift(tint.opacity(0.7), speed: 0.16, s, phase: 3)
            drift(Color(hex: 0x49A6F2), speed: 0.10, s, phase: 4) // a cool counter-note
            drift(Color(hex: 0x7A6FF0), speed: 0.09, s, phase: 1)
        }
    }

    private func drift(_ c: Color, speed sp: Double, _ s: Double, phase ph: Double) -> some View {
        Circle().fill(c).frame(width: 240, height: 240)
            .offset(x: CGFloat(cos(s * sp + ph)) * 150, y: CGFloat(sin(s * sp * 1.3 + ph)) * 300)
            .opacity(0.4)
    }
}

// MARK: - Preview

#Preview("Convergence — mint / merge deck") {
    ConvergenceView(Proposal.sample)
}
