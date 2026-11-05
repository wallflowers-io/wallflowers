//  GraphEventsSource.swift
//  The EVENTS ribbon over the REAL graph — the resolver-owned bi-temporal store, not a fixture.
//
//  Loading is async (FFI reads go off-main, like `GraphService.cachedEntities`); matching is
//  synchronous over the loaded snapshot, so the ribbon can re-filter on every gesture without
//  touching the graph. Refresh on `GraphService.epoch`.

import Foundation
import ArcResolverKit
import SwiftUI

// MARK: - Type mapping

extension EventCategory {
    /// Resolver type → filter dimension.
    ///
    /// Person and Group share a grammatical category (both can be the Agent of a
    /// predicate) but split into two selectors — who you know vs what you belong to
    /// (see `EventCategory`). Plant/Animal/Thing share `things`: one dimension per
    /// grammatical role, not per resolver type.
    static func from(resolverType t: String) -> EventCategory? {
        switch t {
        case "Event":                       return .events
        case "Person":                      return .people
        case "Group", "Org":                return .groups
        case "Place":                       return .places
        case "Project":                     return .projects
        // The market 2×2 (post-retarget) and the legacy roster both participate in
        // commitments as things — one grammatical role, whatever the shelf.
        case "Thing", "Plant", "Animal",
             "Inventory", "Skill", "Trade", "Job":
                                            return .things
        default:                            return nil
        }
    }
}

private func role(from relation: String) -> ParticipantRole {
    switch relation.lowercased() {
    case "subject", "committed_by", "commitment_of": return .subject
    case "at", "location", "located_at":             return .at
    case "about", "concerns", "regarding":           return .about
    default:                                          return .with
    }
}

// MARK: - Loading

/// A snapshot of the graph's commitments, participants resolved. Built off-main.
struct EventsSnapshot {
    var events: [EventVM] = []

    /// Read every Event node and its 1-hop participants out of the resolver.
    ///
    /// An event with no participants is DROPPED, not rendered: it is unreachable under every
    /// filter, so surfacing it would put a row on screen that no query can ever return.
    /// An event with no `validAt` is dropped too — it cannot be sequenced.
    static func load(from resolver: ArcResolver, tint: @escaping (EventCategory) -> Color) -> EventsSnapshot {
        let eventNodes = (try? resolver.entities(type: "Event")) ?? []
        var out: [EventVM] = []

        for node in eventNodes {
            let facts = (try? resolver.neighbors(id: node.id, direction: "out")) ?? []

            // The commitment's time lives on its participant FACTS, not on the Event node:
            // the resolver calls upsert_entity without a date, so an Event node's validAt is
            // always nil. Prefer the `subject` edge (the contract puts the date there), then
            // the earliest dated edge, and only then the node itself.
            let subjectDate = facts.first { role(from: $0.relation) == .subject }?.validAt
            let earliestDate = facts.compactMap(\.validAt).min()
            guard let start = subjectDate ?? earliestDate ?? node.validAt else { continue }

            var participants: [ParticipantVM] = []
            for f in facts where f.isLive {
                // `try?` FLATTENS: getEntity returns ArcEntity?, so `try? getEntity(...)` is
                // ArcEntity? (not ArcEntity??) — one unwrap, not two. A missing entity and a
                // resolver throw both collapse to nil here, and either way we skip the fact.
                guard let other = try? resolver.getEntity(id: f.dst) else { continue }
                guard let cat = EventCategory.from(resolverType: other.entityType), cat != .events else { continue }
                participants.append(
                    ParticipantVM(id: other.id,
                                  name: other.name.isEmpty ? other.label : other.name,
                                  category: cat,
                                  role: role(from: f.relation),
                                  tint: tint(cat))
                )
            }
            guard !participants.isEmpty else { continue }

            out.append(EventVM(id: node.id,
                               startMs: start,
                               endMs: node.invalidAt,
                               title: node.name.isEmpty ? node.label : node.name,
                               participants: participants))
        }
        return EventsSnapshot(events: out.sorted { $0.startMs < $1.startMs })
    }
}

// MARK: - The source

/// Matches over a loaded snapshot. Identical intersection semantics to the seeded source —
/// only where the events come from differs.
@MainActor
struct GraphEventsSource: EventsSource {
    var snapshot: EventsSnapshot

    func events(matching terms: [FilterTerm], window: EventsWindow) -> [EventVM] {
        snapshot.events
            .filter { satisfies($0, terms) && inWindow($0, window) }
            .sorted { $0.startMs < $1.startMs }
    }

    private func satisfies(_ e: EventVM, _ terms: [FilterTerm]) -> Bool {
        terms.allSatisfy { term in
            // The events dimension is satisfied by the event itself — an event is not its
            // own neighbour.
            if term.category == .events {
                return term.entityID == nil || term.entityID == e.id
            }
            return e.participants.contains { p in
                p.category == term.category && (term.entityID == nil || p.id == term.entityID)
            }
        }
    }

    private func inWindow(_ e: EventVM, _ w: EventsWindow) -> Bool {
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        switch w {
        case .upcoming:                return e.startMs >= now
        case .all:                     return true
        case .between(let s, let end): return e.startMs >= s && e.startMs <= end
        }
    }
}
