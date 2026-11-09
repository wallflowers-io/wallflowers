//  ProfileCards.swift
//  The profile's secondary cards: bio (person), info (space), map (located
//  space), and the pin bar.
//
//  BIO AND INFO ARE BANDS, NOT CARDS (26 Aug). They were rounded rectangles on
//  the card grid; they are now lightly shaded regions running the FULL screen
//  width, sized to their own words. A region does not need a border to be a
//  region, and the prose reads better against the page than inside a plaque.
//  The map card and the pin bar keep Sig.modalRadius and the aspect contract
//  (map 2.5:0.8 · bar 64pt).
//
//  The pin bar is NOT its own row implementation — it is `ConversationRow`
//  (Design/Signal.swift, the one identity bar) worn thin: collapsed headline ≡
//  directory row ≡ pin bar, one component, three uses.

import SwiftUI

// MARK: - Bio (person)

/// The bio BAND. Full screen width, lightly shaded, no corners and no border —
/// so it no longer takes the profile's deep tone as a ground, and the words are
/// ordinary page text rather than white-on-colour.
struct BioCard: View {
    let text: String
    /// The gutter the CONTENT keeps. The band itself ignores it and runs edge to
    /// edge; the words stay lined up with the cards above.
    var inset: CGFloat = DesignTokens.Life.gutter

    var body: some View {
        Text(text)
            .font(.system(size: 14.5))
            .foregroundStyle(Theme.textPrimary)
            .frame(maxWidth: .infinity, alignment: .topLeading)
            .padding(.horizontal, inset)
            .padding(.vertical, DesignTokens.Profile.bandPad)
            .background(Theme.surface)
            .accessibilityIdentifier("profile-bio")
    }
}

// MARK: - Info (space)

/// The Space's bio band — the descriptor, with whatever stats can honestly be
/// stated beneath it. Same band as `BioCard`; a Space's is the one that can also
/// carry values.
struct InfoCard: View {
    let info: ProfileInfo
    var inset: CGFloat = DesignTokens.Life.gutter

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if !info.desc.isEmpty {
                Text(info.desc)
                    .font(.system(size: 14.5))
                    .foregroundStyle(Theme.textPrimary)
            }
            // Honest stats: an absent value is an absent stat, never "—".
            HStack(spacing: 18) {
                if let hereNow = info.hereNow {
                    ProfileStat(dot: Theme.ok, value: hereNow, label: "here now")
                }
                if let since = info.since {
                    ProfileStat(dot: nil, value: since, label: "since", flip: true)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .topLeading)
        .padding(.horizontal, inset)
        .padding(.vertical, DesignTokens.Profile.bandPad)
        .background(Theme.surface)
        .accessibilityIdentifier("profile-info")
    }
}

/// One stat on the info card — dot · value · label (`flip` reverses the pair).
struct ProfileStat: View {
    let dot: Color?
    let value: String, label: String
    var flip = false

    var body: some View {
        HStack(spacing: 5) {
            if let dot { Circle().fill(dot.opacity(0.8)).frame(width: 7, height: 7) }
            if flip {
                Text(label).font(.system(size: 12, design: .monospaced))
                    .foregroundStyle(Theme.textSecondary)
                Text(value).font(.system(size: 12, weight: .semibold, design: .monospaced))
                    .foregroundStyle(Theme.textPrimary)
            } else {
                Text(value).font(.system(size: 12, weight: .semibold, design: .monospaced))
                    .foregroundStyle(Theme.textPrimary)
                Text(label).font(.system(size: 12, design: .monospaced))
                    .foregroundStyle(Theme.textSecondary)
            }
        }
    }
}

// MARK: - Map (located space)

/// The space's location card: geohash area cells over a quiet grid texture.
/// Cells arrive as unit fractions; the card multiplies them by its own frame,
/// so the binder never learns the card's size.
struct MapCard: View {
    let cells: [ProfileGeoCell]
    /// The profile's block colour — the cells wear it translucent.
    let color: Color
    let width: CGFloat
    /// nil = the profile's card aspect; a detail's square hero passes its own.
    var tall: CGFloat? = nil

    private var height: CGFloat { tall ?? ProfileGeometry.infoHeight(width: width) }

    var body: some View {
        ZStack {
            Theme.surface
            ProfileGridTexture()
            // The one "road" band — texture, not cartography.
            Rectangle().fill(Color(red: 0.47, green: 0.59, blue: 0.67).opacity(0.16))
                .frame(width: width * 1.2, height: height * 0.09)
                .rotationEffect(.degrees(-4))
                .offset(y: height * 0.10)
            ForEach(cells) { c in
                RoundedRectangle(cornerRadius: 8)
                    .fill(color.opacity(0.38))
                    .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(color.opacity(0.7)))
                    .overlay(alignment: .bottomLeading) {
                        Text(c.hash)
                            .font(.system(size: 10.5, design: .monospaced))
                            .foregroundStyle(.white)
                            .shadow(color: .black.opacity(0.8), radius: 3)
                            .padding(6)
                    }
                    .frame(width: c.w * width, height: c.h * height)
                    .position(x: (c.x + c.w / 2) * width, y: (c.y + c.h / 2) * height)
            }
        }
        .frame(width: width, height: height)
        .clipShape(ProfileGeometry.shape)
        .overlay(ProfileGeometry.shape.strokeBorder(Theme.hairline))
        .accessibilityIdentifier("profile-map")
    }
}

struct ProfileGridTexture: View {
    var body: some View {
        Canvas { ctx, size in
            var col = Path(); var row = Path()
            stride(from: 0.0, to: size.width, by: 34).forEach { x in
                col.move(to: .init(x: x, y: 0)); col.addLine(to: .init(x: x, y: size.height))
            }
            stride(from: 0.0, to: size.height, by: 26).forEach { y in
                row.move(to: .init(x: 0, y: y)); row.addLine(to: .init(x: size.width, y: y))
            }
            ctx.stroke(col, with: .color(.white.opacity(0.045)))
            ctx.stroke(row, with: .color(.white.opacity(0.045)))
        }
    }
}

// MARK: - Pin bar

/// A pinned bar — the collapsed-state unification made literal: it IS
/// `ConversationRow` (the app's one identity bar) worn thin inside the 64pt
/// card. Navigation is the caller's: the stack wraps it in a NavigationLink
/// when the pin carries a destination.
struct PinBar: View {
    let pin: ProfilePin
    /// The disc colour when the pin doesn't carry its own.
    let fallback: Color
    let width: CGFloat

    var body: some View {
        ConversationRow(title: pin.title,
                        snippet: "",
                        time: pin.kind.label,
                        tint: pin.color ?? fallback,
                        initials: pin.initials,
                        blockColor: pin.color ?? fallback,
                        enrichSnippet: false,
                        avatarSize: 36)
            .padding(.horizontal, 14)
            .frame(width: width, height: ProfileGeometry.bar)
            .background(Theme.surface)
            .clipShape(ProfileGeometry.shape)
            .overlay(ProfileGeometry.shape.strokeBorder(Theme.hairline))
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier(pin.accessibilityID ?? "profile-pin-\(pin.id)")
    }
}
