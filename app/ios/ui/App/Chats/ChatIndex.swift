//  ChatIndex.swift
//  Chats and forums, vectorised into the ONE union store — the same
//  LodeDB-backed, on-device-embedded index notes already live in (§3/§4).
//
//  WHAT IS WRITTEN. One document per CHUNK (see ChatChunker): ~500 chars of
//  transcript, ~100-char overlap, both edges snapped to whole messages and to
//  same-author runs. Id "chunk:<conversation>:<author>:<gen>" — the first member
//  row's append-stable fold ref. Metadata = the §3 row (source: chat|forum,
//  groupObjectId = the conversation — the scope key search folds hits back
//  through) plus the chunk's deterministic surround: authors (first-appearance
//  order), ts span, gen span, #tags. The store embeds each chunk with the union
//  embedder — the SAME 512-d vectors notes and the graph carry, so a cosine
//  between a query and a span of conversation means something by construction.
//
//  WHY CHUNKS ARE SAFE NOW (they were rejected on 2 Aug): a late-arriving low-gen
//  message inserts mid-transcript and recomposes every chunk downstream — under
//  the old count-watermark that meant stranded stale rows and a silent mass drift.
//  The DIFF PASS is what changed: every ingest recomputes the chunk set, upserts
//  only ids whose digest changed, and REMOVES ids that vanished, so recomposition
//  is self-cleaning. The digests make the common case (append) cheap: untouched
//  chunks match and skip. A (count, last-ref) gate skips the whole pass when the
//  transcript hasn't changed — this runs on the sync tick.
//
//  WHAT IS DERIVED. #tags are parsed from the text ("#parsnips" → "parsnips") at
//  index time and at list-build time — a pure function of the transcript, never a
//  second store. The CHAT search bar's tag lane ("#…" queries) reads the derived
//  sets; the semantic lane reads the union store; the exact lane reads the rows
//  the list already folded. Lanes fuse by RANK (RRF, K=60 — NoteSearch's rule):
//  a cosine and a substring have no common scale, so scores never mix.
//
//  STATE. Per conversation: the tag cursor (count of messages whose #tags are
//  accumulated) and the chunk map (docId → digest) the diff runs against. Deleting
//  the state file re-indexes from scratch — the index is a cache of the fold,
//  rebuildable at will, exactly like the graph topology. A conversation with no
//  chunk map yet is a MIGRATION: its old per-message "msg:…" docs are swept from
//  the store in the same pass that writes its first chunks.

import Foundation
import PacificCore     // ChatMsg — the folded message row being indexed
import PacificStore

/// The chat/forum lane of the union store: ingest (vectorise) + semantic search.
/// #tags come from `Hashtags.extract` — the ONE extractor CHAT and NOTE share.
@MainActor
final class ChatIndex: ObservableObject {
    private let store: LocalStore

    /// Extra metadata keys chat rows carry beyond §3, namespaced like the note
    /// producer's (`note.tags` etc.) so producers can never collide on a bare
    /// word. Values are strings (LodeDB's metadata contract); tags and authors
    /// are comma-joined (neither charset can contain ',').
    enum ChatMeta {
        static let tags = "chat.tags"
        static let authors = "chat.authors"
        static let tsStart = "chat.tsStart"
        static let tsEnd = "chat.tsEnd"
        static let genStart = "chat.genStart"
        static let genEnd = "chat.genEnd"
    }

    /// conversation id → how many of its messages have fed the accumulated #tag
    /// set (the TAG cursor — embedding is driven by the chunk digests below).
    private var indexed: [String: Int]
    /// conversation id → the chunk docIds currently in the store and their digests.
    /// The diff pass runs against this; ids that vanish are removed from the store.
    private var chunkHashes: [String: [String: String]]
    /// conversation id → "count|lastRef" of the last successfully indexed pass —
    /// the cheap gate that keeps the sync tick from re-chunking unchanged chats.
    /// In-memory only: a relaunch re-runs one diff per chat, which the digests
    /// then reduce to no-ops.
    private var chunkClean: [String: String] = [:]
    /// conversation id → its accumulated #tag set. Grown at ingest from NEW messages
    /// only, so the CHAT list's tag lane never re-scans a full transcript per rebuild
    /// (that scan was O(total history) on the main thread, every sync tick). Absent
    /// for a conversation from before tags were persisted — `tags(conversation:msgs:)`
    /// backfills it once from the transcript the rebuild already folded.
    private var tagSets: [String: Set<String>]
    private let watermarkURL: URL

    /// On-disk shape (v3 — `chunks` absent means a v2 file: every conversation then
    /// migrates off per-message docs on its next pass). The v1 file was a bare
    /// `[id: count]`; decoding falls back to it, leaving `tagSets` empty so each
    /// conversation backfills once.
    private struct Persisted: Codable {
        var counts: [String: Int]
        var tags: [String: [String]]
        var chunks: [String: [String: String]]?
    }

    init(store: LocalStore, stateURL: URL? = nil) {
        self.store = store
        if let stateURL {
            watermarkURL = stateURL   // tests: a throwaway location
        } else {
            let base = FileManager.default
                .urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
                .appendingPathComponent("pacific", isDirectory: true)
            try? FileManager.default.createDirectory(at: base, withIntermediateDirectories: true)
            watermarkURL = base.appendingPathComponent("chat-index.json")
        }
        if let data = try? Data(contentsOf: watermarkURL),
           let p = try? JSONDecoder().decode(Persisted.self, from: data) {
            indexed = p.counts
            tagSets = p.tags.mapValues(Set.init)
            chunkHashes = p.chunks ?? [:]
        } else if let data = try? Data(contentsOf: watermarkURL),
                  let legacy = try? JSONDecoder().decode([String: Int].self, from: data) {
            indexed = legacy
            tagSets = [:]
            chunkHashes = [:]
        } else {
            indexed = [:]
            tagSets = [:]
            chunkHashes = [:]
        }
    }

    // MARK: - Ingest (vectorise)

    /// Chunk `conversation`'s transcript and reconcile the union store with it. Called
    /// from the CHAT list's rebuild with the transcript it ALREADY folded — no second
    /// fold. Chunking + embedding run off-main on a snapshot; failures are logged and
    /// nothing is marked clean, so the next pass retries rather than silently skipping.
    func ingest(conversation id: String, kind: MetadataSource, msgs: [ChatMsg]) {
        // Tags stay message-cursor incremental, as before. A nil set means the pre-tags
        // (v1) state: leave it nil — `tags(conversation:msgs:)` backfills from the FULL
        // transcript on this same rebuild pass, and a union here would mask that backfill.
        let from = indexed[id] ?? 0
        if msgs.count > from {
            if var t = tagSets[id] {
                let before = t.count
                for m in msgs[from...] { t.formUnion(Hashtags.extract(m.text)) }
                if t.count != before { tagSets[id] = t }
            }
            indexed[id] = msgs.count
            persist()
        }

        // The cheap gate: this runs on the sync tick, so an unchanged transcript must
        // cost nothing. (count, last-ref) moves on any append AND any mid-insert.
        let mark = "\(msgs.count)|\(msgs.last.map { "\($0.author):\($0.gen)" } ?? "")"
        guard chunkClean[id] != mark else { return }

        // Snapshot to Sendable rows before leaving the main actor.
        let rows = msgs.map {
            ChatChunkRow(author: $0.author, gen: $0.gen, ts: $0.ts,
                         text: $0.text.trimmingCharacters(in: .whitespacesAndNewlines))
        }
        let prior = chunkHashes[id]
        // No chunk map yet ⇒ this conversation still has per-message docs from the
        // previous index shape. Their ids are reconstructible from the fold — sweep
        // them in the same pass that writes the first chunks.
        let legacyIds = prior == nil ? msgs.map { "msg:\(id):\($0.author):\($0.gen)" } : []

        Task.detached(priority: .utility) { [store] in
            let chunks = ChatChunker.chunks(conversation: id, rows: rows)
            // Start refs are unique by construction (strict-suffix overlap); first-wins
            // uniquing is a no-op that keeps a malformed fold from crashing the pass.
            let current = Dictionary(chunks.map { ($0.docId, $0.hash) },
                                     uniquingKeysWith: { a, _ in a })
            let stale = (prior ?? [:]).keys.filter { current[$0] == nil }
            let fresh = chunks.filter { (prior ?? [:])[$0.docId] != $0.hash }
            do {
                for docId in stale { try await store.remove(id: docId) }
                for legacy in legacyIds { try await store.remove(id: legacy) }
                for c in fresh {
                    var meta = Metadata.document(source: kind, objectId: c.docId,
                                                 groupObjectId: id).build()
                    if !c.tags.isEmpty { meta[ChatMeta.tags] = c.tags.joined(separator: ",") }
                    meta[ChatMeta.authors] = c.authors.joined(separator: ",")
                    meta[ChatMeta.tsStart] = String(c.tsStart)
                    meta[ChatMeta.tsEnd] = String(c.tsEnd)
                    meta[ChatMeta.genStart] = String(c.genStart)
                    meta[ChatMeta.genEnd] = String(c.genEnd)
                    try await store.upsert(id: c.docId, text: c.text, metadata: meta)
                }
                await MainActor.run {
                    self.chunkHashes[id] = current
                    self.chunkClean[id] = mark
                    self.persist()
                }
            } catch {
                NSLog("[ChatIndex] chunk ingest failed for \(id.prefix(12))…: \(error)")
            }
        }
    }

    /// The conversation's #tag set for the CHAT list's tag lane. Reads the accumulated
    /// set; a conversation without one (v1 file, fresh install) derives it ONCE from
    /// the transcript the caller already folded, persists it, and stays incremental
    /// from then on.
    func tags(conversation id: String, msgs: [ChatMsg]) -> Set<String> {
        if let t = tagSets[id] { return t }
        let t = Set(msgs.flatMap { Hashtags.extract($0.text) })
        tagSets[id] = t
        persist()
        return t
    }

    private func persist() {
        let p = Persisted(counts: indexed, tags: tagSets.mapValues { $0.sorted() },
                          chunks: chunkHashes)
        if let data = try? JSONEncoder().encode(p) {
            try? data.write(to: watermarkURL, options: .atomic)
        }
    }

    // MARK: - The semantic lane

    /// Conversations ranked by cosine against `query` — the union store's hybrid
    /// search, folded back to conversation ids (best message wins the rank).
    /// Empty for a blank or "#tag" query (tags are exact intent, not similarity).
    /// A failed search returns empty AND logs — the exact lane still answers, and
    /// the list must not blank because the embedder hiccuped mid-keystroke.
    func semanticRanks(query: String, k: Int = 40) async -> [String: Int] {
        let q = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !q.isEmpty, !q.hasPrefix("#") else { return [:] }
        do {
            // Over-fetch ×3, NoteSearch's rule for the same reason: the union holds
            // every producer's rows (notes, entities, …) and we keep only the
            // chat/forum subset — a flat k would let note hits starve chat recall.
            let hits = try await store.search(q, k: k * 3, scope: .all)
            var ranks: [String: Int] = [:]
            var next = 0
            for hit in hits {
                guard let source = Metadata.source(hit.metadata),
                      source == .chat || source == .forum,
                      let convo = Metadata.groupObjectId(hit.metadata),
                      ranks[convo] == nil else { continue }
                next += 1
                ranks[convo] = next
            }
            return ranks
        } catch {
            NSLog("[ChatIndex] semantic search failed: \(error)")
            return [:]
        }
    }
}
