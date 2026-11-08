//  Waker.swift
//  The WAKE tier — the third plane, and the only one that knows which device to poke.
//
//  THE TENSION (RINGFENCE.txt §6). A push needs something that knows WHICH DEVICE to wake.
//  The relay must never know that: it addresses by opaque rotating tags and holds no identity —
//  that is the whole point of the public tier. So the push sender cannot be the relay. Hence a
//  third tier, the same separation the rail already makes:
//
//      relay   blind   tags + sealed blobs, knows nobody          (public)
//      rail    money   Stripe secrets, per-sale ledger            (Pacific-run)
//      waker   wake    token <-> tag registrations, APNs key      (Pacific-run)
//
//  The waker is JUST ANOTHER RELAY CLIENT: it SUBs to the tags it has been asked to watch,
//  exactly as a phone does, and the relay cannot tell it apart from any other subscriber. When
//  a blob lands on a watched tag it sends a CONTENT-FREE APNs push to the tokens registered for
//  it. It cannot read the blob (sealed), cannot name the sender, and never holds a group id.
//
//  WHAT IT LEARNS, stated because it is a real cost: a stable device token bound to a set of
//  tags, which links those tags to each other and to one install across rotations. The mailbox
//  alone never permitted that. Signal makes the identical trade — its server knows which account
//  to wake, never what was said. There is no push without something knowing who to poke; the
//  choice is only whether that is stated or discovered.
//
//  TAG ROTATION is the hard part. Relay tags are per-epoch (mls.rs `relay_tag` folds `epoch_be`),
//  so a registration goes stale the moment a group advances. The app therefore re-asserts its
//  CURRENT tag set as the set changes — idempotent, cheap, self-healing. A stale tag simply
//  stops being watched.

import Foundation
import CryptoKit

// MARK: - Endpoint

/// Where the wake tier lives. Same shape as `CoreService.railUrl`: an env override for dev
/// runs, else the shipped default. A separate host from the relay ON PURPOSE — this one holds
/// the APNs credential and a token↔tag map, the relay holds nothing.
enum WakerEndpoint {
    /// The production wake tier: the waker plane behind the Arc's one origin (`/v1/wake`).
    /// If it is down the Settings row says "unreachable" rather than pretending.
    static let defaultWakerUrl = "https://arc.wallflowers.io"

    static var url: String {
        if let env = ProcessInfo.processInfo.environment["PACIFIC_WAKER_URL"], !env.isEmpty { return env }
        return defaultWakerUrl
    }

    /// `<waker>/v1/wake/<leaf>`, tolerant of a trailing slash on the base.
    static func path(_ leaf: String) -> URL? {
        var base = url
        while base.hasSuffix("/") { base.removeLast() }
        return URL(string: base + "/v1/wake/" + leaf)
    }

    /// The host alone, for a settings row that must stay one line.
    static var host: String { URLComponents(string: url)?.host ?? url }
}

// MARK: - Health

/// What the waker says about itself. `apnsReady` is optional on purpose: a waker that does not
/// report it is UNKNOWN, not ready — there is no APNs auth key in the project yet, and guessing
/// "ready" here is exactly the lie that makes "notifications don't work" unexplainable.
struct WakerHealth: Equatable {
    var reachable: Bool
    var apnsReady: Bool?
    var detail: String

    static let unknown = WakerHealth(reachable: false, apnsReady: nil, detail: "Not checked")

    /// The settings-row value — one line, label-or-value only.
    var label: String {
        guard reachable else { return detail }
        switch apnsReady {
        case .some(true):  return "Up · APNs key present"
        case .some(false): return "Up · no APNs key"
        case .none:        return "Up · APNs unknown"
        }
    }
}

// MARK: - Client

/// Talks to the wake tier. Stateless by design — the caller (PushRegistrar) owns the debounce,
/// so this type can be exercised, replaced or stubbed without dragging registration policy in.
struct WakerClient {
    var session: URLSession = .shared
    var timeout: TimeInterval = 15

    enum Failure: Error, LocalizedError {
        case noEndpoint
        case refused(Int)
        case transport(String)

        var errorDescription: String? {
            switch self {
            case .noEndpoint:        return "No waker URL"
            case .refused(let code): return "Refused · HTTP \(code)"
            case .transport(let s):  return s
            }
        }
    }

    /// Ask the waker to watch `tags` for this device. Idempotent on the waker's side: the same
    /// token with a new tag set REPLACES the old set, which is what makes epoch rotation heal
    /// itself rather than accumulating dead registrations.
    func register(token: String, tags: [String], platform: String = "ios") async throws {
        try await post("register", body: ["token": token, "tags": tags, "platform": platform])
    }

    /// Drop this device from the wake tier entirely. Not called on a schedule — it exists so
    /// turning wake OFF is a real revocation rather than merely a local flag.
    func forget(token: String) async throws {
        try await post("forget", body: ["token": token])
    }

    /// Is the wake tier up, and does it hold an APNs credential? Never throws: a health check
    /// that can fail the caller is a health check nobody runs.
    func health() async -> WakerHealth {
        guard let url = WakerEndpoint.path("health") else {
            return WakerHealth(reachable: false, apnsReady: nil, detail: "No waker URL")
        }
        var req = URLRequest(url: url)
        req.timeoutInterval = timeout
        do {
            let (data, resp) = try await session.data(for: req)
            guard let http = resp as? HTTPURLResponse else {
                return WakerHealth(reachable: false, apnsReady: nil, detail: "No response")
            }
            guard (200..<300).contains(http.statusCode) else {
                return WakerHealth(reachable: false, apnsReady: nil, detail: "HTTP \(http.statusCode)")
            }
            return WakerHealth(reachable: true, apnsReady: Self.apnsReady(data), detail: "Up")
        } catch {
            return WakerHealth(reachable: false, apnsReady: nil,
                               detail: Self.reason(error))
        }
    }

    private func post(_ leaf: String, body: [String: Any]) async throws {
        guard let url = WakerEndpoint.path(leaf) else { throw Failure.noEndpoint }
        var req = URLRequest(url: url)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.httpBody = try JSONSerialization.data(withJSONObject: body)
        req.timeoutInterval = timeout
        do {
            let (_, resp) = try await session.data(for: req)
            guard let http = resp as? HTTPURLResponse else { throw Failure.transport("No response") }
            guard (200..<300).contains(http.statusCode) else { throw Failure.refused(http.statusCode) }
        } catch let f as Failure {
            throw f
        } catch {
            throw Failure.transport(Self.reason(error))
        }
    }

    /// Read the waker's APNs readiness out of whatever shape its health document takes. Probed
    /// rather than decoded into a fixed struct because this app does not own that contract — an
    /// unrecognised document reports UNKNOWN, which is true, instead of failing the check.
    static func apnsReady(_ data: Data) -> Bool? {
        guard let doc = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return nil }
        if let b = doc["apns"] as? Bool { return b }
        if let apns = doc["apns"] as? [String: Any] {
            for key in ["ready", "configured", "ok", "key"] {
                if let b = apns[key] as? Bool { return b }
            }
        }
        for key in ["apnsReady", "apns_ready", "apnsConfigured", "apns_configured"] {
            if let b = doc[key] as? Bool { return b }
        }
        return nil
    }

    /// Name the failure in the words that say what to do about it — the RelayProbe rule.
    static func reason(_ error: Error) -> String {
        switch (error as? URLError)?.code {
        case .some(.notConnectedToInternet): return "No internet"
        case .some(.cannotFindHost):         return "Host not found"
        case .some(.cannotConnectToHost):    return "Cannot connect"
        case .some(.timedOut):               return "Timed out"
        case .some(.secureConnectionFailed),
             .some(.serverCertificateUntrusted),
             .some(.serverCertificateHasBadDate),
             .some(.serverCertificateHasUnknownRoot):
            return "TLS failed"
        default: return (error as NSError).localizedDescription
        }
    }
}

// MARK: - Debounce

/// WHEN to re-register, as a pure value — no network, no clock of its own, no side effects, so
/// the policy can be tested exhaustively rather than observed in production.
///
/// Three rules, in order:
///   1. NEVER per tick. A registration is sent only when the fingerprint (token + tag SET)
///      actually changes. Tags rotate per epoch, so the tick rate and the change rate are
///      completely different numbers; conflating them is how a phone DDoSes its own backend.
///   2. RE-ASSERT WHEN STALE. An unchanged set is re-sent after `reassertAfter` so a waker
///      restart, an expiry, or a dropped row heals without the user doing anything.
///   3. BACK OFF ON FAILURE, exponentially and capped. A waker that is down must not be
///      hammered every three seconds by every phone that has ever seen it.
struct WakeDebounce: Equatable {
    /// The fingerprint of what the waker was last told, or nil if it has never been told.
    private(set) var sentFingerprint: String?
    private(set) var sentAt: Date?
    private(set) var failures: Int = 0
    private(set) var retryNotBefore: Date?

    var reassertAfter: TimeInterval = 30 * 60
    var backoffFloor: TimeInterval = 30
    var backoffCeiling: TimeInterval = 15 * 60

    /// The identity of a registration: the token plus the tag SET. Order-insensitive and
    /// duplicate-insensitive — the same tags in a different order are the same registration,
    /// and treating them as different would re-post on every sync for no reason.
    static func fingerprint(token: String, tags: [String]) -> String {
        var hasher = SHA256()
        hasher.update(data: Data(token.utf8))
        for tag in Set(tags).sorted() {
            hasher.update(data: Data("\n".utf8))
            hasher.update(data: Data(tag.lowercased().utf8))
        }
        return hasher.finalize().map { String(format: "%02x", $0) }.joined()
    }

    /// Should this (token, tags) be sent now? Pure: the caller supplies `now`.
    func shouldSend(token: String?, tags: [String], now: Date) -> Bool {
        guard let token, !token.isEmpty else { return false }
        // Nothing to watch is not a registration — it is a no-op that would otherwise churn a
        // row on the waker every time a device sat between epochs.
        guard !tags.isEmpty else { return false }
        if let retryNotBefore, now < retryNotBefore { return false }
        let fp = Self.fingerprint(token: token, tags: tags)
        if fp != sentFingerprint { return true }
        guard let sentAt else { return true }
        return now.timeIntervalSince(sentAt) >= reassertAfter
    }

    mutating func recordSuccess(token: String, tags: [String], now: Date) {
        sentFingerprint = Self.fingerprint(token: token, tags: tags)
        sentAt = now
        failures = 0
        retryNotBefore = nil
    }

    /// A failure does NOT record the fingerprint — the waker was not told, so the next attempt
    /// must still consider it new. What it records is the backoff that stops that next attempt
    /// happening on the very next tick.
    mutating func recordFailure(now: Date) {
        failures += 1
        let delay = min(backoffCeiling, backoffFloor * pow(2, Double(failures - 1)))
        retryNotBefore = now.addingTimeInterval(delay)
    }

    /// The token changed (reinstall, restore, some OS updates) — everything previously asserted
    /// is about a token that no longer exists.
    mutating func reset() {
        sentFingerprint = nil
        sentAt = nil
        failures = 0
        retryNotBefore = nil
    }
}
