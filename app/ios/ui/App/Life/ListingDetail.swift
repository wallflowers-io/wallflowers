//  ListingDetail.swift
//  The LISTING's maximised detail — a Thing on the market — and the half every
//  non-egoic binder shares (`KindBind`).
//
//  An Event, a Thing and a Post are things that were MADE rather than things that
//  ARE, so all three answer the same questions (`Design/Kind/`): what does it look
//  like, what are its values, whose is it, what did people say. What differs is
//  only which rows a binder answers.
//
//  POSTURE LEADS, because "offers" and "wants" are opposite claims about the same
//  object, and reading the price first tells you nothing about which one you are
//  looking at. The posture is the market's to write (ListingEditor); this page
//  writes the Thing's own profile, its pictures and whether it is still for sale.

import SwiftUI
import PacificCore

// MARK: - Listings (Thing)

@MainActor
enum ListingDetail {
    /// The market's own colour — the TRADE slot, so a listing's page and its jewel
    /// cannot drift apart.
    static var tint: Color { PaletteStore.base(5) }

    static func open(_ objectId: String, core: CoreService, shell: ShellState, origin: CGRect? = nil) {
        shell.beginNew(KindSession(noun: "listing",
                                     source: ListingObjectDetailSource(core: core, objectId: objectId),
                                     origin: origin))
    }
}

/// The binder for a held Thing — what somebody is offering, wanting or holding.
@MainActor
struct ListingObjectDetailSource: KindSource {
    let core: CoreService
    let objectId: String

    private var row: ThingRow? { core.things.first { $0.id == objectId } }

    func canEdit() async -> Bool { core.isGroupOwner(objectId) }

    func headline() async -> KindHeadline? {
        guard let row else { return nil }
        let more = core.thingDetail(objectId)
        let owner = core.isGroupOwner(objectId)
        let profile = "thing.setProfile"
        let r = KindBind.rows(owner: owner, core: core)

        var market = [row.posture.uppercased(), row.price].filter { !$0.isEmpty }
        if row.deadline > 0 { market.append(EventWhen.label(startMs: row.deadline, endMs: 0)) }
        var rows = [KindRow(label: "Market", value: market.joined(separator: " · "))]
        rows.append(r.choice("Disposition", more?.disposition ?? "", op: "thing.setDisposition", arg: "state"))
        rows.append(r.choice("Kind", row.category, op: profile, arg: "category"))
        rows.append(r.choice("Condition", more?.condition ?? "", op: profile, arg: "condition"))
        rows.append(r.text("Describe", row.descriptor, op: profile, arg: "descriptor", input: .multiline))
        rows.append(r.text("Description", more?.description ?? "", op: profile, arg: "description", input: .multiline))
        rows.append(KindRow(label: "Area", value: row.area))

        return KindHeadline(
            title: row.name.isEmpty ? row.descriptor : row.name,
            titleEdit: owner ? KindEdit(op: profile, arg: "name", input: .line, current: row.name) : nil,
            photo: row.photo.isEmpty ? nil : Data(base64Encoded: row.photo),
            gallery: KindBind.pictures(more?.photos ?? []),
            rows: rows,
            adds: owner ? [.picture] : [])
    }

    func author() async -> KindAuthor? {
        KindBind.author(of: objectId, core: core, tint: ListingDetail.tint)
    }

    func comments() async -> [KindComment] { KindBind.comments(of: objectId, core: core) }

    func commentsHeld() async -> Bool { core.index.commentsForum(of: objectId) != nil }

    func post(_ text: String) async -> Bool { await KindBind.post(text, on: objectId, core: core) }

    /// A disposition is its own op, stamped with when it was said; the profile's rows
    /// restate the profile.
    func edit(_ edit: KindEdit, to value: String) async -> Bool {
        guard edit.op == "thing.setDisposition" else {
            return await KindBind.edit(edit, to: value, on: objectId, core: core)
        }
        guard !value.isEmpty else { return false }
        return await core.apply(objectId, op: edit.op,
                                ["state": .text(v: value), "at": .int(v: KindBind.nowMs)])
    }

    func addPicture(_ data: Data) async -> Bool {
        await core.addPicture(objectId, op: "thing.addPhoto", arg: "photo", data, keyed: true)
    }
}

