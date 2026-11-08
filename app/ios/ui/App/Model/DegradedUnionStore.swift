//  DegradedUnionStore.swift
//  WHAT THE APP IS WHEN THE UNION STORE COULD NOT BE OPENED.
//
//  WHY IT EXISTS. `WallFlowersApp.init()` used to `fatalError` three times over: once
//  if the union EMBEDDER could not be built, once more after the English retry,
//  and once if the STORE itself would not open. All three are reachable without a
//  bug anywhere in this app — Apple's on-device NL models (LinguisticData, and
//  NLContextualEmbedding for the CJK lane) are OS assets that a freshly created
//  simulator has not downloaded, and a harness provisions exactly that. The app
//  did not start. There was no screen, no log a person could read, and no way to
//  make an account on a device that had never run Pacific before.
//
//  A CRASH IS NOT INTEGRITY. The honesty contract those `fatalError`s were
//  protecting is real — there must be no silent in-memory store quietly losing
//  every write — but dying satisfies it by refusing to run at all, which is the
//  most expensive possible way to say "the index is unavailable". This type says
//  the same thing and lets the person get to their identity:
//
//    • Every call FAILS, loudly, with a message naming the real cause. Nothing is
//      accepted and dropped; a caller that writes a note is told the note did not
//      persist, and NotesStore already traces exactly that.
//    • Every failure REPEATS the underlying fault, so a log read hours later still
//      says "the NL embedder asset is missing" rather than "store unavailable".
//    • It announces itself once at construction, as a fixture must.
//
//  WHAT IS LOST while this is in place: note persistence, chat/forum indexing,
//  semantic search, the graph projection's store half. What is NOT lost: the
//  identity, the recovery key, the fold, messaging — none of which has ever gone
//  through this store. The MLS information architecture is untouched by this file.

import Foundation
import PacificStore

/// The one error this store raises. It carries the ORIGINAL fault, because "the
/// union store is unavailable" is not actionable and "LinguisticData asset for
/// 'en' is not installed" is.
struct UnionStoreUnavailable: LocalizedError {
    /// What actually went wrong, in the words of whatever threw it.
    let cause: String

    var errorDescription: String? {
        "the union store is not open on this device — \(cause). Nothing written "
        + "here is persisted or searchable until it is."
    }
}

/// A `LocalStore` that holds nothing and pretends nothing. Reads come back empty
/// (an empty index is a true description of an index that does not exist); writes
/// throw, so no caller can mistake a dropped write for a durable one.
struct DegradedUnionStore: LocalStore {
    let cause: String

    init(cause: String) {
        self.cause = cause
        NSLog("[union] DEGRADED — the union store is NOT open: %@", cause)
        NSLog("[union] DEGRADED — notes will not persist, chat/note search and the "
            + "graph projection are inert. The identity, the recovery key and the "
            + "fold are unaffected.")
    }

    private var unavailable: UnionStoreUnavailable { UnionStoreUnavailable(cause: cause) }

    // MARK: Writes — refused, never swallowed

    func upsert(id: String, text: String, metadata: [String: String]) async throws {
        NSLog("[union] DEGRADED — refusing to store '%@': %@", id, cause)
        throw unavailable
    }

    func remove(id: String) async throws {
        // A delete against a store that holds nothing is vacuously done, and
        // throwing here would turn every "this note is empty, drop it" into a
        // logged fault for a row that never existed.
        NSLog("[union] DEGRADED — nothing to remove for '%@' (the store is not open)", id)
    }

    // MARK: Reads — honestly empty

    func search(_ query: String, k: Int, scope: SearchScope) async throws -> [Hit] {
        NSLog("[union] DEGRADED — search returns nothing: %@", cause)
        return []
    }

    func listDocuments(type: EntityType?) async throws -> [Document] { [] }

    func get(id: String) async throws -> Document? { nil }

    func reindexAll() async throws {
        NSLog("[union] DEGRADED — there is no index to rebuild: %@", cause)
        throw unavailable
    }
}
