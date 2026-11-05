//  NotificationService.swift
//  The Notification Service Extension — how a message reaches a LOCKED phone.
//
//  WHY THIS EXISTS. Everything the app raises today is a LOCAL notification: it can only fire
//  while the app is running, so a suspended or killed Pacific delivers nothing. A messenger
//  cannot work that way. The delivery path is: the Arc sends an ALERT push carrying no content,
//  iOS wakes THIS extension (not the app), and the extension turns "something arrived under a
//  tag you hold" into a notification with a real sender and body.
//
//  THE CONTRACT, from UNNotificationServiceExtension.h verbatim:
//    "Call contentHandler with the modified notification content to deliver. If the handler is
//     not called before the service's time expires then the unmodified notification will be
//     delivered."
//  (Swift renames the ObjC selector `didReceiveNotificationRequest:withContentHandler:` to
//   `didReceive(_:withContentHandler:)`.)
//  So the push MUST already read acceptably unmodified — that is the fallback the user sees if
//  we are slow, offline, or wrong. Hence `Fallback.body`: never a lie, just less.
//
//  WHY IT DOES NOT DECRYPT YET, and this is deliberate rather than unfinished:
//  decrypting means opening pacific-core in a SECOND process while the app may hold it. Two
//  hazards, neither hypothetical:
//    1. `directory.rs` sets `journal_mode = WAL` but no `busy_timeout` and takes no advisory
//       lock, so a concurrent writer fails immediately rather than waiting.
//    2. Worse than a DB race: MLS ratchet state. If this extension decrypts and advances the
//       ratchet while the app does too, the two diverge and messages become PERMANENTLY
//       undecryptable. That is not a crash, it is silent data loss.
//  Core state also lives at `~/.pacific` INSIDE the app sandbox (paths.rs), which an extension
//  cannot read at all — moving it to the App Group is a migration of the identity key, and this
//  build has no identity recovery, so a botched migration is unrecoverable.
//
//  So this stage proves the DELIVERY CHAIN — APNs → extension → banner on a locked phone —
//  and touches no core state. Decryption lands once pacific-core can be opened safely from two
//  processes (cross-process lock + busy_timeout + App Group state dir + a tested migration).

import UserNotifications

/// What the user sees when we cannot do better. Deliberately honest and content-free: it says
/// something arrived, never who from or what it says, because at this stage we genuinely do not
/// know — and a plausible guess in a notification is worse than a vague truth.
private enum Fallback {
    static let title = "WallFlowers"
    static let body  = "New message"
}

final class NotificationService: UNNotificationServiceExtension {

    /// Held so `serviceExtensionTimeWillExpire` can still deliver something.
    private var contentHandler: ((UNNotificationContent) -> Void)?
    private var bestAttempt: UNMutableNotificationContent?

    override func didReceive(
        _ request: UNNotificationRequest,
        withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void
    ) {
        self.contentHandler = contentHandler
        let mutable = request.content.mutableCopy() as? UNMutableNotificationContent
        bestAttempt = mutable

        guard let content = mutable else {
            // Nothing to modify — deliver exactly what the Arc sent rather than dropping it.
            contentHandler(request.content)
            return
        }

        // The wake carries a TAG and nothing else. It is an opaque rotating mailbox address, so
        // it identifies no person and no conversation to anyone who intercepts the push —
        // including Apple, which sees every push in transit.
        let tag = request.content.userInfo["tag"] as? String

        // Stage 1: acknowledge the wake honestly. Stage 2 replaces this block with a tag drain
        // + MLS unseal, which is why the tag is read and logged now — so the payload contract is
        // exercised end to end before anything depends on it.
        NSLog("[nse] woke for tag=%@", tag.map { String($0.prefix(12)) + "…" } ?? "none")

        // NORMALISE, don't merely fill gaps. The Arc must never put content in a push — Apple
        // sees every payload in transit — so whatever text arrived is at best a placeholder and
        // at worst a leak. Overwriting it unconditionally is the correct stage-1 behaviour AND
        // the only externally observable proof that this extension ran at all.
        content.title = Fallback.title
        content.body = Fallback.body

        contentHandler(content)
    }

    /// The system is about to kill us. Deliver the best we have — otherwise the user gets the
    /// unmodified push, which is the same thing here but will not be once decryption lands.
    override func serviceExtensionTimeWillExpire() {
        if let contentHandler, let bestAttempt {
            NSLog("[nse] time expired — delivering best attempt")
            contentHandler(bestAttempt)
        }
    }
}
