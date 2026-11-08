//  Offices.swift
//  An office is an APPOINTMENT — a label over a real `group.setMemberRole` — and it
//  outlived the wizard that used to present it.
//
//  Extracted when OrgWizard was purged (18 Aug). What survives here is the MODEL and its
//  store; the office BOARD and holder picker went with the wizard, because the only way
//  to reach them was through it.
//
//  The live caller is Connect/DevLinks.swift. If offices are to have a surface again it
//  belongs on the Space's own page, next to the roster it overlays — which is where the
//  wizard's own header said it should have been.

import Foundation
import SwiftUI

/// An office in a constituted community body. The committee vocabulary of an English
/// unincorporated association (Chair · Treasurer · Secretary), which is what almost every
/// allotment/garden association actually is.
///
/// NOTE ON THE LEGAL FRAME: minting this incorporates nothing. It mirrors a governance
/// structure; it does not create a CIC, a CIO or a registered charity. Copy in the UI
/// must never imply otherwise. (The disclaimer text went with OrgWizard; whatever
/// surfaces offices next owes the user the same sentence.)
enum Office: String, CaseIterable, Identifiable, Codable {
    case chair, treasurer, secretary

    var id: String { rawValue }

    var title: String {
        switch self {
        case .chair:     return "Chair"
        case .treasurer: return "Treasurer"
        case .secretary: return "Secretary"
        }
    }

    var glyph: String {
        switch self {
        case .chair:     return "person.bust"
        case .treasurer: return "banknote"
        case .secretary: return "text.book.closed"
        }
    }

    /// What the office may do, in the member's own words. This is the constitution
    /// rendered as UI — the same sentences a paper constitution would carry.
    var mandate: String {
        switch self {
        case .chair:
            return "Opens and closes votes. Holds the succession pin, so the body outlives its founder."
        case .treasurer:
            return "The only member who may bind the finance connector or author a spend."
        case .secretary:
            return "Keeps the members' register and authors the minutes."
        }
    }

    /// The CORE role this office's authority maps onto (`group.setMemberRole` arg "role").
    /// Core knows owner|admin|member|viewer|guest and nothing about committees, so the
    /// office is the label and this is the power.
    var coreRole: String {
        switch self {
        case .chair:                 return "owner"
        case .treasurer, .secretary: return "admin"
        }
    }
}

/// The three states an office is ever in, from the holder's point of view. Everything
/// underneath — roster adds, prekey consumption, epoch commits, role deltas — is
/// sequencing WE own, not vocabulary the user should ever meet.
enum OfficeStatus: String, Codable {
    case vacant     // nobody asked
    case pending    // asked; the roster add and/or the role delta has not landed yet
    case accepted   // on the roster AND the setMemberRole delta folded

    var label: String {
        switch self {
        case .vacant:   return "Invite"
        case .pending:  return "Pending"
        case .accepted: return "Accepted"
        }
    }
}

@MainActor
enum OrgOffices {
    /// Read the appointments off the Group's own fold. There is no local dictionary
    /// any more: an office and its holder are `group.setOffice` deltas, so they sync
    /// to every member, survive the phone, and reach the graph as `holds_office`.
    static func holders(_ objectId: String, core: CoreService) -> [String: String] {
        var out: [String: String] = [:]
        for o in core.orgView(objectId)?.offices ?? [] {
            out[o.office] = IdentityKey.display(o.holder)
        }
        return out
    }

    static func status(_ objectId: String, office: Office, core: CoreService) -> OfficeStatus {
        guard let row = (core.orgView(objectId)?.offices ?? [])
            .first(where: { $0.office == office.rawValue }) else { return .vacant }
        return OfficeStatus(rawValue: row.status) ?? .vacant
    }

    static func holder(_ objectId: String, office: Office, core: CoreService) -> String? {
        (core.orgView(objectId)?.offices ?? [])
            .first { $0.office == office.rawValue }
            .map { IdentityKey.display($0.holder) }
    }

    /// Appoint. Owner-only and owner-sequenced, so this returns false for anyone else
    /// rather than writing a local truth nobody would ever receive.
    @discardableResult
    static func setHolder(_ objectId: String, office: Office, space: String,
                          status: OfficeStatus = .accepted,
                          core: CoreService) async -> Bool {
        await core.setOffice(objectId, office: office.rawValue,
                             holderSpace: space, status: status.rawValue)
    }

    @discardableResult
    static func clear(_ objectId: String, office: Office, core: CoreService) async -> Bool {
        await core.clearOffice(objectId, office: office.rawValue)
    }

    // MARK: - One-time migration off UserDefaults
    //
    // Appointments made before offices were deltas live in `pacific.org.offices.<id>`
    // on whichever device made them. They can only be RE-AUTHORED by the owner, so
    // this runs on the owner's device, replays each into the log, and drops the local
    // keys. An appointment made on a phone that is not to hand cannot be recovered —
    // that is the cost of it never having been a delta, and it is why this exists.

    private static func legacyKey(_ id: String) -> String { "pacific.org.offices.\(id)" }
    private static func legacyStatusKey(_ id: String) -> String { "pacific.org.status.\(id)" }
    private static func legacyTypeKey(_ id: String) -> String { "pacific.org.type.\(id)" }

    /// Replay any device-local appointments for `objectId` into its log. Idempotent:
    /// the keys are removed only once every office has been authored, so a failure
    /// part-way leaves the rest to retry rather than losing them.
    static func migrateIfNeeded(_ objectId: String, core: CoreService) async {
        let d = UserDefaults.standard
        guard core.isGroupOwner(objectId) else { return }

        // The org TYPE became a real shape (`community`), so it moves to setProfile.
        if let legacyType = d.string(forKey: legacyTypeKey(objectId)),
           let view = core.orgView(objectId),
           legacyType == "community", view.shape != "community" {
            _ = await core.setOrgShape(objectId, shape: "community")
            d.removeObject(forKey: legacyTypeKey(objectId))
        }

        guard let local = d.dictionary(forKey: legacyKey(objectId)) as? [String: String],
              !local.isEmpty else { return }
        let statuses = (d.dictionary(forKey: legacyStatusKey(objectId)) as? [String: String]) ?? [:]

        var authored = 0
        for (office, space) in local {
            let status = OfficeStatus(rawValue: statuses[office] ?? "") ?? .accepted
            if await core.setOffice(objectId, office: office, holderSpace: space,
                                    status: status == .vacant ? "pending" : status.rawValue) {
                authored += 1
            }
        }
        if authored == local.count {
            d.removeObject(forKey: legacyKey(objectId))
            d.removeObject(forKey: legacyStatusKey(objectId))
            NSLog("[offices] migrated %d appointment(s) for %@ into the log", authored, objectId)
        }
    }
}
