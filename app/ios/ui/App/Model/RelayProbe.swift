//  RelayProbe.swift
//  Can THIS device reach the relay it messages through?
//
//  `/v1/arc`'s `planes.relay.live` is the Arc describing its own upstream — a different claim
//  from "your phone can open this socket", and the two came apart badly in practice: an Arc
//  reporting `relay: live` behind a gateway that 401s the WebSocket, a custom domain whose
//  certificate never issued, a network that only lets 443 out. Each of those looks identical
//  in the picker (a green Relay pill) and identical in the chat (messages that just never
//  arrive). The only honest answer comes from dialling it from here.
//
//  So the probe speaks the REAL protocol rather than settling for a socket that opened: it
//  SUBs to a random tag and waits for the `eose` that closes the backlog, which is proof a
//  Semaphore is on the other end and not merely something that accepted an upgrade. Nothing
//  is published, the tag is random, so the check writes nothing and reaches no one.

import Foundation

@MainActor
final class RelayProbe: ObservableObject {
    enum Outcome: Equatable {
        case idle
        case checking
        case reachable(ms: Int)
        case unreachable(String)

        /// The settings-row value. The failure carries its own reason because "why can't I
        /// message" is the entire question this screen exists to answer.
        var label: String {
            switch self {
            case .idle:                 return ""
            case .checking:             return "Checking"
            case .reachable(let ms):    return "Connected · \(ms) ms"
            case .unreachable(let why): return why
            }
        }
    }

    @Published private(set) var outcome: Outcome = .idle

    func check(_ raw: String) async {
        guard let url = URL(string: raw), let scheme = url.scheme,
              scheme == "ws" || scheme == "wss" else {
            outcome = .unreachable("Not a ws:// or wss:// URL")
            return
        }
        outcome = .checking
        let started = Date()

        let cfg = URLSessionConfiguration.ephemeral
        cfg.timeoutIntervalForRequest = 8
        cfg.waitsForConnectivity = false
        let session = URLSession(configuration: cfg)
        let task = session.webSocketTask(with: url)
        task.resume()
        defer {
            task.cancel(with: .goingAway, reason: nil)
            session.invalidateAndCancel()
        }

        do {
            let tag = (0..<32).map { _ in String(format: "%02x", UInt8.random(in: 0...255)) }.joined()
            try await task.send(.string(#"{"t":"sub","tags":["\#(tag)"],"since":0}"#))
            try await Self.race(seconds: 8) {
                // A random tag has no backlog, so `eose` is the next frame — but bound the loop
                // anyway rather than trusting the far side to be well-behaved.
                for _ in 0..<8 {
                    let text: String
                    switch try await task.receive() {
                    case .string(let s): text = s
                    case .data(let d):   text = String(decoding: d, as: UTF8.self)
                    @unknown default:    continue
                    }
                    if text.contains("\"eose\"") { return }
                }
                throw ProbeError.notARelay
            }
            outcome = .reachable(ms: Int(Date().timeIntervalSince(started) * 1000))
        } catch {
            outcome = .unreachable(Self.reason(task: task, error: error))
        }
    }

    private enum ProbeError: Error { case notARelay, timedOut }

    /// `URLSessionWebSocketTask.receive()` has no timeout of its own — a server that upgrades and
    /// then says nothing would hang the screen forever. Race it against a sleep.
    private static func race(seconds: Double, _ work: @escaping @Sendable () async throws -> Void) async throws {
        try await withThrowingTaskGroup(of: Void.self) { group in
            group.addTask { try await work() }
            group.addTask {
                try await Task.sleep(nanoseconds: UInt64(seconds * 1_000_000_000))
                throw ProbeError.timedOut
            }
            defer { group.cancelAll() }
            try await group.next()
        }
    }

    /// Name the failure in the words that tell you what to do about it. The HTTP status is the
    /// one that matters most: a 401 here means the Arc's gateway is gating the relay path, which
    /// is a fix on the ARC, not on the phone — and is indistinguishable from "offline" without it.
    private static func reason(task: URLSessionWebSocketTask, error: Error) -> String {
        if let http = task.response as? HTTPURLResponse, !(200..<300).contains(http.statusCode) {
            return http.statusCode == 401 ? "Refused · HTTP 401" : "Refused · HTTP \(http.statusCode)"
        }
        if error is ProbeError {
            return (error as? ProbeError) == .notARelay ? "Answered, but not a relay" : "Timed out"
        }
        switch (error as? URLError)?.code {
        case .some(.notConnectedToInternet):        return "No internet"
        case .some(.cannotFindHost):                return "Host not found"
        case .some(.cannotConnectToHost):           return "Cannot connect"
        case .some(.timedOut):                      return "Timed out"
        case .some(.secureConnectionFailed),
             .some(.serverCertificateUntrusted),
             .some(.serverCertificateHasBadDate),
             .some(.serverCertificateHasUnknownRoot):
            return "TLS failed"
        case .some(.appTransportSecurityRequiresSecureConnection):
            return "Blocked · needs wss://"
        default:
            return (error as NSError).localizedDescription
        }
    }
}
