//  GroupRoleRunsTests.swift
//  The PEOPLE lane's role runs — the pure half of GroupDetailView's roster
//  grouping. Two defaults carry the whole behaviour and neither is readable from
//  `roles` alone: the OWNER comes off the object, and a member with no overlay is
//  a Member (the core's `None` means "no overlay", not "no access").

import XCTest
@testable import WallFlowers
import PacificCore

final class GroupRoleRunsTests: XCTestCase {

    private let owner = String(repeating: "a", count: 64)
    private let admin = String(repeating: "b", count: 64)
    private let plain = String(repeating: "c", count: 64)
    private let guest = String(repeating: "d", count: 64)

    private func key(_ hex: String) -> String { IdentityKey.display(hex) }

    private func runs(_ roster: [String], owner ownerHex: String,
                      _ roles: [(String, String)] = []) -> [GroupDetailView.RoleRun] {
        GroupDetailView.roleRuns(
            roster: roster.map(key), ownerHex: ownerHex,
            roles: roles.map { GroupRoleRow(member: key($0.0), role: $0.1) })
    }

    /// The founder of a group nobody has roled still heads the lane as its owner —
    /// the case every Space starts in, and the one `roles` cannot answer.
    func test_ownerComesOffTheObject_notTheOverlay() {
        let out = runs([owner], owner: owner)
        XCTAssertEqual(out.map(\.role), ["owner"])
        XCTAssertEqual(out[0].members, [key(owner)])
    }

    /// No overlay is the common case, and it means Member — never a run of its own,
    /// and never a member dropped from the lane.
    func test_unroledMemberIsAMember() {
        let out = runs([owner, plain], owner: owner)
        XCTAssertEqual(out.map(\.role), ["owner", "member"])
        XCTAssertEqual(out[1].members, [key(plain)])
    }

    /// Standing order, and only the runs that have cards.
    func test_runsAreOrderedAndSparse() {
        let out = runs([owner, admin, plain, guest], owner: owner,
                       [(admin, "admin"), (guest, "guest")])
        XCTAssertEqual(out.map(\.role), ["owner", "admin", "member", "guest"])
        XCTAssertFalse(out.contains { $0.role == "viewer" }, "no empty VIEWER heading")
    }

    /// The object's owner outranks an overlay that says otherwise: two headings for
    /// one person is the one outcome the lane must not produce.
    func test_ownerOverlayDoesNotDuplicateTheOwner() {
        let out = runs([owner, plain], owner: owner,
                       [(owner, "viewer"), (plain, "member")])
        XCTAssertEqual(out.map(\.role), ["owner", "member"])
        XCTAssertEqual(out.flatMap(\.members).filter { $0 == key(owner) }.count, 1)
    }

    /// The core owns the vocabulary and may widen it. An unknown word gets its own
    /// run at the end rather than swallowing the member.
    func test_unknownRoleKeepsItsMember() {
        let out = runs([owner, plain], owner: owner, [(plain, "steward")])
        XCTAssertEqual(out.map(\.role), ["owner", "steward"])
        XCTAssertEqual(out.last?.members, [key(plain)])
    }

    /// Renderings differ across the seams — the roster is written `ed25519:…`,
    /// `objectOwner` answers bare hex, and a role row carries whatever its fold
    /// wrote (including the retired `space1` form). Comparison is on hex alone.
    func test_comparesOnHexAcrossRenderings() {
        let out = GroupDetailView.roleRuns(
            roster: [IdentityKey.display(owner), "space1" + admin, plain],
            ownerHex: owner,
            roles: [GroupRoleRow(member: admin, role: "admin")])
        XCTAssertEqual(out.map(\.role), ["owner", "admin", "member"])
    }
}
