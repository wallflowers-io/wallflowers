//  InAppNotify.swift
//  Foreground message announcements — in-app, never iOS.
//
//  THE RULE. While the app is on screen, a new message is announced by the APP (a small
//  top banner that routes to its conversation), not by an iOS notification banner over
//  it. iOS banners belong to the background (the waker's content-free push → InboundWake's
//  real local notification); NotifPresenter suppresses them in the foreground and this
//  layer takes over. One announcement surface per app state, never both.
//
//  WHAT SHOWS. Sender (or room) + one-line preview. Tap → the conversation, through the
//  same pacificOpenDM/pacificOpenRoom seam every other route uses. Auto-dismisses; a
//  second arrival replaces the banner rather than stacking.
//
//  WHAT NEVER SHOWS. The conversation currently on screen (ChatFocus) — announcing what
//  the user is looking at is noise. InboundWake's seen-set still records those, so they
//  are not re-announced later by a background wake.

import SwiftUI

/// Which conversation is ON SCREEN right now. ChatView / ThreadView report entry and
/// exit; the foreground announcer skips it. Keys match InboundWake's conversation keys:
/// `dm:<peer>` / `grp:<object>`.
@MainActor
final class ChatFocus {
    static let shared = ChatFocus()
    private(set) var current: String?
    func enter(_ id: String) { current = id }
    func leave(_ id: String) { if current == id { current = nil } }
}

/// The one in-app banner. `show` replaces; the newest arrival owns the slot.
@MainActor
final class InAppNotify: ObservableObject {
    struct Item: Equatable {
        let title: String
        let body: String
        let routeKey: String
        let routeValue: String
    }

    @Published private(set) var item: Item?
    private var hideTask: Task<Void, Never>?

    func show(title: String, body: String, routeKey: String, routeValue: String) {
        hideTask?.cancel()
        withAnimation(.spring(duration: 0.3)) {
            item = Item(title: title, body: body, routeKey: routeKey, routeValue: routeValue)
        }
        hideTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: 4_000_000_000)
            guard let self, !Task.isCancelled else { return }
            withAnimation(.easeOut(duration: 0.25)) { self.item = nil }
        }
    }

    func dismiss() {
        hideTask?.cancel()
        withAnimation(.easeOut(duration: 0.2)) { item = nil }
    }
}

/// The banner itself — mounted once at the app root, over everything (including an
/// open conversation, which is exactly where a banner for ANOTHER chat matters).
struct InAppBannerHost: View {
    @ObservedObject var notify: InAppNotify

    var body: some View {
        if let item = notify.item {
            HStack(spacing: 10) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(item.title)
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.white)
                        .lineLimit(1)
                    Text(item.body)
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 14).padding(.vertical, 10)
            .pixelCard(Theme.surface2, shadow: true)
            .padding(.horizontal, 16)
            .contentShape(Rectangle())
            .onTapGesture {
                _ = InboundWake.route([item.routeKey: item.routeValue])
                notify.dismiss()
            }
            .transition(.move(edge: .top).combined(with: .opacity))
            .accessibilityIdentifier("inapp-banner")
            .accessibilityLabel("\(item.title), \(item.body)")
        }
    }
}
