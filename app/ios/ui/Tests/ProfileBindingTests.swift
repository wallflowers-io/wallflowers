//  ProfileBindingTests.swift
//  The real Profile binder's pure half (`ProfileBind`) against CONSTRUCTED FFI
//  rows — no core, no I/O. Covers the icon resolve order (clip → photo →
//  colour+initials, incl. clip precedence and the malformed-slot fall-through),
//  and the honest-nil paths (no placebo values — absent is absent).

import XCTest
@testable import WallFlowers
import PacificCore

final class ProfileBindingTests: XCTestCase {

    // "hello" / "world" — small, valid, distinguishable payloads.
    private let clipB64 = "aGVsbG8="
    private let photoB64 = "d29ybGQ="

    // MARK: Row builders (constructed, never read from a store)

    private func card(note: String = "", photo: String = "", clip: String = "",
                      color: String = "", city: String = "") -> ProfileCard {
        ProfileCard(org: "", title: "", note: note, emails: [], phones: [], urls: [],
                    tags: [], photo: photo, photoMime: photo.isEmpty ? "" : "image/jpeg",
                    clip: clip, clipMime: clip.isEmpty ? "" : "video/mp4",
                    capturedAt: 0, avatarColor: color, city: city)
    }

    private func groupRow(name: String = "Jimmy's Pizza", note: String = "",
                          photo: String = "", clip: String = "", color: String = "",
                          cover: String = "", coverMime: String = "") -> GroupRow {
        GroupRow(displayName: name, shape: "organisation", presence: "", canSync: true,
                 org: "", title: "", note: note, photo: photo,
                 emails: [], phones: [], urls: [], tags: [], roles: [], credentials: [],
                 affiliations: [], offices: [],
                 photoMime: photo.isEmpty ? "" : "image/jpeg",
                 clip: clip, clipMime: clip.isEmpty ? "" : "video/mp4",
                 capturedAt: 0, avatarColor: color, cover: cover, coverMime: coverMime)
    }

    private func placeRow(name: String = "Jimmy's", descriptor: String = "") -> PlaceRow {
        PlaceRow(id: "p1", name: name, descriptor: descriptor,
                 placed: false, lat: 0, lng: 0, label: "", address: "",
                 landParcel: "", landProprietor: "", landCompanyNo: "",
                 landSource: "", landAsOf: 0, landPermission: "unknown",
                 doorbell: "", access: "private", visibility: "private")
    }

    private func eventRow(title: String = "Late Night Special", descriptor: String = "",
                          banner: String = "", bannerMime: String = "") -> EventRow {
        EventRow(id: "e1", title: title, descriptor: descriptor, startMs: 1, endMs: 0,
                 venue: "", venuePlace: "", venuePlaceName: "", venueAt: 0,
                 placed: false, lat: 0, lng: 0,
                 listed: false, priceCents: 0, currency: "", capacity: 0,
                 open: false, sold: 0, overCapacity: false, listing: "",
                 banner: banner, bannerMime: bannerMime, photoCount: 0, hasClip: false,
                 lineup: [], recurrence: "")
    }

    // MARK: Icon resolve order — clip → photo → colour+initials

    func testClipTakesPrecedenceOverPhoto() {
        let icon = ProfileBind.icon(clipB64: clipB64, photoB64: photoB64, colorHex: "FF0000")
        XCTAssertEqual(icon, .clip(Data(base64Encoded: clipB64)!))
    }

    func testPhotoWhenNoClip() {
        let icon = ProfileBind.icon(clipB64: "", photoB64: photoB64, colorHex: "FF0000")
        XCTAssertEqual(icon, .photo(Data(base64Encoded: photoB64)!))
    }

    func testMalformedClipFallsThroughToPhoto() {
        // A present-but-undecodable slot must not blank the face.
        let icon = ProfileBind.icon(clipB64: "%%%not-base64%%%", photoB64: photoB64, colorHex: "")
        XCTAssertEqual(icon, .photo(Data(base64Encoded: photoB64)!))
    }

    func testColourWhenNoMedia() {
        XCTAssertEqual(ProfileBind.icon(clipB64: "", photoB64: "", colorHex: "AB12CD"),
                       .color("AB12CD"))
    }

    func testDefaultBlockColourWhenUnset() {
        XCTAssertEqual(ProfileBind.icon(clipB64: "", photoB64: "", colorHex: ""),
                       .color(CoreService.defaultAvatarColorHex))
    }

    // MARK: Banner — real media or honest nil

    func testBannerAbsentIsNil() {
        XCTAssertNil(ProfileBind.bannerKind(b64: "", mime: "image/jpeg"))
        XCTAssertNil(ProfileBind.bannerKind(b64: clipB64, mime: "application/pdf"))
    }

    func testBannerKindsFollowMime() {
        XCTAssertEqual(ProfileBind.bannerKind(b64: clipB64, mime: "image/jpeg"), .photo)
        XCTAssertEqual(ProfileBind.bannerKind(b64: clipB64, mime: "image/gif"), .gif)
        XCTAssertEqual(ProfileBind.bannerKind(b64: clipB64, mime: "video/mp4"), .video)
    }

    func testGroupHeadlineBannerFromCover() {
        XCTAssertEqual(ProfileBind.headline(group: groupRow(cover: clipB64,
                                                            coverMime: "video/mp4")).banner,
                       .video)
        // No cover = the block ground, never a fabricated photo state.
        XCTAssertEqual(ProfileBind.headline(group: groupRow()).banner, .block)
    }

    func testEventHeadlineBannerFromMedia() {
        XCTAssertEqual(ProfileBind.headline(event: eventRow(banner: clipB64,
                                                            bannerMime: "image/jpeg")).banner,
                       .photo)
        XCTAssertEqual(ProfileBind.headline(event: eventRow()).banner, .block)
    }

    func testPersonAndPlaceHaveNoBannerPrimitive() {
        // §5 decision 6 is a pending ruling — until it lands the kinds stay .block.
        XCTAssertEqual(ProfileBind.headline(person: "Dare", card: card(photo: photoB64)).banner,
                       .block)
        XCTAssertEqual(ProfileBind.headline(place: placeRow()).banner, .block)
    }

    // MARK: Names per-kind (displayName / name / title), unified at the binder

    func testHeadlineNamesPerKind() {
        XCTAssertEqual(ProfileBind.headline(person: "Dare Balogun", card: nil).name, "Dare Balogun")
        XCTAssertEqual(ProfileBind.headline(group: groupRow(name: "NTS London")).name, "NTS London")
        XCTAssertEqual(ProfileBind.headline(place: placeRow(name: "Jimmy's")).name, "Jimmy's")
        XCTAssertEqual(ProfileBind.headline(event: eventRow(title: "Closing Night")).name, "Closing Night")
    }

    func testUnnamedGroupFallsBackToRosterLabelNeverHex() {
        let h = ProfileBind.headline(group: groupRow(name: ""), fallbackName: "Nye & Tom")
        XCTAssertEqual(h.name, "Nye & Tom")
    }

    func testHeadlineIconSlotsAndHonestCity() {
        let h = ProfileBind.headline(person: "Dare Balogun",
                                     card: card(photo: photoB64, clip: clipB64))
        XCTAssertEqual(h.iconClip, Data(base64Encoded: clipB64))
        XCTAssertEqual(h.iconPhoto, Data(base64Encoded: photoB64))
        XCTAssertNil(h.city)   // no city field exists anywhere (map §3)
        XCTAssertEqual(h.kind, .person)
        XCTAssertEqual(h.initials, "DB")
    }

    // MARK: Initials

    func testInitials() {
        XCTAssertEqual(ProfileBind.initials("Dare Balogun"), "DB")
        XCTAssertEqual(ProfileBind.initials("Jimmy's"), "JI")
        XCTAssertEqual(ProfileBind.initials(""), "?")
    }

    // MARK: Info cards — bio binds note, descriptor stays a one-liner, honest nil

    func testPersonInfoBindsCardNote() {
        XCTAssertEqual(ProfileBind.info(person: card(note: "NTS resident DJ."))?.desc,
                       "NTS resident DJ.")
        XCTAssertNil(ProfileBind.info(person: card()))   // empty note = no card
        XCTAssertNil(ProfileBind.info(person: nil))      // no published card at all
    }

    func testGroupInfoStatsAreHonest() {
        let info = ProfileBind.info(group: groupRow(note: "Family-owned."))
        XCTAssertEqual(info?.desc, "Family-owned.")
        // No tenure read model crosses the FFI, and "here now" has no primitive
        // (§5 decision 7: defer) — both stay nil, never a fabricated value.
        XCTAssertNil(info?.since)
        XCTAssertNil(info?.hereNow)
    }

    func testGroupInfoNilWhenNothingToSay() {
        // The roster count is no longer a stat, so the note is all the card has:
        // an empty note earns no card at all.
        XCTAssertNil(ProfileBind.info(group: groupRow()))
    }

    func testPlaceAndEventInfoBindDescriptor() {
        XCTAssertEqual(ProfileBind.info(place: placeRow(descriptor: "561 Wilson Ave"))?.desc,
                       "561 Wilson Ave")
        XCTAssertNil(ProfileBind.info(place: placeRow()))
        XCTAssertEqual(ProfileBind.info(event: eventRow(descriptor: "Nightly."))?.desc, "Nightly.")
        XCTAssertNil(ProfileBind.info(event: eventRow()))
    }
}
