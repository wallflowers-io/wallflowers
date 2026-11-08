//  ArcCatalog.swift
//  The LIVE Arc directory, fetched from the Arc Fleet Console (arc/console) — the real
//  list of community micro-datacentres, each at its own static IP. This replaces the
//  hardcoded seed: the picker now shows the actual mesh from the control plane.
//
//  Honest by default: an Arc is selectable ONLY once it is provisioned (has endpoints);
//  the rest are shown as "coming soon". No silent fallback — a console we cannot reach
//  surfaces as an error state in the picker, never a fabricated directory.

import Foundation

/// The console's per-Arc JSON (`GET /api/arcs` → `{ arcs: [...] }`). We decode only the
/// fields the picker needs; the console carries much more (capacity, dossier, geo, …).
private struct ConsoleArc: Decodable {
    let id: String
    let name: String
    let community: String?
    let region: String?
    let status: String?
    let endpoints: Endpoints?

    struct Endpoints: Decodable {
        let host: String?
        /// v1: the ONE origin fronting every /v1/* plane. Preferred when present.
        let origin: String?
        let relay_url: String?
        /// Pre-unification faces — folded into `origin` during the transition (ICD §12).
        let arc_url: String?
        let node_url: String?
    }
}

private struct ArcListResponse: Decodable {
    let arcs: [ConsoleArc]
}

enum ArcCatalogError: LocalizedError {
    case badURL
    case http(Int)
    case transport(String)

    var errorDescription: String? {
        switch self {
        case .badURL:           return "The Arc console URL is invalid."
        case .http(let code):   return "The Arc console returned HTTP \(code)."
        case .transport(let m): return m
        }
    }
}

/// Fetches the real Arc list from the Fleet Console. Fails LOUDLY — a throw here means the
/// picker shows an error state, never a faked directory.
enum ArcCatalogClient {
    /// The console base URL: `PACIFIC_CONSOLE_URL` (e.g. a hosted console) if set, else the
    /// local dev control plane on :8787. The iOS Simulator reaches the host's localhost
    /// directly; a device build must point `PACIFIC_CONSOLE_URL` at a reachable console.
    static var consoleBaseURL: String {
        if let env = ProcessInfo.processInfo.environment["PACIFIC_CONSOLE_URL"], !env.isEmpty {
            return env
        }
        // The official directory's home is wallflowers.io. It serves no /api/arcs today, so this
        // fails LOUDLY (the picker's error state) until it does. Only a host we hold may serve it:
        // a list of Arcs from a host we may not hold is how a device would be offered a hostile
        // one. A dev
        // build can still point PACIFIC_CONSOLE_URL at a local console.
        return "https://wallflowers.io"
    }

    static func fetch() async throws -> [Arc] {
        guard let url = URL(string: "\(consoleBaseURL)/api/arcs") else {
            throw ArcCatalogError.badURL
        }
        let data: Data
        let resp: URLResponse
        // Short timeout: the directory is a nicety, liveness is not. Default (60s) let a
        // non-resolving host stall everything sequenced behind this call.
        var req = URLRequest(url: url)
        req.timeoutInterval = 5
        do {
            (data, resp) = try await URLSession.shared.data(for: req)
        } catch {
            throw ArcCatalogError.transport(error.localizedDescription)
        }
        if let http = resp as? HTTPURLResponse, !(200..<300).contains(http.statusCode) {
            throw ArcCatalogError.http(http.statusCode)
        }
        let decoded = try JSONDecoder().decode(ArcListResponse.self, from: data)
        return decoded.arcs.map(Self.map)
    }

    /// Map one console Arc → the app's `Arc`. v1: an Arc is ONE `origin` + a relay URL; plane
    /// liveness is discovered at runtime via `GET /v1/arc` (ArcDiscovery), never baked here. During
    /// the directory transition the origin is taken from `endpoints.origin`, else folded from the
    /// legacy `node_url` / `arc_url` / `host`.
    private static func map(_ c: ConsoleArc) -> Arc {
        let ep = c.endpoints
        return Arc(
            id: c.id,
            name: c.name,
            region: c.community ?? c.region ?? "",
            origin: originFrom(ep),
            relayURL: ep?.relay_url ?? ""
        )
    }

    /// The Arc's v1 origin: prefer `endpoints.origin`; else fold a legacy face into an origin
    /// (node_url is already an origin; arc_url is folded to its origin; host → https://host).
    private static func originFrom(_ ep: ConsoleArc.Endpoints?) -> String {
        guard let ep else { return "" }
        if let o = ep.origin, !o.isEmpty { return canonicalOrigin(o) }
        if let n = ep.node_url, !n.isEmpty { return canonicalOrigin(n) }
        if let a = ep.arc_url, !a.isEmpty { return canonicalOrigin(a) }
        if let h = ep.host, !h.isEmpty, h != "—" { return canonicalOrigin(h) }
        return ""
    }

    /// scheme://host[:port] from any URL-ish string (drops any path).
    private static func canonicalOrigin(_ raw: String) -> String {
        ArcDiscovery.normalizeOrigin(raw) ?? ""
    }
}
