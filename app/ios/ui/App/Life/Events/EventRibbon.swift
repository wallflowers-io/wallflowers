//  EventRibbon.swift
//  The EVENTS ribbon — commitments in chronological order, vertically, in a column whose
//  width is set by its jewel.
//
//  The jewel IS the width control: contracted it is a Ø56 circle, selected it expands to a
//  pill. So a row is never laid out for one width — it degrades across three tiers, showing
//  strictly less as the ribbon narrows and never reflowing into something unreadable.
//
//  Rows carry no dividers and no captions: rhythm comes from spacing, meaning from the
//  time, the commitment, and the tinted participant dots.

import SwiftUI

// MARK: - Width tiers

/// How much a row may say at the ribbon's current width. Driven by measured width, not by
/// a selected/unselected flag — a ribbon mid-animation is a real width and must render.
enum RibbonTier {
    /// Circle-width. A date tick and the participants as bare dots.
    case tick
    /// Between. Date and the commitment, clipped to one line.
    case compact
    /// Pill-width and beyond. Date, span, commitment, named participants.
    case full

    init(width: CGFloat) {
        switch width {
        case ..<76:   self = .tick
        case ..<168:  self = .compact
        default:      self = .full
        }
    }
}

// MARK: - The ribbon

struct EventRibbon: View {
    let events: [EventVM]
    /// The ribbon's width — handed down from the jewel, animatable.
    let width: CGFloat
    var onSelect: (EventVM) -> Void = { _ in }

    private var tier: RibbonTier { RibbonTier(width: width) }

    var body: some View {
        ScrollView(showsIndicators: false) {
            LazyVStack(alignment: .leading, spacing: tier == .tick ? 14 : 18) {
                ForEach(events) { event in
                    EventRow(event: event, tier: tier)
                        .contentShape(Rectangle())
                        .onTapGesture { onSelect(event) }
                }
            }
            .padding(.vertical, 12)
        }
        .frame(width: width)
        .animation(.spring(response: 0.34, dampingFraction: 0.78), value: width)
    }
}

// MARK: - One commitment

private struct EventRow: View {
    let event: EventVM
    let tier: RibbonTier

    var body: some View {
        switch tier {
        case .tick:    tick
        case .compact: compact
        case .full:    full
        }
    }

    // Circle-width: the date, stacked, and the participants as dots. Nothing else fits,
    // and a clipped word reads worse than none.
    private var tick: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(EventFormat.day(event.startMs))
                .font(.system(size: 17, weight: .bold, design: .rounded))
                .foregroundStyle(.primary)
            Text(EventFormat.weekday(event.startMs))
                .font(.system(size: 9, weight: .semibold))
                .tracking(0.6)
                .foregroundStyle(.secondary)
            dots
        }
    }

    private var compact: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(EventFormat.dayAndWeekday(event.startMs))
                .font(.system(size: 11, weight: .semibold))
                .tracking(0.5)
                .foregroundStyle(.secondary)
            Text(event.title)
                .font(.system(size: 14, weight: .medium))
                .foregroundStyle(.primary)
                .lineLimit(1)
            dots
        }
    }

    private var full: some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack(spacing: 8) {
                Text(EventFormat.dayAndWeekday(event.startMs))
                    .font(.system(size: 11, weight: .semibold))
                    .tracking(0.5)
                    .foregroundStyle(.secondary)
                if let span = EventFormat.span(event.startMs, event.endMs) {
                    Text(span)
                        .font(.system(size: 11, weight: .medium))
                        .foregroundStyle(.secondary)
                }
                if let r = event.recurrenceLabel {
                    Text(r)
                        .font(.system(size: 11, weight: .medium))
                        .foregroundStyle(.secondary)
                }
            }
            Text(event.title)
                .font(.system(size: 15, weight: .medium))
                .foregroundStyle(.primary)
                .lineLimit(2)
            participants
        }
    }

    /// Contracted participation: one tinted dot per participant, subject first.
    private var dots: some View {
        HStack(spacing: 3) {
            ForEach(ordered.prefix(5)) { p in
                Circle().fill(p.tint).frame(width: 5, height: 5)
            }
        }
    }

    /// Named participation. The subject reads plain; the rest carry their role's preposition
    /// so "Bristol" becomes "at Bristol" without a caption doing the explaining.
    private var participants: some View {
        HStack(spacing: 10) {
            ForEach(ordered.prefix(3)) { p in
                HStack(spacing: 4) {
                    Circle().fill(p.tint).frame(width: 5, height: 5)
                    Text(p.role == .subject ? p.name : "\(preposition(p.role)) \(p.name)")
                        .font(.system(size: 12))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
        }
    }

    /// Subject first — it is the entity the commitment is ON — then the rest as given.
    private var ordered: [ParticipantVM] {
        event.participants.sorted { a, b in
            (a.role == .subject ? 0 : 1) < (b.role == .subject ? 0 : 1)
        }
    }

    private func preposition(_ r: ParticipantRole) -> String {
        switch r {
        case .subject: return ""
        case .at:      return "at"
        case .with:    return "with"
        case .about:   return "on"
        }
    }
}

// MARK: - Formatting

enum EventFormat {
    /// Commitments are FLOATING dates: a date-only commitment is anchored to midnight UTC with
    /// no zone attached. Rendering that in the device's local zone shifts it a whole day
    /// backwards anywhere west of UTC — "Saturday" displays as FRI. So a date-only
    /// commitment must be read back in the zone it was anchored in, which is UTC.
    ///
    /// This holds only while commitments carry no real time-of-day. Once a source writes a
    /// wall-clock time (the gallery's 1000–1800), it must carry its TZID and be rendered in
    /// THAT zone — see EVENTS-EXTRACTION.md on recurrence and TZID.
    private static let zone = TimeZone(identifier: "UTC") ?? .gmt

    private static var cal: Calendar {
        var c = Calendar(identifier: .gregorian)
        c.timeZone = zone
        return c
    }

    private static func formatter(_ pattern: String) -> DateFormatter {
        let f = DateFormatter()
        f.dateFormat = pattern
        f.timeZone = zone
        return f
    }

    private static func date(_ ms: Int64) -> Date {
        Date(timeIntervalSince1970: TimeInterval(ms) / 1000)
    }

    static func day(_ ms: Int64) -> String {
        String(cal.component(.day, from: date(ms)))
    }

    static func weekday(_ ms: Int64) -> String {
        formatter("EEE").string(from: date(ms)).uppercased()
    }

    static func dayAndWeekday(_ ms: Int64) -> String {
        "\(weekday(ms)) \(day(ms))"
    }

    /// "10–18" for a span, nil for a point in time — a start time alone is not a span and
    /// printing "10–10" would be a lie.
    static func span(_ startMs: Int64, _ endMs: Int64?) -> String? {
        guard let endMs, endMs > startMs else { return nil }
        let f = formatter("HH")
        return "\(f.string(from: date(startMs)))–\(f.string(from: date(endMs)))"
    }
}
