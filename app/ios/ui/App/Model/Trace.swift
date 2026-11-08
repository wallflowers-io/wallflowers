//  Trace.swift
//  The device-side observability spine: what actually happened, in order.
//
//  The half no server log can see: whether a note persisted, whether the outline projected,
//  whether proposals were derived, whether a notification was raised, and what an Arc answered
//  a membership check. Together they answer "where did it stop?" without guessing from a
//  timestamp.
//
//  CORRELATION. Every event carries the OBJECT ID it concerns (a note's UUID, for one), so a
//  single object can be followed across every stage that touched it.
//
//  Ring buffer, in memory, capped: this is a live diagnostic, not a second source of truth,
//  and it must never become a store that can itself be wrong.
//
//  THE LOG FILE. Every line is also appended to `LogFile` (Library/Logs/wallflowers.log), so a
//  device run is readable with no console attached (`just device-log`). It is written, never
//  read back by the app: a diagnostic, not state.

import Foundation
import PacificCore

@MainActor
final class Trace: ObservableObject {
    /// The one instance. A singleton because every layer (notes, proposals, membership) emits into
    /// it and threading it through each initialiser would be ceremony for a diagnostic.
    static let shared = Trace()

    struct Event: Identifiable {
        let id = UUID()
        let at: Date
        /// The stage, e.g. notes, outline, proposals, notify, membership.
        let stage: String
        /// The object this concerns — a note's UUID; "" when it concerns no single object.
        let object: String
        /// What happened, in the fewest words that are still specific.
        let detail: String
        let ok: Bool
    }

    @Published private(set) var events: [Event] = []
    private let cap = 300

    func log(_ stage: String, object: String = "", _ detail: String, ok: Bool = true) {
        events.insert(Event(at: Date(), stage: stage, object: object, detail: detail, ok: ok), at: 0)
        if events.count > cap { events.removeLast(events.count - cap) }
        // Mirrored to BOTH log sinks, because they reach different tools:
        //   NSLog  → os_log  → `xcrun simctl spawn booted log stream` (simulator)
        //   print  → stdout  → `devicectl device process launch --console` (real device)
        // The device console carries stdout ONLY, which is why a device-attached run showed
        // nothing all night while the same build was perfectly chatty in the simulator.
        let line = "[trace] \(ok ? "ok " : "ERR") \(stage) \(object.isEmpty ? "" : "(\(object.prefix(8))) ")\(detail)"
        NSLog("%@", line)
        print(line)
        LogFile.shared.append(line)
    }

    func clear() { events.removeAll() }

    /// The events for one object, oldest first: the story of a single note.
    func story(for object: String) -> [Event] {
        events.filter { $0.object == object }.reversed()
    }
}

/// The log a device keeps: each line with its time, in Library/Logs/wallflowers.log, rolled
/// to `.1` at `cap` bytes, so at most twice that is kept. Appends from any thread, in order.
final class LogFile: @unchecked Sendable {
    static let shared = LogFile(url: FileManager.default
        .urls(for: .libraryDirectory, in: .userDomainMask)[0]
        .appendingPathComponent("Logs/wallflowers.log"))

    let url: URL
    private let cap: Int
    private let queue = DispatchQueue(label: "wallflowers.logfile")
    private let stamp = ISO8601DateFormatter()

    init(url: URL, cap: Int = 1 << 20) {
        self.url = url
        self.cap = cap
        stamp.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    }

    func append(_ line: String) {
        let at = Date()
        queue.async { self.write("\(self.stamp.string(from: at)) \(line)\n") }
    }

    /// Wait for every append so far.
    func flush() { queue.sync {} }

    private func write(_ text: String) {
        let fm = FileManager.default
        try? fm.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        let size = (try? fm.attributesOfItem(atPath: url.path)[.size] as? Int) ?? 0
        if size >= cap {
            let old = url.appendingPathExtension("1")
            try? fm.removeItem(at: old)
            try? fm.moveItem(at: url, to: old)
        }
        guard let data = text.data(using: .utf8) else { return }
        if let h = try? FileHandle(forWritingTo: url) {
            defer { try? h.close() }
            _ = try? h.seekToEnd()
            try? h.write(contentsOf: data)
        } else {
            try? data.write(to: url)
        }
    }
}

/// The core's `tracing` events, into the one log. Called on the core's threads.
final class CoreLog: LogSink {
    func line(target: String, level: String, message: String) {
        Task { @MainActor in
            Trace.shared.log("core:\(target)", message, ok: level != "WARN" && level != "ERROR")
        }
    }
}
