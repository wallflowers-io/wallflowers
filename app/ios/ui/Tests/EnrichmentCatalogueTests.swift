//  EnrichmentCatalogueTests.swift
//  Unit tests for the exact-alias trie matcher — pure logic, no fixtures: every
//  entity is constructed inside the test. Covers the invariants the chat surface
//  depends on: tight spans (only the mention lights, never the sentence),
//  greedy longest-match, non-overlap, token boundaries, and the single-token
//  noise guards.

import Testing
import Foundation
@testable import WallFlowers

struct EnrichmentCatalogueTests {

    private func catalogue(_ entities: [EnrichedEntity]) -> EnrichmentCatalogue {
        var c = EnrichmentCatalogue()
        c.replace(origin: .sysiphus, with: entities)
        return c
    }

    private func design(_ id: String, _ name: String, forms: [String] = []) -> EnrichedEntity {
        EnrichedEntity(id: id, name: name, kind: .product, origin: .sysiphus, surfaceForms: forms)
    }

    @Test func onlyTheMentionLightsNeverTheSentence() {
        let c = catalogue([design("d1", "Ram Cargo Pant", forms: ["ram cargos"])])
        let text = "can you pull stock on the ram cargos?"
        let ms = c.matches(in: text)
        #expect(ms.count == 1)
        #expect(String(text[ms[0].range]) == "ram cargos")
        #expect(ms[0].entityId == "d1")
    }

    @Test func greedyLongestMatchWinsOverPrefix() {
        // Both the bare block and the full design×season code are registered;
        // the scan must take the longer form and consume it once.
        let c = catalogue([design("d2", "Mini Problemo", forms: ["NP20033.06", "NP20033"])])
        let text = "codes crossed — NP20033.06 is on the rail"
        let ms = c.matches(in: text)
        #expect(ms.count == 1)
        #expect(String(text[ms[0].range]) == "NP20033.06")
    }

    @Test func multipleMentionsAreDistinctAndNonOverlapping() {
        let c = catalogue([
            design("a", "A", forms: ["NP20033.06"]),
            design("b", "B", forms: ["NP20009.06"]),
        ])
        let text = "found two more — NP20033.06 and NP20009.06 are swapped"
        let ms = c.matches(in: text)
        #expect(ms.map { String(text[$0.range]) } == ["NP20033.06", "NP20009.06"])
        #expect(ms.map(\.entityId) == ["a", "b"])
    }

    @Test func tokenBoundariesPreventSubwordHits() {
        // "ram" must never light inside "programme" — the trie is token-based.
        let c = catalogue([design("d3", "Ram", forms: ["ram"])])
        #expect(c.matches(in: "the programme is late").isEmpty)
        #expect(c.matches(in: "the ram is here").count == 1)
    }

    @Test func shortAndStopSingleTokensNeverMatchAlone() {
        // Single tokens that are too short or over-common are noise, not entities —
        // but the same words inside a multi-word form still match.
        let c = catalogue([
            design("d4", "The", forms: []),          // stop word as a name
            design("d5", "Do", forms: []),           // < 3 chars
            design("d6", "The Row", forms: []),      // multi-word: fine
        ])
        #expect(c.matches(in: "the do list").isEmpty)
        let text = "stocking the row today"
        let ms = c.matches(in: text)
        #expect(ms.count == 1)
        #expect(String(text[ms[0].range]) == "the row")
    }

    @Test func matchingIsCaseInsensitive() {
        let c = catalogue([design("d7", "Ram Cargo Pant", forms: ["ram cargos"])])
        let text = "RAM CARGOS on the rail"
        let ms = c.matches(in: text)
        #expect(ms.count == 1)
        #expect(String(text[ms[0].range]) == "RAM CARGOS")
    }
}
