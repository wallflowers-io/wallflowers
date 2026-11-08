//  LiveSync.swift
//  The foreground live tap: near-real-time delivery WITHOUT touching the sync engine.
//
//  THE GAP IT CLOSES. Messages were poll-bound: a sealed blob sits on the relay until
//  the 3-second `autoSyncLoop` tick picks it up (measured 4 Aug: send→relay-ack ~175ms,
//  authored→folded median ~1.8s, p95 ~2.8s — the poll, not the network). The relay's
//  `Sub` frame already replays-then-STREAMS live (pacific-wire `Frame`), and the waker
//  plane is exactly this client. While the app is foregrounded, the phone can be its
//  own waker: hold ONE WebSocket SUBbed to this device's current wake tags and, the
//  moment any blob lands, run the ordinary sync immediately. Delivery becomes
//  ack + push + drain instead of ack + up-to-3s + drain.
//
//  WHAT IT IS NOT. Not a transport: nothing is read from the blob (it is sealed, and
//  this class never decodes one past its `seq`); the drain/fold path stays exactly
//  `syncNow()`. Not a background channel: the socket lives only while the scene is
//  active — backgrounded delivery stays the waker's job (APNs). The 3s poll keeps
//  running as the reconciler, so a dropped socket or missed frame costs at most one
//  poll interval, never a message.
//
//  TAGS ROTATE. Relay tags are per-(group, epoch), so the SUB set goes stale on every
//  commit. `assertTags` is called with the CURRENT set on each sync tick (the same
//  cadence that re-asserts the waker registration); the relay's `Sub` REPLACES a
//  connection's subscriptions, so re-SUBbing the changed set is one frame, idempotent.
//
//  THE CURSOR. Relay `seq` is a wall-clock microsecond stamp (the waker's convention):
//  a fresh socket SUBs from `since = now` so it wakes on NEW traffic only, and across
//  a reconnect it resumes from the highest seq it saw, so a blob that landed during
//  the drop still triggers a sync rather than waiting out the poll.

import Foundation

@MainActor
final class LiveSync: ObservableObject {
    private weak var core: CoreService?
    private var task: URLSessionWebSocketTask?
    private var active = false          // scene is foregrounded; reconnects allowed
    private var subbed: [String] = []   // tags the OPEN socket is subscribed to
    private var maxSeq: UInt64 = 0      // highest relay seq seen (reconnect cursor)
    private var backoff: TimeInterval = 0.5
    private var syncRunning = false
    private var syncAgain = false

    /// Foreground entry: open (or keep) the socket. Idempotent.
    func start(core: CoreService) {
        self.core = core
        active = true
        guard task == nil else { return }
        connect()
    }

    /// Background exit: drop the socket. The waker (APNs) owns delivery from here.
    func stop() {
        active = false
        task?.cancel(with: .goingAway, reason: nil)
        task = nil
        subbed = []
    }

    /// Re-SUB when the device's tag set actually changed (tags rotate per epoch).
    func assertTags(_ tags: [String]) {
        guard active, let task, tags != subbed else { return }
        subbed = tags
        send(sub(tags: tags), over: task)
    }

    // MARK: - the socket

    private func connect() {
        guard active, let core, let url = URL(string: core.relayUrl) else { return }
        let t = URLSession.shared.webSocketTask(with: url)
        task = t
        subbed = core.wakeTags()
        t.resume()
        // Fresh cursor = now (µs, the relay's seq domain); reconnect resumes from maxSeq.
        if maxSeq == 0 { maxSeq = UInt64(Date().timeIntervalSince1970 * 1_000_000) }
        send(sub(tags: subbed), over: t)
        receiveLoop(t)
    }

    private func sub(tags: [String]) -> String {
        let obj: [String: Any] = ["t": "sub", "tags": tags, "since": maxSeq]
        let data = (try? JSONSerialization.data(withJSONObject: obj)) ?? Data()
        return String(data: data, encoding: .utf8) ?? #"{"t":"sub","tags":[]}"#
    }

    private func send(_ json: String, over t: URLSessionWebSocketTask) {
        t.send(.string(json)) { [weak self] error in
            guard error != nil else { return }
            Task { @MainActor in self?.dropAndRetry(t) }
        }
    }

    private func receiveLoop(_ t: URLSessionWebSocketTask) {
        t.receive { [weak self] result in
            Task { @MainActor in
                guard let self, self.task === t else { return }   // a stale socket's callback
                switch result {
                case .success(let message):
                    self.backoff = 0.5
                    if case .string(let text) = message { self.handle(text) }
                    self.receiveLoop(t)
                case .failure:
                    self.dropAndRetry(t)
                }
            }
        }
    }

    /// One frame. Only `msg` matters (something landed on a watched tag); the blob
    /// stays opaque — its `seq` advances the cursor and the SYNC reads the mailbox.
    private func handle(_ text: String) {
        guard let data = text.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              obj["t"] as? String == "msg" else { return }
        if let seq = obj["seq"] as? UInt64, seq > maxSeq { maxSeq = seq }
        else if let seq = (obj["seq"] as? NSNumber)?.uint64Value, seq > maxSeq { maxSeq = seq }
        scheduleSync()
    }

    /// LEADING-EDGE: the first frame syncs IMMEDIATELY — a trailing debounce here
    /// spent 150ms of the end-to-end budget waiting for a burst that usually is not
    /// one. Frames that land while a sync is in flight coalesce into exactly ONE
    /// follow-up pass, so a commit-plus-messages burst still costs two syncs, not N.
    private func scheduleSync() {
        if syncRunning { syncAgain = true; return }
        syncRunning = true
        Task { [weak self] in
            guard let self else { return }
            repeat {
                self.syncAgain = false
                await self.core?.syncNow(silent: true)
            } while self.syncAgain
            self.syncRunning = false
        }
    }

    private func dropAndRetry(_ t: URLSessionWebSocketTask) {
        guard task === t else { return }
        task?.cancel(with: .goingAway, reason: nil)
        task = nil
        subbed = []
        guard active else { return }
        let delay = backoff
        backoff = min(backoff * 2, 8)
        Task { [weak self] in
            try? await Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000))
            guard let self, self.active, self.task == nil else { return }
            self.connect()
        }
    }
}
