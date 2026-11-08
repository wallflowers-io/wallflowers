//  ChatKit.swift
//  The read-model bridging the real MLS core into a Signal-shaped Chats surface.
//  Conversations are either GROUPS (forum objects) or DMs (1:1 connections). The
//  core's folded transcript is the authoritative content+order; we parse its
//  "[<author8> g<gen>] <text>" lines and align "mine" via the deterministic author
//  token (no heuristics, no fabricated messages). Titles + read-state are local
//  metadata the core doesn't hold, persisted alongside.

import SwiftUI
import PacificCore

struct Conversation: Identifiable, Hashable {
    enum Kind: String, Hashable { case group, dm }
    let id: String          // objectId (group) or peerId (dm)
    var kind: Kind
    var title: String
    var tint: Color

    var initials: String { Avatar.initials(title) }

    static func == (l: Conversation, r: Conversation) -> Bool { l.id == r.id && l.kind == r.kind }
    func hash(into h: inout Hasher) { h.combine(id); h.combine(kind) }
}

/// One rendered message: the STRUCTURED core record (`ChatMsg` — real MLS
/// authorship, Lamport `gen`, send timestamp, reply linkage, reaction groups)
/// plus the demo-seed DISPLAY overlay (`author`/`mine`) that ONLY the faked
/// single-device demo uses. In real-ops mode the overlay is inert and `mine`/
/// `author` come straight from MLS authorship. Reactions and replies always
/// target the REAL `(author, gen)` ref on `msg` — never the display overlay.
struct ChatRow: Identifiable {
    let msg: ChatMsg
    var mine: Bool          // display; defaults to msg.mine, overlay may remap
    var author: String      // display label — a name (overlay) or the 8-hex token

    init(_ msg: ChatMsg) {
        self.msg = msg
        self.mine = msg.mine
        self.author = String(msg.author.prefix(8))
    }

    // Stable identity = the MsgRef (author hex + Lamport gen) — unique per message.
    var id: String { msg.author + "#" + String(msg.gen) }

    var text: String { msg.text }
    var reactions: [ChatReaction] { msg.reactions }
    var isReply: Bool { !msg.replyToAuthor.isEmpty }
    var replyPreview: String { msg.replyPreview }

    /// WhatsApp-style delivery/read code the core stamped on OUR OWN messages:
    /// 0 = none (incoming), 1 = sent, 2 = delivered to everyone, 3 = read by
    /// everyone. Drives the ✓ / ✓✓ / ✓✓-blue ticks (see `ReceiptTicks`).
    var receipt: Int { Int(msg.receipt) }

    /// Locale-aware short time of the send stamp ("9:41 AM" / "09:41"); empty when
    /// the message carries no stamp (ts == 0).
    var timeLabel: String {
        guard msg.ts > 0 else { return "" }
        return Date(timeIntervalSince1970: Double(msg.ts) / 1000)
            .formatted(date: .omitted, time: .shortened)
    }

    static func lastSnippet(_ msgs: [ChatMsg]) -> String { msgs.last?.text ?? "" }
}

/// Local conversation metadata the core doesn't store: a last-read watermark for
/// unread counts. The MLS core stays the source of truth for content + order; this
/// is presentation metadata only. Persisted to Application Support. (The old
/// demo-seed display layer — fake per-line authors, time labels, contexts — is
/// gone: real-ops only, no fixtures.)
///
/// Titles are deliberately NOT here. They live in the MLS GroupContext, so they
/// reach every member; a device-local title mirror is how a renamed or joined group
/// ends up named on one phone and unnamed on all the others.
@MainActor
final class ConversationStore: ObservableObject {
    private struct Meta: Codable {
        var lastRead: [String: Int] = [:]
    }

    @Published private var meta = Meta()
    private let url: URL

    init() {
        let base = FileManager.default
            .urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("pacific", isDirectory: true)
        try? FileManager.default.createDirectory(at: base, withIntermediateDirectories: true)
        url = base.appendingPathComponent("conversations.json")
        if let data = try? Data(contentsOf: url),
           let decoded = try? JSONDecoder().decode(Meta.self, from: data) {
            meta = decoded
        }
    }

    private func persist() {
        if let data = try? JSONEncoder().encode(meta) { try? data.write(to: url, options: .atomic) }
    }

    /// First time we see a conversation, watermark it as read (so your own
    /// pre-existing messages don't all show as unread).
    func noteSeen(_ id: String, count: Int) {
        if meta.lastRead[id] == nil { meta.lastRead[id] = count; persist() }
    }

    func markRead(_ id: String, count: Int) {
        meta.lastRead[id] = count
        persist()
    }

    func unread(_ id: String, count: Int) -> Int {
        max(0, count - (meta.lastRead[id] ?? count))
    }

}
