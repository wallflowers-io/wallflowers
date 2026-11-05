//  NewChatSheet.swift
//  The compose surface, Signal/WhatsApp-shaped: a top-of-list set of actions
//  (New group · Invite to Pacific · Show my QR · Enter a code) over YOUR
//  connections — tap a connection to open its DM. No fabricated peers; DMs appear
//  only once a real MLS connection is formed. "Invite" piggybacks iOS Contacts
//  (see InviteSheet). Group creation makes a real forum object you own.

import SwiftUI
import PacificCore

struct NewChatSheet: View {
    @EnvironmentObject private var core: CoreService
    @EnvironmentObject private var shell: ShellState   // row-overlay host: close + QR swap

    var onOpen: (Conversation) -> Void

    @State private var showInvite = false    // invite from Contacts
    @State private var showGroup = false     // new group
    @State private var showPaste = false     // enter a code
    @State private var showScan = false      // scan their QR (camera)

    private var connections: [ConnectionRow] { core.connectionRows() }

    var body: some View {
        NavigationStack {
            List {
                Section {
                    ComposeActionRow("New group", "person.3.fill") { showGroup = true }
                    ComposeActionRow("Invite to WallFlowers", "person.crop.circle.badge.plus") { showInvite = true }
                    // The QR is a SIBLING under the same centred row, not a nested
                    // sheet — the discs stay put and only the content swaps.
                    ComposeActionRow("Show my QR code", "qrcode") { shell.swapOverlay(.connect) }
                    ComposeActionRow("Scan a QR code", "qrcode.viewfinder") { showScan = true }
                    ComposeActionRow("Enter a code", "character.cursor.ibeam") { showPaste = true }
                }

                Section("Your connections") {
                    if connections.isEmpty {
                        Text("No connections yet. Invite someone, or show your QR code to connect in person.")
                            .font(.footnote).foregroundStyle(.secondary)
                    } else {
                        ForEach(connections, id: \.peerId) { c in
                            Button { openDM(c) } label: { ConnectionRowLabel(c: c) }
                                .buttonStyle(.plain)
                                // Lead the row's a11y label with the contact name — the
                                // Button otherwise flattens to the avatar initials first,
                                // so VoiceOver / name lookups wouldn't find it by name.
                                .accessibilityLabel("\(displayName(c)), \(c.connected ? "Connected" : "Pending")")
                        }
                    }
                }
            }
            .scrollContentBackground(.hidden)
            .background(Theme.bg)
            .navigationTitle("New chat")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .topBarLeading) { Button("Cancel") { shell.dismissSettings() } } }
            .sheet(isPresented: $showInvite) { InviteSheet() }
            .sheet(isPresented: $showGroup) { NewGroupSheet(onOpen: onOpen) }
            .sheet(isPresented: $showPaste) { PasteCodeSheet(onOpen: onOpen) }
            .sheet(isPresented: $showScan) { ScanCodeSheet(onOpen: onOpen) }
        }
    }

    private func openDM(_ c: ConnectionRow) {
        let name = displayName(c)
        onOpen(Conversation(id: c.peerId, kind: .dm, title: name, tint: tintFor(c.peerId)))
    }

    private func displayName(_ c: ConnectionRow) -> String {
        c.displayName.isEmpty ? String(c.peerId.prefix(12)) + "…" : c.displayName
    }
}

/// One tappable action row (leading tinted icon + title).
private struct ComposeActionRow: View {
    let title: String
    let icon: String
    let action: () -> Void
    init(_ title: String, _ icon: String, action: @escaping () -> Void) {
        self.title = title; self.icon = icon; self.action = action
    }
    var body: some View {
        Button(action: action) {
            HStack(spacing: 14) {
                Image(systemName: icon)
                    .font(.system(size: 17))
                    .foregroundStyle(Brand.accent)
                    .frame(width: 30, height: 30)
                Text(title).foregroundStyle(.primary)
                Spacer()
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

/// A connection row: avatar + name + connected/pending state.
private struct ConnectionRowLabel: View {
    @EnvironmentObject private var core: CoreService
    let c: ConnectionRow
    private var name: String { c.displayName.isEmpty ? String(c.peerId.prefix(12)) + "…" : c.displayName }
    var body: some View {
        HStack(spacing: 12) {
            Avatar(initials: Avatar.initials(name), tint: tintFor(c.peerId), size: 40,
                   photo: core.peerPhoto(c.peerId), color: core.peerColor(c.peerId))
            VStack(alignment: .leading, spacing: 1) {
                Text(name).foregroundStyle(.primary)
                Text(c.connected ? "Connected" : "Pending")
                    .font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
        }
        .contentShape(Rectangle())
    }
}

// MARK: - New group

/// Name + create a real forum object you own (a group of 1 to start). Members are
/// added by their invite code/QR — the same MLS owner-add the pairing uses.
struct NewGroupSheet: View {
    @EnvironmentObject private var core: CoreService
    @Environment(\.dismiss) private var dismiss
    var onOpen: (Conversation) -> Void

    @State private var name = ""
    private var canCreate: Bool { !name.trimmingCharacters(in: .whitespaces).isEmpty }

    var body: some View {
        NavigationStack {
            Form {
                Section("Group name") {
                    TextField("e.g. Fieldwork — Central Valley", text: $name)
                        .accessibilityIdentifier("group-name-field")
                }
                Section {
                    Text("Creates a group you own. Add people with their invite code or QR — the same handshake as a 1:1.")
                        .font(.footnote).foregroundStyle(.secondary)
                }
            }
            .scrollContentBackground(.hidden)
            .background(Theme.bg)
            .navigationTitle("New group")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Create") { create() }.disabled(!canCreate).bold()
                }
            }
        }
    }

    private func create() {
        let title = name.trimmingCharacters(in: .whitespaces)
        // The name rides the MLS GroupContext extension, so every member — including
        // late joiners, via their Welcome — reads it, and the owner can rewrite it
        // later with a rename commit. No title delta, no device-local mirror.
        guard let id = core.createGroupConversation(name: title) else { return }
        dismiss()
        onOpen(Conversation(id: id, kind: .group, title: title, tint: tintFor(id)))
    }
}

// MARK: - Enter a code (the sim-friendly stand-in for camera scanning)

/// Paste someone's connect code (their signed bundle): pair_scan forms the 1:1
/// and seals the Welcome to them over the relay; they accept on their side.
struct PasteCodeSheet: View {
    @EnvironmentObject private var core: CoreService
    @Environment(\.dismiss) private var dismiss
    var onOpen: (Conversation) -> Void

    @State private var code = ""
    @State private var pairing = false
    private var canPair: Bool {
        !code.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !pairing
    }

    var body: some View {
        NavigationStack {
            Form {
                Section("Their connect code") {
                    TextField("Paste the code they shared", text: $code, axis: .vertical)
                        .accessibilityIdentifier("pair-code-field")
                        .lineLimit(2...5)
                        .font(.caption.monospaced())
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                }
                Section {
                    Button { pair() } label: {
                        if pairing {
                            Label("Pairing…", systemImage: "arrow.triangle.2.circlepath")
                        } else {
                            Label("Connect", systemImage: "link")
                        }
                    }
                    .disabled(!canPair)
                    Text("Forms the encrypted 1:1 over the relay. They accept on their side to complete it.")
                        .font(.footnote).foregroundStyle(.secondary)
                }
            }
            .scrollContentBackground(.hidden)
            .background(Theme.bg)
            .navigationTitle("Enter a code")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .topBarLeading) { Button("Cancel") { dismiss() } } }
        }
    }

    private func pair() {
        let bundle = code.trimmingCharacters(in: .whitespacesAndNewlines)
        pairing = true
        Task {
            defer { pairing = false }
            guard let peer = await core.pairScan(bundle) else { return }
            let title = core.connection(peer)?.displayName ?? String(peer.prefix(12)) + "…"
            dismiss()
            onOpen(Conversation(id: peer, kind: .dm, title: title, tint: tintFor(peer)))
        }
    }
}

/// Real pairing: shows my signed contact bundle as a QR + my SAS for out-of-band
/// verification. Scanning (or pasting) this on another device forms the connection.
struct ConnectSheet: View {
    @EnvironmentObject private var core: CoreService
    @EnvironmentObject private var shell: ShellState   // row-overlay host: the ground + disc both close
    @State private var copied = false

    /// THE SHEET WATCHES FOR THE SCAN IT INVITED. Holding your code out is the one
    /// moment you are certainly looking at this screen, and the scan used to land
    /// in silence: the connection appeared in the log, the QR sat there unchanged,
    /// and the person showing it had no idea. The sync tick already drains the
    /// intro; this gives the moment somewhere to appear, in front of the person who
    /// just scanned — which is exactly where consent means something.
    ///
    /// `known` is the connection set at open, so only a connection that arrives
    /// AFTER that, un-accepted, counts as "someone just scanned me" — old pending
    /// rows stay out of it.
    @State private var known: Set<String> = []
    @State private var inbound: (peer: String, name: String)? = nil
    @State private var accepted = false

    /// The other half of pairing. Holding your code out and scanning someone else's are
    /// the same act from the two ends, so they belong on one screen — you should not have
    /// to know in advance which of you is holding the phone up.
    @State private var scanning = false

    var body: some View {
        // Full-screen black: who you are, then only the QR — the code, a text-only
        // copy link, the three words small and grey. No chrome — the whole ground
        // dismisses (it carries the "Done" accessibility label so a tap anywhere is
        // the close, spoken as such), and the centred red disc closes too.
        ZStack {
            Theme.bg.ignoresSafeArea()
                .contentShape(Rectangle())
                .onTapGesture { shell.dismissSettings() }
                .accessibilityLabel("Done")
                .accessibilityAddTraits(.isButton)

            VStack(spacing: 24) {
                Spacer()
                VStack(spacing: 10) {
                    Avatar(initials: Avatar.initials(core.displayName), tint: core.avatarColor,
                           size: 64, photo: core.avatarPhoto, color: core.avatarColor)
                    if !core.displayName.isEmpty {
                        Text(core.displayName)
                            .font(.headline)
                            .foregroundStyle(.white)
                    }
                }
                let bundle = core.contactBundle()
                if bundle.isEmpty {
                    Label("Couldn’t create your connect code", systemImage: "exclamationmark.triangle")
                        .font(.subheadline).foregroundStyle(.orange)
                } else {
                    QRView(payload: bundle, size: 260)
                        .padding(16)
                        .background(.white, in: RoundedRectangle(cornerRadius: 16))

                    Button {
                        UIPasteboard.general.string = bundle
                        copied = true
                    } label: {
                        Text(copied ? "copied" : "copy code")
                            .font(.footnote)
                            .foregroundStyle(.gray)
                    }
                    .buttonStyle(.plain)
                }

                if !core.sasWords.isEmpty {
                    Text(core.sasWordsDotted)
                        .font(.footnote)
                        .foregroundStyle(.gray)
                        .multilineTextAlignment(.center)
                }
                Spacer()

                // SCAN THEIRS. A pairing is a DOUBLE OPT-IN, so this screen has to work
                // from both ends: the QR above is you being scanned, this is you scanning.
                // Without it the `+` on PEOPLE could only ever wait to be found.
                Button { scanning = true } label: {
                    Label("Scan a code", systemImage: "camera.fill")
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.white)
                        .padding(.horizontal, 20)
                        .frame(height: CircleButton.discSize)
                        .background(Capsule().fill(Theme.surface2))
                        .overlay(Capsule().strokeBorder(Theme.hairline, lineWidth: 1))
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("connect-scan")
                .padding(.bottom, 8)

                if let inbound { inboundCard(inbound).padding(.bottom, 24) }
            }
            .padding(.horizontal)
        }
        .presentationBackground(.black)
        // Scanning THEIRS closes this screen the moment a conversation exists — the
        // pairing is done and there is nothing left to hold up.
        .sheet(isPresented: $scanning) {
            ScanCodeSheet { _ in
                scanning = false
                shell.dismissSettings()
            }
        }
        .onAppear { known = Set(core.connectionRows().map(\.peerId)) }
        .onReceive(core.$syncTick) { _ in noticeInbound() }
    }

    /// The scan, answered where it happened. Name + Accept, nothing else.
    @ViewBuilder
    private func inboundCard(_ hit: (peer: String, name: String)) -> some View {
        HStack(spacing: 12) {
            Image(systemName: accepted ? "checkmark.circle.fill" : "person.crop.circle.badge.plus")
                .foregroundStyle(accepted ? Color.green : Brand.accent)
            Text(accepted ? hit.name : "\(hit.name) wants to connect")
                .font(.subheadline)
            Spacer()
            if !accepted {
                Button("Accept") {
                    core.pairAccept(hit.peer)
                    accepted = true
                    UINotificationFeedbackGenerator().notificationOccurred(.success)
                }
                .buttonStyle(.pixel)
                .controlSize(.small)
            }
        }
        .padding(.horizontal, 14).padding(.vertical, 10)
        .background(.bar, in: RoundedRectangle(cornerRadius: 12))
        .transition(.move(edge: .bottom).combined(with: .opacity))
    }

    /// A connection that was not here when the sheet opened, and is not yet
    /// accepted, is a scan that just landed. First one wins — the person in front
    /// of you is the one you are answering.
    private func noticeInbound() {
        guard inbound == nil else { return }
        guard let hit = core.connectionRows().first(where: { !known.contains($0.peerId) && !$0.connected })
        else { return }
        let name = hit.displayName.isEmpty ? String(hit.peerId.prefix(8)) + "…" : hit.displayName
        withAnimation { inbound = (peer: hit.peerId, name: name) }
        UINotificationFeedbackGenerator().notificationOccurred(.warning)
    }
}
