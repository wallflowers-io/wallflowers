//  ObjectCard.swift
//  THE card (FEED-TRANSMISSION 0-ANSWER-G S28/S34) — ONE component, inherited by
//  every GroupObject kind. The Instagram frame: a header above the square (the
//  author's avatar, ringed by provenance, their name and the place leading; the
//  kind's glyph trailing),
//  the SQUARE (media when the item carries it, dark ground otherwise, the title
//  as caption), and the footer line BELOW: the footer leading, how long ago the
//  object was made trailing. Cards breathe — spacing lives
//  in the owning stack.
//
//  The model is `FeedItem`, which carries its `category` — so a card always knows
//  WHICH kind of object it is, and the tap contract is uniform everywhere: TAP =
//  INSTANCE DETAIL (the object's own page), routed by kind at the call site
//  (Life.openObject). Browsing a KIND belongs to the jewel rail, never to a card.
//
//  Adopters: FeedView (the Active Scroll Feed) · TypedListPage (every maximised
//  jewel). Add adopters by reusing THIS view — never by re-implementing the frame
//  ([[reuse-dont-copy-components]]).

import SwiftUI
import UIKit

struct ObjectCard: View {
    let row: FeedItem
    /// The author DOOR (11 Aug): tapping the header name opens the author's
    /// profile — person or Space — routed at the call site like every card tap.
    /// nil handler or nil route = plain text, never a dead button.
    var onAuthor: ((AuthorRoute) -> Void)? = nil
    @EnvironmentObject private var core: CoreService

    var body: some View {
        VStack(alignment: .leading, spacing: DesignTokens.Card.vstackSpacing) {
            // ABOVE THE CARD: who made it on the left (avatar, name, and the place under
            // the name), what it is on the right, as its glyph alone.
            HStack(alignment: .center, spacing: DesignTokens.Card.headerGap) {
                if !row.author.isEmpty {
                    avatar
                }
                VStack(alignment: .leading, spacing: 1) {
                    if !row.author.isEmpty { author }
                    if !row.location.isEmpty {
                        Text(row.location)
                            .font(.system(size: DesignTokens.Card.locationSize))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                }
                Spacer(minLength: 12)
                if let kind {
                    Image(systemName: kind.glyph)
                        .font(.system(size: DesignTokens.Card.kindSize, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .accessibilityLabel(kind.noun)
                        .accessibilityIdentifier("card-kind-\(kind.rawValue)")
                }
            }
            .padding(.horizontal, DesignTokens.Card.lineInset)
            // The title says all of this aloud (`spoken`).
            .accessibilityHidden(true)

            // The title's ink depends on its ground: white over media (the scrim
            // guarantees it in both keys), themed ink over the bare card.
            let banner = BannerCache.shared.image(id: row.id, b64: row.bannerB64)
            ZStack(alignment: .bottomLeading) {
                if let ui = banner {
                    Color.clear
                        .overlay(Image(uiImage: ui).resizable().scaledToFill())
                        .clipped()
                    LinearGradient(colors: [.black.opacity(0.72), .clear],
                                   startPoint: .bottom, endPoint: .center)
                } else {
                    Rectangle().fill(Theme.fill)
                }
                Text(row.title)
                    .font(.system(size: DesignTokens.Card.titleSize, weight: .semibold))
                    .foregroundStyle(banner == nil ? Theme.textPrimary : Color.white)
                    .lineLimit(2)
                    .accessibilityLabel(spoken)
                    .padding(DesignTokens.Card.titleInset)
            }
            .aspectRatio(1, contentMode: .fit)
            .clipShape(square)
            .overlay(square.strokeBorder(Theme.edge, lineWidth: DesignTokens.Card.hairline))
            .background(square.fill(Theme.edge).offset(x: DesignTokens.Skin.shadow, y: DesignTokens.Skin.shadow))

            // BELOW: the footer leading (an event's date, a listing's posture), and how
            // long ago the object was made trailing.
            if !row.footer.isEmpty || row.createdAt != nil {
                HStack {
                    Text(row.footer)
                    Spacer(minLength: 12)
                    if let made = row.createdAt {
                        Text(FeedAssembler.recency(made, nowMs: Int64(Date().timeIntervalSince1970 * 1000)))
                            .accessibilityIdentifier("card-recency")
                    }
                }
                .font(.system(size: DesignTokens.Card.footerSize))
                .foregroundStyle(.secondary)
                .padding(.horizontal, DesignTokens.Card.lineInset)
            }
        }
        .padding(.horizontal, DesignTokens.Life.gutter)
        // The author's door is an action on the card, not a button lost inside it.
        .accessibilityActions {
            if let route = row.authorRoute, let onAuthor {
                Button(row.author) { onAuthor(route) }
            }
        }
    }

    /// What the title says aloud: the card's title first, then what it is, who made it
    /// and where. The header above is hidden from VoiceOver, as this already says it.
    private var spoken: String {
        [row.title, kind?.noun ?? "", row.author.isEmpty ? "" : "by \(row.author)", row.location]
            .filter { !$0.isEmpty }
            .joined(separator: ", ")
    }

    private var square: RoundedRectangle {
        RoundedRectangle(cornerRadius: DesignTokens.Skin.radius, style: .continuous)
    }

    /// The author's name: a door to their profile where the row has one.
    @ViewBuilder private var author: some View {
        let name = Text(row.author)
            .font(.system(size: DesignTokens.Card.authorSize, weight: .semibold))
            .lineLimit(1)
        if let route = row.authorRoute, let onAuthor {
            Button { onAuthor(route) } label: { name }
                .buttonStyle(.plain)
                .accessibilityIdentifier("card-author")
        } else {
            name.accessibilityIdentifier("card-author")
        }
    }

    /// The author's face, from the index: their photo, else their colour, else the
    /// colour their key derives. Its ring is the provenance mark.
    private var avatar: some View {
        let id = row.authorID ?? ""
        return Avatar(initials: Avatar.initials(row.author), tint: Theme.fill,
                      size: DesignTokens.Card.avatar,
                      photo: id.isEmpty ? nil : core.index.photo(id),
                      color: id.isEmpty ? nil
                          : core.index.color(id) ?? Color(hex: CoreService.derivedAvatarColorHex(forIdentityKey: id)))
            .overlay(provenanceRing)
            .accessibilityIdentifier("card-avatar")
    }

    @ViewBuilder private var provenanceRing: some View {
        switch row.provenance {
        case .member:
            Circle().strokeBorder(Color.primary, lineWidth: DesignTokens.Card.ring)
        case .hops:
            Circle().strokeBorder(style: StrokeStyle(lineWidth: DesignTokens.Card.ring, dash: [2, 2]))
                .foregroundStyle(.secondary)
        case .hydrated:
            Circle().strokeBorder(.secondary, lineWidth: DesignTokens.Card.ring)
        }
    }

    /// The kind the card shows, as the creator names it (the same glyph as RED's dial).
    private var kind: NewKind? {
        switch row.category {
        case .events:   return .event
        case .posts:    return .post
        case .trade:    return .listing
        case .places:   return .place
        case .projects: return .project
        case .groups:   return .space
        case .channels: return .channel
        case .people, .things, .job: return nil
        }
    }
}

/// Decoded banners, cached by row id — the b64 rides the fold (EventRow.banner);
/// decoding once per card, not per render, is what keeps the scroll at 60.
final class BannerCache {
    static let shared = BannerCache()
    private let cache = NSCache<NSString, UIImage>()

    func image(id: String, b64: String) -> UIImage? {
        guard !b64.isEmpty else { return nil }
        if let hit = cache.object(forKey: id as NSString) { return hit }
        guard let data = Data(base64Encoded: b64), let ui = UIImage(data: data) else { return nil }
        cache.setObject(ui, forKey: id as NSString)
        return ui
    }
}
