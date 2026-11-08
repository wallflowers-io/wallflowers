//  Components.swift
//  The small shared design vocabulary: brand accent, the rounded "logo" badge,
//  tag chips, section headers, filter pills, and the quiet on-device trust line.
//  Kept deliberately small and SwiftUI-native.

import SwiftUI

// Brand tokens route through the theme — one edit re-themes the app.
enum Brand {
    static let accent = Theme.accent
    static let card = Theme.surface
    static let groupBG = Theme.bg
}

/// A rounded-square identity badge with initials. Deliberately monochrome —
/// identity is chrome, not a contextual marker, so colour is withheld here (it is
/// spent on enriched entities, status and real media instead).
struct LogoBadge: View {
    var initials: String
    var tint: Color
    var size: CGFloat = 44

    var body: some View {
        Color.clear
            .frame(width: size, height: size)
            .pixelCard(Theme.surface2, in: RoundedRectangle(cornerRadius: size * 0.28, style: .continuous))
            .overlay(
                Text(initials.prefix(2))
                    .font(.system(size: size * 0.38, weight: .semibold, design: .rounded))
                    .foregroundStyle(Theme.textSecondary)
            )
    }
}

/// A small pill describing a tag like "selling: SDK".
struct TagChip: View {
    var text: String
    var emphasized: Bool = false

    var body: some View {
        Text(text)
            .font(.caption2.weight(.medium))
            .lineLimit(1)
            .padding(.horizontal, 9)
            .padding(.vertical, 6)
            .pixelCard(emphasized ? Brand.accent : Theme.fill, in: Capsule(), bevel: false)
            .foregroundStyle(emphasized ? Theme.onAccent : Theme.textSecondary)
    }
}

/// A "warm path" badge: a mutual who can introduce you.
struct WarmPathBadge: View {
    var via: String
    var body: some View {
        Label("via \(via) · 1 intro away", systemImage: "arrow.triangle.branch")
            .font(.caption2.weight(.semibold))
            .foregroundStyle(Brand.accent)
    }
}

/// THE EMPTY WORD — what a surface with nothing on it says: ITS OWN NAME.
///
/// NOTE says NOTE, CHAT says CHAT, and a profile lane with no cards says the word
/// its jewel carries (`EventCategory.title`). Never a sentence about the absence:
/// "Nothing yet" / "No channels" / "No posts yet" were three different ways of
/// saying the same thing on the same page, and none of them named the lane. The
/// New modal's header is deliberately the same type (Design/New.swift) so a mint
/// surface reads as the empty screen it opened from.
///
/// Two scales, both already drawn in the app before this component existed: the
/// SCREEN word (26/2 — NotesView, ChatsView) and the LANE word (18/1.4 —
/// EventsPage's blank lens). This is where they live now.
struct EmptyWord: View {
    let word: String
    var scale: Scale = .screen
    /// Place the word in the space available, a fifth of it above centre — the
    /// whole-surface empty state. Off = the bare word, laid out by the caller
    /// (CHAT's empty-scope row inside a List).
    var placed = true

    enum Scale {
        /// A whole screen with nothing on it.
        case screen
        /// One lane of a profile, or a lens of a page.
        case lane
        var size: CGFloat { self == .screen ? 26 : 18 }
        var tracking: CGFloat { self == .screen ? 2 : 1.4 }
    }

    init(_ word: String, scale: Scale = .screen, placed: Bool = true) {
        self.word = word
        self.scale = scale
        self.placed = placed
    }

    var body: some View {
        if placed {
            GeometryReader { geo in
                text.frame(width: geo.size.width, height: geo.size.height)
                    .offset(y: -geo.size.height * 0.2)   // sit above centre
            }
        } else {
            text
        }
    }

    private var text: some View {
        Text(word)
            .font(.system(size: scale.size, weight: .semibold))
            .tracking(scale.tracking)
            .foregroundStyle(.secondary)
    }
}

struct SectionHeader: View {
    var title: String
    var accessory: String? = nil
    var body: some View {
        HStack(alignment: .firstTextBaseline) {
            // The kicker: uppercase, tracked, gold.
            Text(title)
                .font(.headline)
                .textCase(.uppercase)
                .tracking(2)
                .foregroundStyle(Brand.accent)
            Spacer()
            if let accessory {
                Text(accessory)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(.horizontal)
        .padding(.top, 8)
    }
}

/// Horizontal selectable filter pills: the selected one gold, the rest bedrock.
struct FilterPills: View {
    var options: [String]
    @Binding var selection: String
    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                ForEach(options, id: \.self) { opt in
                    let on = opt == selection
                    Text(opt)
                        .font(.subheadline.weight(.medium))
                        .padding(.horizontal, 14).padding(.vertical, 9)
                        .pixelCard(on ? Brand.accent : Theme.fill, in: Capsule(), bevel: false)
                        .foregroundStyle(on ? Theme.onAccent : Theme.textSecondary)
                        .onTapGesture { selection = opt }
                }
            }
            .padding(.horizontal)
        }
    }
}

/// The quiet on-device reassurance line — never a modal.
struct TrustLine: View {
    var text: String = "On your device · end-to-end encrypted · no server has your data"
    var body: some View {
        Label(text, systemImage: "lock.fill")
            .font(.caption2)
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, alignment: .center)
            .padding(.vertical, 6)
    }
}

/// A labelled section of a profile (Bio / Buying / Selling / Network).
struct ProfileSection<Content: View>: View {
    var title: String
    var systemImage: String
    var hint: String? = nil
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 6) {
                Image(systemName: systemImage).foregroundStyle(Theme.textSecondary)
                Text(title).font(.subheadline.weight(.semibold))
                if let hint {
                    Text(hint).font(.caption2).foregroundStyle(.secondary)
                }
            }
            content
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding()
        .pixelCard(Brand.card)
    }
}

/// A flowing set of chips (used for buying/selling lists).
struct ChipFlow: View {
    var items: [String]
    var emphasized: Bool = false
    var body: some View {
        FlexibleWrap(items: items) { item in
            TagChip(text: item, emphasized: emphasized)
        }
    }
}

/// Minimal wrapping layout for chips (iOS 16+ Layout).
struct FlexibleWrap<Item: Hashable, Content: View>: View {
    var items: [Item]
    @ViewBuilder var content: (Item) -> Content
    var body: some View {
        WrapLayout(spacing: 6) {
            ForEach(items, id: \.self) { content($0) }
        }
    }
}

struct WrapLayout: Layout {
    var spacing: CGFloat = 6
    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let maxW = proposal.width ?? .infinity
        var x: CGFloat = 0, y: CGFloat = 0, rowH: CGFloat = 0
        for v in subviews {
            let s = v.sizeThatFits(.unspecified)
            if x + s.width > maxW { x = 0; y += rowH + spacing; rowH = 0 }
            x += s.width + spacing
            rowH = max(rowH, s.height)
        }
        return CGSize(width: maxW == .infinity ? x : maxW, height: y + rowH)
    }
    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let maxW = bounds.width
        var x: CGFloat = bounds.minX, y: CGFloat = bounds.minY, rowH: CGFloat = 0
        for v in subviews {
            let s = v.sizeThatFits(.unspecified)
            if x - bounds.minX + s.width > maxW { x = bounds.minX; y += rowH + spacing; rowH = 0 }
            v.place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(s))
            x += s.width + spacing
            rowH = max(rowH, s.height)
        }
    }
}
