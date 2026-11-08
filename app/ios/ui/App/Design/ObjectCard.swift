//  ObjectCard.swift
//  THE card (FEED-TRANSMISSION 0-ANSWER-G S28/S34) — ONE component, inherited by
//  every GroupObject kind. The Instagram frame: a header LINE above the square
//  (author leading, location trailing, the provenance mark as the avatar slot),
//  the SQUARE (media when the item carries it, dark ground otherwise, the title
//  as caption), and the footer line BELOW, leading. Cards breathe — spacing lives
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

    var body: some View {
        VStack(alignment: .leading, spacing: DesignTokens.Card.vstackSpacing) {
            HStack(alignment: .firstTextBaseline, spacing: DesignTokens.Card.headerGap) {
                provenanceMark
                    .frame(width: DesignTokens.Card.provenanceDot, height: DesignTokens.Card.provenanceDot)
                    .alignmentGuide(.firstTextBaseline) { d in d[VerticalAlignment.center] + 3 }
                if !row.author.isEmpty {
                    if let route = row.authorRoute, let onAuthor {
                        Button { onAuthor(route) } label: {
                            Text(row.author)
                                .font(.system(size: DesignTokens.Card.authorSize, weight: .semibold))
                                .lineLimit(1)
                        }
                        .buttonStyle(.plain)
                        .accessibilityIdentifier("card-author")
                    } else {
                        Text(row.author)
                            .font(.system(size: DesignTokens.Card.authorSize, weight: .semibold))
                            .lineLimit(1)
                    }
                }
                Spacer(minLength: 12)
                if !row.location.isEmpty {
                    Text(row.location)
                        .font(.system(size: DesignTokens.Card.locationSize))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            .padding(.horizontal, DesignTokens.Card.lineInset)

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
                    .padding(DesignTokens.Card.titleInset)
            }
            .aspectRatio(1, contentMode: .fit)
            .clipShape(square)
            .overlay(square.strokeBorder(Theme.edge, lineWidth: DesignTokens.Card.hairline))
            .background(square.fill(Theme.edge).offset(x: DesignTokens.Skin.shadow, y: DesignTokens.Skin.shadow))

            if !row.footer.isEmpty {
                Text(row.footer)
                    .font(.system(size: DesignTokens.Card.footerSize))
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, DesignTokens.Card.lineInset)
            }
        }
        .padding(.horizontal, DesignTokens.Life.gutter)
    }

    private var square: RoundedRectangle {
        RoundedRectangle(cornerRadius: DesignTokens.Skin.radius, style: .continuous)
    }

    @ViewBuilder private var provenanceMark: some View {
        switch row.provenance {
        case .member:
            Circle().fill(Color.primary)
        case .hops:
            Circle().strokeBorder(style: StrokeStyle(lineWidth: 1.4, dash: [2, 2]))
                .foregroundStyle(.secondary)
        case .hydrated:
            Circle().strokeBorder(lineWidth: 1.4)
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
