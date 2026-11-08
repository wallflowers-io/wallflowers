//  ThreadView.swift
//  The ChatRoom surface: a Forum GroupObject rendered as a Reddit-style nested
//  thread. It binds to `core.objectThread` — the pre-order flattened tree (each
//  post carrying its `depth` and `replyCount`) — reuses the Signal reaction
//  components, and authors REAL `forum.post`/`forum.react` deltas over the same
//  MLS+relay transport the flat Chats tab and the Work tab ride. Distinct from the
//  Signal-style Chats tab: this is threaded, collapsible, reply-in-place.
//
//  There is no display state in the tree — nesting, order and reply counts are all
//  the core `ForumState::thread()` projection. Collapse is the only local state.

import SwiftUI
import PacificCore

struct ThreadView: View {
    @EnvironmentObject private var core: CoreService
    /// The read watermark the CHAT list and the Space's CHANNELS lane both read.
    /// Opening a room IS reading it — ChatView has always cleared this on load, and
    /// a room opened through the other door has to clear the same ledger or the
    /// count never comes down.
    @EnvironmentObject private var convos: ConversationStore
    let objectId: String
    var title: String = "Channel"

    /// The channel's constituent Rooms — each a full Forum of its own, folded
    /// from the PARENT's log (`base.setPart`, role room), surfaced as tabs above the thread.
    @State private var rooms: [ForumRoomRow] = []
    /// The open tab: nil = the channel itself, else a room's object id.
    @State private var selectedRoom: String? = nil
    /// Owner-only affordances: the + tab that mints a room.
    @State private var isOwner = false
    @State private var naming = false
    @State private var roomName = ""

    /// The forum the surface is bound to right now — the channel, or the open room.
    private var boundId: String { selectedRoom ?? objectId }

    /// The folded thread, pre-order with depth — reloaded from the core, never mutated here.
    @State private var rows: [ForumThreadRow] = []
    /// True once the first fold has been read. Before it, the ground shows alone —
    /// rendering the "No posts yet" word for one frame and then replacing it with
    /// the list is the flash the eye catches on every open.
    @State private var loaded = false
    /// ROSTER NAMES BY IDENTITY HEX — one map per rebuild, never a lookup per row
    /// (the CHAT list's rule; a thread can be hundreds of posts deep).
    ///
    /// Without it this surface printed the author's ADDRESS. A thread was the one
    /// place in the app where you read someone's words beside their hex, while the
    /// same person's name showed correctly one screen away in CHAT.
    @State private var names: [String: String] = [:]
    /// Keys (author#gen) of the subtree roots the reader has collapsed.
    @State private var collapsed: Set<String> = []
    /// The post the composer is replying to; nil = composing a new top-level post.
    @State private var replyingTo: ChatMsg?
    /// The row currently showing its emoji picker (author#gen), if any.
    @State private var reactingTo: String?
    @State private var draft = ""
    /// Bumped on Reply so the composer raises the keyboard with the quote strip
    /// (WhatsApp's behaviour — replying IS typing).
    @State private var focusTick = 0

    private var roomConversation: Conversation {
        Conversation(id: boundId, kind: .group, title: title, tint: Brand.accent)
    }

    private static func key(_ post: ChatMsg) -> String { "\(post.author)#\(post.gen)" }

    /// One renderable row: a stable identity + its folded data. Rows whose subtree
    /// root is collapsed are dropped here (pre-order makes a subtree exactly the
    /// next `replyCount` rows), so the list stays a plain flat render.
    private struct VisibleRow: Identifiable {
        let id: String
        let data: ForumThreadRow
    }

    private var visibleRows: [VisibleRow] {
        var out: [VisibleRow] = []
        var skipUntil = -1
        for (i, row) in rows.enumerated() {
            if i <= skipUntil { continue }
            let id = Self.key(row.post)
            out.append(VisibleRow(id: id, data: row))
            if collapsed.contains(id) { skipUntil = i + Int(row.replyCount) }
        }
        return out
    }

    /// Invitations on this channel. They live in the Contact fold, NOT in the message
    /// log — a chat message carries only `text` (there are no attachments), so an
    /// invitation is a thing that happened between you rather than a thing one of you
    /// typed, and it renders beside the messages instead of inside them.
    @State private var invites: [InviteRow] = []

    /// The other member of a 1:1 thread — who an invitation is with. Nil for a group
    /// thread, where there is no single pairwise channel to read.
    private var invitePeer: String? {
        let others = core.objectMembers(objectId).filter { $0 != core.myIdentityHex }
        return others.count == 1 ? others.first : nil
    }

    var body: some View {
        VStack(spacing: 0) {
            if !rooms.isEmpty || isOwner { roomTabs }
            if rows.isEmpty {
                if loaded { emptyState } else { Spacer() }
            } else {
                ScrollView {
                    LazyVStack(spacing: 0) {
                        if let peer = invitePeer, !invites.isEmpty {
                            VStack(spacing: 10) {
                                ForEach(invites, id: \.event) { inv in
                                    InviteCard(invite: inv, peer: peer)
                                }
                            }
                            .padding(.horizontal, 16)
                            .padding(.vertical, 12)
                        }
                        ForEach(visibleRows) { vr in
                            ThreadRowView(
                                row: vr.data,
                                names: names,
                                isCollapsed: collapsed.contains(vr.id),
                                showReactionBar: reactingTo == vr.id,
                                onReply: {
                                    withAnimation(.easeOut(duration: 0.15)) { replyingTo = vr.data.post }
                                    focusTick += 1
                                },
                                onVote: { dir in vote(vr.data.post, dir) },
                                onToggleCollapse: { toggleCollapse(vr.id) },
                                onOpenReactions: {
                                    reactingTo = (reactingTo == vr.id) ? nil : vr.id
                                },
                                onPickReaction: { pickReaction(vr.data.post, $0) },
                                onTogglePill: { emoji, active in
                                    reactInRoom(vr.data.post, emoji, active)
                                }
                            )
                        }
                    }
                }
                .scrollDismissesKeyboard(.interactively)
            }
            if let parent = replyingTo { replyStrip(parent) }
            Composer(text: $draft, focusTick: focusTick, onSend: submit)
        }
        // The room's ground is new NOTE's: deep slate under the drifting jewel
        // light, dimmed further — subtle lighting on black you can read over,
        // not the compose card's full glow.
        .background {
            ZStack {
                Theme.bg
                NoteAmbient().opacity(0.45)
            }
            .ignoresSafeArea()
        }
        .navigationTitle(title)
        .navigationBarTitleDisplayMode(.inline)
        // A room opens scrolled to its latest post, so without this the bar
        // paints its scroll-edge slab over the ambient ground from the start.
        .glassTopBar()
        .onAppear {
            reloadRooms()
            reload()
            markRead()
            ChatFocus.shared.enter("grp:" + boundId)   // on screen — no banner for this room
        }
        .onDisappear { ChatFocus.shared.leave("grp:" + boundId) }
        .onReceive(core.$syncTick) { _ in reloadRooms(); reload() }   // inbound posts appear on sync
        // A ROOM SWITCH is a new room opened: the tab band rebinds `boundId` without
        // this view ever appearing again.
        .onChange(of: boundId) { _, _ in markRead() }
        // Posts landing while you are looking at them are read on arrival — the same
        // rule ChatView applies when `rows.count` grows, not a check every sync tick.
        .onChange(of: rows.count) { _, _ in markRead() }
    }

    // MARK: - Room tabs (the channel's constituent Rooms)

    /// True when this surface fronts a real `forum`-kind object — the only kind
    /// that hosts rooms. ThreadView also fronts Event logs and DM threads, where
    /// a rooms read would be a loud kind error.
    private var isChannel: Bool {
        core.groupObjects.contains { $0.id == objectId }
    }

    /// The tab band: the channel itself, then its rooms in attach order, then
    /// the owner's +. An unheld room is a tab you know of but are not in yet —
    /// rendered muted, not enterable (GroupDetailView's unheld dress).
    private var roomTabs: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                roomTab(title.isEmpty ? "Channel" : title, id: nil, held: true)
                ForEach(rooms, id: \.id) { room in
                    roomTab(room.name.isEmpty ? "Room" : room.name, id: room.id, held: room.held)
                }
                if isOwner {
                    Button { naming = true } label: {
                        Image(systemName: "plus")
                            .font(.footnote.weight(.semibold))
                            .foregroundStyle(Theme.textSecondary)
                            .padding(.horizontal, 10)
                            .padding(.vertical, 6)
                            .pixelCard(Theme.fill, in: Capsule(), bevel: false)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("New room")
                    .accessibilityIdentifier("room-new")
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
        }
        .alert("New room", isPresented: $naming) {
            TextField("Name", text: $roomName)
            Button("Create", action: createRoom)
            Button("Cancel", role: .cancel) { roomName = "" }
        }
    }

    private func roomTab(_ label: String, id: String?, held: Bool) -> some View {
        let selected = selectedRoom == id
        return Button { open(room: id) } label: {
            Text(label)
                .font(.footnote.weight(.semibold))
                .lineLimit(1)
                .foregroundStyle(selected ? Theme.onAccent : Theme.textSecondary)
                .padding(.horizontal, DesignTokens.Room.tabPadH)
                .padding(.vertical, DesignTokens.Room.tabPadV)
                .pixelCard(selected ? Brand.accent : Theme.fill, in: Capsule(), bevel: false)
        }
        .buttonStyle(.plain)
        .disabled(!held)
        .opacity(held ? 1 : 0.4)
        .accessibilityIdentifier(id.map { "room-tab-\($0.prefix(12))" } ?? "room-tab-channel")
        .accessibilityAddTraits(selected ? [.isSelected] : [])
    }

    /// Switch tabs: rebind the whole surface (thread, composer, focus) to the
    /// room's forum — or back to the channel for nil.
    private func open(room id: String?) {
        guard id != selectedRoom else { return }
        ChatFocus.shared.leave("grp:" + boundId)
        withAnimation(.easeOut(duration: 0.15)) {
            selectedRoom = id
            replyingTo = nil
            reactingTo = nil
            collapsed = []
        }
        ChatFocus.shared.enter("grp:" + boundId)
        reload()
    }

    private func createRoom() {
        let name = roomName.trimmingCharacters(in: .whitespaces)
        roomName = ""
        guard !name.isEmpty else { return }
        Task {
            if let id = await core.newForumRoom(objectId, name: name) {
                reloadRooms()
                open(room: id)
            }
        }
    }

    private func reloadRooms() {
        guard isChannel else { return }
        rooms = core.forumRooms(objectId)
        isOwner = core.isGroupOwner(objectId)
        // The open room was detached under us — fall back to the channel.
        if let open = selectedRoom, !rooms.contains(where: { $0.id == open }) {
            self.open(room: nil)
        }
    }

    // MARK: - Composer (the ONE chat input bar — see `Composer` in Signal.swift)

    /// WhatsApp's quote strip: a compact rounded block AT THE COMPOSER'S OWN
    /// WIDTH, flush above the field — the composer growing a quote, never an
    /// edge-to-edge band that reads as a new panel. One line, rail + name +
    /// snippet + ✕, and the keyboard is already rising (see `focusTick`).
    private func replyStrip(_ parent: ChatMsg) -> some View {
        HStack {
            QuotedReply(author: parent.mine ? "You" : ThreadRowView.label(parent.author, names),
                        snippet: parent.text,
                        onCancel: { withAnimation(.easeOut(duration: 0.12)) { replyingTo = nil } })
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 2)
        .pixelCard(Theme.surface2)
        .padding(.horizontal, 12)   // the composer's own inset — same width as its slot
        .transition(.move(edge: .bottom).combined(with: .opacity))
    }

    private var emptyState: some View {
        VStack(spacing: 8) {
            Spacer()
            Image(systemName: "bubble.left.and.bubble.right")
                .font(.largeTitle).foregroundStyle(Theme.textSecondary)
            Text("No posts yet").font(.headline).foregroundStyle(Theme.textPrimary)
            Text("Start the thread below.").font(.subheadline).foregroundStyle(Theme.textSecondary)
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    // MARK: - Actions (all author real deltas via CoreService)

    /// Clear the room's unread mark, and send the real receipts.
    ///
    /// TWO DIFFERENT "READ", both owed the moment a room is on screen: the local
    /// watermark that drives the pill (ConversationStore), and the ✓✓ receipt the
    /// other side sees (`markConversationRead`). ChatView does both on load; this
    /// surface is the other door onto the same rooms and did neither, so a channel
    /// opened from a Space's CHANNELS lane kept its count forever.
    ///
    /// The count is read with `core.messages` — NOT `rows.count` — because that is
    /// the basis the lists compute the pill from. Watermarking against a different
    /// fold of the same log is how the number on the bar and the number of posts on
    /// the page come to disagree.
    private func markRead() {
        let convo = Conversation(id: boundId, kind: .group, title: "", tint: .clear)
        convos.markRead(boundId, count: core.messages(convo).count)
        Task { await core.markConversationRead(convo) }
    }

    private func reload() {
        rows = core.objectThread(boundId)
        names = core.index.names
        invites = invitePeer.map { core.invites(peer: $0) } ?? []
        loaded = true
    }

    private func toggleCollapse(_ id: String) {
        if collapsed.contains(id) { collapsed.remove(id) } else { collapsed.insert(id) }
    }

    private func submit() {
        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        let parent = replyingTo
        Task {
            let ok = parent != nil
                ? await core.reply(roomConversation, text: text, to: parent!)
                : await core.sendToGroup(boundId, text)
            if ok { draft = ""; replyingTo = nil }   // keep the draft on failure (CHAT's rule)
            reload()
        }
    }

    /// Emoji picked from the floating bar: picking the one you already hold clears it
    /// (Signal LWW), otherwise it replaces/sets yours.
    private func pickReaction(_ post: ChatMsg, _ emoji: String) {
        reactingTo = nil
        let makeActive = post.reactions.first(where: { $0.mine })?.emoji != emoji
        reactInRoom(post, emoji, makeActive)
    }

    private func reactInRoom(_ post: ChatMsg, _ emoji: String, _ active: Bool) {
        Task {
            _ = await core.react(roomConversation, to: post, emoji: emoji, active: active)
            reload()
        }
    }

    /// Author a real `forum.vote` delta (dir +1 | −1 | 0 = clear) and re-fold.
    private func vote(_ post: ChatMsg, _ dir: Int8) {
        Task {
            _ = await core.votePost(boundId, on: post, dir: dir)
            reload()
        }
    }
}

/// One post in the thread: indented by nesting `depth` with a per-level rail, a
/// header (author · time · collapse toggle), the body, its reaction pills, and an
/// action row (react · reply). The optional floating emoji bar appears in place.
private struct ThreadRowView: View {
    let row: ForumThreadRow
    /// Roster names by identity hex — see ThreadView.names.
    let names: [String: String]
    let isCollapsed: Bool
    let showReactionBar: Bool
    var onReply: () -> Void
    var onVote: (Int8) -> Void = { _ in }
    var onToggleCollapse: () -> Void
    var onOpenReactions: () -> Void
    var onPickReaction: (String) -> Void
    var onTogglePill: (String, Bool) -> Void

    private var post: ChatMsg { row.post }

    /// A NAME, or an address that cannot be mistaken for one. Same shape as
    /// `CoreService.displayName(forSpace:)` — the map first, then a truncated hex
    /// with an ellipsis. Raw hex is an address, and the app never shows one as a
    /// name; the map is keyed on hex because renderings differ across the seams
    /// (`ed25519:`, the legacy `space1`, bare).
    static func label(_ author: String, _ names: [String: String]) -> String {
        let hex = IdentityKey.hex(author)
        if let n = names[hex], !n.isEmpty { return n }
        return String(hex.prefix(10)) + "\u{2026}"
    }
    private var myEmoji: String? { post.reactions.first(where: { $0.mine })?.emoji }
    private static let indentUnit: CGFloat = DesignTokens.Room.indentUnit

    var body: some View {
        HStack(alignment: .top, spacing: DesignTokens.Room.railGap) {
            rails
            VStack(alignment: .leading, spacing: DesignTokens.Room.postSpacing) {
                header
                bubble
                if isCollapsed, row.replyCount > 0 {
                    Text("\(row.replyCount) repl\(row.replyCount == 1 ? "y" : "ies") hidden")
                        .font(.caption2).foregroundStyle(Theme.textSecondary)
                }
                if !post.reactions.isEmpty {
                    ReactionPills(reactions: post.reactions, onToggle: onTogglePill)
                }
                actions
                if showReactionBar {
                    ReactionBar(current: myEmoji, onPick: onPickReaction)
                        .padding(.top, 2)
                }
            }
            Spacer(minLength: 0)
        }
        .padding(.vertical, DesignTokens.Room.postPadV)
        .padding(.horizontal, DesignTokens.Room.postPadH)
        .contentShape(Rectangle())
    }

    /// The post body wears CHAT's bubble: the same slate fills, text style and
    /// padding as `ChatBubble`, rounded over on every corner. Uniform radius, no
    /// tail — the tail is a left/right alignment cue and a tree is left-anchored,
    /// so here authorship is carried by the fill (yours = the outgoing slate).
    private var bubble: some View {
        EnrichedText(post.text,
                     font: .preferredFont(forTextStyle: .body),
                     color: post.mine ? Sig.outgoingText : Sig.incomingText)
            .fixedSize(horizontal: false, vertical: true)
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(post.mine ? AnyShapeStyle(Sig.outgoingBubble) : AnyShapeStyle(Sig.incomingBubble),
                        in: RoundedRectangle(cornerRadius: Sig.bubbleRadius, style: .continuous))
    }

    /// One thin rail per ancestor level — the Reddit nesting cue.
    private var rails: some View {
        HStack(spacing: 0) {
            ForEach(0..<Int(row.depth), id: \.self) { _ in
                Rectangle().fill(Theme.hairline).frame(width: 1)
                Spacer(minLength: Self.indentUnit - 1)
            }
        }
        .frame(width: CGFloat(row.depth) * Self.indentUnit, alignment: .leading)
    }

    private var header: some View {
        HStack(spacing: 6) {
            // The author is a DOOR, like a feed card's and a DM header's: tapping
            // it opens that person's page. It was plain Text, so a thread was the
            // one surface in the app where you could read someone's words and had
            // no way to reach them. Yours is not a link — you are already here.
            if post.mine {
                Text("You")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(Brand.accent)
            } else {
                NavigationLink(value: PersonLink(id: post.author)) {
                    Text(Self.label(post.author, names))
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(Theme.textPrimary)
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("post-author")
            }
            if post.ts > 0 {
                Text("· \(Self.relTime(post.ts))")
                    .font(.caption2).foregroundStyle(Theme.textSecondary)
            }
            Spacer(minLength: 4)
            if row.replyCount > 0 {
                Button(action: onToggleCollapse) {
                    HStack(spacing: 2) {
                        Image(systemName: isCollapsed ? "chevron.right" : "chevron.down")
                        if isCollapsed { Text("\(row.replyCount)") }
                    }
                    .font(.caption2).foregroundStyle(Theme.textSecondary)
                }
                .buttonStyle(.plain)
            }
        }
    }

    private var actions: some View {
        HStack(spacing: 18) {
            votes
            Button(action: onOpenReactions) {
                Image(systemName: "face.smiling")
            }
            Button(action: onReply) {
                Label("Reply", systemImage: "arrowshape.turn.up.left")
            }
        }
        .font(.caption2)
        .foregroundStyle(Theme.textSecondary)
        .buttonStyle(.plain)
    }

    /// The Reddit cluster: ▲ score ▼. Tapping your held direction clears it
    /// (dir 0); the lit arrow is YOUR vote, the score is the room's (ups − downs
    /// — derived from the fold, never stored).
    private var votes: some View {
        HStack(spacing: 6) {
            Button { onVote(post.myVote == 1 ? 0 : 1) } label: {
                Image(systemName: "arrow.up")
                    .fontWeight(.semibold)
                    .foregroundStyle(post.myVote == 1 ? Brand.accent : Theme.textSecondary)
            }
            .accessibilityLabel("Upvote")
            .accessibilityIdentifier("vote-up")
            Text("\(Int(post.up) - Int(post.down))")
                .monospacedDigit()
                .fontWeight(.semibold)
                .foregroundStyle(post.myVote == 0 ? Theme.textSecondary : Theme.textPrimary)
                .accessibilityIdentifier("vote-score")
            Button { onVote(post.myVote == -1 ? 0 : -1) } label: {
                Image(systemName: "arrow.down")
                    .fontWeight(.semibold)
                    .foregroundStyle(post.myVote == -1 ? Brand.accent : Theme.textSecondary)
            }
            .accessibilityLabel("Downvote")
            .accessibilityIdentifier("vote-down")
        }
    }

    private static func relTime(_ ms: UInt64) -> String {
        let date = Date(timeIntervalSince1970: Double(ms) / 1000)
        let f = RelativeDateTimeFormatter()
        f.unitsStyle = .abbreviated
        return f.localizedString(for: date, relativeTo: Date())
    }
}
