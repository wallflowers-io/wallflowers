//  ChatsView.swift
//  The CHATS tab — a Signal conversation list over the real MLS core. Groups are
//  forum objects; DMs are 1:1 connections. Real-ops only: every row is the
//  residue of real deltas (pairing + real posts) — no seed, no fixtures, one
//  list, one write path.

import SwiftUI
import PacificCore   // ObjectRow — the forums the list is built from

struct ChatsView: View {
    @EnvironmentObject private var core: CoreService
    @EnvironmentObject private var convos: ConversationStore
    @EnvironmentObject private var shell: ShellState
    @EnvironmentObject private var chatIndex: ChatIndex

    @State private var rows: [ConvRowVM] = []
    /// The rebuild reading now, cancelled by the next.
    @State private var rebuilding: Task<Void, Never>?
    @State private var query = ""
    /// The semantic lane's answer for the CURRENT query: conversation id → rank,
    /// from the union store's cosine search over every indexed message. Refreshed
    /// by the debounced task below; empty while blank/"#" queries are up.
    @State private var semantic: [String: Int] = [:]

    /// The scope lives on the shell (`ShellState.chatScope`) and the GOLD disc is
    /// its only control — see `ChatScope`. The four chips that used to sit here
    /// are gone: "All" answered nothing, and Groups/Channels split one idea.
    private var filtered: [ConvRowVM] {
        // A room is one a SPACE hosts — `forumHosts` is the forum→Group map the list
        // already joins on to dress a row "Group / Room". Anything absent from it is a
        // conversation between people, DM or group alike.
        let kept = rows.filter {
            shell.chatScope.admits(hostedRoom: core.forumHosts[$0.conversation.id] != nil)
        }
        let q = query.trimmingCharacters(in: .whitespaces).lowercased()
        guard !q.isEmpty else { return kept }

        // "#…" is a TAG query — exact intent over the transcripts' #tags, prefix
        // matched ("#pars" finds #parsnips). No cosine: a tag is a name, not a theme.
        if q.hasPrefix("#") {
            let tag = String(q.dropFirst())
            guard !tag.isEmpty else { return kept }
            return kept.filter { r in r.tags.contains { $0.hasPrefix(tag) } }
        }

        // Two lanes, fused by RANK (RankFusion — the one home of the RRF rule; a
        // cosine and a substring share no scale). EXACT = the contains-filter in
        // recency order; SEMANTIC = the union store's cosine over every indexed
        // message, folded to conversations. A row either lane found is shown;
        // found by both, it outranks either.
        var exactRank: [String: Int] = [:]
        var n = 0
        for r in kept where exactMatch(r, q) { n += 1; exactRank[r.conversation.id] = n }
        return kept
            .filter { exactRank[$0.conversation.id] != nil || semantic[$0.conversation.id] != nil }
            .sorted { a, b in
                let sa = RankFusion.score(exactRank[a.conversation.id], semantic[a.conversation.id])
                let sb = RankFusion.score(exactRank[b.conversation.id], semantic[b.conversation.id])
                return sa == sb ? a.lastTs > b.lastTs : sa > sb
            }
    }

    /// The exact lane: does the query literally appear in what the row shows —
    /// its title, its last message, or (for a forum) its hosting group's name?
    private func exactMatch(_ r: ConvRowVM, _ q: String) -> Bool {
        r.conversation.title.lowercased().contains(q)
            || r.snippet.lowercased().contains(q)
            || (r.host?.lowercased().contains(q) ?? false)
    }

    var body: some View {
        // Selecting a conversation hands it to the SHELL, which presents it above the
        // disc row (see `ShellState.open`) — not as an overlay here, which would sit
        // UNDER the row and blend with it instead of covering it.
        NavigationStack { chatList }
        .onAppear { rebuild() }
        // Preloaded screen: onAppear fires once at launch; a real arrival is the
        // shell's page landing here (see ShellState.screen).
        .onChange(of: shell.screen) { _, s in if s == .chat { rebuild() } }
        .onReceive(NotificationCenter.default.publisher(for: .pacificOpenDM)) { note in
            // Deep link (pacific://open-dm) → open the peer's DM.
            guard let peer = note.userInfo?["peer"] as? String else { return }
            rebuild()
            let title = core.connection(peer)?.displayName ?? String(peer.prefix(12)) + "…"
            open(Conversation(id: peer, kind: .dm, title: title, tint: tintFor(peer)))
        }
        // New chat + the QR ride the shell's row overlay (the Settings choreography:
        // red disc centres as Close, content under the row) — not sheets.
        .onReceive(NotificationCenter.default.publisher(for: .pacificNewChat)) { _ in shell.openOverlay(.newChat) }
        .onReceive(NotificationCenter.default.publisher(for: .pacificShowQR)) { _ in shell.openOverlay(.connect) }
        .onChange(of: core.syncTick) { _, _ in rebuild() }   // background sync landed → refresh the list
        // The semantic lane, debounced a beat behind the keystroke: cosine over the
        // union store's message vectors. The exact lane answers instantly from the
        // rows in hand; this one augments the same list when it lands.
        .task(id: query) {
            try? await Task.sleep(for: .milliseconds(250))
            guard !Task.isCancelled else { return }
            semantic = await chatIndex.semanticRanks(query: query)
        }
    }

    /// Hand a conversation to the shell, which presents it and runs the whole morph.
    /// A Group-hosted room is a FORUM — it opens as the threaded room (ThreadView,
    /// via the shell's room cover), never as a flat chat; the cover's title carries
    /// the same "Group / Room" its row wears. Free-standing conversations keep the
    /// Signal surface.
    private func open(_ convo: Conversation) {
        if convo.kind == .group, let host = core.forumHosts[convo.id] {
            let hostName = host.groupName.isEmpty ? "Group" : host.groupName
            NotificationCenter.default.post(
                name: .pacificOpenRoom, object: nil,
                userInfo: ["object": convo.id,
                           "title": "\(hostName) / \(convo.title)",
                           "group": host.group])   // the title deep-links back to the group
            return
        }
        // A Channel's Room is likewise a forum — it opens threaded, wearing its
        // channel. No group deep-link: its host is a forum, not a group page.
        if convo.kind == .group, let room = core.roomHosts[convo.id] {
            let hostName = room.forumName.isEmpty ? "Channel" : room.forumName
            NotificationCenter.default.post(
                name: .pacificOpenRoom, object: nil,
                userInfo: ["object": convo.id,
                           "title": "\(hostName) / \(convo.title)"])
            return
        }
        shell.open(convo)
    }

    /// Names what is being left, so the dialog is about a conversation rather than
    /// about "this item".
    private func leaveTitle(_ convo: Conversation) -> String {
        "Leave \(convo.title.isEmpty ? "this conversation" : convo.title)?"
    }
    /// Named when leaving is REFUSED — an owner must hand a Space over first.
    @State private var cannotLeave = ""

    /// LEAVE, which is what deleting a conversation actually is: the transcript is not
    /// yours alone to destroy, so this removes YOU from it — an MLS Remove another member
    /// completes, after which nothing new reaches you. Core's own precondition is honoured
    /// rather than discovered at the commit — an owner with members still on the roster
    /// has to hand over first (`mustHandOverBeforeLeaving`).
    private func leave(_ convo: Conversation) {
        guard !core.mustHandOverBeforeLeaving(convo.id) else {
            cannotLeave = "Hand \(convo.title.isEmpty ? "this Space" : convo.title) to another member before leaving it."
            return
        }
        Task { await core.leaveGroup(convo.id); rebuild() }
    }

    /// PROMOTE — the chat's people, carried into a new Space. The conversation itself is
    /// untouched: nothing is merged, no transcript is copied, no edge is drawn. Only WHO
    /// comes across, which is the same ruling `NewSpaceFromChat` was built on.
    ///
    /// The members ride the COMMIT CLOSURE, not the audience dial: that dial is
    /// deliberately inert until visibility is a flag on every delta, and seeding people
    /// through it would drop them silently — the exact bug the field declaration exists
    /// to prevent.
    private func promote(_ convo: Conversation) {
        let peers = convo.kind == .dm
            ? [convo.id]
            : core.objectMembers(convo.id).filter { CoreService.identityHex($0) != core.myIdentityHex }
        shell.beginNew(NewSession(kind: .space) { draft in
            guard let id = await core.mint(kind: NewKind.space.rawValue, draft: draft.mint(for: .space))
            else { return nil }          // core surfaced the failure; the surface stays up
            await core.setOutside(id, from: draft)
            for peer in peers { _ = await core.addContact(id, peerId: peer) }
            core.refreshOrgs()
            return .saved
        })
    }

    /// The conversation list — the root of the Chats tab.
    private var chatList: some View {
        Group {
            if rows.isEmpty {
                emptyState
            } else {
                // The shared search field, not `.searchable` — the system one relocates to
                // the bottom of the screen on iOS 26. It sits where NOTE's does, and hides
                // upward as you scroll into the list. No accessory rides under it any
                // more: the scope moved to the GOLD disc, which freed this whole band.
                SearchableList(text: $query) {
                    List {
                        if filtered.isEmpty {
                            // The SAME word as the whole-surface empty state. An empty
                            // scope is not a different condition from an empty tab —
                            // both are CHAT with nothing in it — and "Nothing here" was
                            // the surface commenting on itself instead of naming itself.
                            emptyWord
                                .frame(maxWidth: .infinity)
                                .padding(.top, 80)
                                .listRowBackground(Color.clear)
                                .listRowSeparator(.hidden)
                        }
                        ForEach(filtered) { r in
                            Button {
                                open(r.conversation)
                            } label: {
                                ConversationRow(title: r.conversation.title,
                                                snippet: r.snippet,
                                                time: r.time,
                                                unread: r.unread,
                                                tint: r.conversation.tint,
                                                initials: r.conversation.initials,
                                                photo: r.photo,
                                                blockColor: r.blockColor,
                                                // A Group-hosted room wears the forum dress:
                                                // the room glyph + "Host … / Room".
                                                host: r.host)
                            }
                            .buttonStyle(.plain)
                            // Lead the row's a11y label with the conversation title. A Button
                            // flattens its children, and the row's avatar-initials text would
                            // otherwise come first — so VoiceOver (and name-based UI lookups)
                            // wouldn't hear/find the row by contact name.
                            .accessibilityLabel(a11y(r))
                            // The row owns its horizontal drag, so the shell's page-swipe
                            // must leave drags that start here alone. `blocksPageSwipe`
                            // publishes THIS ROW's frame — never the whole list, which
                            // would kill paging across the tab.
                            // Leave on GROUP chats only: a DM is a connection, and leaving one
                            // is unpairing — a different operation, not offered here
                            // (membership-through-mls.md §16).
                            .leaveSwipe(if: r.conversation.kind == .group,
                                        title: leaveTitle(r.conversation)) {
                                leave(r.conversation)
                            }
                            .swipeActions(edge: .leading, allowsFullSwipe: false) {
                                Button { promote(r.conversation) } label: {
                                    Label("New Space", systemImage: "person.3.fill")
                                }
                                .tint(DiscSlot.life.blockColor)
                                .accessibilityIdentifier("chat-promote")
                            }
                        }
                        .listRowBackground(Color.clear)
                        .listRowSeparator(.hidden)
                        .listRowInsets(EdgeInsets(top: 2, leading: 16, bottom: 2, trailing: 16))
                    }
                    .listStyle(.plain)
                    .scrollContentBackground(.hidden)
                    .refreshable { await core.syncNow(); rebuild() }
                }
                // No `blocksPageSwipe()` on these rows on purpose: they have no swipe actions,
                // so there is nothing for the page-swipe to yield to. Blocking it anyway only
                // cost CHAT its swipe to NOTE and LIFE once it had a single conversation.
            }
        }
        .background(Theme.bg)
        .accountToolbar()
        .tabChrome("Chats", newLabel: "New chat", showsTitle: false, showsNotifications: true) { shell.openOverlay(.newChat) }
        .alert("Can\u{2019}t leave yet", isPresented: Binding(get: { !cannotLeave.isEmpty },
                                                            set: { if !$0 { cannotLeave = "" } })) {
            Button("OK", role: .cancel) {}
        } message: { Text(cannotLeave) }
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


    /// A hosted room reads "Host / Room" everywhere a name is spoken or searched.
    private func a11y(_ r: ConvRowVM) -> String {
        var name = r.conversation.title
        if let host = r.host { name = "\(host.isEmpty ? "Group" : host) / \(name)" }
        return r.snippet.isEmpty ? name : "\(name), \(r.snippet)"
    }

    /// The list, re-read OFF the main thread and drawn from what it read: an arrival on
    /// CHAT shows the last rows at once, and the new ones when they land. A rebuild asked
    /// for while one is reading supersedes it.
    private func rebuild() {
        rebuilding?.cancel()
        rebuilding = Task {
            // The forum→host map, re-read with the list: a room minted or detached
            // since the last pass must arrive already wearing (or shedding) its group.
            await core.refreshForumHosts().value
            // An object's comments section is reached from the object, not listed here (#34).
            let groups = core.groupObjects.filter { !core.index.isCommentsSection($0.id) }
            let snap = await core.chatSnapshot(groups: groups)
            guard !Task.isCancelled else { return }
            rows = build(groups: groups, snap)
        }
    }

    /// The rows from a snapshot: no FFI here, it all came with `snap`.
    private func build(groups: [ObjectRow], _ snap: CoreService.ChatSnapshot) -> [ConvRowVM] {
        var out: [ConvRowVM] = []

        // One roster-name map for the whole rebuild, not one lookup per row. Blocked peers
        // included: a blocked member's group messages still render, with a name.
        let peerNames = core.index.names

        for o in groups {
            let rows = snap.messages[o.id] ?? []
            convos.noteSeen(o.id, count: rows.count)
            // The MLS GroupContext name is authoritative and synced — it arrives with
            // the Welcome and is rewritten by an owner rename commit. An unnamed group
            // (a Project ChatRoom, a dev-link room) gets a roster-derived label; the id
            // hex is an address and is never shown as a name.
            let title = o.name.isEmpty
                ? core.derivedGroupName(members: core.index.object(o.id)?.members ?? [], names: peerNames) : o.name
            let snip = ChatRow.lastSnippet(rows)
            let lastTs = Int64(rows.last?.ts ?? 0)
            // A group-hosted forum wears its group; a channel-hosted room wears
            // its channel — the same "Host / Room" dress either way.
            let host = core.forumHosts[o.id]?.groupName
                ?? core.roomHosts[o.id].map { $0.forumName.isEmpty ? "Channel" : $0.forumName }
            // Vectorise the transcript we just folded (watermarked — only new
            // messages embed). The #tag set comes from the index's accumulated,
            // persisted sets — never a full-transcript rescan per rebuild.
            chatIndex.ingest(conversation: o.id, kind: host == nil ? .chat : .forum, msgs: rows)
            out.append(ConvRowVM(
                conversation: Conversation(id: o.id, kind: .group, title: title, tint: tintFor(o.id)),
                snippet: snip.isEmpty ? "No messages yet" : snip,
                unread: convos.unread(o.id, count: rows.count),
                time: Self.activityLabel(lastTs),
                lastTs: lastTs,
                host: host,
                tags: chatIndex.tags(conversation: o.id, msgs: rows)
            ))
        }

        // The blocklist is THE people seam (`connectionRows`): a blocked peer has no row.
        for c in snap.connections where !BlockStore.shared.contains(c.peerId) {
            let rows = snap.messages[c.peerId] ?? []
            convos.noteSeen(c.peerId, count: rows.count)
            // A bare CONNECTED connection with no chat yet belongs under "+", not in
            // Chats. An UN-accepted one is different: it is an inbound request, and
            // Chats is the only surface that leads to the accept bar. Hiding it here
            // is why a scanned person saw nothing at all — a fresh pairing carries no
            // messages, so the request was filtered out before it could be answered.
            if rows.isEmpty && c.connected { continue }
            let title = c.displayName.isEmpty ? String(c.peerId.prefix(12)) + "…" : c.displayName
            let snip = ChatRow.lastSnippet(rows)
            let lastTs = Int64(rows.last?.ts ?? 0)
            chatIndex.ingest(conversation: c.peerId, kind: .chat, msgs: rows)
            out.append(ConvRowVM(
                conversation: Conversation(id: c.peerId, kind: .dm, title: title, tint: tintFor(c.peerId)),
                snippet: snip.isEmpty ? (c.connected ? "Connected — say hello" : "Pending connection") : snip,
                unread: convos.unread(c.peerId, count: rows.count),
                time: Self.activityLabel(lastTs),
                lastTs: lastTs,
                tags: chatIndex.tags(conversation: c.peerId, msgs: rows),
                photo: core.index.photo(c.peerId),
                blockColor: core.index.color(c.peerId)
            ))
        }

        // Newest activity first: sort by the last message's stamp descending, regardless of
        // group-vs-DM or the core's own listing order. A conversation with no messages (ts 0)
        // falls to the bottom.
        return out.sorted { $0.lastTs > $1.lastTs }
    }

    /// A Signal-style last-activity label from an epoch-ms stamp: the time today, "Yesterday",
    /// a weekday within the past week, else the date. Empty for a conversation with no messages.
    private static func activityLabel(_ ts: Int64) -> String {
        guard ts > 0 else { return "" }
        let date = Date(timeIntervalSince1970: Double(ts) / 1000)
        let cal = Calendar.current
        if cal.isDateInToday(date) { return date.formatted(date: .omitted, time: .shortened) }
        if cal.isDateInYesterday(date) { return "Yesterday" }
        let days = cal.dateComponents([.day], from: cal.startOfDay(for: date),
                                      to: cal.startOfDay(for: Date())).day ?? 0
        if days < 7 { return date.formatted(.dateTime.weekday(.abbreviated)) }
        return date.formatted(.dateTime.day().month(.abbreviated))
    }

    private var emptyState: some View { EmptyWord(shell.chatScope.word) }

    /// What an empty CHAT says: its own name — the app's one empty word, worn bare
    /// so the List row can place it.
    private var emptyWord: some View {
        EmptyWord(shell.chatScope.word, placed: false)
    }
}

struct ConvRowVM: Identifiable {
    var conversation: Conversation
    var snippet: String
    var unread: Int
    var time: String
    /// Last message's send-stamp (epoch ms), 0 when there are none — the recency sort key so the
    /// list reads newest-active first.
    var lastTs: Int64 = 0
    /// The hosting Group's name when this forum is a group's room ("" = a group
    /// whose profile was never set); nil = a free-standing conversation.
    var host: String? = nil
    /// Every #tag anywhere in this conversation's transcript, lowercased —
    /// derived from the fold at list build, never stored. The "#…" query lane.
    var tags: Set<String> = []
    /// The face a DM's peer PUBLISHED to us, read with the list: a group has no single
    /// face, so it keeps initials.
    var photo: Data? = nil
    var blockColor: Color? = nil
    var id: String { conversation.kind.rawValue + ":" + conversation.id }
}

private extension View {
    /// The Leave swipe, only where leaving is an operation this app offers.
    @ViewBuilder
    func leaveSwipe(if enabled: Bool, title: String, perform: @escaping () -> Void) -> some View {
        if enabled {
            self.swipeToRemove(title: title, verb: "Leave", perform: perform)
        } else {
            self
        }
    }
}
