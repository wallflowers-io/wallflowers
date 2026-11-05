//  NotifPresenter.swift
//  The app's one UNUserNotificationCenter delegate: how a local notification is presented while
//  the app is in front, and where a tap on one goes.
//
//  Set at the composition root (WallFlowersApp.init), before launch finishes, so a tap that launched
//  the app is still delivered to it. The center holds its delegate weakly, hence `shared`.

import Foundation
import UserNotifications

/// Presents notifications while foregrounded and ROUTES a tap to the thing it is about, rather
/// than dead-ending on the launch tab.
final class NotifPresenter: NSObject, UNUserNotificationCenterDelegate {
    static let shared = NotifPresenter()

    func userNotificationCenter(_ center: UNUserNotificationCenter,
                                willPresent notification: UNNotification) async
        -> UNNotificationPresentationOptions {
        // A WAKE notification in the foreground gets no banner. The wake already synced, so
        // the chat on screen is current — announcing it would be telling the user about
        // something they are looking at. (InboundWake declines to raise one at all while
        // active; this covers the race where the app foregrounds between the two.)
        if notification.request.content.categoryIdentifier == InboundWake.category { return [] }
        // A REMOTE push in the foreground is the waker's content-free "New message" —
        // while the app is open, messages are announced IN-APP (InboundWake.foregroundTick
        // → InAppNotify), never as an iOS banner over the open app.
        if notification.request.trigger is UNPushNotificationTrigger { return [] }
        return [.banner, .sound]
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter,
                                didReceive response: UNNotificationResponse) async {
        // A wake notification's tap opens the conversation it came from, through the existing
        // pacificOpenDM / pacificOpenRoom seam.
        if response.notification.request.content.categoryIdentifier == InboundWake.category {
            let info = response.notification.request.content.userInfo
            await MainActor.run { _ = InboundWake.route(info) }
            return
        }
        // A mint suggestion's Accept / Dismiss. Announced rather than executed here: this
        // object has no store, and the decision touches core.
        let action = response.actionIdentifier
        if action == ProposalStore.acceptAction || action == ProposalStore.dismissAction,
           let id = response.notification.request.content.userInfo["proposal"] as? String {
            await MainActor.run {
                NotificationCenter.default.post(
                    name: .pacificProposalDecision, object: nil,
                    userInfo: ["proposal": id, "accept": action == ProposalStore.acceptAction])
            }
            return
        }
        // An RSVP reminder's tap reopens the row it reminded about. Posted twice: once for a
        // mounted LIFE, and again after the shell has switched screens for the cold case —
        // re-opening the same cover is a no-op, so the double delivery is harmless.
        if let row = response.notification.request.content.userInfo["rsvpEvent"] as? String {
            await MainActor.run {
                NotificationCenter.default.post(name: .pacificOpenEventRow, object: nil,
                                                userInfo: ["id": row])
            }
            try? await Task.sleep(nanoseconds: 600_000_000)
            await MainActor.run {
                NotificationCenter.default.post(name: .pacificOpenEventRow, object: nil,
                                                userInfo: ["id": row])
            }
        }
    }
}

extension Notification.Name {
    /// An RSVP reminder was tapped: open the EVENTS row it reminded about. userInfo:
    /// ["id": the feed row id — "obj:…" (minted page) or "ra:…"/"art:…" (reference page)].
    static let pacificOpenEventRow = Notification.Name("pacificOpenEventRow")
}
