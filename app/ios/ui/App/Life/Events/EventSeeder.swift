//  EventSeeder.swift
//  DEBUG-only: seed the two real launch events through the REAL pipe — THE mint, with the
//  poster in the draft, then setEventTickets — so a headless drive (UI tests, simctl
//  screenshots) has true GroupObjects to stand in front of, never fixture rows. The bundled
//  posters are the banners (SeedMedia/*.jpg); the mint carries them detached (#112, #56),
//  since a poster inline is over the Delta ceiling.
//
//  Idempotent by title: a relaunch with the flag re-seeds nothing that already exists.
//  Compiles to nothing in Release, like DevLinks — the seam this follows.

#if DEBUG
import Foundation
import PacificCore

@MainActor
enum EventSeeder {
    static let launchArg = "-pacificSeedEvents"

    /// True when a headless drive asked to land on the EVENTS page (`-pacificShowEvents`,
    /// or opening a specific event / RA listing implies it).
    static var wantsEventsScreen: Bool {
        ProcessInfo.processInfo.arguments.contains("-pacificShowEvents")
            || UserDefaults.standard.string(forKey: "pacificOpenEvent") != nil
            || UserDefaults.standard.string(forKey: "pacificOpenListing") != nil
    }

    static func seedIfRequested(core: CoreService) async {
        guard ProcessInfo.processInfo.arguments.contains(launchArg) else { return }
        await seed(core: core)
    }

    static func seed(core: CoreService) async {
        await core.refreshEvents().value
        let existing = Set(core.events.map(\.title))
        let cal = Calendar.current
        let today = cal.startOfDay(for: Date())
        func hour(_ h: Int) -> Int64 {
            Int64((cal.date(byAdding: .hour, value: h, to: today) ?? today).timeIntervalSince1970 * 1000)
        }

        let beach = "BEACHCRIMES — Rooftop Day Party"
        if !existing.contains(beach) {
            if let id = core.mint(kind: "event", draft: draft(
                    beach, descriptor: "City Graffiti · Truthlive · All Day All Night",
                    startMs: hour(13), endMs: hour(18), venue: "Hotel VIA Rooftop, SF",
                    banner: poster("beachcrimes"))) {
                // PRICED through the real pipe: the fold requires acct + delegate iff
                // priced, so the seed names THIS device's own identity as its fulfilment
                // delegate — a real pk, not a stub that would be refused.
                _ = core.setEventTickets(id, priceCents: 2000, currency: "usd",
                                         capacity: 150, acct: "acct_seed_beachcrimes",
                                         delegate: core.myIdentityHex)
                NSLog("[seed-events] %@ id=%@ err=%@", beach, id, core.error ?? "none")
            } else {
                NSLog("[seed-events] %@ MINT FAILED: %@", beach, core.error ?? "no error set")
            }
        }

        // A guaranteed-FUTURE row for the feed suites: both posters above are
        // seeded at TODAY's hours, so an evening run finds them already past and
        // the feed honestly empty — flaky by clock. This one lands tomorrow
        // 19:00 and is titled by that date, so each day's first seeded run puts
        // exactly one future event on the feed at ANY hour (idempotent-by-title,
        // like the rest of the seeder).
        let df = DateFormatter()
        df.dateFormat = "yyyy-MM-dd"
        let qaDay = df.string(from: cal.date(byAdding: .day, value: 1, to: today) ?? today)
        let qa = "QA Fixture \(qaDay)"
        // It also carries the two facets nothing else seeded did — a LINEUP and a
        // repeat rule — because the event page grew a bar per artist and a series
        // line, and a surface with no fixture behind it is a surface nobody sees
        // until a user does. Invented names on an INVENTED row: the two real nights
        // above keep saying exactly what their promoters said, which is why the
        // fixture, not BEACHCRIMES, is the one that grew a lineup.
        if !existing.contains(qa) {
            if let id = core.mintEvent(title: qa,
                                       descriptor: "FeedUITests' deterministic future row",
                                       startMs: hour(24 + 19), endMs: hour(24 + 21),
                                       venue: "QA",
                                       recurrence: "FREQ=WEEKLY;BYDAY=TH",
                                       lineup: "QA Support\nQA Headliner") {
                _ = core.setEventTickets(id, priceCents: 0, currency: "usd", capacity: 0)
                NSLog("[seed-events] %@ id=%@ err=%@", qa, id, core.error ?? "none")
            }
        }

        let upward = "Monroe presents UPWARD"
        if !existing.contains(upward) {
            if let id = core.mint(kind: "event", draft: draft(
                    upward, descriptor: "Apex Martin · Cloopez · Joey Trip · 21+",
                    startMs: hour(6), endMs: hour(11), venue: "Monroe, 473 Broadway, SF",
                    banner: poster("upward"))) {
                _ = core.setEventTickets(id, priceCents: 0, currency: "usd", capacity: 120)
                NSLog("[seed-events] %@ id=%@ err=%@", upward, id, core.error ?? "none")
            } else {
                NSLog("[seed-events] %@ MINT FAILED: %@", upward, core.error ?? "no error set")
            }
        }

        // EVERY seeded night gets its publisher, not just one: the feed sorts by
        // time and a test taps whatever is on top, so a single hosted event makes
        // beat 2 a coin toss. The diagram says the same thing — its feed cards each
        // carry an author ('Beachcrimes', 'Monroe', 'NTS London').
        await seedHost(core: core, space: "Monroe",      publishing: upward)
        await seedHost(core: core, space: "Beachcrimes", publishing: beach)
        await seedHost(core: core, space: "QA Promoter", publishing: qa)
    }

    /// THE JOURNEY'S HOST (one-pager beats 2–3). An event minted by this device has
    /// no publisher, so `EventDetailPage`'s HOST capsule — which reads
    /// `creatorOfObject`, i.e. a Space carrying a `created` affiliation to the event
    /// — had nothing to render and the journey stopped dead at beat 2.
    ///
    /// So the seed mints the promoter as a real Space and lets it CLAIM the night,
    /// through the same two calls the app makes: `mintOrg` then `setAffiliation(rel:
    /// "created")`. The edge is authored on the SPACE's own log, which is the
    /// one-sided rule this codebase holds everywhere — nobody writes on someone
    /// else's log, which is exactly why `creatorOfObject` scans rather than
    /// dereferences.
    ///
    /// Then one CHANNEL on that Space (beat 3), through `groupForumNew` — a real
    /// Forum GroupObject plus its `part_of` edge (`base.setPart`), never a row.
    private static func seedHost(core: CoreService, space host: String, publishing title: String) async {
        await core.refreshOrgs().value
        let space: String
        if let existing = core.orgs.first(where: { $0.name == host }) {
            space = existing.id
        } else {
            guard let id = core.mintOrg(name: host, shape: "organisation",
                                        purpose: "Publishes nights.") else {
                NSLog("[seed-events] host %@ MINT FAILED: %@", host, core.error ?? "no error set")
                return
            }
            space = id
        }

        // Claim the night. Idempotent: the fold is LWW per (peer, rel), so a
        // relaunch rewrites the same edge rather than accumulating them.
        await core.refreshEvents().value
        if let event = core.events.first(where: { $0.title == title }) {
            _ = core.setAffiliation(space, peer: event.id, rel: "created", name: event.title)
        }

        // One channel, if the Space has none yet.
        if core.groupForums(space).isEmpty {
            Task { @MainActor in
                _ = await core.newGroupForum(space, name: "general")
            }
        }
    }

    private static func draft(_ title: String, descriptor: String, startMs: Int64, endMs: Int64,
                              venue: String, banner: Data?) -> MintDraft {
        MintDraft(name: title, descriptor: descriptor, shape: "", card: "", category: "",
                  startMs: startMs, endMs: endMs, venue: venue, recurrence: "", lineup: "",
                  form: "", link: "", placed: false, lat: 0, lng: 0, status: "",
                  icon: Data(), banner: banner ?? Data(), price: "", stance: "", place: "")
    }

    private static func poster(_ name: String) -> Data? {
        Bundle.main.url(forResource: name, withExtension: "jpg")
            .flatMap { try? Data(contentsOf: $0) }
    }
}
#endif
