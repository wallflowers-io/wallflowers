//  PlaygroundPage.swift
//  Every shared element, live, beside its options. The sample words are the only
//  stand-in data in the project and live only in this target.

import SwiftUI

struct PlaygroundPage: View {
    @Binding var key: String

    @State private var discSlot = "chat"
    @State private var discActive = true
    @State private var discEnabled = true
    @State private var discGlyph = false

    @State private var buttonPrimary = true
    @State private var buttonEnabled = true

    @State private var cardShape = "rounded"
    @State private var cardFill = "surface"
    @State private var cardBevel = true
    @State private var cardShadow = true

    @State private var composeTint = "note"
    @State private var composeFlash = "none"
    @State private var composeOpaque = false

    @State private var headerAccessory = true
    @State private var profileHint = true
    @State private var chipsEmphasized = false
    @State private var tagEmphasized = false
    @State private var filter = "All"
    @State private var unread = 3
    @State private var reactionMine = true
    @State private var reactionCount = 2
    @State private var barPick: String? = "❤️"
    @State private var switchThree = true
    @State private var switchPick = "event"

    @State private var rowUnread = 2
    @State private var rowForum = false
    @State private var rowAvatar = true
    @State private var rowMark = false

    @State private var bubbleMine = false
    @State private var bubbleAuthor = true
    @State private var bubbleQuote = false
    @State private var bubbleReceipt = 3

    @State private var quoteCancel = false
    @State private var composerOpen = true
    @State private var draft = ""
    @State private var search = ""

    @State private var avatarSize = 56.0
    @State private var avatarColour = false
    @State private var badgeSize = 44.0
    @State private var emptyLane = false

    @State private var feedProvenance = "member"
    @State private var feedAuthor = true
    @State private var feedLocation = true
    @State private var feedFooter = true

    private static let slots: [String: DiscSlot] = ["note": .note, "chat": .chat, "life": .life]
    private static let fills: [String: Color] = ["surface": Theme.surface, "raised": Theme.surface2,
                                                 "fill": Theme.fill, "accent": Theme.accent]

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                HStack {
                    Text("PLAYGROUND").font(.title2.weight(.bold)).tracking(2)
                    Spacer()
                    Switch(options: ["kenjin", "pacific"], word: { $0 == "pacific" ? "LIGHT" : "DARK" }, selection: $key)
                        .frame(width: 280)
                }
                // One column in portrait, two in landscape: a bench slot needs ~560pt.
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 560), spacing: 24, alignment: .top)],
                          alignment: .leading, spacing: 24) {
                    elements
                }
            }
            .padding(32)
        }
        .foregroundStyle(Theme.textPrimary)
        .background(Theme.bg)
    }

    @ViewBuilder private var elements: some View {
        element("Disc") {
            let slot = Self.slots[discSlot]
            CircleButton(label: "Disc", systemImage: slot == nil ? "bell" : "xmark",
                         blockColor: slot?.blockColor, showsGlyph: discGlyph,
                         isActive: discActive, enabled: discEnabled) {}
        } options: {
            pick(["note", "chat", "life", "glyph"], $discSlot)
            Switch("ACTIVE", "REST", isOn: $discActive)
            Switch("ENABLED", "DISABLED", isOn: $discEnabled)
            Switch("GLYPH", "PLAIN", isOn: $discGlyph)
        }

        element("Button") {
            Button("Commit") {}
                .buttonStyle(PixelButtonStyle(primary: buttonPrimary))
                .disabled(!buttonEnabled)
        } options: {
            Switch("PRIMARY", "SECONDARY", isOn: $buttonPrimary)
            Switch("ENABLED", "DISABLED", isOn: $buttonEnabled)
        }

        element("Card") {
            card
        } options: {
            pick(["rounded", "capsule", "circle"], $cardShape)
            pick(["surface", "raised", "fill", "accent"], $cardFill)
            Switch("BEVEL", "FLAT", isOn: $cardBevel)
            Switch("SHADOW", "NONE", isOn: $cardShadow)
        }

        element("Compose card") {
            Text("Compose")
                .frame(width: 240, height: 130)
                .composeGlass(tint: Self.slots[composeTint]!.blockColor,
                              flash: ["ok": Theme.ok, "danger": Theme.danger][composeFlash],
                              opaque: composeOpaque)
        } options: {
            pick(["note", "chat", "life"], $composeTint)
            pick(["none", "ok", "danger"], $composeFlash)
            Switch("OPAQUE", "CARD", isOn: $composeOpaque)
        }

        element("Section header") {
            SectionHeader(title: "Upcoming", accessory: headerAccessory ? "3" : nil)
        } options: {
            Switch("ACCESSORY", "NONE", isOn: $headerAccessory)
        }

        element("Profile section") {
            ProfileSection(title: "Selling", systemImage: "tag", hint: profileHint ? "public" : nil) {
                ChipFlow(items: ["SDK", "Consulting", "Design"], emphasized: chipsEmphasized)
            }
        } options: {
            Switch("HINT", "NONE", isOn: $profileHint)
            Switch("EMPHASISED", "PLAIN", isOn: $chipsEmphasized)
        }

        element("Tag chip") {
            TagChip(text: "selling: SDK", emphasized: tagEmphasized)
        } options: {
            Switch("EMPHASISED", "PLAIN", isOn: $tagEmphasized)
        }

        element("Filter pills") {
            FilterPills(options: ["All", "Unread", "Groups", "Rooms"], selection: $filter)
        } options: {}

        element("Unread pill") {
            UnreadPill(count: unread)
        } options: {
            Stepper("Count \(unread)", value: $unread, in: 0...120)
        }

        element("Reaction pill") {
            ReactionPill(emoji: "❤️", count: reactionCount, mine: reactionMine) { reactionMine.toggle() }
        } options: {
            Switch("MINE", "THEIRS", isOn: $reactionMine)
            Stepper("Count \(reactionCount)", value: $reactionCount, in: 1...9)
        }

        element("Reaction bar") {
            ReactionBar(current: barPick) { barPick = barPick == $0 ? nil : $0 }
        } options: {}

        element("Switch") {
            Switch(options: switchThree ? ["event", "place", "group"] : ["event", "place"],
                   word: { $0.uppercased() }, selection: $switchPick)
        } options: {
            Switch("THREE", "TWO", isOn: Binding(get: { switchThree }, set: { three in
                if !three, switchPick == "group" { switchPick = "event" }
                switchThree = three
            }))
        }

        element("Conversation row") {
            ConversationRow(title: rowForum ? "Reading room" : "Ada Lovelace",
                            snippet: "The engine is running.", time: "9:41", unread: rowUnread,
                            tint: DiscSlot.chat.blockColor, host: rowForum ? "Library" : nil,
                            avatar: rowAvatar, mark: rowMark ? "(me)" : nil)
        } options: {
            Stepper("Unread \(rowUnread)", value: $rowUnread, in: 0...99)
            Switch("FORUM", "PERSON", isOn: $rowForum)
            Switch("AVATAR", "NONE", isOn: $rowAvatar)
            Switch("MARK", "NONE", isOn: $rowMark)
        }

        element("Chat bubble") {
            ChatBubble(text: "Are we meeting at the library?", mine: bubbleMine, author: "Ada",
                       showAuthor: bubbleAuthor,
                       quoted: bubbleQuote ? (author: "Ada", snippet: "The engine is running.") : nil,
                       time: "9:41", receipt: bubbleReceipt)
        } options: {
            Switch("MINE", "THEIRS", isOn: $bubbleMine)
            Switch("AUTHOR", "NONE", isOn: $bubbleAuthor)
            Switch("QUOTE", "NONE", isOn: $bubbleQuote)
            Stepper("Receipt \(bubbleReceipt)", value: $bubbleReceipt, in: 0...3)
        }

        element("Quoted reply") {
            QuotedReply(author: "Ada", snippet: "The engine is running.", onCancel: quoteCancel ? {} : nil)
        } options: {
            Switch("CANCEL", "NONE", isOn: $quoteCancel)
        }

        element("Composer") {
            Composer(text: $draft, extended: composerOpen) { draft = "" }
        } options: {
            Switch("OPEN", "DISC", isOn: Binding(get: { composerOpen }, set: { open in
                withAnimation(open ? Motion.extend : Motion.retract) { composerOpen = open }
            }))
        }

        element("Search field") {
            SearchField(text: $search)
        } options: {}

        element("Avatar") {
            Avatar(initials: "AL", tint: .clear, size: avatarSize,
                   color: avatarColour ? DiscSlot.life.blockColor : nil)
        } options: {
            Slider(value: $avatarSize, in: 24...96)
            Switch("COLOUR", "INITIALS", isOn: $avatarColour)
        }

        element("Logo badge") {
            LogoBadge(initials: "WF", tint: .clear, size: badgeSize)
        } options: {
            Slider(value: $badgeSize, in: 24...96)
        }

        element("Empty word") {
            EmptyWord("CHAT", scale: emptyLane ? .lane : .screen, placed: false)
        } options: {
            Switch("LANE", "SCREEN", isOn: $emptyLane)
        }

        element("Object card") {
            ObjectCard(row: FeedItem(id: "playground", category: .events, title: "Library night",
                                     author: feedAuthor ? "Ada Lovelace" : "",
                                     location: feedLocation ? "Marylebone" : "",
                                     footer: feedFooter ? "Thursday · 10:00" : "", whenMs: 0,
                                     provenance: ["hops": .hops(2), "hydrated": .hydrated][feedProvenance] ?? .member,
                                     objectID: nil))
                .frame(maxWidth: 320)
        } options: {
            pick(["member", "hops", "hydrated"], $feedProvenance)
            Switch("AUTHOR", "NONE", isOn: $feedAuthor)
            Switch("LOCATION", "NONE", isOn: $feedLocation)
            Switch("FOOTER", "NONE", isOn: $feedFooter)
        }

        element("Date separator · trust line") {
            VStack {
                DateSeparator(label: "Today")
                TrustLine()
            }
        } options: {}
    }

    @ViewBuilder private var card: some View {
        let fill = Self.fills[cardFill]!
        let label = Text("Card").foregroundStyle(cardFill == "accent" ? Theme.onAccent : Theme.textPrimary)
        switch cardShape {
        case "capsule":
            label.padding(.horizontal, 32).padding(.vertical, 16)
                .pixelCard(fill, in: Capsule(), bevel: cardBevel, shadow: cardShadow)
        case "circle":
            label.frame(width: 110, height: 110)
                .pixelCard(fill, in: Circle(), bevel: cardBevel, shadow: cardShadow)
        default:
            label.frame(width: 200, height: 110)
                .pixelCard(fill, bevel: cardBevel, shadow: cardShadow)
        }
    }

    private func pick(_ options: [String], _ selection: Binding<String>) -> some View {
        Switch(options: options, word: { $0.uppercased() }, selection: selection)
    }

    /// One bench slot: the element on the left, its options on the right.
    private func element<C: View, O: View>(_ name: String, @ViewBuilder _ component: () -> C,
                                           @ViewBuilder options: () -> O) -> some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(name.uppercased())
                .font(.caption.weight(.semibold))
                .tracking(1.6)
                .foregroundStyle(Theme.textTertiary)
            HStack(alignment: .center, spacing: 20) {
                component()
                    .frame(maxWidth: .infinity, minHeight: 110)
                VStack(alignment: .leading, spacing: 8) { options() }
                    .frame(width: 240)
            }
        }
        .padding(20)
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay(RoundedRectangle(cornerRadius: DesignTokens.Skin.radius, style: .continuous)
            .strokeBorder(Theme.hairline, lineWidth: 1))
    }
}
