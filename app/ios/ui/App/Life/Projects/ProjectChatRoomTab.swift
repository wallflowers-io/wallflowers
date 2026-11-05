//  ProjectChatRoomTab.swift
//  Tab 3 of a Project — its linked ChatRoom: the threaded Forum GroupObject the
//  Project references via a `forum`-kind subscription edge. Renders the same
//  Reddit-style ThreadView the Chats/Live surfaces use. If nothing is linked yet,
//  offers to mint + link one (a real forum object + a real subscribe delta).

import SwiftUI
import PacificCore

struct ProjectChatRoomTab: View {
    @EnvironmentObject private var core: CoreService
    let objectId: String

    @State private var forumId: String?
    @State private var linking = false

    var body: some View {
        Group {
            if let forumId {
                ThreadView(objectId: forumId, title: "Channel")
            } else {
                VStack(spacing: 12) {
                    Spacer()
                    Image(systemName: "bubble.left.and.bubble.right")
                        .font(.largeTitle).foregroundStyle(Theme.textSecondary)
                    Text("No Channel yet").font(.headline).foregroundStyle(Theme.textPrimary)
                    Text("Link a threaded Channel to this project — its own MLS group, connected by an edge.")
                        .font(.subheadline).foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                    Button(action: link) {
                        HStack(spacing: 8) {
                            if linking { ProgressView().controlSize(.small).tint(Theme.onAccent) }
                            Text("Create Channel")
                        }
                    }
                    .buttonStyle(.pixel)
                    .foregroundStyle(Theme.onAccent)
                    .disabled(linking)
                    Spacer()
                }
                .padding(24)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .background(Brand.groupBG)
        .onAppear { forumId = core.projectChatRoom(objectId) }
        .onChange(of: core.syncTick) { _, _ in
            if forumId == nil { forumId = core.projectChatRoom(objectId) }
        }
    }

    private func link() {
        linking = true
        Task {
            forumId = await core.linkChatRoom(objectId)
            linking = false
        }
    }
}
