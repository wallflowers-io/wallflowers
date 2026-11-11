//  GroupObjectEventsSource.swift
//  Minted Event GroupObjects, read as EVENTS — the third provenance in the feed.
//
//  The graph records what the resolver OBSERVED; the market rows are postures you ASSERTED;
//  these are events you MINTED — kind-29 GroupObjects with a signed log, a ticket listing and
//  a sales ledger. They are the durable truth the composer writes and the ticket capsule reads.
//
//  DEDUP (v1 heuristic, flagged in the plan): the resolver may have observed the same event a
//  minted GroupObject now IS — same words, same day. The merged union drops the graph row when
//  a GroupObject row shares its (normalized title, start day): the minted object WINS, because
//  it is the one with a log, a listing and a wallet behind it. Day granularity is UTC — crude,
//  and accepted as such until event entity-reconciliation lands (backlog, Epic 12).

import SwiftUI
import PacificCore   // EventRow

enum GroupObjectEventsSource {

    /// Minted Event GroupObjects as feed rows. `me` names the organizer — every minted event
    /// is YOUR commitment, and `participants` must never be empty (an event with no
    /// participants is unreachable by the intersection filter).
    static func events(from rows: [EventRow], me: String) -> [EventVM] {
        rows.map { row in
            var participants = [
                ParticipantVM(id: "me", name: me, category: .people,
                              role: .subject, tint: EventsSeed.tint(.people)),
            ]
            if !row.venuePlace.isEmpty {
                // A REFERENCED Place (event.setVenue): the id IS the Place object id,
                // so EventsChannel.venue's identity match pins it exactly.
                participants.append(
                    ParticipantVM(id: row.venuePlace,
                                  name: row.venuePlaceName.isEmpty ? row.venuePlace : row.venuePlaceName,
                                  category: .places,
                                  role: .at, tint: EventsSeed.tint(.places)))
            } else if !row.venue.isEmpty {
                // Named by the row, pinned by EventsChannel.venue's name-match against the
                // placed PLACES — the same hop a resolver-lifted venue takes.
                participants.append(
                    ParticipantVM(id: row.venue, name: row.venue, category: .places,
                                  role: .at, tint: EventsSeed.tint(.places)))
            }
            return EventVM(id: "obj:\(row.id)",
                           startMs: row.startMs,
                           endMs: row.endMs > 0 ? row.endMs : nil,
                           title: row.title,
                           participants: participants)
        }
    }

    /// The dedup key: normalized title × UTC start day (the v1 heuristic above).
    static func dedupKey(title: String, startMs: Int64) -> String {
        let t = title.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return "\(t)|\(startMs / 86_400_000)"
    }
}
