//  GoogleCalendarService.swift
//  The Google Calendar seam — `events.insert` for the EVENTS mirror.
//
//  Extracted from LifeMint.swift when the mint wizards were purged (18 Aug). It was never
//  part of a wizard: the mirror is a property of minting an Event, not of the surface that
//  collected it, and `Life.mirrorToGoogleCalendar` is its one caller.
//
//  Whether the mirror should exist AT ALL is still Ralph's call (ruling R2):
//  repointing the EVENTS `+` at the real Event(29) mint kept it
//  so that repointing would not silently delete a working feature. Answering "no" deletes
//  this file and its caller together.

import Foundation

protocol GoogleAuthProviding: Sendable {
    func accessToken() async throws -> String
}

enum GoogleCalendarError: LocalizedError {
    case notConnected, badResponse
    case http(Int, String)
    var errorDescription: String? {
        switch self {
        case .notConnected:        return "Google isn't connected — no Calendar access token."
        case .badResponse:         return "Google Calendar returned an unexpected response."
        case .http(let c, let b):  return "Google Calendar API error \(c): \(b)"
        }
    }
}

/// Default token source, assuming OAuth is handled elsewhere: a bearer token from the
/// environment (`GOOGLE_OAUTH_ACCESS_TOKEN`) or UserDefaults (`google.accessToken`).
/// Swap for the real GoogleSignIn session's token when it lands.
struct EnvGoogleAuth: GoogleAuthProviding {
    func accessToken() async throws -> String {
        if let t = ProcessInfo.processInfo.environment["GOOGLE_OAUTH_ACCESS_TOKEN"], !t.isEmpty { return t }
        if let t = UserDefaults.standard.string(forKey: "google.accessToken"), !t.isEmpty { return t }
        throw GoogleCalendarError.notConnected
    }
}

/// Minimal Google Calendar API v3 client, built against the documented `events.insert`
/// endpoint — `POST /calendars/{calendarId}/events` with a `Bearer` token and an Event
/// resource body (summary + start/end{dateTime,timeZone} + description). Assumes OAuth
/// is handled: the auth provider supplies the token.
struct GoogleCalendarService {
    var auth: GoogleAuthProviding = EnvGoogleAuth()
    var session: URLSession = .shared
    var calendarId: String = "primary"

    struct CreatedEvent { let id: String; let htmlLink: String }

    func insertEvent(summary: String, description: String?, start: Date, end: Date,
                     timeZone: TimeZone = .current) async throws -> CreatedEvent {
        let token = try await auth.accessToken()
        guard let url = URL(string:
            "https://www.googleapis.com/calendar/v3/calendars/\(calendarId)/events") else {
            throw GoogleCalendarError.badResponse
        }
        var req = URLRequest(url: url)
        req.httpMethod = "POST"
        req.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")

        // RFC3339 dateTime paired with the IANA timeZone — the documented Event shape.
        let iso = ISO8601DateFormatter()
        iso.timeZone = timeZone
        iso.formatOptions = [.withInternetDateTime]
        var body: [String: Any] = [
            "summary": summary,
            "start": ["dateTime": iso.string(from: start), "timeZone": timeZone.identifier],
            "end":   ["dateTime": iso.string(from: end),   "timeZone": timeZone.identifier],
        ]
        if let description, !description.isEmpty { body["description"] = description }
        req.httpBody = try JSONSerialization.data(withJSONObject: body)

        let (data, resp) = try await session.data(for: req)
        guard let http = resp as? HTTPURLResponse else { throw GoogleCalendarError.badResponse }
        guard (200..<300).contains(http.statusCode) else {
            throw GoogleCalendarError.http(http.statusCode, String(data: data, encoding: .utf8) ?? "")
        }
        guard let json = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              let id = json["id"] as? String else { throw GoogleCalendarError.badResponse }
        return CreatedEvent(id: id, htmlLink: json["htmlLink"] as? String ?? "")
    }
}
