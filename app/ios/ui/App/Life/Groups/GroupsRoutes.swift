//  GroupsRoutes.swift
//  The Groups surface's load-bearing route types. They outlive GroupsPage (the
//  old flat list, superseded by the LIFE rail's TypedListPage ground) because
//  every group navigation pushes by VALUE through them: Life.swift registers
//  the destinations, CHAT's room cover deep-links through `GroupLink`, and the
//  Space profile pushes rooms through `GroupForumLink`.

import SwiftUI

/// Value-based route to a group's page. Typed (not a bare String) so it can
/// never collide with the String route that opens a Project.
struct GroupLink: Hashable {
    let id: String
}

/// Value-based route into one of a group's rooms.
struct GroupForumLink: Hashable {
    let id: String
    let name: String
}
