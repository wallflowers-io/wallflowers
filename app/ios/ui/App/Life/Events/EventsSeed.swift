//  EventsSeed.swift
//  Seed data for developing the filter and the ribbon before the resolver feeds them.
//
//  SEED, NOT A MILESTONE. This exists so the UI can be built and previewed; a run against
//  it proves the filter algebra and the layout, and proves nothing about extraction.
//
//  `SeedEventsSource` performs the REAL 1-hop intersection over the seeded participants
//  rather than returning canned rows — so the matching semantics are exercised here and
//  the graph-backed source later only has to swap where the events come from.

import SwiftUI

// MARK: - Seed world

enum EventsSeed {
    /// Category tints for the seed. The real ribbon takes these from the palette.
    static func tint(_ c: EventCategory) -> Color {
        switch c {
        case .events:   return .orange
        case .people:   return .blue
        case .groups:   return .indigo
        case .places:   return .green
        case .projects: return .pink
        case .things:   return .teal
        case .trade:    return .teal
        case .job:      return .brown
        // Posts stay chrome-quiet: a neutral warm gray, not a competing hue.
        case .posts:    return Color(red: 0.62, green: 0.58, blue: 0.54)
        // Channels sit beside posts and stay as quiet: the same neutral, cooled.
        case .channels: return Color(red: 0.55, green: 0.59, blue: 0.62)
        }
    }

    // The cast. Ids are stable so filter terms can bind to them.
    static let hope    = p("hope",    "Hope",           .people)
    static let ralph   = p("ralph",   "Ralph",          .people)
    static let alice   = p("alice",   "Alice",          .people)
    static let bristol = p("bristol", "Bristol",        .places)
    static let gallery = p("gallery", "The Gallery",    .places)
    static let studio  = p("studio",  "Studio",         .places)
    static let car     = p("car",     "The car",        .things)
    static let bike    = p("bike",    "The bike",       .things)
    // A collective — same grammatical category as the individuals, its own selector.
    static let choir   = p("choir",   "Thursday choir", .groups)
    static let northwind = p("northwind", "Northwind",  .projects)

    private static func p(_ id: String, _ name: String, _ c: EventCategory,
                          _ role: ParticipantRole = .with) -> ParticipantVM {
        ParticipantVM(id: id, name: name, category: c, role: role, tint: tint(c))
    }

    /// Re-role a cast member for one commitment — the same entity is a subject in one
    /// event and merely present in another.
    private static func r(_ base: ParticipantVM, _ role: ParticipantRole) -> ParticipantVM {
        ParticipantVM(id: base.id, name: base.name, category: base.category,
                      role: role, tint: base.tint)
    }

    private static let now = Int64(Date().timeIntervalSince1970 * 1000)
    private static let hour: Int64 = 3_600_000
    private static let day: Int64 = 86_400_000

    /// The seeded commitments. Deliberately spans the cases the ribbon must handle: a point
    /// in time, a span, a recurrence, a past commitment, and events sharing participants so
    /// intersections actually intersect.
    static let events: [EventVM] = [
        EventVM(id: "e-bristol",
                startMs: now + 2 * day + 9 * hour,
                title: "Drive to Bristol",
                participants: [r(hope, .subject), r(bristol, .at), r(car, .with)]),

        EventVM(id: "e-gallery-open",
                startMs: now + 3 * day + 10 * hour,
                endMs:   now + 3 * day + 18 * hour,
                title: "Gallery opening hours",
                participants: [r(gallery, .subject), r(choir, .with)],
                recurrenceLabel: "Tue & Thu"),

        EventVM(id: "e-choir",
                startMs: now + 4 * day + 19 * hour,
                endMs:   now + 4 * day + 21 * hour,
                title: "Choir rehearsal",
                participants: [r(choir, .subject), r(gallery, .at), r(hope, .with)]),

        EventVM(id: "e-northwind",
                startMs: now + 6 * day + 14 * hour,
                title: "Northwind review",
                participants: [r(alice, .subject), r(northwind, .about), r(studio, .at)]),

        EventVM(id: "e-ride",
                startMs: now + 9 * day + 8 * hour,
                title: "Ride to the studio",
                participants: [r(ralph, .subject), r(studio, .at), r(bike, .with)]),

        EventVM(id: "e-bristol-past",
                startMs: now - 11 * day + 9 * hour,
                title: "Bristol, last visit",
                participants: [r(hope, .subject), r(bristol, .at), r(car, .with)]),
    ]

    /// Everything the ribbons can list, by category — what a ribbon shows when its
    /// dimension is unbound.
    static var entities: [EventCategory: [ParticipantVM]] {
        var out: [EventCategory: [ParticipantVM]] = [:]
        for e in events {
            for p in e.participants {
                out[p.category, default: []].append(p)
            }
        }
        // dedupe by id, keep first
        return out.mapValues { list in
            var seen = Set<String>()
            return list.filter { seen.insert($0.id).inserted }
                .sorted { $0.name < $1.name }
        }
    }
}

// MARK: - The seeded source (real intersection, seeded data)

@MainActor
struct SeedEventsSource: EventsSource {
    var all: [EventVM] = EventsSeed.events

    func events(matching terms: [FilterTerm], window: EventsWindow) -> [EventVM] {
        all.filter { satisfies($0, terms) && inWindow($0, window) }
           .sorted { $0.startMs < $1.startMs }
    }

    /// Conjunction: EVERY locked dimension must be satisfied, one hop from the event.
    private func satisfies(_ e: EventVM, _ terms: [FilterTerm]) -> Bool {
        terms.allSatisfy { term in
            // The events dimension is satisfied by the event ITSELF, not by a participant —
            // an event is not its own neighbour.
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
        case .upcoming:                 return e.startMs >= now
        case .all:                      return true
        case .between(let s, let end):  return e.startMs >= s && e.startMs <= end
        }
    }
}
