//  KindBind.swift
//
//  The shared half of every non-egoic binder.

import SwiftUI
import PacificCore

/// What every non-egoic binder answers the same way. A GroupObject's author, its
/// comment log and the way its owner writes a row do not vary by kind — only the
/// rows do — so these live once rather than once per kind.
@MainActor
enum KindBind {

    static var nowMs: Int64 { Int64(Date().timeIntervalSince1970 * 1000) }

    /// A comment, written to the object's comments section. False when the device
    /// does not hold that section, or is not on its roster.
    static func post(_ text: String, on objectId: String, core: CoreService) async -> Bool {
        guard let forum = core.index.commentsForum(of: objectId) else { return false }
        return await core.sendToGroup(forum, text)
    }

    /// Whose it is, from the `created` edge on the creator's own log. A Space when
    /// a Space published it, a person when a person did.
    static func author(of objectId: String, core: CoreService, tint: Color) -> KindAuthor? {
        guard let creator = core.creatorOfObject(objectId) else { return nil }
        let name = creator.name.isEmpty ? String(creator.id.prefix(8)) : creator.name
        let isSpace = core.spaces.contains { $0.id == creator.id }
        return KindAuthor(id: creator.id, name: name, initials: Avatar.initials(name),
                            isSpace: isSpace, color: tint)
    }

    /// The object's comments section: the Forum minted with it in the comments
    /// role (ICD `kinds.*.parts`), read flat. None until the device holds it. Each
    /// comment wears its author's own face from the index — their photo, else their
    /// colour, else the colour their key derives — never the kind's.
    static func comments(of objectId: String, core: CoreService) -> [KindComment] {
        guard let forum = core.index.commentsForum(of: objectId) else { return [] }
        let me = core.displayName.isEmpty ? "You" : core.displayName
        return core.objectThread(forum).map { row in
            let id = row.post.mine ? core.myIdentityHex : row.post.author
            let who = row.post.mine ? me : (core.index.name(id) ?? String(id.prefix(8)))
            return KindComment(id: "\(row.post.author):\(row.post.gen)",
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
    static func edit(_ e: KindEdit, to value: String, on objectId: String, core: CoreService) async -> Bool {
        let v: OpValue
        switch e.input {
        case .date, .toggle:
            v = value.isEmpty ? .text(v: "") : .int(v: Int64(value) ?? 0)
        default:
            v = .text(v: value)
        }
        return await core.edit(objectId, op: e.op, [e.arg: v])
    }

    static func pictures(_ media: [MediaRow]) -> [KindPicture] {
        media.enumerated().compactMap { i, m in
            Data(base64Encoded: m.data).map { KindPicture(id: i, data: $0, alt: m.alt) }
        }
    }

    /// Builds rows that are writable for the owner and values for everyone else.
    static func rows(owner: Bool, core: CoreService) -> RowMaker { RowMaker(owner: owner, core: core) }

    @MainActor
    struct RowMaker {
        let owner: Bool
        let core: CoreService

        func text(_ label: String, _ value: String, op: String, arg: String,
                  input: KindInput = .line, link: URL? = nil, markdown: Bool = false) -> KindRow {
            KindRow(label: label, value: value, link: link, markdown: markdown,
                      edit: owner ? KindEdit(op: op, arg: arg, input: input, current: value) : nil)
        }

        /// A closed vocabulary, offered in the ICD's words for this op's arg.
        func choice(_ label: String, _ value: String, op: String, arg: String) -> KindRow {
            KindRow(label: label, value: value,
                      edit: owner ? KindEdit(op: op, arg: arg, input: .choice(core.vocabulary(op, arg)),
                                               current: value) : nil)
        }

        /// A web address the owner writes and a viewer opens.
        func link(_ label: String, _ value: String, op: String, arg: String) -> KindRow {
            text(label, value, op: op, arg: arg, link: URL(string: value).flatMap { $0.scheme == nil ? nil : $0 })
        }
    }
}
