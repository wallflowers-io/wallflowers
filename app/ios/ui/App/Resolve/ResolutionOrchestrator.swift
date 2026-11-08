//  ResolutionOrchestrator.swift
//  Workstream C (spine). The resolve pipeline made real: mention-in-context -> buildContext ->
//  candidates (LodeDB, on device) -> decide (heuristic; an ambiguous span abstains) ->
//  ResolvedSpan -> cache. Views read the cache; they never call this in `body`.
//
//  DETECTION is intentionally NOT here. Which substrings of a message are mentions is the
//  callsite's job — today the chat enrichment catalogue's trie. Duplicating that trie here
//  would create a second source of truth (exactly what the ground rules forbid), so this spine
//  takes PRE-DETECTED mentions and resolves them. The chat-receive callsite (deferred, to be
//  coordinated with the enrichment owner) feeds detected spans in and this writes the result.

import SwiftUI

@MainActor
final class ResolutionOrchestrator: ObservableObject {
    /// A span the callsite's detector already located in the text.
    struct DetectedMention {
        let range: Range<String.Index>
        let text: String
    }

    private let resolution: ResolutionService
    private let store: ResolvedSpansStore
    private let shaper: MentionShaper
    private let heuristic: Decider

    init(resolution: ResolutionService,
         store: ResolvedSpansStore,
         shaper: MentionShaper = IdentityShaper(),
         heuristic: Decider = HeuristicDecider()) {
        self.resolution = resolution
        self.store = store
        self.shaper = shaper
        self.heuristic = heuristic
    }

    /// Resolve pre-detected mentions to spans and cache them under `contentId` + the current
    /// index epoch (so a System (re)index invalidates them and the content re-resolves).
    func resolve(_ mentions: [DetectedMention],
                 in text: String,
                 contentId: String,
                 anchors: [String] = []) async {
        var spans: [ResolvedSpan] = []
        for m in mentions {
            let query = shaper.build(mention: m.text, window: text, anchors: anchors)
            let candidates = resolution.candidates(query, k: 5)
            guard !candidates.isEmpty else { continue }

            // An abstention stands: an ambiguous span is left unresolved rather than guessed.
            let decision = await heuristic.decide(query, candidates)

            if let id = decision.entityId, !decision.abstained {
                spans.append(ResolvedSpan(range: m.range, entityId: id,
                                          kind: "design", confidence: decision.confidence))
            }
        }
        store.put(spans, for: contentId, epoch: resolution.indexEpoch)
    }
}
