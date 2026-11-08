//  DetailSurface.swift
//  The detail modal — media, whose it is, what people said.
//
//  IT INHERITS THE NEWITEM SURFACE and re-rolls none of it: the same
//  `ComposeGround`, the same `ComposeGlass` card, the same departure curves, and
//  the shell's own three circles as its buttons (it is a `ShellSurface`, so it is
//  hosted UNDER the disc row rather than presented over it). What it does NOT
//  inherit is the mint's body — a form of declared fields — because this surface
//  is not making anything.
//
//  RED EDGE GLOW. A mint is cast in the colour of the kind it mints, which is how
//  you can tell a New Event from a New Space before reading a word. Nothing is
//  being minted here, so the card wears the RED channel's own colour — the same
//  red as the disc that closes it. One glance says "this is a thing you are
//  looking at", not "this is a thing you are making".
//
//  WHY THE COMMENTS ARE FLAT. The object's log carries a real threaded projection
//  (`ForumThreadRow.depth`, what ThreadView renders). This shows it flat, newest
//  last, because a nesting tree is a ROOM's information architecture and this is a
//  detail screen with comments under it. Someone who wants the room can open it.

import SwiftUI

struct DetailSurface: View {
    @ObservedObject var session: DetailSession
    /// Close without keeping. Routed to the SAME action RED takes, so tapping the
    /// ground and tapping the disc can never disagree about what closing means.
    var onClose: () -> Void

    @EnvironmentObject private var shell: ShellState
    @FocusState private var typing: Bool
    @State private var cardFrame: CGRect = .zero
    @State private var travel: CGFloat = 900

    private var exit: NewExit? { session.exit }

    /// THE RED EDGE GLOW — the NOTE channel's colour, which is also the colour of
    /// the disc that closes this. Read from `DiscSlot` rather than written as a
    /// hex here, so a repalette moves the modal with the discs.
    private var tint: Color { DiscSlot.note.blockColor }
    private var glow: Color { DiscSlot.note.blockColor.darkened(0.3) }

    var body: some View {
        ZStack {
            // Tap off the canvas to close — the same action RED takes (Ralph,
            // 18 Aug: one behaviour for every surface).
            ComposeGround(tint: tint, opaque: true, canvas: cardFrame, onTapOutside: onClose)
                // The ground goes FIRST and fast, so the screen underneath is back
                // before the card in front of it has finished leaving.
                .opacity(exit == nil ? 1 : 0)
                .animation(Motion.noteGroundOut, value: exit)

            card
                .opacity(exit == nil ? 1 : 0)
                .offset(y: exit == .shared ? -travel : 0)
                .animation(Motion.noteFade, value: exit)
                .padding(.horizontal, 16)
                .padding(.top, ShellMetrics.searchFieldTop)
                // Stop above the disc row — those circles ARE this surface's buttons.
                .padding(.bottom, ShellMetrics.bottomBarHeight + 6)
                .background(GeometryReader { geo in
                    Color.clear.onAppear { travel = geo.size.height * 1.4 }
                })
        }
        .allowsHitTesting(exit == nil)
        .task { await session.load() }
        .onChange(of: exit) { _, leaving in if leaving != nil { typing = false } }
        .accessibilityIdentifier("detail-surface")
    }

    // MARK: The glass card

    private var card: some View {
        VStack(spacing: 0) {
            if session.sharing {
                shareFace
            } else {
                detailFace
            }
        }
        .frame(maxWidth: .infinity)
        .modifier(ComposeGlass(tint: tint, glow: glow, flash: flash, opaque: true))
        .background(GeometryReader { geo in
            Color.clear.preference(key: DetailCardFrameKey.self,
                                   value: geo.frame(in: .global))
        })
        .onPreferenceChange(DetailCardFrameKey.self) { cardFrame = $0 }
    }

    private var flash: Color? {
        switch exit {
        case .saved:  tint
        case .shared: Color(hex: 0x49A6F2)      // the ambient's azure
        default:      nil
        }
    }

    // MARK: The three things

    private var detailFace: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                media
                facts
                if let author = session.author { authorBar(author) }
                comments
            }
        }
        .scrollIndicators(.hidden)
        .safeAreaInset(edge: .bottom) {
            if session.takesComment { composer }
        }
    }

    /// MEDIA — the object's picture, square, edge to edge inside the glass. The
    /// stripes when there is none: it says THERE IS NO PICTURE, which is a fact,
    /// rather than implying one is on its way.
    @ViewBuilder private var media: some View {
        let shape = UnevenRoundedRectangle(topLeadingRadius: Sig.modalRadius,
                                           bottomLeadingRadius: 0, bottomTrailingRadius: 0,
                                           topTrailingRadius: Sig.modalRadius,
                                           style: .continuous)
        Color.clear
            .aspectRatio(1, contentMode: .fit)
            .frame(maxWidth: .infinity)
            .overlay {
                if let bytes = session.headline?.photo, let image = UIImage(data: bytes) {
                    Image(uiImage: image).resizable().scaledToFill()
                        .accessibilityIdentifier("detail-media")
                } else if let url = session.headline?.photoURL {
                    PosterImage(url: url, imageID: "detail-media") { FlyerStripes() }
                } else {
                    FlyerStripes().accessibilityIdentifier("detail-media")
                }
            }
            .clipShape(shape)
    }

    /// The title, then the facts printed under it — values only, in the order the
    /// binder gave them. No captions: a value that needs one is the wrong value.
    private var facts: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(session.headline?.title ?? "")
                .font(.system(size: 22, weight: .heavy))
                .tracking(-0.2)
                .foregroundStyle(Theme.textPrimary)
                .accessibilityIdentifier("detail-title")
            ForEach(Array((session.headline?.facts ?? []).enumerated()), id: \.offset) { _, fact in
                Text(fact)
                    .font(.system(size: 14))
                    .foregroundStyle(Theme.textSecondary)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 18)
        .padding(.top, 16)
    }

    /// AUTHOR — a door to their profile, which is where the egoic IA lives. It
    /// closes this modal first: the profile is a page in LIFE's stack, and a route
    /// fired into a surface that is still leaving is silently dropped.
    private func authorBar(_ author: DetailAuthor) -> some View {
        Button {
            let id = author.id, isSpace = author.isSpace
            onClose()
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.4) {
                NotificationCenter.default.post(
                    name: isSpace ? .pacificOpenGroup : .pacificOpenPerson,
                    object: nil, userInfo: ["object": id])
            }
        } label: {
            ConversationRow(title: author.name,
                            snippet: "",
                            time: author.isSpace ? "SPACE" : "PERSON",
                            tint: author.color ?? tint,
                            initials: author.initials,
                            blockColor: author.color ?? tint,
                            enrichSnippet: false,
                            avatarSize: 36)
                .padding(.horizontal, 18)
                .frame(height: ProfileGeometry.bar)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("detail-author")
    }

    /// COMMENTS — flat, oldest first. An object nobody has spoken about shows
    /// nothing at all rather than an empty-state telling you to be the first.
    @ViewBuilder private var comments: some View {
        if !session.comments.isEmpty {
            VStack(alignment: .leading, spacing: 14) {
                ForEach(session.comments) { comment in
                    HStack(alignment: .top, spacing: 10) {
                        Avatar(initials: comment.initials,
                               tint: comment.color ?? tint,
                               size: 28,
                               color: comment.color ?? tint)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(comment.author)
                                .font(.system(size: 12, weight: .semibold))
                                .foregroundStyle(Theme.textSecondary)
                            Text(comment.body)
                                .font(.system(size: 14.5))
                                .foregroundStyle(Theme.textPrimary)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        Spacer(minLength: 0)
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 18)
            .padding(.top, 18)
            .padding(.bottom, 8)
            .accessibilityIdentifier("detail-comments")
        }
    }

    /// The one field on the surface. BLUE sends it — there is no send button here,
    /// because the disc IS the send button.
    private var composer: some View {
        TextField("", text: $session.draft, axis: .vertical)
            .lineLimit(1...4)
            .font(.system(size: 15))
            .foregroundStyle(Theme.textPrimary)
            .focused($typing)
            .padding(.horizontal, 14)
            .padding(.vertical, 11)
            .background(RoundedRectangle(cornerRadius: 14, style: .continuous)
                .fill(Theme.fill.opacity(0.6)))
            .padding(.horizontal, 18)
            .padding(.bottom, 16)
            .padding(.top, 8)
            .background(.clear)
            .accessibilityIdentifier("detail-comment-field")
    }

    // MARK: The share face — GOLD's turn

    /// Where this can go: the Spaces you are actually in. The card TURNS to it, the
    /// same move the mint's audience dial makes, rather than raising a menu over
    /// itself — a menu would cover the discs that opened it.
    private var shareFace: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("SHARE TO")
                .font(.system(size: 11, weight: .semibold, design: .monospaced))
                .kerning(1.4)
                .foregroundStyle(Theme.textTertiary)
                .padding(.horizontal, 18)
                .padding(.top, 20)
                .padding(.bottom, 10)
            ScrollView {
                VStack(spacing: 0) {
                    ForEach(session.targets) { target in
                        Button {
                            Task { await session.share(to: target) }
                        } label: {
                            ConversationRow(title: target.name,
                                            snippet: "",
                                            time: "SPACE",
                                            tint: tint,
                                            initials: Avatar.initials(target.name),
                                            blockColor: tint,
                                            enrichSnippet: false,
                                            avatarSize: 32)
                                .padding(.horizontal, 18)
                                .frame(height: ProfileGeometry.bar)
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
            .scrollIndicators(.hidden)
            .padding(.bottom, 12)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityIdentifier("detail-share-face")
    }
}

/// The card's live frame, reported up so the ground's bloom can anchor to it.
private struct DetailCardFrameKey: PreferenceKey {
    static var defaultValue: CGRect = .zero
    static func reduce(value: inout CGRect, nextValue: () -> CGRect) {
        let next = nextValue()
        if next != .zero { value = next }
    }
}
