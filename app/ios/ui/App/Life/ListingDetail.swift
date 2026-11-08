//  ListingDetail.swift
//  The LISTING's and the POST's detail modals — the other two non-egoic kinds.
//
//  Three kinds, one screen. An Event, a Thing on the market and a Post are all
//  things that were MADE rather than things that ARE, so all three answer the same
//  three questions (`Design/Detail`): what does it look like, whose is it, what
//  did people say. What differs between them is only which facts print under the
//  title, which is why this file is two small binders and not two screens.
//
//  POSTS GOT A PAGE OUT OF THIS. Before, `openObject` routed `.trade` at the EVENT
//  page — a listing opened a night's page and rendered mostly empty — and `.posts`
//  fell through the `default:` arm to nothing at all, so a post card was a card you
//  could tap forever. Both now open the thing they are a card for.

import SwiftUI
import PacificCore

// MARK: - Listings (Thing)

@MainActor
enum ListingDetail {
    /// The market's own colour — the TRADE slot, so a listing's modal and its jewel
    /// cannot drift apart.
    static var tint: Color { PaletteStore.base(5) }

    static func open(_ objectId: String, core: CoreService, shell: ShellState) {
        shell.beginNew(DetailSession(noun: "listing",
                                     source: ListingObjectDetailSource(core: core,
                                                                       objectId: objectId)))
    }
}

/// The binder for a minted Thing — what somebody is offering, wanting or holding.
@MainActor
struct ListingObjectDetailSource: DetailSource {
    let core: CoreService
    let objectId: String

    private var row: ThingRow? { core.things.first { $0.id == objectId } }

    /// The listing's face, and the few facts a listing actually has. POSTURE leads,
    /// because "offers" and "wants" are opposite claims about the same object and
    /// reading the price first tells you nothing about which one you are looking at.
    func headline() async -> DetailHeadline? {
        guard let row else { return nil }
        var facts: [String] = []
        if !row.posture.isEmpty { facts.append(row.posture.uppercased()) }
        // Core only ever stores a price on an ACTIVE posture, so an empty string
        // here is a genuine absence rather than a dropped field.
        if !row.price.isEmpty { facts.append(row.price) }
        if !row.category.isEmpty { facts.append(row.category) }
        if row.deadline > 0 { facts.append(EventWhen.label(startMs: row.deadline, endMs: 0)) }
        if !row.area.isEmpty { facts.append(row.area) }
        if !row.descriptor.isEmpty { facts.append(row.descriptor) }
        return DetailHeadline(title: row.name.isEmpty ? row.descriptor : row.name,
                              photo: row.photo.isEmpty ? nil : Data(base64Encoded: row.photo),
                              facts: facts)
    }

    func author() async -> DetailAuthor? {
        DetailBind.author(of: objectId, core: core, tint: ListingDetail.tint)
    }

    func comments() async -> [DetailComment] {
        DetailBind.comments(of: objectId, core: core, tint: ListingDetail.tint)
    }

    func post(_ text: String) async -> Bool { await DetailBind.post(text, on: objectId, core: core) }
}

// MARK: - Posts

@MainActor
enum PostDetail {
    /// POSTS' own slot.
    static var tint: Color { PaletteStore.base(4) }

    static func open(_ objectId: String, core: CoreService, shell: ShellState) {
        shell.beginNew(DetailSession(noun: "post",
                                     source: PostDetailSource(core: core, objectId: objectId)))
    }
}

/// The binder for a Post — the simplest kind in the catalog, and the simplest
/// binder: a picture, the prose, and the thread under it.
@MainActor
struct PostDetailSource: DetailSource {
    let core: CoreService
    let objectId: String

    private var row: PostRow? { core.posts.first { $0.id == objectId } }

    /// A RETRACTED post shows the tombstone and NOT the words. The words are still
    /// in the log — retraction is a claim on the log, not an erasure of it — but a
    /// surface that renders them anyway would make the author's take-down a lie.
    func headline() async -> DetailHeadline? {
        guard let row else { return nil }
        if row.retracted {
            return DetailHeadline(title: row.title, facts: ["RETRACTED"])
        }
        return DetailHeadline(title: row.title,
                              photo: row.banner.isEmpty ? nil : Data(base64Encoded: row.banner),
                              facts: row.body.isEmpty ? [] : [row.body])
    }

    func author() async -> DetailAuthor? {
        DetailBind.author(of: objectId, core: core, tint: PostDetail.tint)
    }

    func comments() async -> [DetailComment] {
        DetailBind.comments(of: objectId, core: core, tint: PostDetail.tint)
    }

    func post(_ text: String) async -> Bool { await DetailBind.post(text, on: objectId, core: core) }
}

// MARK: - The shared half

/// What every non-egoic binder answers the same way. A GroupObject's author and
/// its comment log do not vary by kind — only the facts above them do — so these
/// live once rather than three times.
@MainActor
enum DetailBind {

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
    /// role (ICD `kinds.*.parts`), read flat. None until the device holds it.
    static func comments(of objectId: String, core: CoreService, tint: Color) -> [DetailComment] {
        guard let forum = core.index.commentsForum(of: objectId) else { return [] }
        let names = core.index.names
        let me = core.displayName.isEmpty ? "You" : core.displayName
        return core.objectThread(forum).map { row in
            let who = row.post.mine ? me : (names[row.post.author] ?? String(row.post.author.prefix(8)))
            return DetailComment(id: "\(row.post.author):\(row.post.gen)",
                                 author: who,
                                 initials: Avatar.initials(who),
                                 body: row.post.text,
                                 whenMs: Int64(row.post.ts),
                                 mine: row.post.mine,
                                 color: tint)
        }
    }
}
