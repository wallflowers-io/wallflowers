//  LodeLocalStore.swift
//  The one and only `LocalStore`,
//  LodeDB-backed and wired at the composition root. There is no in-memory alternative:
//  the tests open this same type on a throwaway directory.
//
//  It realises the §2 union-store contract and the §4 read/write model directly on
//  the on-device LodeDB engine:
//
//    ONE union store    at `UnionConfig.storeURL()`, WAL commit mode, opened with the
//                       §2 options and `modelIdentity` so a mismatched model / dimension
//                       fails closed rather than silently degrading recall.
//    MULTI-PRODUCER     writes go through a shared-lock `LodeAppender` (NO exclusive
//                       writer is ever held), so the Rust-direct route (WS-G) and any
//                       other producer can append into the same store concurrently.
//    RUNNING CHECKPOINT a single in-app `LodeCheckpointer` folds the WAL into committed
//                       generations continuously on a background loop (a `Task` on this
//                       actor); appended records become durable shortly after logging.
//    SNAPSHOT READS     a `LodeDB.openReadOnly` handle; every read `refresh()`es first
//                       to overlay the current WAL tail (read-your-writes) before it
//                       searches / lists / gets.
//
//  Honesty contract (house rule): nothing here silently falls back. A checkpoint
//  failure is captured and re-thrown from the next operation (a dead fold means writes
//  stop becoming durable — a real fault, surfaced loudly) rather than swallowed by the
//  background loop. A model/dimension mismatch throws at `init`. `nil`/`[]` are only
//  ever genuinely-absent / genuinely-empty results, never a masked error.
//
//  Concurrency: the native handles (`LodeAppender` / `LodeCheckpointer` / `LodeDB`) are
//  each single-threaded and not `Sendable`; this actor owns them and serialises every
//  native call, which is exactly the discipline each type documents ("serialize calls
//  to it"). The embedder is used only through the `LodeEmbedder` PROTOCOL — WS-D's
//  concrete `ONNXTextEmbedder` is injected at the composition root, never depended on
//  here.

import Foundation
import LodeDBCore

/// The production `LocalStore`: one union store, one appender, one running
/// checkpointer, one read-only snapshot handle — all owned and serialised by this
/// actor. Construct it once at the composition root and share it.
public actor LodeLocalStore: LocalStore {

    // MARK: - Stored state

    /// The embedder, used only via the `LodeEmbedder` protocol (WS-D's ONNX type is
    /// injected here; this file never names it).
    private let embedder: LodeEmbedder
    /// `<AppSupport>/pacific/union.lode`.
    private let storeURL: URL
    /// How long the fold loop sleeps between checkpoints (nanoseconds).
    private let checkpointIntervalNanos: UInt64

    /// Shared-lock appender: the ONLY write path. Nil once `close()`d.
    private var appender: LodeAppender?
    /// The running checkpointer that folds the WAL. Nil once `close()`d.
    private var checkpointer: LodeCheckpointer?
    /// Read-only snapshot handle; `refresh()`ed before every read. Nil once `close()`d.
    private var reader: LodeDB?

    /// Whether the background fold loop should run (init choice).
    private let checkpointerEnabled: Bool
    /// The background fold loop; started lazily on first operation.
    private var checkpointTask: Task<Void, Never>?
    /// The first error the fold loop hit, if any. Once set, the loop stops and every
    /// public operation re-throws it — a stalled checkpointer is surfaced, not hidden.
    private var checkpointFailure: Error?
    /// Set by `close()`; afterwards every operation throws `.unsupported`.
    private var closed = false

    // MARK: - Lifecycle

    /// Opens the union store and brings up the appender, the running checkpointer, and
    /// the read-only snapshot handle.
    ///
    /// - Parameters:
    ///   - embedder: the shared embedder (`LodeEmbedder`). Its `dimension` MUST equal
    ///     `UnionConfig.vectorDimension`, and — when it declares a `modelIdentity` — it
    ///     MUST equal `UnionConfig.modelIdentity(for: language)`; a mismatch throws
    ///     (fail closed).
    ///   - language: the configured union language (F-3) — selects which model identity
    ///     the store binds to. Defaults to `.english`, the shipped behaviour. A store
    ///     built under a DIFFERENT language fails closed here; run `migrateStore`
    ///     first (the composition root's launch reconcile does).
    ///   - storeURL: override the on-disk location (defaults to `UnionConfig.storeURL()`,
    ///     creating the enclosing directory). Tests pass a temp path.
    ///   - checkpointIntervalNanos: fold-loop sleep between checkpoints (default 200ms).
    ///   - startCheckpointer: start the background fold loop (default true). Pass false
    ///     to drive folds manually via `checkpointNow()` (tests / migrations).
    public init(
        embedder: LodeEmbedder,
        language: NLLanguageIdentifier = .english,
        storeURL: URL? = nil,
        checkpointIntervalNanos: UInt64 = 200_000_000,
        startCheckpointer: Bool = true
    ) throws {
        let contractIdentity = UnionConfig.modelIdentity(for: language)
        guard embedder.dimension == UnionConfig.vectorDimension else {
            throw LodeDBError.invalidArgument(
                "embedder dimension \(embedder.dimension) does not match the union contract "
                + "(\(UnionConfig.vectorDimension))")
        }
        if let identity = embedder.modelIdentity, identity != contractIdentity {
            throw LodeDBError.invalidArgument(
                "embedder model '\(identity)' does not match the union contract "
                + "'\(contractIdentity)'")
        }

        let url = try storeURL ?? UnionConfig.storeURL()
        self.embedder = embedder
        self.storeURL = url
        self.checkpointIntervalNanos = checkpointIntervalNanos
        self.checkpointerEnabled = startCheckpointer

        // Bootstrap: a momentary exclusive writer creates the index on a fresh store
        // (with the §2 dimension, model identity, and retention) or validates an
        // existing one, then releases the writer lock. Without this, the appender —
        // which never creates an index — would have nothing to append into, and
        // `openReadOnly` would find no index. It must happen BEFORE the appender,
        // checkpointer, and reader open.
        let bootstrap = try LodeDB(
            path: url,
            vectorDimension: UnionConfig.vectorDimension,
            options: UnionConfig.lodeStoreOptions(acquireWriterLock: true),
            modelIdentity: contractIdentity
        )
        try bootstrap.close()

        // The write path: a shared-lock appender (no exclusive writer held), opened with
        // the store's retention + chunk limit so folded records match the store's policy.
        self.appender = try LodeAppender.open(
            at: url,
            durability: UnionConfig.durability,
            storeText: UnionConfig.storeText,
            indexText: UnionConfig.indexText,
            acquireWriterLock: false,
            chunkCharacterLimit: UnionConfig.chunkCharacterLimit
        )

        // The running checkpointer: holds only the fold lease, takes the writer lock for
        // each brief fold. Same retention/chunk limit or a fold would rewrite the store.
        self.checkpointer = try LodeCheckpointer.open(
            at: url,
            durability: UnionConfig.durability,
            storeText: UnionConfig.storeText,
            indexText: UnionConfig.indexText,
            chunkCharacterLimit: UnionConfig.chunkCharacterLimit
        )

        // The read path: a lock-free generation snapshot; each read `refresh()`es to
        // overlay the WAL tail. `modelIdentity` re-validates the persisted model.
        self.reader = try LodeDB.openReadOnly(
            path: url,
            options: UnionConfig.lodeStoreOptions(acquireWriterLock: false),
            modelIdentity: contractIdentity
        )

        // The background fold loop is started lazily on the first operation
        // (`ensureCheckpointLoop`): a nonisolated actor `init` cannot safely spawn a
        // Task that touches actor-isolated state. Nothing needs folding until the
        // first write anyway.
    }

    deinit {
        // Stop the fold loop; the native handles release their locks/lease as this
        // actor deallocates.
        checkpointTask?.cancel()
    }

    /// Stops the fold loop and releases the store handles (writer lease + shared lock +
    /// snapshot). Idempotent. After `close()`, every operation throws `.unsupported`.
    public func close() {
        guard !closed else { return }
        closed = true
        checkpointTask?.cancel()
        checkpointTask = nil
        try? reader?.close()
        reader = nil
        // Dropping the last reference releases the appender's shared lock and the
        // checkpointer's lease.
        appender = nil
        checkpointer = nil
    }

    // MARK: - Writes (§4)

    public func upsert(id: String, text: String, metadata: [String: String]) async throws {
        try requireOperational()
        guard let appender else { throw LodeDBError.unsupported("local store is closed") }
        guard !id.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw LodeDBError.invalidArgument("id is required")
        }
        // Chunks + embeds `text` (via the injected embedder) and logs one post-embedding
        // record to the WAL. Keyed by `id`, so a re-ingest replaces rather than
        // duplicates (idempotent upsert). The running checkpointer folds it in shortly;
        // reads see it immediately via `refresh()`.
        _ = try appender.append(text: text, id: id, metadata: metadata, embedder: embedder)
    }

    public func remove(id: String) async throws {
        try requireOperational()
        guard let appender else { throw LodeDBError.unsupported("local store is closed") }
        // A delete of an absent id folds to a no-op; not an error (§4).
        _ = try appender.delete(ids: [id])
    }

    // MARK: - Reads (§4)

    public func search(_ query: String, k: Int, scope: SearchScope) async throws -> [Hit] {
        try requireOperational()
        guard k > 0 else { return [] }
        guard let reader else { throw LodeDBError.unsupported("local store is closed") }

        // Fold the WAL tail into the snapshot first so a just-written row is searchable
        // (read-your-writes), then run hybrid BM25+vector (RRF) search under `scope`.
        try reader.refresh()
        let hits = try reader.search(
            text: query,
            k: k,
            mode: .hybrid,
            embedder: embedder,
            filter: Self.searchFilter(for: scope)
        )

        // Search hits are payload-free; batch-fetch the retained snippets in one call.
        let ids = hits.map(\.id)
        let texts = ids.isEmpty ? [:] : try reader.getTexts(ids)
        return hits.map { hit in
            Hit(
                id: hit.id,
                chunkId: hit.chunkID,
                score: hit.score,
                metadata: hit.metadata,
                text: texts[hit.id]
            )
        }
    }

    public func listDocuments(type: EntityType?) async throws -> [Document] {
        try requireOperational()
        guard let reader else { throw LodeDBError.unsupported("local store is closed") }

        try reader.refresh()
        let filter = type.map { MetadataFilter([Metadata.Key.type: $0.rawValue]) } ?? MetadataFilter()
        let records = try reader.listDocuments(filter: filter)

        // Enumeration is payload-free; batch-fetch retained text for display content.
        let ids = records.map(\.id)
        let texts = ids.isEmpty ? [:] : try reader.getTexts(ids)
        return records.map { record in
            Document(
                id: record.id,
                text: texts[record.id],
                metadata: record.metadata,
                chunkCount: record.chunkCount
            )
        }
    }

    public func get(id: String) async throws -> Document? {
        try requireOperational()
        guard let reader else { throw LodeDBError.unsupported("local store is closed") }

        try reader.refresh()
        guard let record = try reader.getDocument(id) else { return nil }
        let text = try reader.get(id)
        return Document(
            id: record.id,
            text: text,
            metadata: record.metadata,
            chunkCount: record.chunkCount
        )
    }

    // MARK: - Maintenance (§4)

    /// Rebuilds the index from retained text — e.g. after an embedder / model change
    /// (WS-E5). Enumerates every document, re-embeds its retained text with the CURRENT
    /// embedder, and re-appends it (an idempotent replace by id). Fails loud on any
    /// document whose text was not retained: it cannot be re-embedded from text, and
    /// leaving its stale vector in place would be a silent half-rebuild.
    public func reindexAll() async throws {
        try requireOperational()
        guard let reader, let appender else { throw LodeDBError.unsupported("local store is closed") }

        try reader.refresh()
        let records = try reader.listDocuments()
        for record in records {
            guard let text = try reader.get(record.id) else {
                throw LodeDBError.unsupported(
                    "cannot reindex '\(record.id)': no retained text to re-embed")
            }
            _ = try appender.append(
                text: text,
                id: record.id,
                metadata: record.metadata,
                embedder: embedder
            )
        }
    }

    /// The LANGUAGE-CHANGE reindex —
    /// `reindexAll` for the case the store's PERSISTED IDENTITY must change, which a
    /// live handle cannot do (LodeDB binds identity at index creation and fails a
    /// mismatched open closed). Call it BEFORE the store is opened — the composition
    /// root reconciles setting ↔ store here at launch, ChatIndex-doctrine style ("the
    /// index is a cache of the fold"), except retained text is carried across because
    /// the union is ALSO the notes' home, not only a cache.
    ///
    /// Mechanics, fail-loud and old-store-preserving:
    ///   1. no store on disk → nothing to migrate (the open will create it fresh);
    ///   2. read the persisted identity; already `language`'s → no-op;
    ///   3. snapshot every document's retained text + metadata (a document with no
    ///      retained text throws — same rule as `reindexAll`: no silent half-rebuild);
    ///   4. build a COMPLETE new store beside the old one (`union.lode.migrating`),
    ///      re-embedding through `embedder`;
    ///   5. only then swap directories. A crash mid-build leaves the old store live
    ///      and a stale `.migrating` directory that the next run deletes.
    ///
    /// Returns whether a rebuild happened. Single-writer maintenance: nothing else
    /// may hold the store open while this runs.
    @discardableResult
    public static func migrateStore(
        to language: NLLanguageIdentifier,
        embedder: LodeEmbedder,
        storeURL: URL? = nil,
        fileManager: FileManager = .default
    ) throws -> Bool {
        let url = try storeURL ?? UnionConfig.storeURL()
        guard fileManager.fileExists(atPath: url.path) else { return false }

        let expected = UnionConfig.modelIdentity(for: language)
        guard embedder.dimension == UnionConfig.vectorDimension else {
            throw LodeDBError.invalidArgument(
                "embedder dimension \(embedder.dimension) does not match the union contract "
                + "(\(UnionConfig.vectorDimension))")
        }
        if let identity = embedder.modelIdentity, identity != expected {
            throw LodeDBError.invalidArgument(
                "embedder model '\(identity)' is not '\(expected)' — wrong embedder for \(language.rawValue)")
        }

        // Open WITHOUT an identity so the persisted one can be READ (openReadOnly
        // only validates when an identity is passed).
        let reader = try LodeDB.openReadOnly(
            path: url,
            options: UnionConfig.lodeStoreOptions(acquireWriterLock: false)
        )
        let persisted = try reader.stats().model
        if persisted == expected {
            try reader.close()
            return false
        }

        // Snapshot the whole store: retained text is the migration's raw material.
        try reader.refresh()
        let records = try reader.listDocuments()
        var documents: [(id: String, text: String, metadata: [String: String])] = []
        documents.reserveCapacity(records.count)
        for record in records {
            guard let text = try reader.get(record.id) else {
                try? reader.close()
                throw LodeDBError.unsupported(
                    "cannot migrate '\(record.id)': no retained text to re-embed")
            }
            documents.append((record.id, text, record.metadata))
        }
        try reader.close()

        // Build the new store ASIDE — the old store stays live until it is complete.
        let rebuilt = url.deletingLastPathComponent()
            .appendingPathComponent(UnionConfig.storeName + ".migrating", isDirectory: true)
        if fileManager.fileExists(atPath: rebuilt.path) {
            try fileManager.removeItem(at: rebuilt)   // a stale half-migration
        }
        let writer = try LodeDB(
            path: rebuilt,
            vectorDimension: UnionConfig.vectorDimension,
            options: UnionConfig.lodeStoreOptions(acquireWriterLock: true),
            modelIdentity: expected
        )
        for document in documents {
            try writer.addText(
                document.text,
                id: document.id,
                metadata: document.metadata,
                embedder: embedder,
                chunkCharacterLimit: UnionConfig.chunkCharacterLimit
            )
        }
        try writer.close()

        // The swap: the one non-atomic instant. Old out, new in.
        try fileManager.removeItem(at: url)
        try fileManager.moveItem(at: rebuilt, to: url)
        return true
    }

    // MARK: - Checkpointing

    /// Folds the WAL tail once, synchronously, and returns the number of records folded
    /// (`0` when nothing new was appended). Serialised with the background loop by the
    /// actor. Useful to force durability at a known point (tests / before backup).
    @discardableResult
    public func checkpointNow() throws -> UInt64 {
        try requireOperational()
        guard let checkpointer else { throw LodeDBError.unsupported("local store is closed") }
        return try checkpointer.checkpoint()
    }

    /// The continuous fold loop: checkpoint, sleep, repeat until cancelled, closed, or a
    /// fold fails. On failure it records the error (surfaced by the next operation) and
    /// stops — it never spins silently on a broken store.
    private func runCheckpointLoop() async {
        while !Task.isCancelled {
            guard foldOnce() else { return }
            try? await Task.sleep(nanoseconds: checkpointIntervalNanos)
        }
    }

    /// One background fold. Returns false (stopping the loop) when closed or when a fold
    /// threw — the error is stashed in `checkpointFailure` for the next operation to
    /// re-throw.
    private func foldOnce() -> Bool {
        guard !closed, checkpointFailure == nil, let checkpointer else { return false }
        do {
            _ = try checkpointer.checkpoint()
            return true
        } catch {
            checkpointFailure = error
            return false
        }
    }

    // MARK: - Helpers

    /// Throws if the store is closed or the background checkpointer has failed, so a
    /// stalled fold never masquerades as a healthy store. Also brings up the fold loop
    /// on first use (deferred out of the nonisolated `init`).
    private func requireOperational() throws {
        if closed { throw LodeDBError.unsupported("local store is closed") }
        if let checkpointFailure { throw checkpointFailure }
        ensureCheckpointLoop()
    }

    /// Starts the background fold loop once, on first operation.
    private func ensureCheckpointLoop() {
        guard checkpointerEnabled, checkpointTask == nil, !closed else { return }
        checkpointTask = Task { [weak self] in
            await self?.runCheckpointLoop()
        }
    }

    /// The §3 scope predicate: unrestricted for `.all`; for `.groups` it admits rows
    /// with no `groupObjectId` (local-only, always visible) OR whose `groupObjectId` is
    /// in the authorised set. An empty authorised set therefore matches only un-scoped
    /// rows (expressed as `exists:false` alone to avoid an empty `$in`).
    private static func searchFilter(for scope: SearchScope) -> MetadataFilter {
        switch scope {
        case .all:
            return MetadataFilter()
        case .groups(let authorised):
            if authorised.isEmpty {
                return MetadataFilter(predicate: .exists(Metadata.Key.groupObjectId, false))
            }
            return MetadataFilter(predicate: .or([
                .exists(Metadata.Key.groupObjectId, false),
                .inSet(Metadata.Key.groupObjectId, authorised)
            ]))
        }
    }
}
