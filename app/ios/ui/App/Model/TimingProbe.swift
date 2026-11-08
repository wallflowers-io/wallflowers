//  TimingProbe.swift
//  End-to-end message-latency instrumentation for harness runs.
//
//  OFF unless `PACIFIC_TIMING=1` rides the launch environment (the harness passes it
//  the same way it passes PACIFIC_RELAY_URL) or `-pacificTiming` is a launch argument.
//  Disabled it costs one boolean check per hook — no folds, no writes.
//
//  WHAT IT MEASURES.
//    send  one authored message: `ack_ms` is the wall time of the blocking FFI send —
//          seal + relay round trip (the relay has acknowledged the append when it returns).
//    recv  one newly folded INBOUND message: `e2e_ms` = the moment this device's sync
//          fold first contains the message, minus the authored stamp it carries
//          (`ChatMsg.ts`, epoch-ms at the SENDER). Authored→visible-in-fold, the full
//          pipeline: seal → relay → poll → drain → decrypt → fold.
//
//  CLOCKS. e2e_ms subtracts the sender's clock from the receiver's. On the 2-sim
//  harness both sims share the host clock, so the difference is real latency. Across
//  physical devices, mind the skew before quoting numbers.
//
//  OUTPUT. One JSON object per line to Documents/timing.jsonl (fetch with
//  `simctl get_app_container <udid> <bundle> data`), mirrored to NSLog for live tails.
//  A relaunch mid-run resets the high-water marks and would re-log old messages as
//  fresh receives — the harness launches once, so that is out of scope here.

import Foundation
import SwiftUI
import PacificCore

@MainActor
final class TimingProbe {
    static let shared = TimingProbe()

    let enabled = ProcessInfo.processInfo.environment["PACIFIC_TIMING"] == "1"
        || ProcessInfo.processInfo.arguments.contains("-pacificTiming")

    /// conversation id → folded message count already examined.
    private var seen: [String: Int] = [:]

    private let fileURL = FileManager.default
        .urls(for: .documentDirectory, in: .userDomainMask)[0]
        .appendingPathComponent("timing.jsonl")

    nonisolated static var nowMs: Int64 { Int64(Date().timeIntervalSince1970 * 1000) }

    func send(convo: String, kind: String, chars: Int, ackMs: Int64) {
        guard enabled else { return }
        record(["ev": "send", "convo": String(convo.prefix(12)), "kind": kind,
                "chars": chars, "ack_ms": ackMs, "t": Self.nowMs])
        NSLog("[timing] send %@ %@ ack=%dms", kind, String(convo.prefix(12)), Int(ackMs))
    }

    /// Walk every conversation's fold and log each newly seen inbound message.
    /// Called after every successful sync; the guard makes it free when disabled.
    func drain(core: CoreService) {
        guard enabled else { return }
        let now = Self.nowMs
        var convos: [Conversation] = core.groupObjects.map {
            Conversation(id: $0.id, kind: .group, title: "", tint: .clear)
        }
        convos += core.connectionRows().map {
            Conversation(id: $0.peerId, kind: .dm, title: "", tint: .clear)
        }
        for convo in convos {
            let msgs = core.messages(convo)
            let from = seen[convo.id] ?? 0
            guard msgs.count > from else { continue }
            for m in msgs[from...] where !m.mine {
                let e2e = now - Int64(m.ts)
                record(["ev": "recv", "convo": String(convo.id.prefix(12)), "kind": convo.kind == .dm ? "dm" : "group",
                        "gen": m.gen, "authored": m.ts, "seen": now,
                        "e2e_ms": e2e, "chars": m.text.count])
                NSLog("[timing] recv %@ gen=%d e2e=%dms", String(convo.id.prefix(12)), Int(m.gen), Int(e2e))
            }
            seen[convo.id] = msgs.count
        }
    }

    private func record(_ obj: [String: Any]) {
        guard let data = try? JSONSerialization.data(withJSONObject: obj),
              let line = String(data: data, encoding: .utf8) else { return }
        let payload = Data((line + "\n").utf8)
        if let h = try? FileHandle(forWritingTo: fileURL) {
            defer { try? h.close() }
            _ = try? h.seekToEnd()
            try? h.write(contentsOf: payload)
        } else {
            try? payload.write(to: fileURL)
        }
    }
}
