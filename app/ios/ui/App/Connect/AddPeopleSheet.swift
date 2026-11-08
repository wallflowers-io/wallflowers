//  AddPeopleSheet.swift
//  Add a member to ANY GroupObject — a Project, a group chat, or a ChatRoom — by
//  their connect code. This closes the single biggest UI hole: `groupAddMember`
//  was reachable only through the pacific:// dev seam, so every object a user
//  created stayed a permanent group-of-1 with no tap path to invite anyone. This
//  drives the SAME real MLS owner-add the demo scripts do; it also shows the live
//  roster (`objectMembers`) so you can see who's already in.

import SwiftUI
import PacificCore

struct AddPeopleSheet: View {
    @EnvironmentObject private var core: CoreService
    @Environment(\.dismiss) private var dismiss
    let objectId: String
    var label: String = "this space"
    var onChange: () -> Void = {}

    @State private var code = ""
    @State private var busy = false
    @State private var members: [String] = []
    @State private var note: String?

    var body: some View {
        NavigationStack {
            Form {
                Section("In \(label)") {
                    if members.isEmpty {
                        Text("Just you so far.").foregroundStyle(.secondary)
                    } else {
                        ForEach(members, id: \.self) { m in
                            HStack(spacing: 10) {
                                Avatar(initials: String(m.prefix(2)).uppercased(), tint: Brand.accent, size: 26)
                                Text(m == core.myIdentityHex ? "You" : String(m.prefix(8)))
                                    .font(.system(.subheadline,
                                                  design: m == core.myIdentityHex ? .default : .monospaced))
                            }
                        }
                    }
                }
                // TAP-TO-ADD: your existing contacts, added by consuming a stocked
                // prekey — no code exchange. This is the contacts list that was missing.
                Section("Your contacts") {
                    let contacts = core.connectionRows().filter { $0.connected }
                    if contacts.isEmpty {
                        Text("No contacts yet. Pair with someone, or paste their code below.")
                            .foregroundStyle(.secondary)
                    } else {
                        ForEach(contacts, id: \.peerId) { c in
                            Button {
                                busy = true
                                note = nil
                                Task {
                                    let ok = await core.addContact(objectId, peerId: c.peerId)
                                    busy = false
                                    if ok {
                                        reload()
                                        onChange()
                                    } else {
                                        note = core.error ?? "Couldn't add — they may need to refresh their keys."
                                    }
                                }
                            } label: {
                                HStack(spacing: 10) {
                                    Avatar(initials: String((c.displayName.isEmpty ? c.peerId : c.displayName).prefix(2)).uppercased(),
                                           tint: Brand.accent, size: 26)
                                    Text(c.displayName.isEmpty ? String(c.peerId.prefix(8)) : c.displayName)
                                        .foregroundStyle(.primary)
                                    Spacer()
                                    Image(systemName: "plus.circle.fill").foregroundStyle(Brand.accent)
                                }
                            }
                            .disabled(busy)
                            .accessibilityIdentifier("add-contact-\(c.displayName)")
                        }
                    }
                }
                Section {
                    TextField("Paste their connect code", text: $code, axis: .vertical)
                        .accessibilityIdentifier("connect-code-field")
                        .lineLimit(1...4)
                        .font(.system(.footnote, design: .monospaced))
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    Button(action: add) {
                        HStack(spacing: 8) {
                            if busy { ProgressView().controlSize(.small) }
                            Text("Add to \(label)")
                        }
                    }
                    .disabled(code.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || busy)
                } header: {
                    Text("Add someone")
                } footer: {
                    Text(note ?? "They share their connect code from Me → Connect. It's a one-time key package — grab a fresh code for each add.")
                        .foregroundStyle(note == nil ? Color.secondary : Color.red)
                }
            }
            .navigationTitle("People")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
            .onAppear(perform: reload)
        }
    }

    private func reload() { members = core.objectMembers(objectId) }

    private func add() {
        let bundle = code.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !bundle.isEmpty else { return }
        busy = true
        note = nil
        Task {
            let peer = await core.groupAddMember(objectId, bundle: bundle)
            busy = false
            if peer != nil {
                code = ""
                await core.syncNow()
                reload()
                onChange()
            } else {
                note = core.error ?? "Couldn't add — check the connect code is fresh."
            }
        }
    }
}
