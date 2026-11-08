//  Failure.swift
//  A failure NAMED BY THE OPERATION THAT FAILED.
//
//  The rule this exists to enforce: the sentence a person reads is written at the call
//  site, by the code that knows what it was doing. `catch { self.error = String(describing:
//  error) }` throws that away twice over — once by discarding the operation, once by
//  handing a Swift debug dump to a human — and the two screens that present it then have
//  no choice but to invent a title. That is why 109 different failures all read
//  "Couldn't connect".

import Foundation

struct Failure: Equatable {
    /// A verb phrase completing "Couldn't …": "publish your profile", "add them to the
    /// Space". Written where the operation happens; nil only for a legacy call site.
    let operation: String?
    let kind: Kind
    /// The underlying error, for the log and the detail line — never the headline.
    let detail: String
    /// The object this concerns, so a log line can be grepped alongside Trace's.
    let object: String

    enum Kind: Equatable {
        case unreachable      // the relay is not answering
        case notConnected     // that peer has not completed double-opt-in
        case refused          // the core rejected it — malformed, out of order, not allowed
        case identity         // no identity, or its key cannot be opened
        case unknown

        /// `CoreError` is a 17-variant enum that `FfiError::Core(e.to_string())` flattens
        /// to one string at the boundary. thiserror stamps each variant with a stable
        /// prefix ("transport: …", "not connected to peer …"), so the type survives in the
        /// text and this recovers it. Parsing a message is not where this should end up —
        /// widening `FfiError` into these cases is the real fix, and then this switch
        /// reads the variant instead of the prefix.
        static func of(_ message: String) -> Kind {
            let m = message.lowercased()
            if m.contains("no relay configured") || m.contains("transport:") { return .unreachable }
            if m.contains("not connected to peer") || m.contains("unknown peer") { return .notConnected }
            if m.contains("no local identity") || m.contains("identity:")
                || m.contains("at-rest") || m.contains("seal:") { return .identity }
            if m.contains("mls:") || m.contains("commit race") || m.contains("coordinator:")
                || m.contains("handshake:") || m.contains("governance:")
                || m.contains("malformed") { return .refused }
            return .unknown
        }
    }

    init(operation: String?, error: Error, object: String = "") {
        self.init(operation: operation, detail: String(describing: error), object: object)
    }

    /// `FfiError.Core("transport: …")` describes as `Core("transport: …")`. That wrapper is
    /// ours, not the reader's — and several sheets show this text verbatim
    /// (`note = core.error ?? …`), so it reaches people. Strip it back to the message the
    /// core actually wrote.
    static func unwrap(_ s: String) -> String {
        guard s.hasPrefix("Core("), let o = s.firstIndex(of: "\""),
              let c = s.lastIndex(of: "\""), o < c else { return s }
        return String(s[s.index(after: o)..<c])
    }

    init(operation: String?, detail: String, object: String = "") {
        let clean = Self.unwrap(detail)
        self.operation = operation
        self.detail = clean
        self.object = object
        self.kind = Kind.of(clean)
    }

    /// The headline. Says what failed, then what kind of failure it was — so a rejected
    /// delta can never again be read as a dead network.
    var title: String {
        guard let operation else { return genericTitle }
        switch kind {
        case .unreachable:  return "Couldn’t \(operation) — no answer"
        case .notConnected: return "Couldn’t \(operation) — not connected yet"
        case .refused:      return "Couldn’t \(operation)"
        case .identity:     return "Couldn’t \(operation) — identity unavailable"
        case .unknown:      return "Couldn’t \(operation)"
        }
    }

    private var genericTitle: String {
        switch kind {
        case .unreachable:  return "Can’t reach the relay"
        case .notConnected: return "Not connected to them yet"
        case .refused:      return "That was refused"
        case .identity:     return "Identity unavailable"
        case .unknown:      return "Something went wrong"
        }
    }

    /// What to do about it, when there is an answer. Empty when there honestly isn't.
    var guidance: String {
        switch kind {
        case .unreachable:  return "It will go out on its own once the relay answers."
        case .notConnected: return "They have to accept before this can reach them."
        case .refused, .identity, .unknown: return ""
        }
    }

    /// The line that reaches the log: kind, operation, object, underlying text.
    var logLine: String {
        let op = operation ?? "(unnamed operation)"
        let obj = object.isEmpty ? "" : " [\(object.prefix(8))]"
        return "\(kind)\(obj) \(op) — \(detail)"
    }
}
