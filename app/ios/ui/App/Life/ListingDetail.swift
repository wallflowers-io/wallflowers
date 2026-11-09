//  ListingDetail.swift
//  The LISTING's maximised detail — a Thing on the market — and the half every
//  non-egoic binder shares (`DetailBind`).
//
//  An Event, a Thing and a Post are things that were MADE rather than things that
//  ARE, so all three answer the same questions (`Design/Detail`): what does it look
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
        shell.beginNew(DetailSession(noun: "listing",
                                     source: ListingObjectDetailSource(core: core, objectId: objectId),
                                     origin: origin))
    }
}

/// The binder for a held Thing — what somebody is offering, wanting or holding.
@MainActor
struct ListingObjectDetailSource: DetailSource {
    let core: CoreService
    let objectId: String

    private var row: ThingRow? { core.things.first { $0.id == objectId } }

    func headline() async -> DetailHeadline? {
        guard let row else { return nil }
        let more = core.thingDetail(objectId)
        let owner = core.isGroupOwner(objectId)
        let profile = "thing.setProfile"
        let r = DetailBind.rows(owner: owner, core: core)

        var market = [row.posture.uppercased(), row.price].filter { !$0.isEmpty }
        if row.deadline > 0 { market.append(EventWhen.label(startMs: row.deadline, endMs: 0)) }
        var rows = [DetailRow(label: "Market", value: market.joined(separator: " · "))]
        rows.append(r.choice("Disposition", more?.disposition ?? "", op: "thing.setDisposition", arg: "state"))
        rows.append(r.choice("Kind", row.category, op: profile, arg: "category"))
        rows.append(r.choice("Condition", more?.condition ?? "", op: profile, arg: "condition"))
        rows.append(r.text("Describe", row.descriptor, op: profile, arg: "descriptor", input: .multiline))
        rows.append(r.text("Description", more?.description ?? "", op: profile, arg: "description", input: .multiline))
        rows.append(DetailRow(label: "Area", value: row.area))

        return DetailHeadline(
            title: row.name.isEmpty ? row.descriptor : row.name,
            titleEdit: owner ? DetailEdit(op: profile, arg: "name", input: .line, current: row.name) : nil,
            photo: row.photo.isEmpty ? nil : Data(base64Encoded: row.photo),
            gallery: DetailBind.pictures(more?.photos ?? []),
            rows: rows,
            adds: owner ? [.picture] : [])
    }

    func author() async -> DetailAuthor? {
        DetailBind.author(of: objectId, core: core, tint: ListingDetail.tint)
    }

    func comments() async -> [DetailComment] { DetailBind.comments(of: objectId, core: core) }

    func commentsHeld() async -> Bool { core.index.commentsForum(of: objectId) != nil }

    func post(_ text: String) async -> Bool { await DetailBind.post(text, on: objectId, core: core) }

    /// A disposition is its own op, stamped with when it was said; the profile's rows
    /// restate the profile.
    func edit(_ edit: DetailEdit, to value: String) async -> Bool {
        guard edit.op == "thing.setDisposition" else {
            return await DetailBind.edit(edit, to: value, on: objectId, core: core)
        }
        guard !value.isEmpty else { return false }
        return await core.apply(objectId, op: edit.op,
                                ["state": .text(v: value), "at": .int(v: DetailBind.nowMs)])
    }

    func addPicture(_ data: Data) async -> Bool {
        await core.addPicture(objectId, op: "thing.addPhoto", arg: "photo", data, keyed: true)
    }
}

// MARK: - The shared half

/// What every non-egoic binder answers the same way. A GroupObject's author, its
/// comment log and the way its owner writes a row do not vary by kind — only the
/// rows do — so these live once rather than once per kind.
@MainActor
enum DetailBind {

    static var nowMs: Int64 { Int64(Date().timeIntervalSince1970 * 1000) }

    /// A comment, written to the object's comments section. False when the device
    /// does not hold that section, or is not on its roster.
    static func post(_ text: String, on objectId: String, core: CoreService) async -> Bool {
        guard let forum = core.index.commentsForum(of: objectId) else { return false }
        return await core.sendToGroup(forum, text)
    }

    /// Whose it is, from the `created` edge on the creator's own log. A Space when
    /// a Space published it, a person when a person did.
    static func author(of objectId: String, core: CoreService, tint: Color) -> DetailAuthor? {
        guard let creator = core.creatorOfObject(objectId) else { return nil }
        let name = creator.name.isEmpty ? String(creator.id.prefix(8)) : creator.name
        let isSpace = core.spaces.contains { $0.id == creator.id }
        return DetailAuthor(id: creator.id, name: name, initials: Avatar.initials(name),
                            isSpace: isSpace, color: tint)
    }

    /// The object's comments section: the Forum minted with it in the comments
    /// role (ICD `kinds.*.parts`), read flat. None until the device holds it. Each
    /// comment wears its author's own face from the index — their photo, else their
    /// colour, else the colour their key derives — never the kind's.
    static func comments(of objectId: String, core: CoreService) -> [DetailComment] {
        guard let forum = core.index.commentsForum(of: objectId) else { return [] }
        let me = core.displayName.isEmpty ? "You" : core.displayName
        return core.objectThread(forum).map { row in
            let id = row.post.mine ? core.myIdentityHex : row.post.author
            let who = row.post.mine ? me : (core.index.name(id) ?? String(id.prefix(8)))
            return DetailComment(id: "\(row.post.author):\(row.post.gen)",
                                 authorID: id,
                                 author: who,
                                 initials: Avatar.initials(who),
                                 body: row.post.text,
                                 whenMs: Int64(row.post.ts),
                                 mine: row.post.mine,
                                 photo: core.index.photo(id),
                                 color: core.index.color(id)
                                     ?? Color(hex: CoreService.derivedAvatarColorHex(forIdentityKey: id)))
        }
    }

    /// An owner's row, through its whole-replace op (`CoreService.edit`): the held
    /// args restated with this one changed. A date and a switch cross as the integers
    /// the ICD types them; an emptied value clears its arg.
    static func edit(_ e: DetailEdit, to value: String, on objectId: String, core: CoreService) async -> Bool {
        let v: OpValue
        switch e.input {
        case .date, .toggle:
            v = value.isEmpty ? .text(v: "") : .int(v: Int64(value) ?? 0)
        default:
            v = .text(v: value)
        }
        return await core.edit(objectId, op: e.op, [e.arg: v])
    }

    static func pictures(_ media: [MediaRow]) -> [DetailPicture] {
        media.enumerated().compactMap { i, m in
            Data(base64Encoded: m.data).map { DetailPicture(id: i, data: $0, alt: m.alt) }
        }
    }

    /// Builds rows that are writable for the owner and values for everyone else.
    static func rows(owner: Bool, core: CoreService) -> RowMaker { RowMaker(owner: owner, core: core) }

    @MainActor
    struct RowMaker {
        let owner: Bool
        let core: CoreService

        func text(_ label: String, _ value: String, op: String, arg: String,
                  input: DetailInput = .line, link: URL? = nil, markdown: Bool = false) -> DetailRow {
            DetailRow(label: label, value: value, link: link, markdown: markdown,
                      edit: owner ? DetailEdit(op: op, arg: arg, input: input, current: value) : nil)
        }

        /// A closed vocabulary, offered in the ICD's words for this op's arg.
        func choice(_ label: String, _ value: String, op: String, arg: String) -> DetailRow {
            DetailRow(label: label, value: value,
                      edit: owner ? DetailEdit(op: op, arg: arg, input: .choice(core.vocabulary(op, arg)),
                                               current: value) : nil)
        }

        /// A web address the owner writes and a viewer opens.
        func link(_ label: String, _ value: String, op: String, arg: String) -> DetailRow {
            text(label, value, op: op, arg: arg, link: URL(string: value).flatMap { $0.scheme == nil ? nil : $0 })
        }
    }
}
