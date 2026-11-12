//  KindExterior.swift
//  AN OBJECT I AM NOT IN, seen from outside (#76; Ralph, 12 Nov). The abstract other side of
//  KindInterior: the same page, whole and read-only, because the outside is complete and
//  joining adds only authorship. Its one act is the ask: Attend for an Event (D9), Ask to
//  join for everything else. Joining is a follow request; a decline is not told back.
//
//  What it reads is the object's hosted outside (`media.hosted`), fetched by core and checked
//  against its owner, else the carried exterior's own words.

import SwiftUI
import PacificCore

/// A page for an object whose exterior reached me. Each kind with an outside of its own
/// conforms; `KindExteriorBase` is every other kind.
@MainActor
protocol KindExterior: KindSource {
    var row: ExteriorRow { get }
    var core: CoreService { get }
}

extension KindExterior {
    /// Whose it is: the carried exterior's owner, the one key a stranger can check (D16).
    func author() async -> KindAuthor? {
        let name = core.index.name(row.owner) ?? String(row.owner.prefix(8))
        return KindAuthor(id: row.owner, name: name, initials: Avatar.initials(name), isSpace: false,
                          photo: core.index.photo(row.owner))
    }

    /// Outside there is no log to read or speak into.
    func comments() async -> [KindComment] { [] }
    func post(_ text: String) async -> Bool { false }
}

@MainActor
enum KindExteriors {
    /// Open the exterior of an object whose copy reached me, grown from its card. Nothing opens
    /// for one that has gone, or that has no way in.
    static func open(_ objectId: String, core: CoreService, shell: ShellState, origin: CGRect? = nil) {
        guard let row = core.exteriors.first(where: { $0.object == objectId }), row.door else { return }
        let source: any KindSource = row.kind == "event"
            ? EventExterior(row: row, core: core)
            : KindExteriorBase(row: row, core: core)
        shell.beginNew(KindSession(noun: row.kind, source: source, primary: ask(row, core: core), origin: origin))
    }

    /// BLUE: the ask, or its withdrawal while it stands.
    static func ask(_ row: ExteriorRow, core: CoreService) -> KindPrimary {
        let id = row.object
        if row.requested {
            return .action(label: "Requested") { await core.withdrawRequest(id) ? .saved : nil }
        }
        return .action(label: row.kind == "event" ? "Attend" : "Ask to join") { await core.requestJoin(id) ? .saved : nil }
    }

    /// An outside's fields, parsed once.
    nonisolated static func fields(_ outside: OutsideRow?) -> [String: Any] {
        guard let json = outside?.json.data(using: .utf8),
              let f = try? JSONSerialization.jsonObject(with: json) as? [String: Any] else { return [:] }
        return f
    }
}

/// Every kind without an outside of its own: its name, its words and its picture.
@MainActor
struct KindExteriorBase: KindExterior {
    let row: ExteriorRow
    let core: CoreService

    func headline() async -> KindHeadline? {
        let outside = row.hosted ? await core.refreshOutside(row.object) : nil
        let f = KindExteriors.fields(outside)
        let words = (f["description"] as? String) ?? row.description
        let picture = outside?.media.first { $0.field == "banner" } ?? outside?.media.first { $0.field == "photo" }
        let photo: Data? = if let picture { await core.outsideData(picture) } else { nil }
        return KindHeadline(title: (f["title"] as? String) ?? row.name, photo: photo,
                            rows: [KindRow(label: "Description", value: words)])
    }
}

/// An EVENT from outside: the member's page, whole, read-only (BLUE_INTERIOR's EventContent
/// and its blocks, filled from the hosted fields). Before its owner hosts one, its words.
@MainActor
struct EventExterior: KindExterior {
    let row: ExteriorRow
    let core: CoreService

    func headline() async -> KindHeadline? {
        guard row.hosted, let outside = await core.refreshOutside(row.object),
              let c = await EventContent.outside(outside, core: core) else {
            return await KindExteriorBase(row: row, core: core).headline()
        }
        return c.page(objectId: row.object, editor: false, owner: false, core: core,
                      setVenue: { _, _ in false }, setLineup: { _ in false })
    }
}
