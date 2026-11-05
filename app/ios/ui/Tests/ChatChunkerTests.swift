//  ChatChunkerTests.swift
//  The chunking rules, proven on synthetic folds — pure logic, no app, no store.
//
//  The contract under test: ~500-char chunks and ~100-char overlaps whose real edges
//  snap OUTWARD to whole messages and same-author runs; ids from the first member's
//  append-stable (author, gen) ref; metadata (authors, ts span, gen span, tags) a pure
//  function of the fold; and the strict-suffix guard that keeps consecutive chunks from
//  ever sharing a start ref — the invariant ChatIndex's diff pass leans on.

import Testing
@testable import WallFlowers

struct ChatChunkerTests {

    private func row(_ author: String, _ gen: UInt64, _ ts: UInt64, chars: Int) -> ChatChunkRow {
        ChatChunkRow(author: author, gen: gen, ts: ts,
                     text: String(repeating: "x", count: chars))
    }

    // MARK: - Sizing + run snapping

    @Test func shortTranscriptIsOneChunk() {
        let rows = [row("a", 0, 10, chars: 120), row("b", 0, 20, chars: 90)]
        let out = ChatChunker.chunks(conversation: "c1", rows: rows)
        #expect(out.count == 1)
        #expect(out[0].docId == "chunk:c1:a:0")
        #expect(out[0].authors == ["a", "b"])
        #expect(out[0].tsStart == 10 && out[0].tsEnd == 20)
        #expect(out[0].genStart == 0 && out[0].genEnd == 0)
        #expect(out[0].text.count == 120 + 1 + 90)   // joined by newline
    }

    @Test func chunkClosesAtRunEndNotAt500() {
        // A's burst crosses 500 mid-run; the chunk must extend to the end of the run.
        let rows = [row("a", 0, 1, chars: 200), row("a", 1, 2, chars: 200),
                    row("a", 2, 3, chars: 200),                        // 600 — cut lands here
                    row("b", 0, 4, chars: 600)]
        let out = ChatChunker.chunks(conversation: "c", rows: rows)
        #expect(out[0].genEnd == 2)                 // all three of A's messages, not two
        #expect(out[0].authors == ["a"])
        #expect(out[0].text.count == 602)
    }

    // MARK: - Overlap

    @Test func overlapCoversHundredCharsOfWholeMessagesExtendedToRunStart() {
        // Chunk 1 = [C(400), A(120), A(120), A(90)] — closes at A's run end (730 ≥ 500).
        // The nominal 100-char overlap lands on the middle A message; run extension must
        // pull it back to the run's FIRST message, so chunk 2 starts at A's first ref.
        let rows = [row("c", 0, 1, chars: 400),
                    row("a", 0, 2, chars: 120), row("a", 1, 3, chars: 120),
                    row("a", 2, 4, chars: 90),
                    row("b", 0, 5, chars: 600)]
        let out = ChatChunker.chunks(conversation: "cv", rows: rows)
        #expect(out.count == 2)
        #expect(out[0].docId == "chunk:cv:c:0")
        #expect(out[1].docId == "chunk:cv:a:0")     // run start, not the 100-char point
        #expect(out[1].authors == ["a", "b"])
        #expect(out[1].genStart == 0 && out[1].genEnd == 0)  // a:0 … b:0
        #expect(out[1].text.count == 120 + 120 + 90 + 600 + 3)
    }

    @Test func singleMessageChunkCarriesNoOverlap() {
        // A chunk that is one oversized message must not replay into the next chunk —
        // the strict-suffix guard: consecutive chunks never share a start ref.
        let rows = [row("a", 7, 1, chars: 700), row("b", 3, 2, chars: 600)]
        let out = ChatChunker.chunks(conversation: "c", rows: rows)
        #expect(out.count == 2)
        #expect(out[0].docId == "chunk:c:a:7")
        #expect(out[1].docId == "chunk:c:b:3")
        #expect(out[1].text.count == 600)           // no carried text from a:7
    }

    @Test func startRefsAreUniqueAcrossManyChunks() {
        // A long alternating transcript: every id minted once, in fold order.
        var rows: [ChatChunkRow] = []
        for i in 0..<40 {
            rows.append(row(i % 2 == 0 ? "a" : "b", UInt64(i / 2), UInt64(i), chars: 180))
        }
        let out = ChatChunker.chunks(conversation: "c", rows: rows)
        #expect(out.count > 2)
        #expect(Set(out.map(\.docId)).count == out.count)
    }

    // MARK: - Determinism + metadata

    @Test func sameFoldSameChunks() {
        let rows = [row("a", 0, 5, chars: 300), row("b", 0, 6, chars: 300),
                    row("a", 1, 7, chars: 300)]
        let one = ChatChunker.chunks(conversation: "c", rows: rows)
        let two = ChatChunker.chunks(conversation: "c", rows: rows)
        #expect(one == two)
    }

    @Test func editedContentChangesDigestNotId() {
        let rows = [row("a", 0, 1, chars: 300), row("b", 0, 2, chars: 300)]
        var edited = rows
        edited[1] = ChatChunkRow(author: "b", gen: 0, ts: 2,
                                 text: String(repeating: "y", count: 300))
        let before = ChatChunker.chunks(conversation: "c", rows: rows)
        let after = ChatChunker.chunks(conversation: "c", rows: edited)
        #expect(before[0].docId == after[0].docId)
        #expect(before[0].hash != after[0].hash)
    }

    @Test func timestampSpanIsMinMaxNotFirstLast() {
        // Fold order is (gen, author)-sorted — a late-arriving insert can carry an
        // out-of-order ts. The span must still be deterministic: min…max.
        let rows = [row("a", 0, 50, chars: 100), row("b", 0, 10, chars: 100),
                    row("a", 1, 90, chars: 100)]
        let out = ChatChunker.chunks(conversation: "c", rows: rows)
        #expect(out[0].tsStart == 10 && out[0].tsEnd == 90)
    }

    @Test func emptyMessagesCarryNothing() {
        let rows = [row("a", 0, 1, chars: 0), row("b", 0, 2, chars: 120)]
        let out = ChatChunker.chunks(conversation: "c", rows: rows)
        #expect(out.count == 1)
        #expect(out[0].docId == "chunk:c:b:0")      // the empty row can't anchor an id
    }

    @Test func tagsAreTheUnionInFirstAppearanceOrder() {
        let rows = [ChatChunkRow(author: "a", gen: 0, ts: 1, text: "see #beach at #dawn"),
                    ChatChunkRow(author: "b", gen: 0, ts: 2, text: "#dawn yes, #surf too")]
        let out = ChatChunker.chunks(conversation: "c", rows: rows)
        #expect(out[0].tags == ["beach", "dawn", "surf"])
    }
}
