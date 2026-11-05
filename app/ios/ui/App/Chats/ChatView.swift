//  ChatView.swift
//  One conversation — Signal bubbles over the real folded transcript, now with
//  the full modern-chat surface: a UTC-derived send time inside each bubble,
//  emoji reactions (long-press → the six-emoji bar; tap a pill to toggle yours)
//  and replies (swipe a bubble → reply; the quote renders inside the bubble).
//  Every reaction and reply is a REAL forum.react / forum.post Delta over the
//  relay — the UI only projects the folded residue (see ChatRow / CoreService).
//
//  Reactions/replies target the REAL (author, gen) MsgRef on each message —
//  attribution comes from the MLS transcript alone, never a display overlay.

import SwiftUI
import PacificCore

struct ChatView: View {
    @EnvironmentObject private var core: CoreService
    @EnvironmentObject private var convos: ConversationStore
    let conversation: Conversation
    /// The composer's disc⇄field morph, owned by the screen that presented this
    /// conversation (see `Composer.extended`). nil ⇒ no morph, just a composer.
    var composerExtended: Bool? = nil

    @State private var rows: [ChatRow] = []
    @State private var draft = ""
    @State private var syncing = false
    @State private var pendingPeer = false
    @State private var replyingTo: ChatRow? = nil     // composer is quoting this
    @State private var reactingTo: ChatRow? = nil     // long-press bar is open for this
    @State private var showAddPeople = false          // group: invite a member
    @State private var showRename = false             // group: owner-only rename
    /// "New Space with…" — constitutes a Space seeded with this chat's people.
    /// The conversation itself is untouched (see NewSpaceFromChat).
    @State private var showNewSpace = false
    @State private var isOwner = false                // only the owner may add/rename
    @State private var liveTitle = ""                 // re-read each load; survives a rename

    /// The header name: the live GroupContext read once we have it, else the
    /// snapshot the row was tapped with (so the title never flickers empty).
    private var title: String { liveTitle.isEmpty ? conversation.title : liveTitle }

    /// InboundWake's conversation key — what ChatFocus reports while this thread is up.
    private var focusKey: String {
        (conversation.kind == .dm ? "dm:" : "grp:") + conversation.id
    }

    var body: some View {
        VStack(spacing: 0) {
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(spacing: Sig.gapDistinct) {
                        ForEach(rows) { row in
                            MessageRow(
                                row: row,
                                showAuthor: showAuthors && !row.mine,
                                quote: quote(for: row),
                                onReply: { beginReply(row) },
                                onLongPress: { openReactions(row) },
                                onToggleReaction: { emoji, active in toggle(row, emoji, active) }
                            )
                            .id(row.id)
                        }
                        if rows.isEmpty {
                            Text("No messages yet — say something.")
                                .font(.subheadline).foregroundStyle(.secondary)
                                .padding(.top, 40)
                        }
                        Color.clear.frame(height: 1).id(bottomID)
                    }
                    .padding()
                }
                .onChange(of: rows.count) { _, _ in scrollToBottom(proxy); markRead() }  // new message → ack it
                .onChange(of: core.syncTick) { _, _ in load() }   // inbound message/reaction landed
                .onAppear { load(); markRead(); scrollToBottom(proxy) }
            }
            if pendingPeer { acceptBar }
            if let r = replyingTo { replyPreview(r) }
            Composer(text: $draft, extended: composerExtended, onSend: send)
        }
        // This thread is ON SCREEN: the foreground announcer must not banner it.
        .onAppear { ChatFocus.shared.enter(focusKey) }
        .onDisappear { ChatFocus.shared.leave(focusKey) }
        .background(Brand.groupBG)
        .navigationTitle(title)
        .navigationBarTitleDisplayMode(.inline)
        // T1: the person page destinations for THIS stack. ChatView lives in the
        // shell's conversation-overlay NavigationStack (Shell.swift), not LIFE's,
        // so the value routes the page emits must be registered here too —
        // PersonLink (the header door + person pins) and GroupLink (a space pin).
        .navigationDestination(for: PersonLink.self) { link in
            PersonProfilePage(id: link.id,
                              source: RealProfileSource(core: core, subject: .peer(link.id)))
        }
        .navigationDestination(for: GroupLink.self) { link in
            GroupDetailView(objectId: link.id)
        }
        .toolbar {
            // A DM's header name is a DOOR, not a label: it pushes the peer's
            // person page (T1). Groups keep the plain title — their page is the
            // room-cover deep link, not this header.
            if conversation.kind == .dm {
                ToolbarItem(placement: .principal) {
                    NavigationLink(value: PersonLink(id: conversation.id)) {
                        Text(title)
                            .font(.headline)
                            .foregroundStyle(Theme.textPrimary)
                    }
                    .buttonStyle(.plain)
                    .accessibilityIdentifier("chat-person-link")
                }
            }
            // The menu now exists on a DM as well, because "New Space with…" is
            // above all a DM affordance — two people talking is exactly when
            // you want to constitute something around the conversation.
            ToolbarItem(placement: .topBarTrailing) {
                    Menu {
                        if conversation.kind == .group, isOwner {
                            Button { showAddPeople = true } label: {
                                Label("Add people", systemImage: "person.badge.plus")
                            }
                            Button { showRename = true } label: {
                                Label("Rename", systemImage: "pencil")
                            }
                        }
                        // WhatsApp's "New group with…", constituting a SPACE.
                        // The conversation is not converted or merged — only its
                        // PEOPLE are carried over (see NewSpaceFromChat).
                        Button { showNewSpace = true } label: {
                            Label("New Space with…", systemImage: "person.3")
                        }
                        .accessibilityIdentifier("chat-new-space")
                    } label: { Image(systemName: "ellipsis.circle") }
                        .accessibilityLabel("Conversation options")
            }
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    Task { syncing = true; await core.syncNow(); load(); syncing = false }
                } label: {
                    Image(systemName: syncing ? "arrow.triangle.2.circlepath" : "arrow.clockwise")
                }
                .disabled(syncing)
            }
        }
        // A compose OVER the chat, not a page instead of it: clear background so
        // the transcript stays visible behind the ground, the way the NOTE card
        // sits over whatever screen raised it.
        .fullScreenCover(isPresented: $showNewSpace) {
            NewSpaceFromChat(conversation: conversation) { id in
                showNewSpace = false
                NotificationCenter.default.post(name: .pacificOpenGroup, object: nil,
                                                userInfo: ["object": id])
            }
            .environmentObject(core)
            .presentationBackground(.clear)
        }
        .sheet(isPresented: $showAddPeople) {
            AddPeopleSheet(objectId: conversation.id,
                           label: title.isEmpty ? "this group" : title) { load() }
        }
        .sheet(isPresented: $showRename) {
            RenameGroupSheet(objectId: conversation.id, current: liveTitle) { load() }
        }
        .overlay { if let r = reactingTo { reactionBar(for: r) } }
        .alert(core.failure?.title ?? "Something went wrong", isPresented: Binding(
            get: { core.failure != nil },
            set: { if !$0 { core.failure = nil; core.error = nil } }
        )) {
            Button("OK", role: .cancel) {}
        } message: { Text(core.failure.map(Self.message) ?? "") }
    }

    /// Guidance first when there is any; the raw error only as the tail, because it is
    /// for us, not for the person reading it.
    static func message(_ f: Failure) -> String {
        f.guidance.isEmpty ? f.detail : f.guidance + "\n\n" + f.detail
    }

    private let bottomID = "chat-bottom"

    // MARK: - Reaction bar (long-press) overlay

    /// Signal's floating reaction bar over a dimmed chat, plus an explicit Reply
    /// action (swipe is the shortcut). Tapping the scrim dismisses.
    private func reactionBar(for row: ChatRow) -> some View {
        ZStack {
            Color.black.opacity(0.25).ignoresSafeArea()
                .onTapGesture { reactingTo = nil }
            VStack(spacing: 12) {
                ReactionBar(current: myReaction(row)) { emoji in
                    // Re-picking your current emoji clears it; otherwise it replaces.
                    toggle(row, emoji, myReaction(row) != emoji)
                    reactingTo = nil
                }
                Button { beginReply(row); reactingTo = nil } label: {
                    Label("Reply", systemImage: "arrowshape.turn.up.left")
                        .font(.subheadline.weight(.medium))
                        .padding(.horizontal, 16).padding(.vertical, 10)
                        .background(Theme.surface2, in: Capsule())
                }
                .buttonStyle(.plain)
            }
            .padding(.horizontal, 24)
        }
        .transition(.opacity)
    }

    // MARK: - Composer reply preview

    private func replyPreview(_ row: ChatRow) -> some View {
        HStack {
            QuotedReply(author: quote(for: row)?.author ?? replyAuthor(row),
                        snippet: row.text,
                        onCancel: { replyingTo = nil })
        }
        .padding(.horizontal, 14).padding(.vertical, 6)
        .background(Theme.surface2)
    }

    /// Double-opt-in, INBOUND ONLY. This bar is now exactly one thing: someone
    /// scanned your code and you have not answered yet. The scanner's own side is
    /// Connected the moment they scan (`pair_scan` — reading a code in person IS
    /// their consent), so this can no longer appear on the device that scanned.
    ///
    /// The old copy ("Connection pending") asserted something the app cannot know:
    /// `pair_accept` is purely local and sends the peer nothing, so no device ever
    /// learns whether the other side accepted. Naming the person makes the bar say
    /// only what is true and actionable here.
    private var acceptBar: some View {
        HStack(spacing: 10) {
            Image(systemName: "person.crop.circle.badge.questionmark")
                .foregroundStyle(Brand.accent)
            Text("\(conversation.title.isEmpty ? "Someone" : conversation.title) wants to connect")
                .font(.subheadline)
            Spacer()
            Button("Accept") {
                core.pairAccept(conversation.id)
                pendingPeer = core.connection(conversation.id)?.connected == false
            }
            .buttonStyle(.pixel)
            .controlSize(.small)
            .foregroundStyle(Theme.onAccent)
        }
        .padding(.horizontal, 12).padding(.vertical, 8)
        .background(.bar)
    }

    /// Group transcripts label incoming bubbles by author; DMs don't (Signal convention).
    private var showAuthors: Bool {
        conversation.kind == .group
    }

    // MARK: - Actions

    private func load() {
        let raw = core.messages(conversation)
        rows = raw.map(ChatRow.init)   // real MLS attribution only — no display overlays
        convos.markRead(conversation.id, count: raw.count)
        if conversation.kind == .dm {
            pendingPeer = core.connection(conversation.id)?.connected == false
        }
        if conversation.kind == .group {
            // Re-read the title on every load, not just at open: an owner's rename
            // commit lands on a later sync tick, and `conversation` is a value
            // snapshot taken when the row was tapped.
            let name = core.objectTitle(conversation.id)
            let room = name.isEmpty ? core.derivedGroupName(conversation.id) : name
            // A Group-hosted room carries its group in the title, a Channel-hosted
            // room its channel — exactly as its list row reads: "Host / Room".
            if let host = core.forumHosts[conversation.id] {
                liveTitle = "\(host.groupName.isEmpty ? "Group" : host.groupName) / \(room)"
            } else if let hostRoom = core.roomHosts[conversation.id] {
                liveTitle = "\(hostRoom.forumName.isEmpty ? "Channel" : hostRoom.forumName) / \(room)"
            } else {
                liveTitle = room
            }
            isOwner = core.isGroupOwner(conversation.id)
        }
    }

    /// Send READ receipts (✓✓ blue) for everything from the other side — we're
    /// looking at the conversation. Fired on open and when a NEW message lands
    /// (`rows.count` grew), not on every sync tick: one batched receipt per arrival,
    /// never a fold-and-check every 3s. Fire-and-forget; the core is idempotent and
    /// authors nothing when nothing is owed.
    private func markRead() {
        Task { await core.markConversationRead(conversation) }
    }

    private func send() {
        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        let parent = replyingTo?.msg
        Task {
            let ok: Bool
            if let parent {
                ok = await core.reply(conversation, text: text, to: parent)
            } else {
                switch conversation.kind {
                case .group: ok = await core.sendToGroup(conversation.id, text)
                case .dm:    ok = await core.sendToDM(conversation.id, text)
                }
            }
            if ok { draft = ""; replyingTo = nil }   // keep the draft on failure
            load()
        }
    }

    private func beginReply(_ row: ChatRow) {
        withAnimation(.easeOut(duration: 0.15)) { replyingTo = row }
    }

    private func openReactions(_ row: ChatRow) {
        withAnimation(.easeOut(duration: 0.15)) { reactingTo = row }
    }

    /// Toggle my `emoji` reaction on `row` (active ⇒ set/replace, else clear),
    /// then reload the folded view. A real forum.react Delta over the relay.
    private func toggle(_ row: ChatRow, _ emoji: String, _ active: Bool) {
        Task {
            _ = await core.react(conversation, to: row.msg, emoji: emoji, active: active)
            load()
        }
    }

    // MARK: - Derived

    /// My existing reaction emoji on this message, if any (drives bar highlight).
    private func myReaction(_ row: ChatRow) -> String? {
        row.reactions.first(where: { $0.mine })?.emoji
    }

    /// The quote (author label + snippet) to render inside a reply bubble, or nil
    /// when the message isn't a reply. "You" when the parent was ours.
    private func quote(for row: ChatRow) -> (author: String, snippet: String)? {
        guard row.isReply else { return nil }
        return (replyAuthor(row), row.replyPreview)
    }

    /// Display name for a reply's parent author: "You" if ours, else the peer's
    /// name in a DM, else a short token in a group.
    private func replyAuthor(_ row: ChatRow) -> String {
        if row.msg.replyToAuthor == core.myIdentityHex { return "You" }
        return conversation.kind == .dm ? conversation.title : String(row.msg.replyToAuthor.prefix(8))
    }

    private func scrollToBottom(_ proxy: ScrollViewProxy) {
        withAnimation(.easeOut(duration: 0.2)) { proxy.scrollTo(bottomID, anchor: .bottom) }
    }
}

/// One message row: the bubble (with its in-line reply quote + send time), the
/// reaction pills beneath it, a swipe-to-reply gesture and a long-press hook for
/// the reaction bar — Signal's message cell, SwiftUI-native.
private struct MessageRow: View {
    let row: ChatRow
    let showAuthor: Bool
    let quote: (author: String, snippet: String)?
    let onReply: () -> Void
    let onLongPress: () -> Void
    let onToggleReaction: (String, Bool) -> Void

    @State private var dragX: CGFloat = 0
    /// True while the current touch went down ON an enriched entity span (stamped at
    /// touch-down by EnrichedText, ~350ms before a long-press can fire). The entity
    /// info panel owns that press — the reaction bar must not also open.
    @State private var pressingEntity = false
    private let replyThreshold: CGFloat = 56

    var body: some View {
        VStack(alignment: row.mine ? .trailing : .leading, spacing: 3) {
            ChatBubble(text: row.text, mine: row.mine,
                       author: row.author, showAuthor: showAuthor,
                       quoted: quote, time: row.timeLabel, receipt: row.receipt,
                       onEntityPress: { pressingEntity = $0 })
                .offset(x: dragX)
                .overlay(alignment: row.mine ? .leading : .trailing) { replyHint }
                .highPriorityGesture(swipe)
                .onLongPressGesture(minimumDuration: 0.35) {
                    if !pressingEntity { onLongPress() }
                }

            if !row.reactions.isEmpty {
                ReactionPills(reactions: row.reactions, onToggle: onToggleReaction)
                    .padding(row.mine ? .trailing : .leading, 6)
            }
        }
        .frame(maxWidth: .infinity, alignment: row.mine ? .trailing : .leading)
    }

    /// The reply arrow that fades in as the bubble is dragged toward centre.
    private var replyHint: some View {
        Image(systemName: "arrowshape.turn.up.left.fill")
            .font(.footnote)
            .foregroundStyle(.secondary)
            .opacity(Double(min(abs(dragX) / replyThreshold, 1)))
            .offset(x: row.mine ? -30 : 30)
    }

    /// Incoming bubbles swipe right, outgoing swipe left (both toward centre),
    /// clamped; past the threshold on release triggers a reply.
    private var swipe: some Gesture {
        DragGesture(minimumDistance: 14)
            .onChanged { v in
                let dx = v.translation.width
                dragX = row.mine ? max(-84, min(0, dx)) : min(84, max(0, dx))
            }
            .onEnded { _ in
                if abs(dragX) >= replyThreshold { onReply() }
                withAnimation(.spring(response: 0.3, dampingFraction: 0.7)) { dragX = 0 }
            }
    }
}

// MARK: - Rename a group (owner-only)

/// Renaming authors an MLS GroupContextExtensions commit: every existing member
/// folds it off the epoch tag on their next sync, and anyone added later reads the
/// new name from their Welcome. Owner-only, exactly like adding a member — the
/// sheet is only reachable when `isGroupOwner` is true.
private struct RenameGroupSheet: View {
    @EnvironmentObject private var core: CoreService
    @Environment(\.dismiss) private var dismiss
    let objectId: String
    let current: String
    var onDone: () -> Void

    @State private var name = ""
    @State private var busy = false
    @State private var note: String?

    private var canSave: Bool {
        let t = name.trimmingCharacters(in: .whitespaces)
        return !t.isEmpty && t != current && !busy
    }

    var body: some View {
        NavigationStack {
            Form {
                Section("Group name") {
                    TextField("Group name", text: $name)
                        .accessibilityIdentifier("rename-group-field")
                }
                if let note {
                    Section { Text(note).font(.footnote).foregroundStyle(.red) }
                }
            }
            .scrollContentBackground(.hidden)
            .background(Theme.bg)
            .navigationTitle("Rename group")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save", action: save).disabled(!canSave).bold()
                }
            }
            .onAppear { name = current }
        }
    }

    private func save() {
        let next = name.trimmingCharacters(in: .whitespaces)
        busy = true
        note = nil
        Task { @MainActor in
            let ok = await core.renameGroup(objectId, to: next)
            busy = false
            if ok {
                onDone()
                dismiss()
            } else {
                // A lost epoch commit slot is retryable — keep the sheet open.
                note = core.error ?? "Couldn't rename — try again."
                core.error = nil
            }
        }
    }
}
