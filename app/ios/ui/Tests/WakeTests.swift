//  WakeTests.swift
//  The two pieces of the APNs path that are pure logic, and therefore the two that must never
//  be "verified" by watching a phone:
//
//    1. WHEN a registration is sent (WakeDebounce). Tags rotate per epoch and the app asserts
//       its set on every sync tick, so the difference between "on change" and "on tick" is the
//       difference between a few posts a day and a phone hammering its own backend every three
//       seconds. This is exactly the kind of rule that looks right and isn't.
//    2. WHETHER a message has already been announced (InboundSeen). The core's fold is
//       idempotent — every sync re-presents the SAME messages — so without a primed, durable
//       seen-set the user is notified about their entire history on first launch and then again
//       on every wake.

import XCTest
@testable import WallFlowers

// MARK: - Debounce

final class WakeDebounceTests: XCTestCase {

    private let t0 = Date(timeIntervalSince1970: 1_770_000_000)
    private let token = "aabbccdd"
    private let tags = ["11aa", "22bb"]

    // MARK: fingerprint

    func test_fingerprint_isOrderAndDuplicateInsensitive() {
        let a = WakeDebounce.fingerprint(token: token, tags: ["11aa", "22bb"])
        let b = WakeDebounce.fingerprint(token: token, tags: ["22bb", "11aa", "22bb"])
        XCTAssertEqual(a, b, "the same tag SET in any order is the same registration")
    }

    func test_fingerprint_changesWithTokenAndWithTags() {
        let base = WakeDebounce.fingerprint(token: token, tags: tags)
        XCTAssertNotEqual(base, WakeDebounce.fingerprint(token: "eeff", tags: tags))
        XCTAssertNotEqual(base, WakeDebounce.fingerprint(token: token, tags: tags + ["33cc"]))
        XCTAssertNotEqual(base, WakeDebounce.fingerprint(token: token, tags: ["11aa"]))
    }

    func test_fingerprint_doesNotCollideOnTagBoundaries() {
        // "ab","c" and "a","bc" must not hash the same — a separator-free concatenation would.
        XCTAssertNotEqual(WakeDebounce.fingerprint(token: token, tags: ["ab", "c"]),
                          WakeDebounce.fingerprint(token: token, tags: ["a", "bc"]))
    }

    // MARK: the gates

    func test_firstAssertionSends_andIsNotRepeated() {
        var d = WakeDebounce()
        XCTAssertTrue(d.shouldSend(token: token, tags: tags, now: t0))
        d.recordSuccess(token: token, tags: tags, now: t0)
        XCTAssertFalse(d.shouldSend(token: token, tags: tags, now: t0.addingTimeInterval(3)),
                       "an unchanged set must not re-post on the next sync tick")
        XCTAssertFalse(d.shouldSend(token: token, tags: tags.reversed(), now: t0.addingTimeInterval(3)),
                       "reordering is not a change")
    }

    func test_epochRotationSends() {
        var d = WakeDebounce()
        d.recordSuccess(token: token, tags: tags, now: t0)
        // A group advanced: its per-epoch tag is new, the old one lingers in the retained set.
        XCTAssertTrue(d.shouldSend(token: token, tags: tags + ["33cc"], now: t0.addingTimeInterval(3)))
    }

    func test_staleAssertionIsReasserted() {
        var d = WakeDebounce()
        d.recordSuccess(token: token, tags: tags, now: t0)
        XCTAssertFalse(d.shouldSend(token: token, tags: tags, now: t0.addingTimeInterval(29 * 60)))
        XCTAssertTrue(d.shouldSend(token: token, tags: tags, now: t0.addingTimeInterval(31 * 60)),
                      "a waker restart must heal without the user doing anything")
    }

    func test_noTokenAndNoTagsNeverSend() {
        let d = WakeDebounce()
        XCTAssertFalse(d.shouldSend(token: nil, tags: tags, now: t0))
        XCTAssertFalse(d.shouldSend(token: "", tags: tags, now: t0))
        XCTAssertFalse(d.shouldSend(token: token, tags: [], now: t0),
                       "nothing to watch is not a registration")
    }

    func test_failureBacksOffExponentially_andDoesNotRecordTheFingerprint() {
        var d = WakeDebounce()
        XCTAssertTrue(d.shouldSend(token: token, tags: tags, now: t0))
        d.recordFailure(now: t0)
        XCTAssertFalse(d.shouldSend(token: token, tags: tags, now: t0.addingTimeInterval(3)),
                       "a down waker must not be retried on the very next tick")
        // 1st failure = 30s.
        XCTAssertTrue(d.shouldSend(token: token, tags: tags, now: t0.addingTimeInterval(31)),
                      "the same set is still unsent, so it must be attempted again after backoff")
        d.recordFailure(now: t0.addingTimeInterval(31))
        // 2nd failure = 60s.
        XCTAssertFalse(d.shouldSend(token: token, tags: tags, now: t0.addingTimeInterval(80)))
        XCTAssertTrue(d.shouldSend(token: token, tags: tags, now: t0.addingTimeInterval(95)))
    }

    func test_backoffIsCapped() {
        var d = WakeDebounce()
        for i in 0..<20 { d.recordFailure(now: t0.addingTimeInterval(Double(i))) }
        XCTAssertTrue(d.shouldSend(token: token, tags: tags, now: t0.addingTimeInterval(15 * 60 + 60)),
                      "backoff must top out, not grow to hours")
    }

    func test_successAfterFailureClearsTheBackoff() {
        var d = WakeDebounce()
        d.recordFailure(now: t0)
        d.recordSuccess(token: token, tags: tags, now: t0.addingTimeInterval(60))
        XCTAssertTrue(d.shouldSend(token: token, tags: tags + ["44dd"], now: t0.addingTimeInterval(61)))
    }

    func test_resetForgetsEverything() {
        var d = WakeDebounce()
        d.recordSuccess(token: token, tags: tags, now: t0)
        d.reset()
        XCTAssertTrue(d.shouldSend(token: token, tags: tags, now: t0.addingTimeInterval(1)),
                      "a new APNs token invalidates every prior assertion")
    }
}

// MARK: - Seen-set

final class InboundSeenTests: XCTestCase {

    private var suite: String!
    private var defaults: UserDefaults!

    override func setUp() {
        super.setUp()
        suite = "inbound-seen-tests-\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suite)
    }

    override func tearDown() {
        UserDefaults().removePersistentDomain(forName: suite)
        super.tearDown()
    }

    func test_firstPassPrimes_andAnnouncesNothing() {
        var seen = InboundSeen()
        XCTAssertTrue(seen.fresh(["a", "b", "c"]).isEmpty,
                      "installing this build must not announce the whole message history")
        XCTAssertEqual(seen.fresh(["a", "b", "c", "d"]), ["d"])
    }

    func test_reSyncOfTheSameFoldAnnouncesNothing() {
        var seen = InboundSeen(primed: true)
        XCTAssertEqual(seen.fresh(["a", "b"]), ["a", "b"])
        XCTAssertTrue(seen.fresh(["a", "b"]).isEmpty, "the fold is idempotent; the banners must be too")
    }

    func test_orderAndDuplicatesWithinOnePass() {
        var seen = InboundSeen(primed: true)
        XCTAssertEqual(seen.fresh(["b", "a", "b"]), ["b", "a"])
    }

    func test_absorbMarksSeenWithoutAnnouncing() {
        var seen = InboundSeen(primed: true)
        seen.absorb(["x", "y"])
        XCTAssertTrue(seen.fresh(["x", "y"]).isEmpty,
                      "messages read in the foreground must not be announced later")
    }

    func test_setIsBounded_oldestEvictedFirst() {
        var seen = InboundSeen(primed: true)
        seen.cap = 3
        _ = seen.fresh(["1", "2", "3", "4"])
        XCTAssertEqual(seen.order, ["2", "3", "4"])
        XCTAssertEqual(seen.fresh(["1"]), ["1"], "an evicted key is genuinely forgotten")
    }

    func test_survivesRelaunch() {
        let store = InboundSeenStore(defaults: defaults)
        var seen = store.load()
        XCTAssertTrue(seen.fresh(["a"]).isEmpty)   // primes
        _ = seen.fresh(["a", "b"])                  // b announced
        store.save(seen)

        var reloaded = InboundSeenStore(defaults: defaults).load()
        XCTAssertTrue(reloaded.fresh(["a", "b"]).isEmpty,
                      "a relaunch must not re-announce what this device already announced")
        XCTAssertEqual(reloaded.fresh(["a", "b", "c"]), ["c"])
    }
}

// MARK: - Harvest

final class InboundWakeHarvestTests: XCTestCase {

    private func scan(_ id: String, _ msgs: [(String, UInt64, String, UInt64, Bool)]) -> InboundScan {
        InboundScan(conversation: id, title: id.uppercased(),
                    routeKey: "peer", routeValue: id,
                    messages: msgs.map { (author: $0.0, gen: $0.1, text: $0.2, ts: $0.3, mine: $0.4) })
    }

    func test_ownMessagesAreNeverAnnounced() {
        let hits = InboundWakeHarvest.hits([
            scan("dm:p1", [("me", 1, "hi", 100, true), ("p1", 2, "hello", 200, false)])
        ])
        XCTAssertEqual(hits.map(\.key), ["dm:p1:p1:2"])
    }

    func test_keyIsStablePerMessage() {
        let one = InboundWakeHarvest.hits([scan("dm:p1", [("p1", 7, "a", 1, false)])])
        let two = InboundWakeHarvest.hits([scan("dm:p1", [("p1", 7, "a edited", 9, false)])])
        XCTAssertEqual(one[0].key, two[0].key,
                       "(conversation, author, gen) identifies a message forever — the log is append-only")
    }

    func test_oneBannerPerConversation_newestWithACount() {
        let hits = InboundWakeHarvest.hits([
            scan("dm:p1", [("p1", 1, "first", 100, false), ("p1", 2, "latest", 300, false)]),
            scan("dm:p2", [("p2", 1, "solo", 200, false)])
        ])
        let banners = InboundWakeHarvest.banners(hits)
        XCTAssertEqual(banners.count, 2, "a long offline stretch must not fire one banner per message")
        XCTAssertEqual(banners[0].hit.body, "latest")
        XCTAssertEqual(banners[0].more, 1)
        XCTAssertEqual(banners[1].more, 0)
    }

    func test_previewIsFlattenedAndTruncated() {
        XCTAssertEqual(InboundWakeHarvest.preview("  a\nb  "), "a b")
        let long = String(repeating: "x", count: 200)
        let p = InboundWakeHarvest.preview(long)
        XCTAssertEqual(p.count, 141)
        XCTAssertTrue(p.hasSuffix("…"), "a notification is a summons, not a reader")
    }
}
