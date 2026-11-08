//  FixtureLoader.swift
//  DEBUG-only: folds a QA fixture (core/coordination/feed-dataset.schema.json shape)
//  into REAL GroupObjects through the same CoreService calls the UI makes — mint,
//  posture, venue, created — so nothing downstream can tell a fixture row from a
//  tapped one (I-A holds mechanically). Persons are SKIPPED: peers come from real
//  pairing, and cross-device rows must arrive over the relay, not be forged here.
//
//  Reads  Application Support/pacific/fixtures/<name>.json  (the harness drops it),
//  writes Application Support/pacific/dev-fixture.txt — counts + fixtureId=realId
//  lines so the harness can address minted objects by their fixture ids.

#if DEBUG
import Foundation

@MainActor
enum FixtureLoader {
    private struct FEdge: Decodable { let rel: String; let target: String }
    private struct FObject: Decodable {
        let id: String, kind: String, title: String
        let descriptor: String?, posture: String?, category: String?
        let shape: String?, area: String?, system: String?
        let startMs: Int64?, endMs: Int64?
        let lat: Double?, lng: Double?
        let edges: [FEdge]?
    }
    private struct FFile: Decodable { let objects: [FObject] }

    static func load(name: String, core: CoreService) {
        let dir = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("pacific", isDirectory: true)
        let doneKey = "pacific.fixture.loaded.\(name)"
        guard !UserDefaults.standard.bool(forKey: doneKey) else {
            NSLog("[fixture] %@ already loaded", name)
            return
        }
        let file = dir.appendingPathComponent("fixtures/\(name).json")
        guard let data = try? Data(contentsOf: file),
              let fixture = try? JSONDecoder().decode(FFile.self, from: data) else {
            NSLog("[fixture] %@ unreadable at %@", name, file.path)
            return
        }

        var real: [String: String] = [:]      // fixture id → minted object id
        var titles: [String: String] = [:]    // fixture id → title (venue names)
        for o in fixture.objects { titles[o.id] = o.title }
        var counts: [String: Int] = [:]

        // Dependency order: groups and places first (events point at both).
        for o in fixture.objects where o.kind == "group" {
            guard let id = core.mintOrg(name: o.title, shape: o.shape ?? "team",
                                        purpose: o.descriptor ?? "") else { continue }
            real[o.id] = id; counts["group", default: 0] += 1
        }
        for o in fixture.objects where o.kind == "place" {
            guard let id = core.mintPlace(name: o.title, descriptor: o.descriptor ?? "",
                                          placed: o.lat != nil,
                                          lat: o.lat ?? 0, lng: o.lng ?? 0) else { continue }
            real[o.id] = id; counts["place", default: 0] += 1
        }
        for o in fixture.objects where o.kind == "listing" {
            let category = (o.category == "civic") ? "skill" : "artifact"
            guard let id = core.mintThing(name: o.title, descriptor: o.descriptor ?? "",
                                          category: category) else { continue }
            // reach is CONSENT and defaults closed ("absent reach must not mean
            // network" — thing.rs pinned test). QA identities consent explicitly,
            // or nothing ever gossips and the cross-device half is a no-op.
            // area stays UNSET: the fold demands a geohash (parse_area) and the
            // fixture carries neighbourhood slugs — a bad area rejects the WHOLE
            // posture delta (validate-everything-first), which is exactly the
            // silent all-listings-dead failure run 2 hit.
            core.setPosture(id, posture: o.posture ?? "has", reach: "network")
            real[o.id] = id; counts["listing", default: 0] += 1
        }
        for o in fixture.objects where o.kind == "event" {
            guard let start = o.startMs,
                  let id = core.mintEvent(title: o.title, descriptor: o.descriptor ?? "",
                                          startMs: start, endMs: o.endMs ?? 0,
                                          venue: o.area ?? "") else { continue }
            real[o.id] = id; counts["event", default: 0] += 1
            // Free listing so the RSVP posture is real. Capacity must be non-zero:
            // the fold refuses setTickets(capacity: 0) silently (landmine L6).
            // Currency must be a REAL 3-letter code even at price 0 — the fold
            // validates `currency.len() == 3` before it looks at the price
            // (event.rs OP_SET_TICKETS), so "" rejects the whole delta as
            // MalformedArgs and no fixture event ever gets its free listing.
            // That rejection also strands a "Couldn't connect" alert over the
            // whole run (run-3 finding). krw: the dataset is Seoul.
            _ = core.setEventTickets(id, priceCents: 0, currency: "krw", capacity: 500)
            for e in o.edges ?? [] {
                switch e.rel {
                case "happens_at":
                    if let pid = real[e.target] {
                        _ = core.setEventVenue(id, placeId: pid, name: titles[e.target] ?? "")
                    }
                case "organised_by":
                    if let gid = real[e.target] {
                        _ = core.attachCreated(gid, objectId: id, name: o.title)
                    }
                default: break
                }
            }
        }

        let skippedPersons = fixture.objects.filter { $0.kind == "person" }.count
        core.publishMarket()                  // make listings travel now, not next tick
        core.refresh(); core.refreshThings(); core.refreshEvents(); core.refreshOrgs()
        UserDefaults.standard.set(true, forKey: doneKey)

        var lines = ["loaded groups=\(counts["group"] ?? 0) places=\(counts["place"] ?? 0) "
                     + "listings=\(counts["listing"] ?? 0) events=\(counts["event"] ?? 0) "
                     + "skippedPersons=\(skippedPersons)"]
        lines += real.keys.sorted().map { "\($0)=\(real[$0]!)" }
        try? lines.joined(separator: "\n")
            .write(to: dir.appendingPathComponent("dev-fixture.txt"),
                   atomically: true, encoding: .utf8)
        NSLog("[fixture] %@ loaded: %@ (persons skipped: %d)", name,
              String(describing: counts), skippedPersons)
    }

    /// Second pass: ride the sourced images onto ALREADY-MINTED fixture objects.
    /// Reads fixtures/media/manifest.json ({fixtureId: {file: …}}) + the
    /// dev-fixture.txt id map; events get a banner (event.setMedia), listings a
    /// thing photo — both cap-enforced by the fold, both re-published so media
    /// TRAVELS with the gossip. Writes dev-media.txt for the harness.
    static func attachMedia(core: CoreService) async {
        let dir = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("pacific", isDirectory: true)
        guard let mdata = try? Data(contentsOf: dir.appendingPathComponent("fixtures/media/manifest.json")),
              let manifest = (try? JSONSerialization.jsonObject(with: mdata)) as? [String: Any],
              let mapText = try? String(contentsOf: dir.appendingPathComponent("dev-fixture.txt"),
                                        encoding: .utf8) else {
            NSLog("[fixture] attach-media: manifest or id map missing")
            return
        }
        var map: [String: String] = [:]
        for line in mapText.split(separator: "\n") {
            let parts = line.split(separator: "=", maxSplits: 1)
            if parts.count == 2 { map[String(parts[0])] = String(parts[1]) }
        }
        var attached = 0
        for (fid, info) in manifest {
            guard let d = info as? [String: Any], let file = d["file"] as? String,
                  let real = map[fid],
                  let jpeg = try? Data(contentsOf: dir.appendingPathComponent("fixtures/media/\(file)"))
            else { continue }
            if fid.hasPrefix("event:") {
                if core.setEventMedia(objectId: real, bannerJPEG: jpeg, photos: [], clipMP4: nil) {
                    attached += 1
                }
            } else if fid.hasPrefix("listing:") {
                if await core.setThingPhoto(real, jpeg: jpeg) { attached += 1 }
            }
        }
        core.refreshEvents(); core.refreshThings()
        core.publishMarket()
        try? "attached=\(attached)".write(to: dir.appendingPathComponent("dev-media.txt"),
                                          atomically: true, encoding: .utf8)
        NSLog("[fixture] media attached: %d", attached)
    }
}
#endif
