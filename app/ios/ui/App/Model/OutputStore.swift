//  OutputStore.swift
//  The notification tray's persisted list. The tray carries one kind of item: a ticket that
//  landed in the wallet (`TicketSweep.arrivals` found a credential it had not seen before), and
//  confirming it opens the wallet. Injected at the app root; the sweep appends to it, and
//  `NotificationTray` reads it and removes what a human confirms or dismisses.

import Foundation

/// A ticket landed in the wallet — the delivery sweep found a credential it had not seen before.
struct TicketArrival: Codable, Equatable, Identifiable {
    let id: UUID
    let title: String
    let ticketId: String
    let listing: String
    init(id: UUID = UUID(), title: String, ticketId: String, listing: String) {
        self.id = id
        self.title = title
        self.ticketId = ticketId
        self.listing = listing
    }
}

/// One tray item. An enum rather than a bare `TicketArrival` because the file names each item's
/// kind (`{"ticket": …}`), which is the shape earlier builds wrote, so a tray holding tickets
/// survives the upgrade. A file still holding a kind this build does not know fails to decode
/// and the tray starts empty; the tickets themselves stay in the wallet.
enum Output: Equatable, Codable, Identifiable {
    case ticket(TicketArrival)

    /// A stable id (from the payload's UUID) so the tray can ForEach over [Output] directly.
    var id: String {
        switch self {
        case let .ticket(t): return "ticket:\(t.id)"
        }
    }
}

/// The persisted tray, at `<AppSupport>/pacific/outputs.json`.
@MainActor
final class OutputStore: ObservableObject {
    @Published private(set) var outputs: [Output] = []
    private let url: URL

    init() {
        let dir = FileManager.default
            .urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("pacific", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        url = dir.appendingPathComponent("outputs.json")
        if let d = try? Data(contentsOf: url),
           let o = try? JSONDecoder().decode([Output].self, from: d) { outputs = o }
    }

    func append(_ output: Output) {
        outputs.append(output)
        persist()
    }

    func remove(_ output: Output) {
        outputs.removeAll { $0 == output }
        persist()
    }

    private func persist() {
        if let d = try? JSONEncoder().encode(outputs) { try? d.write(to: url) }
    }
}
