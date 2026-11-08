//  ResolvedSpansStore.swift
//  Foundation (workstream F). The cache the callsite discipline requires: RESOLVE fires
//  on a content-mutation event and writes spans here; the render layer only READS.
//  Keyed by (contentId, indexEpoch) so a new index (System connect/refresh, credential
//  change) invalidates stale resolutions and the visible content re-resolves.
//
//  (Consumed by WS-C when the chat/notes render layer is decoupled from resolve().)

import SwiftUI

@MainActor
final class ResolvedSpansStore: ObservableObject {
    /// contentId (message id / note id) -> the epoch it was resolved under + its spans.
    @Published private(set) var version = 0
    private var store: [String: (epoch: Int, spans: [ResolvedSpan])] = [:]

    /// Read cached spans for a piece of content, IF they were resolved under `epoch`.
    /// A miss (nil) means "resolve this" to the caller; stale-epoch entries are misses.
    func spans(for contentId: String, epoch: Int) -> [ResolvedSpan]? {
        guard let entry = store[contentId], entry.epoch == epoch else { return nil }
        return entry.spans
    }

    /// A resolve callsite writes its result here.
    func put(_ spans: [ResolvedSpan], for contentId: String, epoch: Int) {
        store[contentId] = (epoch, spans)
        version += 1
    }

    /// Drop everything (e.g. sign-out). Individual entries self-invalidate on epoch change.
    func clear() {
        store.removeAll()
        version += 1
    }
}
