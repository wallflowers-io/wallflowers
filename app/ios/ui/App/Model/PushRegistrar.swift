//  PushRegistrar.swift
//  Getting an APNs device token, and handing it to the one plane allowed to know it.
//
//  A push is the ONLY way a message reaches this phone while Pacific is suspended or killed.
//  Local notifications cannot: they fire only while the app runs. So the delivery path needs a
//  token, and the token has to reach whatever will do the waking.
//
//  WHAT THE TOKEN COSTS, stated plainly because it is a real change to the privacy posture.
//  The relay is a blind mailbox: it addresses by an opaque ROTATING tag and learns no content,
//  sender, group or graph. A push token is the opposite kind of thing — it is stable, it names
//  one install on one device, and to be useful the WAKER must bind it to the tags this device
//  listens for. That binding LINKS those tags to each other and to one phone, across rotations,
//  which the mailbox alone never permitted. Signal makes the identical trade: its server knows
//  which account to wake, never what was said. There is no push without something knowing who
//  to poke — the choice is only whether that is stated or discovered later.
//
//  WHERE IT GOES: the WAKE tier, not the relay and not the Arc (RINGFENCE.txt §6). See
//  Waker.swift for the separation and Waker.swift's `WakeDebounce` for when a registration is
//  actually sent — tags rotate per epoch, so "on every tick" and "on every change" are wildly
//  different rates and only the second one is sane.
//
//  SwiftUI has no `application(_:didRegisterForRemoteNotificationsWithDeviceToken:)`, so this is
//  the minimal UIApplicationDelegate that exists purely to receive it — plus the background
//  `content-available` path, which is the half that turns a blind push into a real notification.

import SwiftUI
import UIKit
import UserNotifications

/// WHEN Pacific asks iOS for permission to wake you.
///
/// Not at launch. First boot used to demand notifications before the user had a
/// single thing to be notified about — a permission spent on nothing, answered
/// "no" by anyone sensible, and then unrecoverable without a trip to Settings.
/// The prompt now rides the first moment a wake is genuinely wanted: a connection
/// accepted (someone can now message you) or a reminder armed on an event.
///
/// Idempotent and quiet: iOS shows the alert only while the status is
/// `.notDetermined`, so every call after the first is a cheap read, and a user who
/// declined is never asked again by us.
enum WakePermission {
    /// Ask if we have never asked. Returns whether a wake can actually be
    /// delivered, so a caller that is about to schedule one can tell.
    @discardableResult
    static func request() async -> Bool {
        let centre = UNUserNotificationCenter.current()
        let status = await centre.notificationSettings().authorizationStatus
        if status == .notDetermined {
            return (try? await centre.requestAuthorization(options: [.alert, .sound, .badge])) ?? false
        }
        return status == .authorized || status == .provisional || status == .ephemeral
    }

    /// Fire-and-forget, for call sites that only want the prompt raised.
    static func requestNow() { Task { await request() } }
}

/// Where this device stands with the wake tier. Every case is a fact the user can be shown
/// verbatim, because "notifications don't work" is otherwise undiagnosable.
enum WakeRegistration: Equatable {
    /// APNs has not issued a token. Either registration has not finished, or it failed.
    case noToken
    /// A token exists but there is nothing to register: the core does not yet expose the relay
    /// tags this device SUBs. Carries the reason, verbatim — this is a real, current gap.
    case noTags(String)
    case registering
    /// The waker accepted this token watching `tags` tags.
    case registered(tags: Int, at: Date)
    case failed(String)

    var label: String {
        switch self {
        case .noToken:                  return "No token"
        case .noTags(let why):          return why
        case .registering:              return "Registering"
        case .registered(let n, _):     return "Watching \(n) tag\(n == 1 ? "" : "s")"
        case .failed(let why):          return why
        }
    }
}

/// Holds the APNs token for this install and keeps the wake tier's view of it current.
/// Published so the UI can say, truthfully, whether this device can be woken at all — a thing
/// the user previously had no way to find out.
@MainActor
final class PushRegistrar: ObservableObject {
    static let shared = PushRegistrar()

    /// The APNs device token as lowercase hex, or nil if we have not been given one.
    /// NOT persisted: the token can change on reinstall, restore and some OS updates
    /// (`UIApplication.h`), so a cached copy is a stale copy. Re-registering every launch is
    /// the documented contract, not a belt-and-braces extra.
    @Published private(set) var token: String?
    /// Why registration failed, when it did. Surfaced rather than swallowed.
    @Published private(set) var error: String?
    /// Where this device stands with the wake tier.
    @Published private(set) var state: WakeRegistration = .noToken
    /// What the waker says about itself — including whether it holds an APNs key at all.
    @Published private(set) var health: WakerHealth = .unknown
    /// The tag set last handed to `syncRegistration`, so the truth surface can show how many
    /// mailboxes this device is actually asking to have watched.
    @Published private(set) var tags: [String] = []

    /// True once APNs has issued a token — i.e. this device is wakeable IN PRINCIPLE.
    /// Not the same as "will be woken": that also needs a registration and an APNs key on the
    /// waker, which is why the Settings row shows three facts and not one.
    var canBeWoken: Bool { token != nil }

    /// Performs a wake: sync + raise local notifications. Set by the app at launch — the
    /// delegate cannot reach `CoreService` itself (it is a `@StateObject` built in the App's
    /// init), so the app hands the capability down rather than the delegate reaching up.
    var onWake: (@MainActor () async -> Int)?

    var client = WakerClient()
    private var debounce = WakeDebounce()
    private var inFlight = false

    // MARK: - Token

    fileprivate func set(token data: Data) {
        let hex = data.map { String(format: "%02x", $0) }.joined()
        let changed = hex != token
        token = hex
        error = nil
        NSLog("[push] APNs token %@… (%d bytes)", String(hex.prefix(16)), data.count)
        // A NEW token invalidates everything previously asserted — the waker is holding a row
        // for a token that no longer exists.
        if changed { debounce.reset() }
        if state == .noToken { state = tags.isEmpty ? .noTags(Self.tagGap) : .registering }
        syncRegistration(tags: tags)
    }

    fileprivate func set(error message: String) {
        token = nil
        error = message
        state = .noToken
        debounce.reset()
        NSLog("[push] APNs registration FAILED: %@", message)
    }

    /// Ask iOS for a token. Authorization is requested separately (`WakePermission`,
    /// at the first moment a wake is wanted); registration is what actually
    /// produces the token, and it raises no prompt of its own.
    func register() {
        UIApplication.shared.registerForRemoteNotifications()
    }

    // MARK: - The wake tier

    /// The honest reason there is nothing to register today. Named once so the Settings row,
    /// the log line and this file cannot drift apart.
    static let tagGap = "Core exposes no wake tags"

    /// Tell the waker which tags to watch. Called on token acquisition and on every sync tick —
    /// tags rotate per epoch, so the CURRENT set is the only correct thing to assert.
    ///
    /// Cheap to call at any rate: `WakeDebounce` decides, and it sends only when the token or
    /// the tag SET actually changed (or the assertion went stale, or a backoff expired). A
    /// failure sets `state`/`error` and backs off — it never crashes, never retries in a loop,
    /// and never records a fingerprint it did not actually send.
    func syncRegistration(tags: [String]) {
        self.tags = tags
        guard let token, !token.isEmpty else { state = .noToken; return }
        if tags.isEmpty {
            // NOT an error and NOT a silent success: there is genuinely nothing to watch. The
            // current cause is the missing FFI surface (see CoreService.wakeTags), and saying
            // so is the difference between a known gap and a mystery.
            if case .registered = state {} else { state = .noTags(Self.tagGap) }
            return
        }
        guard !inFlight, debounce.shouldSend(token: token, tags: tags, now: Date()) else { return }
        inFlight = true
        state = .registering
        let client = self.client
        Task { @MainActor in
            defer { inFlight = false }
            do {
                try await client.register(token: token, tags: tags)
                debounce.recordSuccess(token: token, tags: tags, now: Date())
                state = .registered(tags: tags.count, at: Date())
                error = nil
            } catch {
                debounce.recordFailure(now: Date())
                let why = (error as? LocalizedError)?.errorDescription ?? String(describing: error)
                state = .failed(why)
                self.error = why
                NSLog("[push] waker register failed: %@", why)
            }
        }
    }

    /// Ask the waker whether it is up and whether it holds an APNs credential. Never throws.
    func checkHealth() async {
        health = await client.health()
    }

    /// Revoke: drop this device from the wake tier. Best-effort by nature — a device that has
    /// been wiped cannot un-register itself, which is why the waker's rows must expire anyway.
    func forget() async {
        guard let token else { return }
        try? await client.forget(token: token)
        debounce.reset()
        state = .noTags(Self.tagGap)
    }
}

/// The only reason this type exists: SwiftUI's App lifecycle does not surface the APNs
/// callbacks — neither the token nor the background `content-available` delivery.
final class PushAppDelegate: NSObject, UIApplicationDelegate {
    func application(_ application: UIApplication,
                     didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
        Task { @MainActor in PushRegistrar.shared.set(token: deviceToken) }
    }

    func application(_ application: UIApplication,
                     didFailToRegisterForRemoteNotificationsWithError error: Error) {
        Task { @MainActor in PushRegistrar.shared.set(error: error.localizedDescription) }
    }

    /// THE WAKE. A content-free push arrived; sync, decrypt, and raise a real notification for
    /// anything genuinely new (InboundWake).
    ///
    /// TWO iOS FACTS THIS PATH LIVES INSIDE, neither of them defects here:
    ///   1. A FORCE-QUIT app is NOT launched by `content-available`. iOS shows only the generic
    ///      alert the waker sent, and this method never runs. Nothing in the app can change that.
    ///   2. iOS budgets background wakes and throttles an app that reports `.noData` often. So
    ///      the return value is the truth — `.newData` only when something actually landed —
    ///      because lying here is how an app gets its wakes taken away.
    func application(_ application: UIApplication,
                     didReceiveRemoteNotification userInfo: [AnyHashable: Any]) async
        -> UIBackgroundFetchResult {
        guard let wake = await PushRegistrar.shared.onWake else { return .noData }
        let raised = await wake()
        return raised > 0 ? .newData : .noData
    }
}
