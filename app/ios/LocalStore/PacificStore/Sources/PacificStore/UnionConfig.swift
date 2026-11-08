//  UnionConfig.swift
//  The ONE union-store config contract.
//
//  Every producer that opens or writes the union store — the Swift writer, the
//  in-app checkpointer, and the Rust-direct appender — MUST agree on these values.
//  A retention/chunk mismatch REWRITES the store, and a model/dimension mismatch
//  makes vectors incomparable, so this is a single frozen source of truth. The
//  Rust side mirrors it in a `const` module; keep the two byte-identical.
//
//  Two layers live here: the raw §2 constants (also the reference the Rust module
//  copies), and thin bridges that project them onto LodeDBCore's own option types
//  so a caller (WS-E) cannot silently diverge from the contract.
//
//  NOTE on the embedding preset: LodeDBCore ships `EmbeddingPreset.bge`, but its
//  `maxSequenceLength` default is 256, whereas §2 mandates 512 for BGE. We must
//  therefore build the preset explicitly rather than reuse `.bge`, or ingest would
//  silently truncate at half the intended window.

import Foundation
import LodeDBCore

/// The frozen configuration of the single on-device "union" store that all Pacific
/// producers append into. Values are namespaced statics (no instances) so there is
/// exactly one of each.
public enum UnionConfig {

    // MARK: - Location

    /// The store's directory name under Application Support.
    public static let applicationSupportSubdirectory = "pacific"
    /// The union store's on-disk name.
    public static let storeName = "union.lode"

    // MARK: - Store contract (§2)

    /// Write-ahead-log commit mode — mandatory: the appender and checkpointer both
    /// require it.
    public static let commitMode: CommitMode = .wal
    // The ACTIVE embedding profile. BGE (below) is the §2 target, but it needs a
    // provisioned ONNX runtime that is not wired yet, so the union currently runs on
    // Apple's on-device NL sentence embedder (`UnionEmbedder.makeUnion()`). These two
    // constants describe THAT embedder: the index is created at this width and a
    // mismatched model still fails closed at `LodeLocalStore.init`.
    //
    // MIGRATION TO BGE: provision the ONNX runtime, point `UnionEmbedder.makeUnion()`
    // at `BGEEmbedderFactory`, set these two back to `bgeVectorDimension` /
    // `bgeModelIdentity`, and run `LodeLocalStore.reindexAll()` — retained text makes
    // the re-embed exact. Vectors written under the two profiles are NOT comparable.

    /// Vector length produced by the ACTIVE embedder; the index is created with this.
    /// The SAME width for every union language: the en sentence model and the CJK
    /// contextual model are both 512-d, so a language change re-embeds (migrates)
    /// without an index-width change.
    public static let vectorDimension = 512
    /// The persisted model identity written into the store so a mismatched model is
    /// rejected rather than silently degrading recall. This is the DEFAULT-language
    /// (en) identity; the store's actual identity follows the configured union
    /// language — see ``modelIdentity(for:)``.
    public static let modelIdentity = "apple.NLEmbedding.sentence.en"

    /// The persisted model identity for a union store built under `language`.
    /// One identity per language because the
    /// vectors are NOT comparable across models: a language change must go through
    /// `LodeLocalStore.migrateStore` (re-embed from retained text), never a reopen.
    public static func modelIdentity(for language: NLLanguageIdentifier) -> String {
        switch language {
        case .english: return modelIdentity
        // The CJK contextual model, mean-pooled (`ContextualUnionEmbedder`) — Apple
        // ships no Korean SENTENCE model, so the sentence.* spelling would be a lie.
        case .korean: return "apple.NLContextualEmbedding.mean.ko"
        }
    }

    /// The §2 BGE target width — used to build ``embeddingPreset``, not the live index.
    public static let bgeVectorDimension = 768
    /// The §2 BGE target identity — used to build ``embeddingPreset``, not the live index.
    public static let bgeModelIdentity = "BAAI/bge-base-en-v1.5"
    /// Retain raw document text (required for `get` / note bodies).
    public static let storeText = true
    /// Index document text for lexical / hybrid search.
    public static let indexText = true
    /// Flush every commit to disk (per-append durability).
    public static let durability: Durability = .fsync
    /// BGE's maximum token window.
    public static let maxSequenceLength = 512
    /// Character cap per chunk (~512 BGE tokens); avoids silent truncation.
    public static let chunkCharacterLimit = 2000

    // MARK: - Embedding contract (§2)

    /// How token embeddings are pooled into one sentence vector. BGE is trained for
    /// CLS pooling.
    public static let pooling: EmbeddingPooling = .cls
    /// BGE's asymmetric query prefix — applied to QUERIES only, never to documents.
    public static let queryPrefix = "Represent this sentence for searching relevant passages: "
    /// BGE applies no document-side prefix.
    public static let documentPrefix = ""

    // MARK: - Location helpers

    /// The Application Support directory for this app (created if missing).
    /// Fails loud if the platform cannot vend it — never silently substitutes a
    /// temp/nowhere path.
    public static func applicationSupportDirectory(
        fileManager: FileManager = .default
    ) throws -> URL {
        try fileManager.url(
            for: .applicationSupportDirectory,
            in: .userDomainMask,
            appropriateFor: nil,
            create: true
        )
    }

    /// `<AppSupport>/pacific/union.lode`. With `createParentDirectory` true (the
    /// default) the enclosing `<AppSupport>/pacific` directory is created if absent;
    /// the `.lode` store directory itself is created by LodeDB on open.
    public static func storeURL(
        createParentDirectory: Bool = true,
        fileManager: FileManager = .default
    ) throws -> URL {
        let parent = try applicationSupportDirectory(fileManager: fileManager)
            .appendingPathComponent(applicationSupportSubdirectory, isDirectory: true)
        if createParentDirectory {
            try fileManager.createDirectory(at: parent, withIntermediateDirectories: true)
        }
        return parent.appendingPathComponent(storeName, isDirectory: true)
    }

    /// Filesystem path form of ``storeURL(createParentDirectory:fileManager:)``,
    /// which is what LodeDB's open options take.
    public static func storePath(
        createParentDirectory: Bool = true,
        fileManager: FileManager = .default
    ) throws -> String {
        try storeURL(createParentDirectory: createParentDirectory, fileManager: fileManager).path
    }

    // MARK: - LodeDBCore bridges (so the contract cannot drift)

    /// The §2 store options as LodeDBCore's own type.
    ///
    /// - Parameter acquireWriterLock: the union is MULTI-PRODUCER — no exclusive
    ///   writer is held (producers append via `LodeAppender` / `CoreAppender`; one
    ///   in-app checkpointer folds the WAL). Appenders and the checkpointer open with
    ///   `false`; a single-writer maintenance/migration path may pass `true`. It is a
    ///   runtime open-mode choice, not part of the frozen contract, so the caller
    ///   states it explicitly.
    ///
    /// `compressText` is left at LodeDB's default (true): §2 does not constrain it and
    /// it does not affect chunk/retention layout, so it cannot rewrite the store.
    public static func lodeStoreOptions(acquireWriterLock: Bool = false) -> LodeStoreOptions {
        LodeStoreOptions(
            durability: durability,
            commitMode: commitMode,
            storeText: storeText,
            indexText: indexText,
            chunkCharacterLimit: chunkCharacterLimit,
            acquireWriterLock: acquireWriterLock
        )
    }

    /// The §2 embedding preset as LodeDBCore's own type. Built explicitly rather than
    /// reusing `EmbeddingPreset.bge` because that preset's `maxSequenceLength` is 256,
    /// half the §2-mandated 512 (which would silently truncate ingest). Dimension,
    /// pooling, prefix and identity all match the constants above.
    /// NOTE: built from the BGE TARGET constants, not the active profile — this preset
    /// describes the BGE pipeline that `BGEEmbedderFactory` will build once the ONNX
    /// runtime is provisioned. It is dormant while the union runs on Apple NL.
    public static let embeddingPreset = EmbeddingPreset(
        name: "bge",
        dimension: bgeVectorDimension,
        maxSequenceLength: maxSequenceLength,
        pooling: pooling,
        queryPrefix: queryPrefix,
        documentPrefix: documentPrefix,
        modelIdentity: bgeModelIdentity
    )
}
