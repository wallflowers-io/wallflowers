//  PostDetail.swift
//  The POST's maximised detail: its picture, its prose, the document it carries,
//  and the thread under it.
//
//  A RETRACTED post shows the tombstone and NOT the words. The words are still in
//  the log — retraction is a claim on the log, not an erasure of it — but a page
//  that renders them anyway would make the author's take-down a lie. Nothing on a
//  tombstone is writable.

import SwiftUI
import PacificCore

@MainActor
enum PostDetail {
    /// POSTS' own slot.
    static var tint: Color { PaletteStore.base(4) }

    static func open(_ objectId: String, core: CoreService, shell: ShellState, origin: CGRect? = nil) {
        shell.beginNew(DetailSession(noun: "post",
                                     source: PostDetailSource(core: core, objectId: objectId),
                                     origin: origin))
    }
}

@MainActor
struct PostDetailSource: DetailSource {
    let core: CoreService
    let objectId: String

    private var row: PostRow? { core.posts.first { $0.id == objectId } }

    func headline() async -> DetailHeadline? {
        guard let row else { return nil }
        if row.retracted {
            return DetailHeadline(title: row.title, rows: [DetailRow(label: "Retracted", value: "RETRACTED")])
        }
        let more = core.postDetail(objectId)
        let owner = core.isGroupOwner(objectId)
        let profile = "post.setProfile"
        let r = DetailBind.rows(owner: owner, core: core)
        let markdown = more?.bodyFormat == "markdown"

        var rows = [
            r.text("Excerpt", more?.excerpt ?? "", op: profile, arg: "excerpt"),
            r.text("Body", row.body, op: profile, arg: "body", input: .multiline, markdown: markdown),
            r.link("Links to", more?.link ?? "", op: profile, arg: "link"),
        ]
        if owner {
            rows.append(r.choice("Kind", more?.form ?? "", op: profile, arg: "form"))
            rows.append(r.choice("Body format", more?.bodyFormat ?? "", op: profile, arg: "bodyFormat"))
        }
        if row.reactions > 0 {
            rows.append(DetailRow(label: "Reactions", value: "\(row.reactions) reactions"))
        }

        let document = more?.document.flatMap { d in
            Data(base64Encoded: d.data).map { DetailDocument(name: d.alt, data: $0) }
        }
        return DetailHeadline(
            title: row.title,
            titleEdit: owner ? DetailEdit(op: profile, arg: "title", input: .line, current: row.title) : nil,
            photo: row.banner.isEmpty ? nil : Data(base64Encoded: row.banner),
            gallery: DetailBind.pictures(more?.assets ?? []),
            document: document,
            rows: rows,
            adds: owner ? [.picture, .document] : [])
    }

    func author() async -> DetailAuthor? {
        DetailBind.author(of: objectId, core: core, tint: PostDetail.tint)
    }

    func comments() async -> [DetailComment] {
        DetailBind.comments(of: objectId, core: core, tint: PostDetail.tint)
    }

    func post(_ text: String) async -> Bool { await DetailBind.post(text, on: objectId, core: core) }

    func edit(_ edit: DetailEdit, to value: String) async -> Bool {
        await DetailBind.edit(edit, to: value, on: objectId, core: core)
    }

    func addPicture(_ data: Data) async -> Bool {
        await core.addPicture(objectId, op: "post.addAsset", arg: "asset", data, keyed: true)
    }

    /// The document rides inline, under the ICD's ceiling for it, or not at all.
    func setDocument(_ data: Data, name: String) async -> Bool {
        let b64 = data.base64EncodedString()
        guard b64.count <= core.ceiling("post.setDocument", "document") else {
            core.fail("attach the document", DetailRefusal.tooLarge, object: objectId)
            return false
        }
        return await core.apply(objectId, op: "post.setDocument",
                                ["document": .text(v: b64), "documentMime": .text(v: "application/pdf"),
                                 "name": .text(v: String(name.prefix(core.ceiling("post.setDocument", "name"))))])
    }
}

enum DetailRefusal: LocalizedError {
    case tooLarge
    var errorDescription: String? { "it is larger than the ICD's ceiling for it" }
}
