//  ExteriorDetail.swift
//  An object I am not in, seen from outside (#76, ICD 2.6.0): its picture, its words, whose
//  it is, and the one act there is — ask to join. Joining is a follow request: the owner is
//  told, and may let me in, or not; a decline is not told back.

import SwiftUI
import PacificCore

@MainActor
enum ExteriorDetail {
    /// Open the exterior of an object whose copy reached me. Nothing opens for one that has
    /// gone, or that has no door to ask at.
    static func open(_ objectId: String, core: CoreService, shell: ShellState) {
        guard let row = core.exteriors.first(where: { $0.object == objectId }), row.door else { return }
        let name = core.index.name(row.owner) ?? String(row.owner.prefix(8))
        let source = ExteriorDetailSource(row: row, ownerName: name, ownerPhoto: core.index.photo(row.owner))
        let primary: DetailPrimary = row.requested
            ? .action(label: "Requested") { await core.withdrawRequest(objectId) ? .saved : nil }
            : .action(label: "Ask to join") { await core.requestJoin(objectId) ? .saved : nil }
        shell.beginNew(DetailSession(noun: "object", source: source, primary: primary))
    }
}

/// The binder for an exterior: what its owner put outside, and nothing from inside. There is
/// no log here to read or speak into.
@MainActor
struct ExteriorDetailSource: DetailSource {
    let row: ExteriorRow
    let ownerName: String
    let ownerPhoto: Data?

    func headline() async -> DetailHeadline? {
        DetailHeadline(title: row.name, rows: [DetailRow(label: "Description", value: row.description)])
    }

    func author() async -> DetailAuthor? {
        DetailAuthor(id: row.owner, name: ownerName, initials: Avatar.initials(ownerName), isSpace: false, photo: ownerPhoto)
    }

    func comments() async -> [DetailComment] { [] }
    func post(_ text: String) async -> Bool { false }
}
