//  PartnerSeeder.swift
//  DEBUG-only: the partners' Spaces, posts and nights — STOMA, Manolin, CYP3, Prodigal —
//  minted through THE mint, each with its media in the draft, so the app can be seen as
//  it will look in use. Every object is `CoreService.mint` — the call RED's creator makes —
//  plus the Space's `created` edge to it. Media rides the draft and nothing else: the mint
//  is what delivers it (#112), so a gap there shows here as a bare card, never papered over
//  by a second media call.
//
//  Reads Application Support/pacific/seed/partners/pack.json, which `just seed-partners`
//  drops (scripts/partner-pack.py). The media is the partners', so it is never in git or
//  the bundle. No pack, no seed: nothing is minted in its place.
//
//  Media or nothing (Ralph, 9 Nov): an item whose pictures don't load is refused, never
//  minted bare. Full size: sizing and offload are the mint's (#56, #111). Idempotent by
//  title, like EventSeeder.

#if DEBUG
import Foundation
import PacificCore

@MainActor
enum PartnerSeeder {
    static let launchArg = "-pacificSeedPartners"

    private struct Post: Decodable { let title, body, banner, icon: String }
    private struct Event: Decodable {
        let title, descriptor, venue, banner: String
        let dayOffset, hour, hours: Int
    }
    private struct Partner: Decodable {
        let name, shape, cover: String
        let posts: [Post]
        let events: [Event]
    }
    private struct Pack: Decodable { let partners: [Partner] }

    static var packDir: URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("pacific/seed/partners", isDirectory: true)
    }

    static func seedIfRequested(core: CoreService) async {
        guard ProcessInfo.processInfo.arguments.contains(launchArg) else { return }
        guard let data = try? Data(contentsOf: packDir.appendingPathComponent("pack.json")),
              let pack = try? JSONDecoder().decode(Pack.self, from: data) else {
            NSLog("[seed-partners] no pack at %@ — run `just seed-partners`", packDir.path)
            return
        }
        for p in pack.partners { await seed(p, core: core) }
    }

    private static func seed(_ p: Partner, core: CoreService) async {
        await core.refreshOrgs().value
        let space: String
        if let existing = core.orgs.first(where: { $0.name == p.name }) {
            space = existing.id
        } else {
            guard let cover = media(p.cover) else {
                NSLog("[seed-partners] %@ REFUSED: no cover %@", p.name, p.cover)
                return
            }
            var d = draft(p.name)
            d.shape = p.shape
            d.banner = cover
            guard let id = await core.mint(kind: "group", draft: d) else { return }
            space = id
        }

        await core.refreshPosts().value
        let posts = Set(core.posts.map(\.title))
        for post in p.posts where !posts.contains(post.title) {
            guard let banner = media(post.banner), let icon = media(post.icon) else {
                NSLog("[seed-partners] %@ REFUSED: no media", post.title)
                continue
            }
            var d = draft(post.title)
            d.descriptor = post.body
            d.form = post.body.isEmpty ? "image" : "article"
            d.icon = icon
            d.banner = banner
            guard let id = await core.mint(kind: "post", draft: d) else { continue }
            _ = core.setAffiliation(space, peer: id, rel: "created", name: post.title)
        }

        await core.refreshEvents().value
        let events = Set(core.events.map(\.title))
        let today = Calendar.current.startOfDay(for: Date())
        for e in p.events where !events.contains(e.title) {
            guard let banner = media(e.banner) else {
                NSLog("[seed-partners] %@ REFUSED: no banner", e.title)
                continue
            }
            // The one invention: the night re-dated into the coming weeks (Ralph, 9 Nov).
            let start = today.addingTimeInterval(TimeInterval((e.dayOffset * 24 + e.hour) * 3600))
            var d = draft(e.title)
            d.descriptor = e.descriptor
            d.venue = e.venue
            d.startMs = Int64(start.timeIntervalSince1970 * 1000)
            d.endMs = d.startMs + Int64(e.hours) * 3_600_000
            d.banner = banner
            guard let id = await core.mint(kind: "event", draft: d) else { continue }
            _ = core.setAffiliation(space, peer: id, rel: "created", name: e.title)
        }
        core.refreshEvents()
    }

    private static func draft(_ name: String) -> MintDraft {
        MintDraft(name: name, descriptor: "", shape: "", card: "", category: "",
                  startMs: 0, endMs: 0, venue: "", recurrence: "", lineup: "",
                  form: "", link: "", placed: false, lat: 0, lng: 0, status: "",
                  icon: Data(), banner: Data(), price: "", stance: "", place: "")
    }

    /// A pack image's bytes. nil = missing: refuse.
    private static func media(_ path: String) -> Data? {
        try? Data(contentsOf: packDir.appendingPathComponent(path))
    }
}
#endif
