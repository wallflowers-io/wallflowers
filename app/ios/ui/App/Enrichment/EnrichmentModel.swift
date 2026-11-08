//  EnrichmentModel.swift
//  The catalogue + matcher behind Pacific's killer feature: behind-the-scenes
//  cross-referencing of linked data, surfaced as inline enrichment. When a word
//  or phrase in a note/message/feed NAMES a known entity (person, org, place,
//  event, project, or a Sysiphus record — product/order/inventory/customer) it is
//  boldened + accent-coloured; long-press opens an info panel.
//
//  DESIGN NOTE: this is ALIAS matching ("does this word name a known entity"),
//  NOT semantic vector search — an exact, case-insensitive, longest-match scan
//  over a token trie. VectorService is a later fuzzy fallback, not the primary
//  matcher. The catalogue is HONEST: an empty catalogue yields zero enrichment —
//  we never fabricate matches or seed placeholder entities.

import SwiftUI

/// What kind of thing a matched entity is (a superset spanning the Notes
/// outline, the Work projects, Sysiphus records, notes and chats).
enum EnrichmentKind: String, Hashable {
    case person, org, place, event, project
    case product, order, inventory, customer
    case topic, generic

    var glyph: String {
        switch self {
        case .person:    return "person.fill"
        case .org:       return "building.2.fill"
        case .place:     return "mappin.circle.fill"
        case .event:     return "calendar"
        case .project:   return "square.grid.2x2.fill"
        case .product:   return "tag.fill"
        case .order:     return "shippingbox.fill"
        case .inventory: return "cube.box.fill"
        case .customer:  return "person.crop.circle"
        case .topic:     return "number"
        case .generic:   return "sparkles"
        }
    }
    var word: String {
        switch self {
        case .person: return "Person";   case .org: return "Organisation"
        case .place: return "Place";      case .event: return "Event"
        case .project: return "Project";  case .product: return "Product"
        case .order: return "Order";      case .inventory: return "Inventory"
        case .customer: return "Customer"; case .topic: return "Topic"
        case .generic: return "Entity"
        }
    }
    /// The systematic marker palette — one hue per entity type, tuned for legibility
    /// on the deep slate ground. This is where colour is deliberately spent: an
    /// enriched word wears its type's colour, in the panel and inline. The hues are
    /// sampled from van Gogh's paintings (the five-most-common-colours pie charts) —
    /// a muted, earthy set that reads as a cohesive palette on slate rather than a
    /// rainbow, while staying distinct enough to tell entity types apart.
    var tint: Color {
        switch self {
        case .person:    return Color(hex: 0x5C77BE)   // cobalt
        case .customer:  return Color(hex: 0xB17445)   // terracotta — stockists / customers
        case .org:       return Color(hex: 0xA8B3C9)   // dusty steel-blue
        case .place:     return Color(hex: 0x6E7331)   // olive
        case .event:     return Color(hex: 0xC2780B)   // burnt orange
        case .project:   return Color(hex: 0x9C7E62)   // umber taupe
        case .product:   return Color(hex: 0xE6A837)   // sunflower gold
        case .order:     return Color(hex: 0xC4BD31)   // citron
        case .inventory: return Color(hex: 0xCAB99D)   // parchment tan
        case .topic:     return Color(hex: 0xC0D2DE)   // pale sky
        case .generic:   return Color(hex: 0xA2A4B1)   // muted grey
        }
    }
}

/// Where a catalogue entity came from — drives the info panel's provenance line
/// and whether we can deep-link into a native detail view.
enum EntityOrigin: String, Hashable {
    case outline    // the Notes tab's entity graph (OutlineModel)
    case timeline   // the Work tab's project workspace (seasons, designs, suppliers, wholesalers)
    case sysiphus   // ingested from the Sysiphus SQL interface
    case note       // a note title
    case chat       // a conversation / member name
    case music      // the Music System — RA's public listings (reference data)
    case art        // the Art System — ArtRabbit's public listings (reference data)

    /// When two entities register the same surface form, the higher-priority
    /// origin wins the match (ties broken deterministically by id — see
    /// `EnrichmentCatalogue.rebuild`). The Work projects are the richest,
    /// most current records, so they sit on top. `.music` and `.art` sit BELOW
    /// everything: they are the world's reference data, and your own data always
    /// beats them (the tie between the two is broken by id, deterministically).
    var priority: Int {
        switch self {
        case .music, .art: return -1
        case .chat: return 0
        case .note: return 1
        case .outline: return 2
        case .sysiphus: return 3
        case .timeline: return 4
        }
    }
}

struct EntityField: Hashable {
    var label: String
    var value: String
}

/// One canonical entity in the catalogue. `surfaceForms` are the phrases that,
/// when found in text, resolve to this entity.
struct EnrichedEntity: Identifiable, Hashable {
    let id: String                 // canonical id, namespaced by origin (e.g. "work:aries")
    var name: String
    var kind: EnrichmentKind
    var origin: EntityOrigin
    var subtitle: String? = nil    // short descriptor
    var detail: String? = nil      // longer note / description
    var fields: [EntityField] = [] // typed key/values (esp. Sysiphus records)
    var surfaceForms: [String] = []  // extra aliases beyond `name`

    var allForms: [String] { [name] + surfaceForms }
}

/// A resolved occurrence of an entity inside a specific string.
struct EnrichmentMatch: Hashable {
    var range: Range<String.Index>
    var entityId: String
}

// MARK: - Tokenisation (shared by the trie builder and the scanner)

enum Tok {
    /// Word-character runs (letters/numbers) with their range in the original string.
    static func tokens(_ s: String) -> [(text: String, range: Range<String.Index>)] {
        var out: [(String, Range<String.Index>)] = []
        var i = s.startIndex
        while i < s.endIndex {
            if s[i].isLetter || s[i].isNumber {
                let start = i
                var j = i
                while j < s.endIndex, (s[j].isLetter || s[j].isNumber) { j = s.index(after: j) }
                out.append((s[start..<j].lowercased(), start..<j))
                i = j
            } else {
                i = s.index(after: i)
            }
        }
        return out
    }

    static func normalise(_ s: String) -> [String] { tokens(s).map(\.text) }
}

// MARK: - The catalogue (token trie + longest-match scanner)

/// A canonical entity catalogue with an exact, case-insensitive, longest-match
/// scanner. Value type so the owning service can publish replacements cheaply.
struct EnrichmentCatalogue {
    private final class Node {
        var children: [String: Node] = [:]
        var terminalEntityId: String? = nil   // last-writer-wins canonical resolution
    }

    private var byID: [String: EnrichedEntity] = [:]
    private var bySource: [EntityOrigin: [String]] = [:]   // origin -> entity ids (for replace)
    private var signatures: [EntityOrigin: [String]] = [:] // origin -> sorted surface-form signature
    private var root = Node()

    /// Over-common single tokens we never treat as an entity name on their own,
    /// to keep enrichment signal high. (Multi-word forms are unaffected.)
    private static let stop: Set<String> = [
        "the", "and", "for", "you", "our", "was", "are", "who", "her", "his",
        "dad", "mum", "day", "new", "now", "one", "two", "all", "can", "got",
    ]

    var isEmpty: Bool { byID.isEmpty }
    var count: Int { byID.count }
    func entity(_ id: String) -> EnrichedEntity? { byID[id] }
    var allEntities: [EnrichedEntity] { Array(byID.values) }

    /// Replace every entity from `origin` with `entities`. Replacement (not append)
    /// keeps re-ingest idempotent — no duplicate index. Returns whether the match
    /// index actually changed: if the surface forms are identical (e.g. editing a
    /// note BODY, which never changes its title's forms), the expensive trie
    /// rebuild is skipped and this returns false.
    @discardableResult
    mutating func replace(origin: EntityOrigin, with entities: [EnrichedEntity]) -> Bool {
        let newSig = entities
            .map { "\($0.id)\u{1}\($0.name)\u{1}\($0.surfaceForms.joined(separator: "\u{2}"))" }
            .sorted()
        if newSig == signatures[origin] {
            for e in entities { byID[e.id] = e }   // payload (fields/detail) may differ; no rematch needed
            return false
        }
        signatures[origin] = newSig
        for id in bySource[origin] ?? [] { byID[id] = nil }
        var ids: [String] = []
        for e in entities { byID[e.id] = e; ids.append(e.id) }
        bySource[origin] = ids
        rebuild()
        return true
    }

    private mutating func rebuild() {
        let fresh = Node()
        // Deterministic insertion order (Dictionary.values is seeded per launch —
        // never iterate it for anything last-writer-wins). Lower-priority origins
        // insert first; within an origin, LARGER ids insert first so the smaller
        // id wins ties (e.g. the NP30201 block code resolves to the .06 cut —
        // the season on the desk — not the .07 one).
        let ordered = byID.values.sorted { a, b in
            if a.origin.priority != b.origin.priority { return a.origin.priority < b.origin.priority }
            return a.id > b.id   // descending: the smaller id inserts last and wins
        }
        for e in ordered {
            for form in e.allForms {
                let toks = Tok.normalise(form)
                guard !toks.isEmpty else { continue }
                // Skip a single short/over-common token to avoid noise.
                if toks.count == 1 {
                    let t = toks[0]
                    if t.count < 3 || Self.stop.contains(t) { continue }
                }
                insert(toks, e.id, into: fresh)
            }
        }
        root = fresh
    }

    private func insert(_ toks: [String], _ id: String, into node: Node) {
        var cur = node
        for t in toks {
            if let next = cur.children[t] { cur = next }
            else { let n = Node(); cur.children[t] = n; cur = n }
        }
        cur.terminalEntityId = id
    }

    /// Non-overlapping, greedy longest-match scan of `text`.
    func matches(in text: String) -> [EnrichmentMatch] {
        guard !byID.isEmpty else { return [] }
        let toks = Tok.tokens(text)
        guard !toks.isEmpty else { return [] }
        var out: [EnrichmentMatch] = []
        var i = 0
        while i < toks.count {
            var cur = root
            var bestEnd = -1            // token index (inclusive) of the longest terminal
            var bestId: String? = nil
            var j = i
            while j < toks.count, let next = cur.children[toks[j].text] {
                cur = next
                if let id = cur.terminalEntityId { bestEnd = j; bestId = id }
                j += 1
            }
            if let id = bestId, bestEnd >= i {
                let lo = toks[i].range.lowerBound
                let hi = toks[bestEnd].range.upperBound
                out.append(EnrichmentMatch(range: lo..<hi, entityId: id))
                i = bestEnd + 1
            } else {
                i += 1
            }
        }
        return out
    }
}
