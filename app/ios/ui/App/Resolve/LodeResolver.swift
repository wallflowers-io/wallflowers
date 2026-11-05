//  LodeResolver.swift
//  The LodeDB-backed candidate generator — hybrid (BM25 lexical + on-device vector)
//  over each entity's rich resolution_text, embedded with the SHARED ONNX/BGE embedder
//  (bge-base-en-v1.5, 768-d, CLS pooling, asymmetric query prefix) that PacificStore's
//  `BGEEmbedderFactory` builds (WS-D). This is the SAME embedder the union store and the
//  Rust-direct route use, so resolution vectors stay parity-compatible with everything
//  else on device.
//
//  Drops in behind DesignResolver the moment the LodeDBCore Swift package is linked.
//  Guarded by `canImport` so the app builds + runs on the lexical backend until the
//  xcframework is integrated — same interface, no behaviour change.
//
//  MIGRATION (WS-E5): this resolver previously embedded with Apple's NaturalLanguage
//  (`NLEmbedder`, no model download / no network). It now embeds with the shared
//  ONNX/BGE embedder. NLEmbedder is a DIFFERENT model — its vectors are NOT BGE, so an
//  NLEmbedder index is not comparable with the union store — and it survives here ONLY
//  as an explicitly-flagged, loudly-logged escape hatch, never a silent fallback (house
//  rule §9). The resolver's store is ephemeral and recreated per index, so this is a
//  pure embedder swap: no data migration.
//
//  ── RECALL A/B (run before trusting ONNX/BGE as the resolution default) ─────────────
//  Making ONNX/BGE the default embedder for RESOLUTION is only justified if BGE recall
//  is at least as good as NLEmbedder's on the resolution task. Validate with an A/B
//  before relying on it:
//    1. Fix a labelled eval set: { mention + window  ->  correct entity id } drawn from
//       real Systems (names, aliases, colloquial phrasings — the hard cases).
//    2. Index the SAME ResolvableEntity corpus twice: once with ONNX/BGE (default) and
//       once with NLEmbedder (set PACIFIC_RESOLVER_ALLOW_NLEMBEDDER_FALLBACK so a forced
//       ONNX failure degrades to NLEmbedder, or instantiate NLEmbedder in the harness).
//    3. Run every eval query through `candidates(_:k:)` on both, computing recall@k
//       (correct id present in the top-k) and MRR over the eval set.
//    4. GATE: keep ONNX/BGE as the default only if recall@k(ONNX) >= recall@k(NLEmbedder)
//       (BGE should win on semantic / paraphrase recall; confirm it does not regress on
//       exact-name hits, which the hybrid lexical arm already covers). Re-run whenever
//       the model, tokenizer, or the resolution_text projection changes.
//  ────────────────────────────────────────────────────────────────────────────────────

import Foundation

#if canImport(LodeDBCore)
import LodeDBCore
// PacificStore vends the shared BGE embedder factory (WS-D). The app target gains this
// package dependency in the serial Integrate step that wires PacificStore into the
// project (mirroring the LodeDBCore ref), alongside the other new app files.
import PacificStore

final class LodeResolver: DesignResolver {
    let backendName = "lodedb (hybrid)"

    private var db: LodeDB                 // recreated per index (fresh corpus each connect)
    private let embedder: LodeEmbedder     // shared ONNX/BGE (or, behind an explicit flag, NLEmbedder)
    private(set) var lastError: String?

    init() throws {
        embedder = try Self.makeEmbedder()
        db = try LodeDB(vectorDimension: embedder.dimension)
    }

    /// The environment flag (convention: `ProcessInfo` env, as CoreService
    /// uses) that PERMITS degrading to `NLEmbedder` when the shared ONNX/BGE
    /// embedder cannot be built. Absent/empty (the default) => ONNX/BGE is mandatory and
    /// a build failure THROWS (fail loud — no silent degrade). Set to any non-empty value
    /// => a failed ONNX build falls back to NLEmbedder with a LOUD log. This is an A/B /
    /// diagnostics escape hatch, NOT a production fallback.
    private static let allowNLEmbedderFallbackFlag = "PACIFIC_RESOLVER_ALLOW_NLEMBEDDER_FALLBACK"

    /// Builds the resolver's embedder: the shared ONNX/BGE embedder by default, or — only
    /// when `allowNLEmbedderFallbackFlag` is set — Apple's `NLEmbedder` if ONNX/BGE cannot
    /// be provisioned. Never returns a silent/placeholder embedder: without the flag a
    /// provisioning failure propagates (and `makeDesignResolver` then degrades to lexical
    /// with its own loud log).
    private static func makeEmbedder() throws -> LodeEmbedder {
        do {
            // The shared union-store embedder (bge-base-en-v1.5, 768-d, CLS pooling, query
            // prefix), byte-faithful to the union store + Rust route so vectors stay
            // comparable. Throws until the BGE .onnx + tokenizer.json are bundled and ONNX
            // Runtime is provisioned (PacificStore's UnprovisionedONNXRuntime is a
            // deliberate loud gate — never a stub session or zero vectors).
            return try BGEEmbedderFactory.makeEmbedder(bundle: .main)
        } catch {
            let fallbackAllowed =
                !(ProcessInfo.processInfo.environment[allowNLEmbedderFallbackFlag] ?? "").isEmpty
            guard fallbackAllowed else {
                // Fail LOUD: the ONNX/BGE embedder is the contract. No silent NLEmbedder,
                // no zero vectors — surface WHY resolution cannot start.
                throw error
            }
            // Explicit opt-in only, and say so LOUDLY. NLEmbedder is a DIFFERENT model: its
            // vectors are NOT BGE-parity, so this index is incomparable with the union
            // store. Use it for the recall A/B or offline diagnostics, never in production.
            print("[LodeResolver] ⚠️ ONNX/BGE embedder unavailable (\(error)). "
                + "\(allowNLEmbedderFallbackFlag) is set — FALLING BACK to Apple NLEmbedder. "
                + "Vectors are NOT BGE-parity (A/B / diagnostics only).")
            return try NLEmbedder()
        }
    }

    // Signature of the last-indexed corpus (id + resolution_text, order-independent) so a
    // redundant re-index — e.g. a System refresh that returns identical Fields — skips the
    // expensive re-embed rather than rebuilding the in-memory store from scratch.
    private var indexSignature: [String]?

    func index(_ entities: [ResolvableEntity]) {
        let signature = entities.map { "\($0.id)\u{1}\($0.text)" }.sorted()
        if signature == indexSignature { return }        // Fields unchanged — no re-embed.

        // Reset the corpus so a re-index (System refresh / credential change) doesn't accrue.
        // Fail LOUD: if the store can't be recreated, keep the prior signature so an identical
        // retry rebuilds — never record a signature for a corpus we didn't actually rebuild.
        let fresh: LodeDB
        do { fresh = try LodeDB(vectorDimension: embedder.dimension) }
        catch { lastError = "index reset: \(error)"; return }
        db = fresh

        var complete = true
        for e in entities {
            var md = e.metadata
            md["kind"] = e.kind
            md["name"] = e.name
            do { try db.addText(e.text, id: e.id, metadata: md, embedder: embedder) }
            catch { lastError = "index \(e.id): \(error)"; complete = false }   // loud, not silent
        }
        // Only remember this corpus as indexed if EVERY entity embedded. A partial index must be
        // retried on the next identical refresh, not wrongly skipped by a poisoned signature.
        indexSignature = complete ? signature : nil
    }

    func candidates(_ query: ResolutionQuery, k: Int) -> [Candidate] {
        // Context in: embed the mention WITH its window so descriptive phrasing ranks.
        // (query.scope is intentionally NOT filtered here: a MetadataFilter on a "scope" key
        // the index never stamps would silently return zero hits. Wire the filter only in the
        // same change that stamps md["scope"] — see the multi-credential seam.)
        let text = query.mention + " " + query.window
        do {
            let hits = try db.search(text: text, k: k, mode: .hybrid, embedder: embedder)
            return hits.map { Candidate(entityId: $0.id, score: $0.score, generator: "hybrid") }
        } catch {
            lastError = "resolve: \(error)"
            return []
        }
    }

}
#endif
