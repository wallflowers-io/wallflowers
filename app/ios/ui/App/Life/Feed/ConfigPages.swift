//  ConfigPages.swift
//  The maximised state of every jewel (FEED-TRANSMISSION 0-ANSWER-G, S27): a flat
//  list of that type's GroupObjects from your graph, rendered with the SAME card
//  the feed uses (S28). Two modes only — this list, or the feed.

import SwiftUI
import PacificCore

struct TypedListPage: View {
    @EnvironmentObject var core: CoreService
    let category: EventCategory
    var onOpen: (FeedItem) -> Void
    var onAuthor: ((AuthorRoute) -> Void)? = nil

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 0) {
                ForEach(items) { item in
                    Group {
                        // PEOPLE and GROUPS are DIRECTORIES (11 Aug order): the
                        // row is the component's collapsed profile bar — the
                        // collapsed headline IS the directory row — never the
                        // square media card.
                        if category == .people || category == .groups {
                            DirectoryBar(item: item, kind: category == .people ? "PERSON" : "SPACE")
                        } else {
                            ObjectCard(row: item, onAuthor: onAuthor)
                        }
                    }
                    .contentShape(Rectangle())
                    .onTapGesture { onOpen(item) }
                    // One addressable element per card (see FeedView's row) —
                    // XCUITest cannot query the identifier on a bare composite.
                    .accessibilityElement(children: .combine)
                    // Tapping it opens the object: it is a button to VoiceOver and to a test.
                    .accessibilityAddTraits(.isButton)
                    .accessibilityIdentifier("list-row-\(item.id)")
                }
                if items.isEmpty {
                    Text("—")
                        .foregroundStyle(.secondary)
                        .padding(.top, 120)
                }
            }
            .padding(.top, RibbonWidth.contracted + 40)
        }
        .accessibilityIdentifier("typed-list-\(category.rawValue)")
    }

    private var items: [FeedItem] {
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        switch category {
        case .groups:
            // SPACES, not every Group object: an identity record is a person and
            // belongs to PEOPLE.
            return core.spaces.map {
                FeedItem(id: "group:\($0.id)", category: .groups, title: $0.name,
                         author: "", location: "", footer: "",
                         whenMs: 0, provenance: .member, objectID: $0.id,
                         createdAt: core.index.createdAt(of: $0.id))
            }
        case .events:
            return FeedAssembler.rows(events: core.events, listings: [], nowMs: now,
                                      makerOf: core.index.maker(of:),
                                      createdOf: core.index.createdAt(of:))
        case .places:
            return core.places.map {
                FeedItem(id: "place:\($0.id)", category: .places, title: $0.name,
                         author: "", location: $0.label, footer: "",
                         whenMs: 0, provenance: .member, objectID: $0.id,
                         createdAt: core.index.createdAt(of: $0.id))
            }
        case .people:
            return core.connectionRows().map {
                FeedItem(id: "person:\($0.peerId)", category: .people,
                         title: $0.displayName.isEmpty ? String($0.peerId.prefix(8)) : $0.displayName,
                         author: "", location: "", footer: "",
                         whenMs: 0, provenance: .member, objectID: $0.peerId)
            }
        case .trade:
            return FeedAssembler.rows(events: [], listings: core.listings, nowMs: now)
        // CHANNELS lists nothing HERE: a channel belongs to a Space, so its list
        // is the Space profile's lane, not a global typed list. Empty is the
        // honest answer rather than a flattened cross-Space roll-up.
        case .job, .projects, .things, .posts, .channels:
            return []
        }
    }
}

/// The collapsed profile bar as a directory row: icon left, name centre-lead,
/// kind trailing — the Profile component's 64pt collapsed-headline grammar.
/// People rows carry the peer's published face; Space rows the group's icon.
/// (Unify with Design/Profile's PinBar when it gains a photo slot.)
private struct DirectoryBar: View {
    @EnvironmentObject private var core: CoreService
    let item: FeedItem
    let kind: String

    var body: some View {
        HStack(spacing: 12) {
            Avatar(initials: Avatar.initials(item.title), tint: .clear, size: 36,
                   photo: photo, color: color)
            Text(item.title)
                .font(.system(size: 15, weight: .semibold))
                .lineLimit(1)
            Spacer(minLength: 12)
            Text(kind)
                .font(.system(size: 10, weight: .semibold, design: .monospaced))
                .kerning(1.2)
                .foregroundStyle(.secondary)
        }
        .padding(.horizontal, 14)
        .frame(height: 64)
        .background(RoundedRectangle(cornerRadius: Sig.modalRadius, style: .continuous)
            .fill(Theme.surface))
        .overlay(RoundedRectangle(cornerRadius: Sig.modalRadius, style: .continuous)
            .stroke(Theme.hairline))
        .padding(.vertical, 4)
    }

    private var photo: Data? {
        guard let oid = item.objectID else { return nil }
        if kind == "PERSON" { return core.index.photo(oid) }
        guard let row = core.orgs.first(where: { $0.id == oid }), !row.photo.isEmpty else { return nil }
        return Data(base64Encoded: row.photo)
    }

    private var color: Color? {
        guard let oid = item.objectID else { return nil }
        if kind == "PERSON" { return core.index.color(oid) }
        guard let row = core.orgs.first(where: { $0.id == oid }), !row.avatarColor.isEmpty else { return nil }
        return Color(hex: row.avatarColor)
    }
}
