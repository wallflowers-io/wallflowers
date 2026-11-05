//  Signal.swift
//  The Signal-derived layout vocabulary for Pacific's rebuild. We keep Pacific's
//  teal `Brand.accent` (it plays Signal's ultramarine STRUCTURAL role — unread
//  pill, outgoing bubble, links, selected pill) but reproduce Signal's geometry:
//  circular avatars, the conversation-list row, incoming/outgoing bubble fills +
//  tail, the composer input bar, and the bottom-trailing compose FAB.
//
//  Deliberately layout-only + SwiftUI-native. Text that can be enriched routes
//  through `EnrichedText` (see App/Enrichment) so matched entities light up.

import SwiftUI
import PacificCore
#if canImport(UIKit)
import UIKit
#endif

/// Signal's colour + metric tokens, mapped onto Pacific's accent.
enum Sig {
    // Bubbles — monochrome slate; colour inside a bubble is the enriched words.
    static var outgoingBubble: Color { Theme.bubbleOutgoing }
    static let outgoingText = Theme.textPrimary
    static let incomingBubble = Theme.bubbleIncoming
    static let incomingText = Theme.textPrimary
    static let bubbleRadius: CGFloat = DesignTokens.Chat.bubbleRadius
    static let bubbleTail: CGFloat = DesignTokens.Chat.bubbleTail   // tightened corner on the sender edge when grouped

    // List
    static let rowAvatar: CGFloat = 56
    static let rowSpacing: CGFloat = 12
    static let rowVPad: CGFloat = 5
    /// The row MARK's size — 80% of the title's `.headline` (17pt), so an
    /// app-written aside sits visibly under the name it follows.
    static let markSize: CGFloat = 13.6

    // Message spacing
    static let gapDistinct: CGFloat = DesignTokens.Chat.gapDistinct
    static let gapGrouped: CGFloat = DesignTokens.Chat.gapGrouped
    static let gapSystem: CGFloat = 20

    /// The Note-modal geometry token — the compose card's corner radius
    /// (always used with `.continuous`). Profile cards share it.
    static let modalRadius: CGFloat = DesignTokens.Profile.radius
}

/// A circular avatar with initials — Signal's row/thread avatar (vs the rounded
/// square `LogoBadge` used in Work/Me).
struct Avatar: View {
    var initials: String
    var tint: Color
    var size: CGFloat = Sig.rowAvatar
    /// An optional profile photo (JPEG/PNG bytes). When set it fills the disc,
    /// circularly clipped — the self avatar passes this; list rows leave it nil.
    var photo: Data? = nil
    /// The block-colour disc fill used when there's no photo. Nil keeps the
    /// monochrome `surface2` + hairline chip (list rows / the neutral fallback);
    /// a colour makes the disc a solid block (the self avatar), initials in white.
    var color: Color? = nil

    var body: some View {
        ZStack {
            if let photo, let ui = UIImage(data: photo) {
                Image(uiImage: ui).resizable().scaledToFill()
            } else if let color {
                color   // a solid block-colour disc; initials only if supplied
                if !initials.isEmpty {
                    Text(initials.prefix(2))
                        .font(.system(size: size * 0.38, weight: .semibold, design: .rounded))
                        .foregroundStyle(.white)
                }
            } else {
                Theme.surface2
                Text(initials.prefix(2))
                    .font(.system(size: size * 0.38, weight: .semibold, design: .rounded))
                    .foregroundStyle(Theme.textSecondary)
            }
        }
        .frame(width: size, height: size)
        .clipShape(Circle())
        .overlay(Circle().strokeBorder(Theme.edge, lineWidth: DesignTokens.Skin.edge))
    }

    /// Initials from a display name / title.
    static func initials(_ name: String) -> String {
        let words = name.split(whereSeparator: { $0 == " " || $0 == "," })
        guard let f = words.first else { return "?" }
        if words.count >= 2 { return (f.prefix(1) + words[1].prefix(1)).uppercased() }
        return f.prefix(2).uppercased()
    }
}

/// A fully-round accent unread pill (count).
struct UnreadPill: View {
    var count: Int
    var body: some View {
        if count > 0 {
            Text("\(count)")
                .font(.footnote.weight(.semibold))
                .foregroundStyle(Theme.onAccent)
                .padding(.horizontal, 7)
                .frame(minWidth: 22, minHeight: 22)
                .pixelCard(Brand.accent, in: Capsule(), bevel: false)
        }
    }
}

/// The Signal conversation-list row: 56pt circular avatar, title (Headline) +
/// trailing timestamp, a 2-line snippet, and a trailing unread pill.
struct ConversationRow: View {
    var title: String
    var snippet: String
    var time: String
    var unread: Int = 0
    var tint: Color
    var initials: String? = nil
    /// The peer's PUBLISHED avatar, when they have sent us one — their photo bytes
    /// and their chosen block colour. Both nil keeps the neutral initials disc, so a
    /// contact who has published nothing looks exactly as it did before.
    var photo: Data? = nil
    var blockColor: Color? = nil
    /// When true the snippet renders through EnrichedText (entities light up).
    var enrichSnippet: Bool = true
    /// Present ⇒ this is a Group-hosted FORUM, and this is the hosting Group's
    /// name. The row wears the forum's distinct dress: the room glyph instead of
    /// initials, and the title reads "Host … / Room" — the host truncating first,
    /// the room name never (it is what you tap for).
    var host: String? = nil
    /// The disc's diameter. The list rows keep the 56pt default; the Profile
    /// PinBar is the same row worn thin (36pt inside the 64pt bar).
    var avatarSize: CGFloat = Sig.rowAvatar
    /// Draw the leading disc at all? A CHANNEL has no face — there is no icon op
    /// for a Forum, and nobody can give one — so a circle of initials on a room
    /// bar invents an identity the object cannot have. Off, the row is its name
    /// and its count.
    var avatar: Bool = true
    /// A MARK after the title — text the APP wrote about the row, not text a
    /// person typed ("(me)" on your own card in a Space's roster). It is dressed
    /// so it can never be read as part of a name: italic, 80% of the title, and a
    /// step lighter. Nothing user-authored is ever set that way.
    var mark: String? = nil

    var body: some View {
        HStack(spacing: Sig.rowSpacing) {
            if avatar {
                if host != nil {
                    forumDisc
                } else {
                    Avatar(initials: initials ?? Avatar.initials(title), tint: tint,
                           size: avatarSize, photo: photo, color: blockColor)
                }
            }
            VStack(alignment: .leading, spacing: 1) {
                HStack(alignment: .firstTextBaseline) {
                    if let host {
                        HStack(spacing: 4) {
                            Text(host.isEmpty ? "Group" : host)
                                .font(.headline)
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                                .layoutPriority(0)
                            Text("/")
                                .font(.headline)
                                .foregroundStyle(.tertiary)
                            Text(title)
                                .font(.headline)
                                .lineLimit(1)
                                .layoutPriority(1)
                        }
                    } else {
                        // Concatenated, not an HStack: the mark then travels WITH the
                        // name through truncation instead of holding its own column.
                        (Text(title).font(.headline)
                         + (mark.map {
                             Text(" " + $0)
                                 .font(.system(size: Sig.markSize, weight: .regular).italic())
                                 .foregroundStyle(Theme.textTertiary)
                           } ?? Text("")))
                            .lineLimit(1)
                    }
                    Spacer(minLength: 8)
                    // With no second line to hold it, the unread pill RIDES THE
                    // FIRST — in the slot the timestamp would have taken. A bar
                    // that carries no snippet (the Profile pin bar, a channel row)
                    // used to grow a blank second line just to seat the pill.
                    if snippet.isEmpty, unread > 0 {
                        UnreadPill(count: unread)
                    } else {
                        Text(time)
                            .font(.subheadline)
                            .foregroundStyle(unread > 0 ? Brand.accent : .secondary)
                    }
                }
                // No second line when there is nothing to put on it (the Profile
                // pin bar) — the pill has already been seated above.
                if !snippet.isEmpty {
                    HStack(alignment: .top) {
                        Group {
                            if enrichSnippet {
                                EnrichedLabel(snippet, font: .subheadline, color: .secondary)
                            } else {
                                Text(snippet).font(.subheadline).foregroundStyle(.secondary)
                            }
                        }
                        .lineLimit(2)
                        Spacer(minLength: 8)
                        UnreadPill(count: unread)
                    }
                }
            }
        }
        .padding(.vertical, Sig.rowVPad)
        .contentShape(Rectangle())
    }

    /// The forum's disc: the room glyph on the row's tint — the same mark the
    /// group's page hangs on its forum rows, so "this is a room" reads the same
    /// on both surfaces. A room has no single face, so never a photo.
    private var forumDisc: some View {
        ZStack {
            Theme.surface2
            Image(systemName: "bubble.left.and.bubble.right.fill")
                .font(.system(size: avatarSize * 0.32, weight: .semibold))
                .foregroundStyle(tint)
        }
        .frame(width: avatarSize, height: avatarSize)
        .clipShape(Circle())
        .overlay(Circle().strokeBorder(Theme.edge, lineWidth: DesignTokens.Skin.edge))
    }
}

/// One Signal message bubble. Incoming left / outgoing right, with a tightened
/// tail corner on the sender edge. Body text routes through EnrichedText so
/// entity matches bolden + colour inline (and long-press opens the info panel).
struct ChatBubble: View {
    var text: String
    var mine: Bool
    var author: String? = nil            // shown above incoming group messages
    var showAuthor: Bool = false
    var quoted: (author: String, snippet: String)? = nil  // reply quote, inside the bubble
    var time: String = ""                // send-time footer ("9:41 AM"), bottom edge
    /// WhatsApp delivery/read code for OUR OWN bubble: 0 none · 1 sent · 2 delivered
    /// · 3 read. Renders ✓ / ✓✓ / ✓✓-blue beside the time; ignored on incoming.
    var receipt: Int = 0
    var onEntityPress: ((Bool) -> Void)? = nil  // touch-down entity report (see EnrichedText)

    private var shape: some InsettableShape {
        .rect(
            topLeadingRadius: Sig.bubbleRadius,
            bottomLeadingRadius: mine ? Sig.bubbleRadius : Sig.bubbleTail,
            bottomTrailingRadius: mine ? Sig.bubbleTail : Sig.bubbleRadius,
            topTrailingRadius: Sig.bubbleRadius,
            style: .continuous
        )
    }

    /// Muted footer tone: a dimmed version of the bubble's own text colour.
    private var footerColor: Color { (mine ? Sig.outgoingText : Sig.incomingText).opacity(0.55) }

    var body: some View {
        HStack {
            if mine { Spacer(minLength: DesignTokens.Chat.bubbleInset) }
            VStack(alignment: mine ? .trailing : .leading, spacing: 2) {
                if showAuthor, !mine, let author {
                    Text(author)
                        .font(.caption2.weight(.semibold))
                        .foregroundStyle(.secondary)
                        .padding(.leading, 12)
                }
                VStack(alignment: .leading, spacing: 4) {
                    if let quoted {
                        QuotedReply(author: quoted.author, snippet: quoted.snippet)
                    }
                    EnrichedText(
                        text,
                        font: .preferredFont(forTextStyle: .body),
                        color: mine ? Sig.outgoingText : Sig.incomingText,
                        accent: Brand.accent,  // enriched words carry colour in either slate bubble
                        onEntityPress: onEntityPress
                    )
                    if !time.isEmpty || (mine && receipt > 0) {
                        HStack(spacing: 3) {
                            if !time.isEmpty {
                                Text(time).font(.caption2).foregroundStyle(footerColor)
                            }
                            if mine, receipt > 0 {
                                ReceiptTicks(status: receipt, base: footerColor, read: Brand.accent)
                            }
                        }
                        .frame(maxWidth: .infinity, alignment: .trailing)
                    }
                }
                .padding(.horizontal, DesignTokens.Chat.bubblePadH)
                .padding(.vertical, DesignTokens.Chat.bubblePadV)
                .pixelCard(mine ? Sig.outgoingBubble : Sig.incomingBubble, in: shape, bevel: false)
            }
            if !mine { Spacer(minLength: DesignTokens.Chat.bubbleInset) }
        }
    }
}

/// WhatsApp's message ticks, shown on your own outgoing bubble beside the time:
/// one ✓ (sent), two ✓✓ (delivered to everyone), two ✓✓ in the accent (read by
/// everyone). The two ticks overlap the WhatsApp way — the back tick peeks out to
/// the right — drawn from the plain `checkmark` glyph (there is no double-tick SF
/// Symbol) inside a fixed frame so the trailing edge never jitters as state climbs.
struct ReceiptTicks: View {
    let status: Int      // 1 sent · 2 delivered · 3 read
    var base: Color      // sent/delivered tone (a dimmed bubble-text colour)
    var read: Color      // the "read" accent

    private var isDouble: Bool { status >= 2 }
    private var tint: Color { status >= 3 ? read : base }

    var body: some View {
        ZStack(alignment: .leading) {
            if isDouble { tick.offset(x: 4) }   // back tick, peeking right
            tick                                 // front tick, at the leading edge
        }
        .frame(width: isDouble ? 14 : 9, height: 9, alignment: .leading)
        .foregroundStyle(tint)
        .accessibilityLabel(status >= 3 ? "Read" : status >= 2 ? "Delivered" : "Sent")
    }

    private var tick: some View {
        Image(systemName: "checkmark")
            .font(.system(size: 9, weight: .bold))
    }
}

// MARK: - Reactions (copied from Signal: MessageReactionPicker + CVReactionCountsView)

/// Signal's default quick-reaction set (`ReactionManager.defaultEmojiSet`), in
/// order. The long-press bar offers exactly these six.
enum Reactions {
    static let defaultSet = ["❤️", "👍", "👎", "😂", "😮", "😢"]
}

/// One reaction count pill under a bubble — Signal's `CVReactionCountsView`
/// geometry (24pt tall, fully-round, hairline border; the accent fill + white
/// glyph marks YOUR reaction). Tapping toggles your own reaction.
struct ReactionPill: View {
    var emoji: String
    var count: Int
    var mine: Bool
    var onTap: () -> Void

    var body: some View {
        Button(action: onTap) {
            HStack(spacing: 3) {
                Text(emoji).font(.system(size: 13))
                if count > 1 {
                    Text("\(count)")
                        .font(.caption2.weight(.semibold))
                        .foregroundStyle(mine ? Theme.onAccent : Theme.textSecondary)
                }
            }
            .padding(.horizontal, 8)
            .frame(height: 26)
            .pixelCard(mine ? Brand.accent : Theme.surface2, in: Capsule(), bevel: false)
        }
        .buttonStyle(.plain)
    }
}

/// The row of reaction pills shown under a bubble (grouped by emoji, ordered as
/// the core folded them). Empty groups never reach here.
struct ReactionPills: View {
    var reactions: [ChatReaction]
    var onToggle: (String, Bool) -> Void  // (emoji, makeActive)

    var body: some View {
        HStack(spacing: 4) {
            ForEach(reactions, id: \.emoji) { r in
                ReactionPill(emoji: r.emoji, count: Int(r.count), mine: r.mine) {
                    // Tapping your own reaction clears it; tapping another adds yours.
                    onToggle(r.emoji, !r.mine)
                }
            }
        }
    }
}

/// The floating long-press reaction bar (Signal's `MessageReactionPicker`): a
/// rounded capsule of the six default emoji. The one you've already picked is
/// highlighted; tapping it again removes it.
struct ReactionBar: View {
    var current: String?               // your existing reaction emoji, if any
    var onPick: (String) -> Void

    var body: some View {
        HStack(spacing: 6) {
            ForEach(Reactions.defaultSet, id: \.self) { e in
                Button { onPick(e) } label: {
                    Text(e)
                        .font(.system(size: 30))
                        .padding(6)
                        .background(current == e ? Brand.accent.opacity(0.22) : Color.clear, in: Circle())
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .pixelCard(Theme.surface2, in: Capsule(), shadow: true)
    }
}

/// A quoted-reply block (Signal's `CVComponentQuotedReply` / composer preview):
/// an accent rail + the quoted snippet. Used both inside an outgoing/incoming
/// reply bubble and above the composer while you're composing a reply.
struct QuotedReply: View {
    var author: String
    var snippet: String
    var onCancel: (() -> Void)? = nil   // shown only in the composer preview

    var body: some View {
        HStack(spacing: 8) {
            RoundedRectangle(cornerRadius: 2).fill(Brand.accent).frame(width: 3)
            VStack(alignment: .leading, spacing: 1) {
                Text(author)
                    .font(.caption2.weight(.semibold))
                    .foregroundStyle(Brand.accent)
                Text(snippet.isEmpty ? "Message" : snippet)
                    .font(.caption)
                    .foregroundStyle(Theme.textSecondary)
                    .lineLimit(1)
            }
            if let onCancel {
                Spacer(minLength: 4)
                Button(action: onCancel) {
                    Image(systemName: "xmark.circle.fill").foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Cancel reply")
                .accessibilityIdentifier("reply-cancel")
            }
        }
        .padding(.vertical, 4)
        // HUG the two text lines. The rail is a Shape, and a Shape accepts any
        // height it is proposed — inside a container with spare vertical room the
        // whole quote inflated into a half-page slab (the composer's reply strip
        // was the visible casualty). Ideal-size vertically, always: this block is
        // two lines of text with a rail, never a panel.
        .fixedSize(horizontal: false, vertical: true)
    }
}

/// A centered date/system separator (secondary, small caps-ish).
struct DateSeparator: View {
    var label: String
    var body: some View {
        Text(label)
            .font(.footnote.weight(.medium))
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity)
            .padding(.vertical, 6)
    }
}

/// The Signal composer input bar — a floating "slot" whose corner radius is EXACTLY
/// the shell's action-disc radius (`CircleButton.discRadius`), so the field reads as the
/// CHAT circle pulled wide. It performs that morph BOTH WAYS: a disc the size of the CHAT
/// button, centered where that disc sat, extends left and right into the full-width field
/// — and collapses back into the disc when you leave the conversation.
///
/// The morph state is NOT owned here. The screen presenting the conversation owns it
/// (ChatsView), because the same boolean also hides and returns the shell's disc row: one
/// state, one animation, so the eye reads one continuous motion — the CHAT circle becoming
/// the place you type, and becoming the CHAT circle again. A composer that nothing morphed
/// into (ThreadView) leaves `extended` nil and is simply present, full width.
struct Composer: View {
    @Binding var text: String
    var placeholder: String = "Message"
    /// The morph state, owned by the presenting screen: `true` extends the disc into the
    /// field, `false` retracts it. nil ⇒ no morph — the composer is just there.
    var extended: Bool? = nil
    /// Bump to focus the field — the reply flow raises the keyboard the moment
    /// the quote strip appears (WhatsApp's behaviour). 0 and never bumped ⇒ inert.
    var focusTick: Int = 0
    var onSend: () -> Void

    @FocusState private var focused: Bool

    private var open: Bool { extended ?? true }
    private var canSend: Bool { !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }

    /// The slot IS a disc when width == height; its radius is the disc's radius, so the
    /// same shape carries the whole morph from circle to stadium.
    private var slot: RoundedRectangle { RoundedRectangle(cornerRadius: CircleButton.discRadius, style: .continuous) }

    var body: some View {
        // GeometryReader gives a CONCRETE full width, so the slot animates its width
        // between the disc's diameter and the full row — a real horizontal extend.
        // (Animating `maxWidth` to `.infinity` can't interpolate; it snaps.)
        GeometryReader { geo in
            let fullW = geo.size.width
            HStack(alignment: .center, spacing: 8) {
                TextField(placeholder, text: $text, axis: .vertical)
                    .textFieldStyle(.plain)
                    .lineLimit(1...5)
                    .padding(.leading, 20)
                    .focused($focused)
                    .onChange(of: focusTick) { _, _ in focused = true }
                    .accessibilityIdentifier("composer-field")
                Button(action: { if canSend { onSend() } }) {
                    Image(systemName: "arrow.up.circle.fill")
                        .font(.system(size: 34))
                        .foregroundStyle(canSend ? Brand.accent : Theme.textTertiary)
                }
                .disabled(!canSend)
                .padding(.trailing, 7)
                .accessibilityLabel("Send")
                .accessibilityIdentifier("composer-send")
            }
            // The contents fade on their OWN schedule (this `.animation` governs only the
            // opacity above it, not the frame below): in after the slot has begun to open,
            // out the moment it starts to close — so they are never crushed inside a disc.
            .opacity(open ? 1 : 0)
            .animation(open ? Motion.contentIn : Motion.contentOut, value: open)
            // The width rides the caller's animation (Motion.extend / .retract).
            .frame(width: open ? fullW : CircleButton.discSize, height: CircleButton.discSize)
            .background(Theme.surface2, in: slot)
            .overlay(slot.strokeBorder(Theme.edge, lineWidth: DesignTokens.Skin.edge))
            .clipShape(slot)
            .frame(width: fullW, alignment: .center)              // collapsed disc rests centered, where CHAT was
        }
        .frame(height: CircleButton.discSize)
        .padding(.horizontal, 12)
        .padding(.top, 4)
        .padding(.bottom, 6)
    }
}

/// A reusable rounded search field (matches Signal's scroll-away search) — the ONE
/// search box the list screens share (CHAT · NOTE). It carries its own padding, top
/// included, so every screen that puts it first in its list stack gets it at exactly the
/// same height: alignment is structural, not two copies happening to agree.
///
/// Deliberately hand-rolled rather than `.searchable` — on iOS 26 the system field
/// relocates to the BOTTOM of the screen, which is what pulled Chats' search away from
/// Notes'. Show it only when the list beneath it has rows: a search box over an empty
/// screen is chrome with nothing to search.
struct SearchField: View {
    @Binding var text: String
    var placeholder: String = "Search"

    /// The composer's slot, taken in a notch: 90% of the disc's diameter, so search reads
    /// as the quieter relative of the field you type into rather than its twin. Still a
    /// stadium — the radius follows the height, so the shape language holds.
    static let height = CircleButton.discSize * 0.9

    private var slot: RoundedRectangle {
        RoundedRectangle(cornerRadius: Self.height / 2, style: .continuous)
    }

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
            TextField(placeholder, text: $text)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
            if !text.isEmpty {
                Button { text = "" } label: {
                    Image(systemName: "xmark.circle.fill").foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.leading, 20)      // matches the composer's text inset
        .padding(.trailing, 16)
        .frame(height: Self.height)
        // Sheerer than the composer's solid slot: the content below should read through
        // it, and search is a resting affordance, not the thing you came to use.
        .background(Theme.surface2.opacity(0.55), in: slot)
        .overlay(slot.strokeBorder(Theme.edge, lineWidth: DesignTokens.Skin.edge))
        // 16, not the composer's own 12: measured on screen, the composer's slot lands
        // 4pt further in than its code padding suggests (its list sits inside a
        // NavigationStack that adds its own margin). Matching what the EYE sees beats
        // matching the source number.
        .padding(.horizontal, 16)
        // The gap under the top bar. NOT a free choice: the toolbar row (avatar · ＋ · bell)
        // is centred in the nav bar, and this number is what decides whether that row reads as
        // centred in the band above the field. Measured on an iPhone 17 Pro: safe-area top 59,
        // nav bar 59→103 (icon centre 84), and the NavigationStack contributes ~13pt of its own
        // margin below it. So 0 here puts the field at ~116 and the icon row within ~4pt of the
        // midpoint of 59…116 — centred to the eye. Any positive value pushes the field down and
        // the row visibly rides high. Keep it here, not per screen: CHAT and NOTE share it.
        .padding(.top, 0)
    }
}

/// A list with the shared search field above it, which slides UP out of the way when you
/// scroll down into the list and drops back when you scroll up — so the list gets the
/// whole screen while you are reading it, and search is a flick away.
///
/// Both list screens (CHAT · NOTE) use this, so the behaviour and the geometry are the
/// same thing in both places rather than two implementations that agree today.
///
/// `accessory` is an optional band under the field (CHAT's type-filter chips) that
/// hides and returns WITH it — WhatsApp's arrangement: search and filters are one
/// piece of chrome, and scrolling into the list clears them together.
struct SearchableList<Content: View, Accessory: View>: View {
    @Binding var text: String
    var placeholder: String = "Search"
    /// WHAT THE FIELD HAS TO CLEAR. 0 on a screen whose top belongs to the list (NOTE,
    /// CHAT). LIFE floats its selector rail over the same top edge, and the field was
    /// revealing itself underneath it — so that host states the strip and the field
    /// opens below it. The CONTENT is not moved: it keeps its own clearance, because on
    /// LIFE the feed is full-bleed on purpose and scrolls up behind the rail.
    ///
    /// DECLARED BEFORE the view builders: the memberwise init takes its parameters in
    /// declaration order, and trailing-closure content/accessory have to stay last.
    var topInset: CGFloat = 0
    @ViewBuilder var content: Content
    @ViewBuilder var accessory: Accessory

    /// OUT OF THE WAY BY DEFAULT (Ralph, 18 Aug). A list you are reading is the point of
    /// the screen; searching it is the exception, so the field is not spent on by default
    /// and a pull down brings it back — the gesture the OS trained everyone on.
    @State private var hidden = true

    var body: some View {
        VStack(spacing: 0) {
            if !hidden {
                SearchField(text: $text, placeholder: placeholder)
                    .padding(.top, topInset)
                    .transition(.move(edge: .top).combined(with: .opacity))
                accessory
                    .transition(.move(edge: .top).combined(with: .opacity))
            }
            // Never auto-hide a field with a query in it: the list under it is filtered
            // BY that text, so taking the text away leaves a list that cannot explain
            // itself.
            content.modifier(HideOnScroll(hidden: $hidden, locked: !text.isEmpty))
        }
        .clipped()   // the field leaves upward; don't let it draw over the top bar
    }
}

extension SearchableList where Accessory == EmptyView {
    /// The plain form (NOTE) — a searchable list with no accessory band.
    init(text: Binding<String>, placeholder: String = "Search",
         topInset: CGFloat = 0,
         @ViewBuilder content: () -> Content) {
        self.init(text: text, placeholder: placeholder, topInset: topInset,
                  content: content, accessory: { EmptyView() })
    }
}

/// Watches the list's scroll offset and reports which way you are going.
///
/// Hiding the field changes the list's height, which moves the content offset, which
/// looks exactly like a scroll — so a naive version oscillates. Two guards: a movement
/// threshold that ignores jitter and rubber-banding, and a short deaf window after each
/// toggle so the reflow it causes can't immediately toggle it back.
private struct HideOnScroll: ViewModifier {
    @Binding var hidden: Bool
    /// True while a query is in the field — nothing may hide it out from under a
    /// filtered list.
    var locked = false
    @State private var last: CGFloat = 0
    @State private var deafUntil = Date.distantPast

    /// How far past the top counts as a PULL rather than an ordinary flick upward. A
    /// small number here would put the field back every time you scrolled to the top,
    /// which is the behaviour being replaced.
    private static let pull: CGFloat = 44

    func body(content: Content) -> some View {
        // Measured against the top INSET, so rest is 0 whatever the safe area is doing
        // and "pulled down" is honestly negative. Reading raw `contentOffset.y` made the
        // thresholds depend on the chrome above the list.
        content.onScrollGeometryChange(for: CGFloat.self) {
            $0.contentOffset.y + $0.contentInsets.top
        } action: { _, y in
            defer { last = y }
            guard Date() >= deafUntil else { return }

            // PULL DOWN past the top to bring it back.
            if hidden, y < -Self.pull {
                deafUntil = Date().addingTimeInterval(0.35)
                withAnimation(Motion.searchHide) { hidden = false }
                return
            }
            guard !locked else { return }

            // Scrolling INTO the list puts it away again. Rubber-banding at the very top
            // is not scrolling, which is what the offset floor is for.
            let dy = y - last
            guard abs(dy) > 8, !hidden, dy > 0, y > 24 else { return }
            deafUntil = Date().addingTimeInterval(0.35)
            withAnimation(Motion.searchHide) { hidden = true }
        }
    }
}

/// The top-left circular avatar toolbar button that opens the account menu
/// (Me + Live core) — Signal's settings-avatar pattern, per the spec.
struct AccountBarButton: View {
    /// The self profile photo (fills the disc) or, with none, a solid block colour —
    /// both flow from CoreService. NO initials: the button is a single clean circle
    /// the size of the `+`/bell glyph circles beside it, just filled with colour/photo
    /// instead of a glyph. `.plain` drops the system glass pill so there's no ring.
    var photo: Data? = nil
    var color: Color = Brand.accent
    /// Matches the visible diameter of the toolbar's `+`/bell glyph circles.
    var diameter: CGFloat = 34
    var action: () -> Void
    var body: some View {
        Button(action: action) {
            Avatar(initials: "", tint: color, size: diameter, photo: photo, color: color)
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Account, profile and live core")
    }
}

// MARK: - Preview: delivery/read receipts on the outgoing bubble

#Preview("Receipts — sent · delivered · read") {
    VStack(alignment: .trailing, spacing: Sig.gapDistinct) {
        ChatBubble(text: "Landing at 9. Grab a table?", mine: false, time: "8:41 AM")
        ChatBubble(text: "On my way ⏱", mine: true, time: "8:42 AM", receipt: 1)          // ✓ sent
        ChatBubble(text: "Two minutes out.", mine: true, time: "8:44 AM", receipt: 2)       // ✓✓ delivered
        ChatBubble(text: "Here — corner booth.", mine: true, time: "8:46 AM", receipt: 3)   // ✓✓ blue, read
    }
    .padding()
    .frame(maxWidth: .infinity)
    .background(Brand.groupBG)
}
