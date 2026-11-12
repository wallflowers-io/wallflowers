//  EventReminders.swift
//  A personal reminder for an event, held by the OS: a pending UNNotificationRequest IS
//  the record, so nothing is persisted here. Being reminded needs nobody; saying you
//  are coming is a `contact.inviteReply`, not this.

import SwiftUI
import UserNotifications

// MARK: - Reminders (the OS holds them; nothing is persisted here)

/// Which events have a local reminder pending. The store of truth is
/// `UNUserNotificationCenter` — this is a published mirror so SwiftUI can render it,
/// rehydrated from the notification centre rather than from any file of our own.
@MainActor
final class EventReminders: ObservableObject {
    static let shared = EventReminders()
    @Published private(set) var armed: Set<String> = []

    private init() {}

    func isArmed(_ id: String) -> Bool { armed.contains(id) }

    /// Re-read the OS. Called on appear, so a reminder set on another launch (or one
    /// the system already delivered and dropped) is reflected rather than remembered.
    func refresh() async {
        let pending = await UNUserNotificationCenter.current().pendingNotificationRequests()
        armed = Set(pending.compactMap { $0.identifier.hasPrefix("rsvp-")
            ? String($0.identifier.dropFirst(5)) : nil })
    }

    /// Arm or cancel. Best-effort: with notifications denied there is simply no
    /// wake-up, and nothing else about the row changes.
    ///
    /// ARMING IS THE ASK. This is one of the two moments a wake is genuinely
    /// wanted (the other is accepting a connection), so the permission is
    /// requested HERE and awaited before the request is added — an alert raised
    /// over the event the user just marked, rather than at launch over nothing.
    /// Adding before the answer would silently drop the reminder.
    func toggle(_ id: String, title: String, startMs: Int64, venue: String?) {
        let centre = UNUserNotificationCenter.current()
        if armed.contains(id) {
            armed.remove(id)
            centre.removePendingNotificationRequests(withIdentifiers: ["rsvp-\(id)"])
            return
        }
        guard let fire = RsvpReminder.fireDate(startMs: startMs) else { return }
        armed.insert(id)
        let c = UNMutableNotificationContent()
        c.title = title
        if let venue { c.body = venue }
        c.sound = .default
        c.userInfo = ["rsvpEvent": id]
        Task {
            guard await WakePermission.request() else {
                armed.remove(id)   // no wake is possible; do not claim one
                return
            }
            // Re-measure AFTER the answer: the user may have sat on the alert, and
            // a non-positive interval is a trigger iOS refuses.
            let delay = fire.timeIntervalSinceNow
            guard delay > 0 else { armed.remove(id); return }
            try? await centre.add(UNNotificationRequest(
                identifier: "rsvp-\(id)", content: c,
                trigger: UNTimeIntervalNotificationTrigger(
                    timeInterval: delay, repeats: false)))
        }
    }
}

/// The reminder a going-mark earns — pure, so the clamp rules are testable: 2h before
/// the start, never in the past, and none at all for something already begun or about
/// to begin (a reminder that fires after the doors open is noise, not help). Multi-day
/// runs whose start is behind us get no reminder either — there is nothing to be late for.
enum RsvpReminder {
    static let lead: TimeInterval = 2 * 3600
    static let minNotice: TimeInterval = 120

    static func fireDate(startMs: Int64, now: Date = Date()) -> Date? {
        let start = Date(timeIntervalSince1970: Double(startMs) / 1000)
        guard start.timeIntervalSince(now) > minNotice else { return nil }
        return max(start.addingTimeInterval(-lead), now.addingTimeInterval(60))
    }
}
