//  JoinRequests.swift
//  Who is asking to join what I own or admit to (#76, ICD 2.6.0 mls.door). A request lives at
//  the object's door on the relay; the store reads it from there, so a new device sees what the
//  old one saw. Answered in the bell tray, ahead of the review deck.

import SwiftUI
import UserNotifications
import PacificCore

@MainActor
final class RequestStore: ObservableObject {
    @Published private(set) var pending: [RequestRow] = []

    /// A door read is a relay round trip per object, so the tick reads at most this often;
    /// opening the tray reads now.
    private static let interval: TimeInterval = 30
    private var lastRead = Date.distantPast
    private var reading = false

    /// Requests already announced, so a banner is raised once per request on this device.
    private var announced: Set<String> = []
    private let announcedURL: URL

    init(directory: URL? = nil) {
        let dir = directory ?? FileManager.default
            .urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("pacific", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        announcedURL = dir.appendingPathComponent("requests-announced.json")
        if let data = try? Data(contentsOf: announcedURL),
           let ids = try? JSONDecoder().decode(Set<String>.self, from: data) {
            announced = ids
        }
    }

    func refresh(core: CoreService, now: Bool = false) async {
        guard !reading, now || Date().timeIntervalSince(lastRead) > Self.interval else { return }
        reading = true
        defer { reading = false }
        lastRead = Date()
        let fresh = await core.pendingRequests()
        announce(fresh.filter { !announced.contains($0.request) }, core: core)
        if fresh != pending { pending = fresh }
    }

    func answer(_ r: RequestRow, admit: Bool, core: CoreService) async {
        guard await core.decideRequest(r, admit: admit) else { return }
        pending.removeAll { $0.request == r.request }
    }

    /// The object's name, as I hold it.
    static func objectName(_ r: RequestRow, core: CoreService) -> String {
        core.objects.first { $0.id == r.object }?.name ?? ""
    }

    private func announce(_ fresh: [RequestRow], core: CoreService) {
        guard !fresh.isEmpty else { return }
        for r in fresh {
            let c = UNMutableNotificationContent()
            c.title = "\(r.askerName.isEmpty ? "Someone" : r.askerName) asks to join"
            c.body = Self.objectName(r, core: core)
            c.sound = .default
            UNUserNotificationCenter.current().add(
                UNNotificationRequest(identifier: "join-\(r.request)", content: c, trigger: nil))
            announced.insert(r.request)
        }
        if let data = try? JSONEncoder().encode(announced) {
            try? data.write(to: announcedURL, options: .atomic)
        }
    }
}

/// The waiting asks, each with Admit and Decline. Done when none is left.
struct JoinRequestsView: View {
    @EnvironmentObject private var requests: RequestStore
    @EnvironmentObject private var core: CoreService
    let onDone: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text("Asking to join").font(.title2.weight(.bold)).foregroundStyle(Theme.textPrimary)
                Spacer()
                Button(action: onDone) {
                    Image(systemName: "xmark").font(.subheadline.weight(.bold))
                        .foregroundStyle(Theme.textSecondary).padding(8)
                }
                .accessibilityIdentifier("requests-close")
            }
            .padding(.bottom, 18)
            ScrollView {
                VStack(spacing: 12) {
                    ForEach(requests.pending, id: \.request) { r in row(r) }
                }
            }
        }
        .padding(.horizontal, 20)
        .padding(.top, 56)
        .background(Theme.bg.ignoresSafeArea())
    }

    private func row(_ r: RequestRow) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(r.askerName.isEmpty ? String(r.asker.prefix(8)) : r.askerName)
                .font(.headline).foregroundStyle(Theme.textPrimary)
            Text(RequestStore.objectName(r, core: core))
                .font(.subheadline).foregroundStyle(Theme.textSecondary)
            if !r.note.isEmpty {
                Text(r.note).font(.body).foregroundStyle(Theme.textPrimary)
            }
            HStack(spacing: 12) {
                Button("Decline") { Task { await requests.answer(r, admit: false, core: core) } }
                    .buttonStyle(.bordered)
                    .accessibilityIdentifier("request-decline-\(r.request)")
                Button("Admit") { Task { await requests.answer(r, admit: true, core: core) } }
                    .buttonStyle(.borderedProminent)
                    .accessibilityIdentifier("request-admit-\(r.request)")
            }
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Theme.surface2, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("request-\(r.request)")
    }
}
