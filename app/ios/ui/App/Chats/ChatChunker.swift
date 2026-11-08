//  ChatChunker.swift
//  Cuts the spans the union store embeds — the lodedb retrieval grain.
//
//  Transcript → embeddable chunks: ~500 characters, ~100-character overlap, both
//  boundaries snapped OUTWARD to whole messages and to runs of consecutive messages
//  by the same author. A chunk never cuts a message, and never strands half a burst:
//  the 500 mark closes a chunk only at the end of the run it lands in, and the 100-char
//  overlap grows backward to the start of the run it lands in. So sizes are nominal;
//  message and run edges are law.
//
//  DETERMINISM IS THE CONTRACT. Everything here is a pure function of the folded
//  transcript: ids are the chunk's first (author, gen) ref — the fold's own append-stable
//  coordinates, exactly the ref scheme the per-message index used — the author list is
//  first-appearance order, the timestamp span is min…max over member rows (fold order is
//  (gen, author)-sorted, so ts alone is not monotone), and the digest is SHA-256 over
//  content + authors + span. Same fold in, same chunks out, on any device, any launch.
//
//  A mid-transcript insert (late-arriving low-gen message) recomposes every chunk from
//  its run onward — that is unavoidable once documents span messages. What makes it safe
//  is the pairing with ChatIndex's diff pass: unchanged (id, digest) pairs are skipped,
//  changed ids re-embed in place, and ids that vanished are REMOVED from the store, so a
//  recomposition can never strand stale rows. The overlap seed is always a strict suffix
//  of the closing chunk (guarded), so consecutive chunks can never share a start ref and
//  ids stay unique by construction.

import Foundation
import CryptoKit

/// One folded message reduced to what chunking needs. Sendable so the whole pass can
/// run off the main actor on a snapshot.
struct ChatChunkRow: Equatable, Sendable {
    let author: String
    let gen: UInt64
    let ts: UInt64
    let text: String
}

/// One embeddable span of a transcript, with the deterministic metadata the union
/// store row carries: who spoke, over what time span, across which fold refs.
struct ChatChunk: Equatable, Sendable {
    let docId: String          // "chunk:<conversation>:<author>:<gen>" of the first row
    let text: String           // member bodies joined by newline; senders live in metadata
    let authors: [String]      // first-appearance order
    let tags: [String]         // union of member #tags, first-appearance order
    let tsStart: UInt64
    let tsEnd: UInt64
    let genStart: UInt64
    let genEnd: UInt64
    /// SHA-256 over content + authors + span — decides re-embedding under a stable id.
    let hash: String
}

enum ChatChunker {
    /// Nominal sizes. Real edges snap outward to message and run boundaries.
    static let targetChars = 500
    static let overlapChars = 100

    /// The whole transcript, chunked. Empty-after-trim rows carry nothing embeddable
    /// and are dropped before any boundary maths — they cannot pad a chunk or a run.
    static func chunks(conversation id: String, rows allRows: [ChatChunkRow]) -> [ChatChunk] {
        let rows = allRows.filter { !$0.text.isEmpty }
        guard !rows.isEmpty else { return [] }

        var out: [ChatChunk] = []
        var cur: [ChatChunkRow] = []
        var curLen = 0
        var seed = 0   // how many rows of `cur` are carried-over overlap, not new work

        for (i, row) in rows.enumerated() {
            cur.append(row)
            curLen += row.text.count
            let runEnd = i + 1 == rows.count || rows[i + 1].author != row.author
            if curLen >= targetChars && runEnd {
                out.append(make(id, cur))
                cur = overlapSuffix(cur)
                seed = cur.count
                curLen = cur.reduce(0) { $0 + $1.text.count }
            }
        }
        // The tail: emit only if it holds rows beyond the carried overlap — a close that
        // landed exactly on the last row owes nothing more.
        if cur.count > seed { out.append(make(id, cur)) }
        return out
    }

    /// The rows the next chunk opens with: the trailing rows covering ≥`overlapChars`,
    /// extended backward to the start of the run containing that point. Never the whole
    /// chunk — an overlap that replays everything would mint the same start ref forever —
    /// so a chunk that is one single run falls back to the nominal suffix, and a chunk
    /// that is one single row carries no overlap at all.
    private static func overlapSuffix(_ chunk: [ChatChunkRow]) -> [ChatChunkRow] {
        var acc = 0
        var j = chunk.count - 1
        while j >= 0, acc < overlapChars {
            acc += chunk[j].text.count
            j -= 1
        }
        let nominal = j + 1                     // first row of the ≥100-char suffix
        var s = nominal
        while s > 0, chunk[s - 1].author == chunk[s].author { s -= 1 }
        if s == 0 { s = nominal }               // run reaches the chunk's start: don't extend
        guard s > 0 else { return [] }          // single-row (or single-run-of-one) chunk
        return Array(chunk[s...])
    }

    private static func make(_ conversation: String, _ rows: [ChatChunkRow]) -> ChatChunk {
        let first = rows[0], last = rows[rows.count - 1]

        var authors: [String] = []
        for r in rows where !authors.contains(r.author) { authors.append(r.author) }

        var seenTags = Set<String>()
        let tags = rows.flatMap { Hashtags.extract($0.text) }.filter { seenTags.insert($0).inserted }

        let text = rows.map(\.text).joined(separator: "\n")
        let tsStart = rows.lazy.map(\.ts).min() ?? first.ts
        let tsEnd = rows.lazy.map(\.ts).max() ?? last.ts

        let digestInput = text + "\u{1F}" + authors.joined(separator: ",") + "\u{1F}\(tsStart)-\(tsEnd)"
        let hash = SHA256.hash(data: Data(digestInput.utf8))
            .map { String(format: "%02x", $0) }.joined()

        return ChatChunk(docId: "chunk:\(conversation):\(first.author):\(first.gen)",
                         text: text, authors: authors, tags: tags,
                         tsStart: tsStart, tsEnd: tsEnd,
                         genStart: first.gen, genEnd: last.gen,
                         hash: hash)
    }
}
