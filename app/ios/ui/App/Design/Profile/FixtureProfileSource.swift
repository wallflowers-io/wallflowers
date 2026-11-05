//  FixtureProfileSource.swift
//  FIXTURE — the injected stand-in binder, so the component and the T1–T4
//  tracks build against the `ProfileSource` seam before the real
//  CoreService-backed binder lands (T5b swaps this behind the same protocol,
//  then this file DIES at M1 with the fixture delete).
//
//  Data ported from prototypes/profile/ProfileComponent.swift's fixtures —
//  the three profiles the spec was drawn with.

import SwiftUI

/// A canned `ProfileSource`. Every answer is immediate and static.
struct FixtureProfileSource: ProfileSource {
    let fixtureHeadline: ProfileHeadline
    let fixtureInfo: ProfileInfo?
    let fixtureCells: [ProfileGeoCell]
    let fixturePins: [ProfilePin]

    func headline() async -> ProfileHeadline? { fixtureHeadline }
    func info() async -> ProfileInfo? { fixtureInfo }
    func geoCells() async -> [ProfileGeoCell] { fixtureCells }
    func pins() async -> [ProfilePin] { fixturePins }

    // MARK: The stub routes (Life.swift's navigationDestinations, pre-T5b)

    /// FIXTURE routing: any person id lands on the one person fixture, any
    /// place id on the located space — enough for the router cases to reach a
    /// real page while the real binder is built.
    static func person(_ id: String) -> FixtureProfileSource { dare }
    static func place(_ id: String) -> FixtureProfileSource { jimmys }

    // MARK: The three profiles

    /// Person, photo banner, bio, one space pin.
    static let dare = FixtureProfileSource(
        fixtureHeadline: ProfileHeadline(
            kind: .person, name: "Dare Balogun", initials: "DB", city: "London",
            banner: .photo,
            color: Color(red: 0.79, green: 0.44, blue: 0.18),
            color2: Color(red: 0.48, green: 0.24, blue: 0.08)),
        fixtureInfo: ProfileInfo(desc: "NTS resident DJ. Potential CMO."),
        fixtureCells: [],
        fixturePins: [
            ProfilePin(initials: "N", title: "NTS London", kind: .space,
                       destinationID: "fixture-nts", color: Color(white: 0.35)),
        ])

    /// Located space: info card with its stat row + the map card's cells.
    static let jimmys = FixtureProfileSource(
        fixtureHeadline: ProfileHeadline(
            kind: .space, name: "Jimmy's Pizza", initials: "JP",
            banner: .photo,
            color: Color(red: 0.78, green: 0.25, blue: 0.18),
            color2: Color(red: 0.43, green: 0.13, blue: 0.09)),
        fixtureInfo: ProfileInfo(desc: "Family-owned since the '70s. 561 Wilson Ave, Bushwick.",
                                 hereNow: "9", since: "1974"),
        fixtureCells: [
            ProfileGeoCell(hash: "dr5rw0", x: 0.36, y: 0.22, w: 0.24, h: 0.36),
            ProfileGeoCell(hash: "dr5rw1", x: 0.60, y: 0.22, w: 0.18, h: 0.36),
            ProfileGeoCell(hash: "dr5rqp", x: 0.36, y: 0.58, w: 0.24, h: 0.28),
        ],
        fixturePins: [
            ProfilePin(initials: "◷", title: "Late Night Special · nightly", kind: .event),
            ProfilePin(initials: "$", title: "$1.50 slice", kind: .listing),
        ])

    /// Unlocated space, live banner, a person pin routing back out.
    static let nts = FixtureProfileSource(
        fixtureHeadline: ProfileHeadline(
            kind: .space, name: "NTS London", initials: "N",
            banner: .live,
            color: Color(white: 0.23),
            color2: Color(white: 0.10)),
        fixtureInfo: ProfileInfo(desc: "Don't assume.",
                                 hereNow: "1,893", since: "2011"),
        fixtureCells: [],
        fixturePins: [
            ProfilePin(initials: "DB", title: "Dare Balogun", kind: .person,
                       destinationID: "fixture-dare",
                       color: Color(red: 0.79, green: 0.44, blue: 0.18)),
            ProfilePin(initials: "▸", title: "Now playing", kind: .post),
        ])
}
