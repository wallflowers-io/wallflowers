//  MarketCommitments.swift
//  Market postures, read as EVENTS.
//
//  This is the 25 Jul design taken at its word: "Wants/Buying/Selling/Offers are COMMITMENTS
//  (Event-shaped, with participants and optional price/deadline); Has is entity state." So a
//  dated posture is not a second kind of row that needs its own feed — it IS an event, and it
//  belongs in the one chronological feed LIFE already has.
//
//  That is what threads THINGS into EVENTS: mint a hammer, say you want it by Friday, and the
//  commitment appears in the EVENTS ribbon alongside every other commitment in it, with
//  the thing as its participant so the multimodal filter can intersect on it.
//
//  DATED ONLY, and deliberately. `EventVM.startMs` is the sort key and is non-optional — the
//  feed is chronological, and an undated row has no place in it (the graph source drops
//  undated events for exactly the same reason). A STANDING want is not a lesser event, it is
//  a different question: "what is my position" rather than "what is coming". That question is
//  answered by the market lens on the THINGS jewel. Neither view is a fallback for the other.
//
//  `Has` never appears here at all: holding something is a state, not a commitment, so there
//  is nothing to put on a timeline even when it carries a date.

import SwiftUI
import PacificCore   // ThingRow

enum MarketCommitments {

    /// Dated market postures as commitments, newest-first ordering left to the caller.
    ///
    /// `me` is the display name for the committed party — the subject of every one of these,
    /// because a posture is always YOUR position on a thing.
    static func events(from things: [ThingRow], me: String) -> [EventVM] {
        things.compactMap { row in
            guard row.deadline > 0 else { return nil }          // undated → the market lens, not the feed
            guard let intent = MarketIntent(rawValue: row.posture.uppercased()) else { return nil }
            guard intent != .has else { return nil }            // a state, not a commitment
            let title = row.name.isEmpty ? row.descriptor : row.name

            return EventVM(
                id: "market:\(row.id)",
                startMs: row.deadline,
                title: "\(intent.rawValue.capitalized) \(title)" + (row.price.isEmpty ? "" : " · \(row.price)"),
                participants: [
                    // The committed party. `subject` is required — an event with no subject is
                    // unreachable by the filter, which is why the graph source drops those.
                    ParticipantVM(id: "me", name: me, category: .people,
                                  role: .subject, tint: EventsSeed.tint(.people)),
                    // The thing itself, so binding THINGS in the filter reaches this commitment.
                    ParticipantVM(id: row.id, name: title, category: .things,
                                  role: .about, tint: EventsSeed.tint(.things)),
                ])
        }
    }
}

/// The EVENTS feed as LIFE actually reads it: what the resolver observed, plus your own dated
/// market commitments, in one chronological order.
///
/// Merging here rather than inside `EventsSnapshot` keeps the graph snapshot a faithful record
/// of what was OBSERVED. Market commitments are things you ASSERTED by minting and posting a
/// posture — a different provenance, and one that must not be written back into the snapshot
/// the resolver owns.
@MainActor
struct MergedEventsSource: EventsSource {
    let graph: GraphEventsSource
    let market: [EventVM]
    /// Connected Google Calendar commitments (a third provenance — structured, so mapped to
    /// EventVM without the resolver). Empty until a calendar System is connected.
    var calendar: [EventVM] = []
    /// Minted Event GroupObjects (see GroupObjectEventsSource) — a fourth provenance. A graph
    /// row sharing a minted row's (normalized title, start day) is DROPPED: the GroupObject
    /// wins, because it is the one with a log, a listing and a wallet behind it (v1 heuristic).
    var groupObjects: [EventVM] = []
    /// The Music System's RA listings (see MusicEventsSource) — the reference provenance.
    /// Same dedup rule as the graph: a minted GroupObject sharing an RA row's (normalized
    /// title, start day) WINS — your object beats the world's listing of it.
    var music: [EventVM] = []
    /// The Art System's ArtRabbit listings (see ArtEventsSource) — reference provenance
    /// too, under the same minted-wins dedup.
    var art: [EventVM] = []

    func events(matching terms: [FilterTerm], window: EventsWindow) -> [EventVM] {
        let minted = Set(groupObjects.map { GroupObjectEventsSource.dedupKey(title: $0.title, startMs: $0.startMs) })
        let observed = graph.events(matching: terms, window: window)
            .filter { !minted.contains(GroupObjectEventsSource.dedupKey(title: $0.title, startMs: $0.startMs)) }
        // The reference feed. Dedup is per-source and DETERMINISTIC: each System keeps
        // a listing once, keyed by the source's own stable id (RA's numeric id, an
        // ArtRabbit ident), and a minted GroupObject sharing a row's (normalized
        // title, start day) wins. The same real-world event listed by BOTH sources is
        // a cross-source collision we deliberately do not handle (ruling of 2 Aug):
        // the sources share no id namespace, so both rows show.
        let reference = (music + art)
            .filter { !minted.contains(GroupObjectEventsSource.dedupKey(title: $0.title, startMs: $0.startMs)) }
        // Asserted (market), connected (calendar), minted (GroupObject) and reference (RA)
        // rows go through the SAME intersection as everything else, so a locked dimension
        // filters them identically — no privileged rows.
        let extra = (market + calendar + groupObjects + reference).filter { event in
            inWindow(event, window) && terms.allSatisfy { term in
                if term.category == .events { return term.entityID == nil || term.entityID == event.id }
                return event.participants.contains { p in
                    p.category == term.category && (term.entityID == nil || term.entityID == p.id)
                }
            }
        }
        return (observed + extra).sorted { $0.startMs < $1.startMs }
    }

    private func inWindow(_ e: EventVM, _ w: EventsWindow) -> Bool {
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        switch w {
        // "Commitments still outstanding" — a span that has opened but not yet closed
        // (an exhibition's run, a meeting in progress) is still outstanding, so
        // `.upcoming` keeps it for its whole span, not just until it starts.
        case .upcoming:                return e.startMs >= now || (e.endMs ?? .min) >= now
        case .all:                     return true
        case .between(let s, let end): return e.startMs >= s && e.startMs <= end
        }
    }
}
