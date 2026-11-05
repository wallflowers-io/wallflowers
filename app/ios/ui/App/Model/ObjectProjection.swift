//  ObjectProjection.swift
//  The delta→graph bridge: fold the app's minted GroupObjects into the COMPLETE snapshot
//  `GraphService.projectObjects` reconciles the graph against. Pure read + reshape — the
//  authority stays with the per-object delta logs (CoreService), the graph derives.
//
//  Kind mapping is the projector's (arc-resolver project.rs): group→Group (individual
//  shape → Person), project/thing/place/event → their types, forums/connections skipped
//  (a conversation is not an entity, and its content does not enter the graph). Members
//  ride as space ids so a person is ONE node whether they arrive via a roster or a
//  connection. An Event's venue reference rides along and lands as its happens_at edge.
//
//  SHAPE. `project` is the one entry point: it gathers the FFI reads OFF the main actor
//  (CoreService.projectionInputs — they are ~2N calls and the triggers fire on the sync
//  cadence), then `snapshots` reshapes them as a pure function. The pure half is what
//  the unit tests exercise.

import Foundation
import ArcResolverKit
import PacificCore   // ObjectRow / EventRow — the folded rows being reshaped

@MainActor
enum ObjectProjection {
    /// Gather inputs off-main, build the snapshot, hand it to the graph. `objects` is
    /// passed by the $objects trigger (a @Published publisher emits BEFORE the property
    /// is set, so reading `core.objects` there would project the list one change behind);
    /// every other caller omits it and the current list is read here.
    static func project(core: CoreService, graph: GraphService,
                        objects: [ObjectRow]? = nil) async {
        let objs = objects ?? core.objects
        let inputs = await core.projectionInputs(for: objs)
        await graph.projectObjects(snapshots(objects: objs, events: core.events,
                                             inputs: inputs))
    }

    /// Every minted object worth a LIFE node, with rosters, names, and affiliations.
    /// Pure — no FFI, no actor state; safe from any context.
    nonisolated static func snapshots(objects: [ObjectRow], events: [EventRow],
                                      inputs: CoreService.ProjectionInputs) -> [ArcObjectSnapshot] {
        // identity → display name, for labelling roster members. KEYED AND QUERIED
        // BY `IdentityKey.hex` — a connection's `peerId` and a roster entry reach
        // us in whichever rendering their side speaks, and keying this map by a
        // rendering meant a member whose id arrived differently silently lost its
        // name (caught 12 Aug by the rename). Compare the bytes, never the text.
        var names: [String: String] = [:]
        for c in inputs.connections where !c.displayName.isEmpty {
            names[IdentityKey.hex(c.peerId)] = c.displayName
        }
        func members(of objectId: String) -> [(space: String, name: String?)] {
            (inputs.members[objectId] ?? []).map { hex in
                (IdentityKey.display(hex), names[IdentityKey.hex(hex)])
            }
        }
        // Events by id in one pass — the old per-event scan was O(objects × events).
        let eventRows = Dictionary(events.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })

        var out: [ArcObjectSnapshot] = []
        for o in objects {
            switch o.kind {
            case "group":
                let view = inputs.groups[o.id]
                let shape = view?.shape
                let name = (view?.displayName.isEmpty == false ? view?.displayName : nil) ?? o.name
                out.append(ArcObjectSnapshot(
                    id: o.id,
                    kind: o.kind,
                    name: name,
                    shape: shape,
                    // An identity record's roster is just its owner — a person is not
                    // a member of themselves, so individual groups carry no roster.
                    members: shape == "individual" ? [] : members(of: o.id),
                    affiliations: (view?.affiliations ?? []).map {
                        (peer: $0.peer, rel: $0.rel, name: $0.name, tether: $0.tether, at: $0.at)
                    }
                ))
            case "project", "thing", "place":
                out.append(ArcObjectSnapshot(id: o.id, kind: o.kind, name: o.name,
                                             members: members(of: o.id)))
            case "event":
                // The folded EventRow carries the venue reference; the object list
                // only knows id/kind/name.
                let row = eventRows[o.id]
                let venue: (place: String, name: String, at: Int64)? = row.flatMap {
                    $0.venuePlace.isEmpty ? nil : ($0.venuePlace, $0.venuePlaceName, $0.venueAt)
                }
                out.append(ArcObjectSnapshot(id: o.id, kind: o.kind,
                                             name: (row?.title.isEmpty == false ? row?.title : nil) ?? o.name,
                                             members: members(of: o.id),
                                             venue: venue))
            default:
                continue
            }
        }
        return out
    }
}
