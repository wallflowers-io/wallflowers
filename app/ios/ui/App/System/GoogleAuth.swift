//  GoogleAuth.swift
//  The app's FIRST on-device web-auth flow: Google OAuth 2.0 for native apps, done right —
//  Authorization Code + PKCE (RFC 7636), no client secret on the device (a native client can't
//  keep one), via `ASWebAuthenticationSession`. The resulting tokens are handed to GoogleService,
//  which seals them in the credential vault (the Keychain), exactly like the Sysiphus token.
//
//  Config lives in Info.plist, not in code: `GIDClientID` is the iOS OAuth client id, and its
//  reversed form is the custom URL scheme Google redirects back to (which must ALSO be listed in
//  CFBundleURLSchemes). No client id ⇒ `isConfigured == false` and CONNECT explains what's missing
//  — we never ship a placeholder secret or fail silently.

import Foundation
import AuthenticationServices
import CryptoKit
import Security
import UIKit

// MARK: - Config (Info.plist)

enum GoogleOAuth {
    static let authorizeEndpoint = URL(string: "https://accounts.google.com/o/oauth2/v2/auth")!
    static let tokenEndpoint     = URL(string: "https://oauth2.googleapis.com/token")!
    /// Read + write calendar EVENTS — covers the EVENTS ribbon's `events.list` and the LIFE
    /// mint's `events.insert`. Least privilege for what Pacific does with the calendar.
    static let scope = "https://www.googleapis.com/auth/calendar.events"

    /// The iOS OAuth client id, e.g. `1234-abc.apps.googleusercontent.com`. Not a secret for a
    /// native PKCE client, so it lives in Info.plist under the Google-standard `GIDClientID` key.
    static var clientID: String? {
        (Bundle.main.object(forInfoDictionaryKey: "GIDClientID") as? String)?.trimmed.nonEmpty
    }

    /// The reversed-client-id scheme Google redirects to (`com.googleusercontent.apps.1234-abc`).
    /// MUST also be registered in Info.plist CFBundleURLSchemes for the callback to reach us.
    static var redirectScheme: String? {
        guard let cid = clientID else { return nil }
        let suffix = ".apps.googleusercontent.com"
        guard cid.hasSuffix(suffix) else { return nil }
        return "com.googleusercontent.apps." + cid.dropLast(suffix.count)
    }

    static var redirectURI: String? { redirectScheme.map { $0 + ":/oauth2redirect" } }
    static var isConfigured: Bool { clientID != nil && redirectScheme != nil }
}

// MARK: - Tokens

/// The token set from Google. `refreshToken` is issued once (with `access_type=offline`) and
/// reused; a refresh response omits it, so we carry the prior one forward.
struct GoogleTokens: Codable, Equatable {
    var accessToken: String
    var refreshToken: String?
    var expiryEpochMs: Int64

    /// A 60s skew keeps us from presenting a token that expires mid-request.
    var isFresh: Bool { expiryEpochMs - 60_000 > Int64(Date().timeIntervalSince1970 * 1000) }
}

enum GoogleOAuthError: LocalizedError {
    case notConfigured
    case cancelled
    case badCallback
    case tokenExchange(Int, String)

    var errorDescription: String? {
        switch self {
        case .notConfigured:
            return "Google isn't set up: add your iOS OAuth client id as GIDClientID (and its reversed URL scheme) in the app config."
        case .cancelled:            return "Sign-in was cancelled."
        case .badCallback:          return "Google returned an unexpected sign-in response."
        case .tokenExchange(let c, let b): return "Google token exchange failed (\(c)): \(b)"
        }
    }
}

// MARK: - The flow

/// One OAuth round trip. `@MainActor` because `ASWebAuthenticationSession` must start on the main
/// thread and needs a presentation anchor (a window) to attach its sheet to.
@MainActor
final class GoogleOAuthFlow: NSObject, ASWebAuthenticationPresentationContextProviding {
    private let session: URLSession
    private var live: ASWebAuthenticationSession?

    init(session: URLSession = .shared) { self.session = session }

    // MARK: Authorize (interactive)

    /// Present Google's consent sheet and exchange the returned code for tokens (PKCE).
    func authorize() async throws -> GoogleTokens {
        guard let clientID = GoogleOAuth.clientID,
              let redirectURI = GoogleOAuth.redirectURI,
              let redirectScheme = GoogleOAuth.redirectScheme else {
            throw GoogleOAuthError.notConfigured
        }

        let verifier = Self.codeVerifier()
        let challenge = Self.codeChallenge(for: verifier)

        var comps = URLComponents(url: GoogleOAuth.authorizeEndpoint, resolvingAgainstBaseURL: false)!
        comps.queryItems = [
            .init(name: "client_id", value: clientID),
            .init(name: "redirect_uri", value: redirectURI),
            .init(name: "response_type", value: "code"),
            .init(name: "scope", value: GoogleOAuth.scope),
            .init(name: "code_challenge", value: challenge),
            .init(name: "code_challenge_method", value: "S256"),
            .init(name: "access_type", value: "offline"),   // ask for a refresh token
            .init(name: "prompt", value: "consent"),
        ]
        let authURL = comps.url!

        let callback: URL = try await withCheckedThrowingContinuation { cont in
            let s = ASWebAuthenticationSession(url: authURL, callbackURLScheme: redirectScheme) { url, error in
                if let url { cont.resume(returning: url); return }
                if let e = error as? ASWebAuthenticationSessionError, e.code == .canceledLogin {
                    cont.resume(throwing: GoogleOAuthError.cancelled)
                } else {
                    cont.resume(throwing: error ?? GoogleOAuthError.badCallback)
                }
            }
            s.presentationContextProvider = self
            s.prefersEphemeralWebBrowserSession = false   // reuse the signed-in Google session
            self.live = s
            s.start()
        }

        guard let code = URLComponents(url: callback, resolvingAgainstBaseURL: false)?
            .queryItems?.first(where: { $0.name == "code" })?.value else {
            throw GoogleOAuthError.badCallback
        }

        return try await exchange(params: [
            "grant_type": "authorization_code",
            "code": code,
            "client_id": clientID,
            "redirect_uri": redirectURI,
            "code_verifier": verifier,
        ], carryingRefresh: nil)
    }

    // MARK: Refresh (silent)

    func refresh(_ refreshToken: String) async throws -> GoogleTokens {
        guard let clientID = GoogleOAuth.clientID else { throw GoogleOAuthError.notConfigured }
        return try await exchange(params: [
            "grant_type": "refresh_token",
            "refresh_token": refreshToken,
            "client_id": clientID,
        ], carryingRefresh: refreshToken)
    }

    // MARK: Token endpoint

    private func exchange(params: [String: String], carryingRefresh prior: String?) async throws -> GoogleTokens {
        var req = URLRequest(url: GoogleOAuth.tokenEndpoint)
        req.httpMethod = "POST"
        req.setValue("application/x-www-form-urlencoded", forHTTPHeaderField: "Content-Type")
        req.httpBody = params
            .map { "\($0.key)=\($0.value.formURLEncoded)" }
            .joined(separator: "&")
            .data(using: .utf8)

        let (data, resp) = try await session.data(for: req)
        let code = (resp as? HTTPURLResponse)?.statusCode ?? 0
        guard (200..<300).contains(code) else {
            throw GoogleOAuthError.tokenExchange(code, String(data: data, encoding: .utf8) ?? "")
        }

        struct TokenResponse: Decodable {
            let access_token: String
            let refresh_token: String?
            let expires_in: Int?
        }
        guard let tr = try? JSONDecoder().decode(TokenResponse.self, from: data) else {
            throw GoogleOAuthError.tokenExchange(code, "unparseable token response")
        }
        let ttl = Int64((tr.expires_in ?? 3600) * 1000)
        return GoogleTokens(accessToken: tr.access_token,
                            // A refresh response omits refresh_token — keep the one we hold.
                            refreshToken: tr.refresh_token ?? prior,
                            expiryEpochMs: Int64(Date().timeIntervalSince1970 * 1000) + ttl)
    }

    // MARK: PKCE

    private static func codeVerifier() -> String {
        var bytes = [UInt8](repeating: 0, count: 64)
        _ = SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes)
        return Data(bytes).base64URLEncoded
    }
    private static func codeChallenge(for verifier: String) -> String {
        Data(SHA256.hash(data: Data(verifier.utf8))).base64URLEncoded
    }

    // MARK: Presentation anchor

    func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .flatMap(\.windows)
            .first(where: \.isKeyWindow) ?? ASPresentationAnchor()
    }
}

// MARK: - Small helpers

extension Data {
    /// Base64URL (RFC 4648 §5): base64 with -/_ and no padding — PKCE's required encoding.
    var base64URLEncoded: String {
        base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }
}

private extension String {
    var trimmed: String { trimmingCharacters(in: .whitespacesAndNewlines) }
    var nonEmpty: String? { isEmpty ? nil : self }
    /// Percent-encode a value for an `application/x-www-form-urlencoded` body.
    var formURLEncoded: String {
        var allowed = CharacterSet.alphanumerics
        allowed.insert(charactersIn: "-._~")
        return addingPercentEncoding(withAllowedCharacters: allowed) ?? self
    }
}
