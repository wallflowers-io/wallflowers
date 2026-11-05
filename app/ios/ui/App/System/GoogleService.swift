//  GoogleService.swift
//  Google Calendar as a live, on-device System — the third credential model in Pacific (after
//  Sysiphus's device-local bearer and Gmail's Arc-brokered Nango): OAuth happens ON the device,
//  and the tokens are sealed in the credential vault (the Keychain), keyed by a content-free
//  handle — byte-for-byte the Sysiphus pattern (SystemService).
//
//  It conforms to `GoogleAuthProviding`, so the already-written `GoogleCalendarService`
//  (LifeMint) and anything else needing a Calendar token get a live, auto-refreshed bearer from
//  here instead of the env stand-in. `events` holds the last upcoming fetch; LIFE maps those to
//  the EVENTS ribbon (CalendarEventsSource). Fails LOUD — never a silent empty.

import SwiftUI

@MainActor
final class GoogleService: ObservableObject, GoogleAuthProviding {
    /// `expired` is its own case on purpose: holding a stale token is neither signed-out
    /// (a refresh may still work) nor connected (a call would fail). Collapsing the two is
    /// exactly what let the UI claim "connected" on a dead token.
    enum Status: Equatable { case signedOut, connecting, connected, expired, error(String) }

    @Published private(set) var status: Status = .signedOut
    /// The connected account (the primary calendar's id IS the account email — from events.list).
    @Published private(set) var account: String?
    /// The last upcoming events fetched, raw. LIFE maps these to EventVM (CalendarEventsSource).
    @Published private(set) var events: [GCalEvent] = []

    private var tokens: GoogleTokens?
    private var handle: CredentialHandle?
    private let flow = GoogleOAuthFlow()
    private let session: URLSession = .shared

    private static let handleKey  = "google.credentialHandle"
    private static let accountKey = "google.account"

    /// False until an iOS OAuth client id is present (GIDClientID). CONNECT shows why, and never
    /// pretends to sign in.
    var isConfigured: Bool { GoogleOAuth.isConfigured }
    /// We HOLD a token. Says nothing about whether it still works.
    var hasCredentials: Bool { tokens != nil }
    /// The token is present AND unexpired — the only form worth calling with. `status` reads
    /// this, so the UI cannot show "connected" on a token that expired an hour ago.
    var isUsable: Bool { tokens?.isFresh == true }
    /// Retained as the sign-in FACT (a refresh may still be possible from an expired token),
    /// deliberately distinct from `isUsable`.
    var isSignedIn: Bool { hasCredentials }

    init() { restore() }

    // MARK: Connect / disconnect

    func connect() async {
        guard isConfigured else {
            status = .error(GoogleOAuthError.notConfigured.errorDescription ?? "Google isn't set up.")
            return
        }
        status = .connecting
        do {
            let t = try await flow.authorize()
            persist(t)
            await refreshEvents()
        } catch GoogleOAuthError.cancelled {
            status = isUsable ? .connected : (hasCredentials ? .expired : .signedOut)
        } catch {
            status = .error((error as? LocalizedError)?.errorDescription ?? String(describing: error))
        }
    }

    func disconnect() {
        tokens = nil
        events = []
        account = nil
        status = .signedOut
        if let h = handle { CredentialVault.remove(h) }
        handle = nil
        UserDefaults.standard.removeObject(forKey: Self.handleKey)
        UserDefaults.standard.removeObject(forKey: Self.accountKey)
    }

    // MARK: GoogleAuthProviding — a live, auto-refreshed bearer

    func accessToken() async throws -> String {
        guard let t = tokens else { throw GoogleCalendarError.notConnected }
        if t.isFresh { return t.accessToken }
        guard let rt = t.refreshToken else { throw GoogleCalendarError.notConnected }
        let fresh = try await flow.refresh(rt)
        persist(fresh)
        return fresh.accessToken
    }

    // MARK: Fetch upcoming events (the EVENTS-ribbon feed)

    func refreshEvents() async {
        guard isSignedIn else { return }
        status = .connecting
        do {
            let token = try await accessToken()

            let iso = ISO8601DateFormatter()
            iso.formatOptions = [.withInternetDateTime]
            var comps = URLComponents(string:
                "https://www.googleapis.com/calendar/v3/calendars/primary/events")!
            comps.queryItems = [
                .init(name: "singleEvents", value: "true"),     // expand recurrences to instances
                .init(name: "orderBy", value: "startTime"),
                .init(name: "timeMin", value: iso.string(from: Date())),
                .init(name: "maxResults", value: "50"),
            ]
            var req = URLRequest(url: comps.url!)
            req.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")

            let (data, resp) = try await session.data(for: req)
            let code = (resp as? HTTPURLResponse)?.statusCode ?? 0
            guard (200..<300).contains(code) else {
                throw GoogleCalendarError.http(code, String(data: data, encoding: .utf8) ?? "")
            }
            let decoded = try JSONDecoder().decode(GCalEventsResponse.self, from: data)
            events = decoded.items ?? []
            if let s = decoded.summary, !s.isEmpty {
                account = s
                UserDefaults.standard.set(s, forKey: Self.accountKey)
            }
            status = .connected
        } catch {
            status = .error((error as? LocalizedError)?.errorDescription ?? String(describing: error))
        }
    }

    // MARK: Vault-backed persistence (handle in UserDefaults, secret in the Keychain)

    private func persist(_ t: GoogleTokens) {
        tokens = t
        let h = handle ?? CredentialHandle.mint(kind: .oauthToken, label: "Google Calendar")
        handle = h
        if let data = try? JSONEncoder().encode(t), let json = String(data: data, encoding: .utf8) {
            CredentialVault.store(json, for: h)
        }
        if let hdata = try? JSONEncoder().encode(h) {
            UserDefaults.standard.set(hdata, forKey: Self.handleKey)
        }
    }

    private func restore() {
        guard let hdata = UserDefaults.standard.data(forKey: Self.handleKey),
              let h = try? JSONDecoder().decode(CredentialHandle.self, from: hdata),
              let json = CredentialVault.secret(for: h),
              let t = try? JSONDecoder().decode(GoogleTokens.self, from: Data(json.utf8)) else { return }
        handle = h
        tokens = t
        account = UserDefaults.standard.string(forKey: Self.accountKey)
        status = .connected
        Task { await refreshEvents() }
    }
}
