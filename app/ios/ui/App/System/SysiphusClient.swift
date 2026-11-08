//  SysiphusClient.swift
//  The HTTP client for Sysiphus — Aries' WordPress-shaped single source of truth
//  (test instance: a small API over Postgres, federating Shopify/BrightPearl/
//  Warewolf). Each employee authenticates with their OWN login; the returned token
//  scopes the System. On the simulator, `localhost` reaches the host Mac directly.

import Foundation

enum SysiphusHTTPError: Error, LocalizedError {
    case badStatus(Int)
    case transport(String)
    case decode(String)
    case unauthorized

    var errorDescription: String? {
        switch self {
        case .badStatus(let s): return "Sysiphus returned HTTP \(s)"
        case .transport(let m): return "Couldn’t reach Sysiphus: \(m)"
        case .decode(let m):    return "Sysiphus response wasn’t understood: \(m)"
        case .unauthorized:     return "Sysiphus login failed"
        }
    }
}

struct SysiphusClient {
    /// Dev default: the test Sysiphus API on the host. Simulator reaches host localhost.
    var baseURL: URL = URL(string: "http://localhost:8787")!
    private let session: URLSession = .shared

    private func decoder() -> JSONDecoder {
        let d = JSONDecoder()
        d.keyDecodingStrategy = .convertFromSnakeCase   // wholesale_price -> wholesalePrice, etc.
        return d
    }

    // MARK: auth

    struct LoginResult: Decodable { let token: String; let employee: Employee }

    func login(username: String, password: String) async throws -> LoginResult {
        var req = URLRequest(url: baseURL.appendingPathComponent("auth/login"))
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.httpBody = try JSONSerialization.data(withJSONObject: ["username": username, "password": password])
        let (data, resp) = try await send(req)
        if (resp as? HTTPURLResponse)?.statusCode == 401 { throw SysiphusHTTPError.unauthorized }
        try check(resp)
        do { return try decoder().decode(LoginResult.self, from: data) }
        catch { throw SysiphusHTTPError.decode(String(describing: error)) }
    }

    // MARK: fields

    private struct DesignsEnvelope: Decodable { let designs: [SysiphusField] }
    private struct DesignEnvelope: Decodable { let design: SysiphusField }

    /// Pull the designs this credential can see — the System's Fields.
    func designs(token: String) async throws -> [SysiphusField] {
        let (data, resp) = try await send(authed(path: "designs", token: token))
        try check(resp)
        do { return try decoder().decode(DesignsEnvelope.self, from: data).designs }
        catch { throw SysiphusHTTPError.decode(String(describing: error)) }
    }

    /// LOOKUP live tier: re-read ONE design from source (freshens stock / open orders).
    func design(token: String, id: String) async throws -> SysiphusField {
        let (data, resp) = try await send(authed(path: "designs/\(id)", token: token))
        try check(resp)
        do { return try decoder().decode(DesignEnvelope.self, from: data).design }
        catch { throw SysiphusHTTPError.decode(String(describing: error)) }
    }

    // MARK: plumbing

    private func authed(path: String, token: String) -> URLRequest {
        var req = URLRequest(url: baseURL.appendingPathComponent(path))
        req.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        return req
    }

    private func send(_ req: URLRequest) async throws -> (Data, URLResponse) {
        do { return try await session.data(for: req) }
        catch { throw SysiphusHTTPError.transport(error.localizedDescription) }
    }

    private func check(_ resp: URLResponse) throws {
        guard let http = resp as? HTTPURLResponse else { throw SysiphusHTTPError.transport("no HTTP response") }
        guard (200..<300).contains(http.statusCode) else { throw SysiphusHTTPError.badStatus(http.statusCode) }
    }
}
