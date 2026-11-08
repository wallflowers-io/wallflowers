//  DevLinks.swift
//  The dev/demo remote seam: `pacific://` URLs drive the SAME real core calls
//  the UI does — nothing here bypasses the MLS handshake or invents state. It
//  exists because `simctl openurl` is the only reliable way to drive TWO
//  simulators through a pairing handshake from a host script (no camera, no
//  synthetic taps). Handlers:
//    pacific://pair-scan?bundle=<percent-encoded contact bundle>
//    pacific://pair-accept?peer=<space id>
//    pacific://sync
//    pacific://union-language?lang=ko|en     (persist the union language authority —
//                                             F-3; the store reindex runs at relaunch)
//    pacific://export-contact                (re-export a FRESH contact bundle — new
//                                             single-use key package — for mesh pairing)
//    pacific://dm-send?peer=<space id>&text=<percent-encoded text>
//    pacific://dm-react?peer=<space id>&emoji=<e>[&author=<hex>&gen=<n>][&active=0]
//                                            (react to a message — default target is
//                                             the last INBOUND message; active=0 clears)
//    pacific://dm-reply?peer=<space id>&text=<…>[&author=<hex>&gen=<n>]
//                                            (reply to a message — default target is
//                                             the last INBOUND message)
//    pacific://seed-dm-demo?peer=<space id>  (this device authors its half of the
//                                             scripted demo through real ops)
//    pacific://open-dm?peer=<space id>       (navigate to the conversation)
//  Ticketing (scripts/two-sim-ticketing-e2e.sh drives exactly these):
//    pacific://event-new?title=…&start=<epoch ms>&price=<cents>&currency=…&capacity=…
//                                            (mint + setTickets; price=0 = the free path)
//    pacific://ticket-request?peer=<space id>&listing=<hex>&qty=1
//    pacific://door-scan?qr=<urlencoded pacific://ticket?…>[&object=<event id>]
//    pacific://show-tickets[?ticket=<id>]    (the wallet; a ticket id opens it full-screen)
//    pacific://dump-wallet | dump-tickets | dump-events   (dev-*.txt, host-readable)
//  A human does the identical flows through NewChatSheet / ChatView (long-press to
//  react, swipe to reply) — these links call the SAME core.react / core.reply.

import Foundation
import SwiftUI
import PacificCore
import PacificStore   // UnionLanguageSetting / ContextualUnionEmbedder — the union-language dev verb

/// Host-readable result drop (same directory the dev-commands drain watches):
/// a headless leg that cannot report its own outcome is a guess, not a drive.
private func devWrite(_ name: String, _ contents: String) {
    let dir = FileManager.default
        .urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        .appendingPathComponent("pacific", isDirectory: true)
    try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    try? contents.write(to: dir.appendingPathComponent(name), atomically: true, encoding: .utf8)
}

extension Notification.Name {
    /// Posted by the open-dm link; ChatsView navigates to the peer's conversation.
    static let pacificOpenDM = Notification.Name("pacificOpenDM")
    /// Posted by the open-project link; the shell switches to Work + navigates in.
    static let pacificOpenProject = Notification.Name("pacificOpenProject")
    /// Posted by the open-room link; the shell presents the threaded ChatRoom.
    static let pacificOpenRoom = Notification.Name("pacificOpenRoom")
    /// Posted by a room cover's title: land on LIFE ▸ GROUPS with the hosting
    /// group's page pushed. userInfo: ["object": the group's object id].
    static let pacificOpenGroup = Notification.Name("pacificOpenGroup")

    /// Open a PERSON's profile in LIFE. The Space half of this pair has existed
    /// since rooms got deep links; the person half arrived with the detail modal,
    /// whose author bar is a door and can point at either.
    static let pacificOpenPerson = Notification.Name("pacificOpenPerson")
    /// Posted by the shell when the BLUE disc is pressed while LIFE is already up:
    /// LIFE returns to its ground state — every jewel minimised, the Feed.
    static let pacificLifeHome = Notification.Name("pacificLifeHome")
    /// Posted by the new-chat link; ChatsView presents the compose sheet.
    static let pacificNewChat = Notification.Name("pacificNewChat")
    /// Posted by the my-qr link; ChatsView presents the Connect (QR + SAS) sheet.
    static let pacificShowQR = Notification.Name("pacificShowQR")
    /// Posted by the project-{settings,people,item} links; an open ProjectDetailView
    /// presents the matching authoring sheet (so a headless drive can screenshot the
    /// new controls that otherwise only open on a tap).
    static let pacificProjectSheet = Notification.Name("pacificProjectSheet")
    /// Posted by the sysiphus-signin link; the shell signs the credentialed System in
    /// (headless drive of the Sysiphus sign-in that otherwise needs the SystemsView form).
    static let pacificSysiphusSignIn = Notification.Name("pacificSysiphusSignIn")
    /// Posted by the project-tab link; sets the visible Project tab (0/1/2) so a
    /// headless drive can screenshot each swipeable tab (simctl can't swipe).
    static let pacificProjectTab = Notification.Name("pacificProjectTab")
    /// Posted by the show-life link; RootShell switches to the LIFE screen (simctl can't tap
    /// the disc row) so a headless drive can screenshot LIFE once objects have projected.
    static let pacificShowLife = Notification.Name("pacificShowLife")
    /// Posted by the show-notes link; RootShell switches to the NOTES list screen so a headless
    /// drive can screenshot that an authored note actually landed in Notes.
    static let pacificShowNotes = Notification.Name("pacificShowNotes")
    /// Posted by the show-account link; RootShell presents the account/Settings sheet so a
    /// headless drive can screenshot it (the avatar tap can't be synthesised).
    static let pacificShowAccount = Notification.Name("pacificShowAccount")
    /// Posted by the bloom link; RootShell fires a press-bloom so it can be screenshotted
    /// (simctl can't tap the disc row).
    static let pacificTestBloom = Notification.Name("pacificTestBloom")
    /// Posted by the door-scan link; the shell presents the DOOR with the scanned payload
    /// injected (a simulator has no camera, so the harness IS the lens).
    /// userInfo: ["object": the event id, "qr": the wallet's verbatim payload].
    static let pacificDoorScan = Notification.Name("pacificDoorScan")
    /// Posted when a MINT notification's Accept/Dismiss action fires. `NotifPresenter` is a
    /// bare NSObject with no store handle, so it ANNOUNCES and the Shell — which already holds
    /// ProposalStore and CoreService — performs the decision. Same seam as pacificOpenProject.
    static let pacificProposalDecision = Notification.Name("pacificProposalDecision")
}

@MainActor
enum DevLinks {
    static func handle(_ url: URL, core: CoreService, notes: NotesStore? = nil,
                       arcs: ArcStore? = nil, membership: ArcMembership? = nil) {
        // DEBUG-only: the pacific:// dev/demo seam compiles to a no-op in Release, so
        // a crafted link can never drive real core calls in a production build. The
        // simulator pairing scripts run Debug builds, so they are unaffected.
        #if DEBUG
        guard url.scheme == "pacific" else { return }
        NSLog("[devlink] %@ relay=%@", url.host ?? "?", core.relayUrl)
        let comps = URLComponents(url: url, resolvingAgainstBaseURL: false)
        func q(_ name: String) -> String? {
            comps?.queryItems?.first(where: { $0.name == name })?.value
        }

        switch url.host {
        case "pair-scan":
            guard let bundle = q("bundle"), !bundle.isEmpty else { return }
            Task { await core.pairScan(bundle) }
        case "pair-accept":
            guard let peer = q("peer"), !peer.isEmpty else { return }
            core.pairAccept(peer)
        case "sync":
            Task { await core.syncNow() }
        case "mint-thing":
            // Headless drive of the ACCEPT half of the hammer journey — the exact calls the
            // swipe deck makes, so what this produces is a real Thing GroupObject with a real
            // posture, not a seeded row. `deadline` (epoch ms) makes the posture a dated
            // COMMITMENT, which is what puts it in the EVENTS feed as well as the market lens.
            guard let name = q("name"), !name.isEmpty else { return }
            // NSLog, not print: only os_log reaches `simctl spawn log show`, and a headless
            // drive that cannot report its own outcome is not a drive, it is a guess.
            NSLog("[devlink] mint-thing name=\(name)")
            guard let id = core.mintThing(name: name, descriptor: q("descriptor") ?? "") else {
                NSLog("[devlink] mint-thing FAILED: \(core.error ?? "no error set")")
                return
            }
            NSLog("[devlink] minted \(id)")
            if let posture = q("posture"), !posture.isEmpty {
                core.setPosture(id,
                                posture: posture,
                                price: q("price") ?? "",
                                deadline: Int64(q("deadline") ?? "") ?? 0)
                NSLog("[devlink] posture=\(posture) err=\(core.error ?? "none") things=\(core.things.count)")
            }
        case "org-new":
            // Headless drive of LIFE ▸ GROUPS ▸ ＋ — the SAME calls OrgWizard.mint makes,
            // so what this produces is a real Group GroupObject with a real setProfile
            // delta, not a seeded row.
            guard let name = q("name"), !name.isEmpty else { return }
            let shape = q("shape") ?? "organisation"
            NSLog("[devlink] org-new name=\(name) shape=\(shape)")
            guard let id = core.mintOrg(name: name, shape: shape, purpose: q("purpose") ?? "") else {
                NSLog("[devlink] org-new FAILED: \(core.error ?? "no error set")")
                return
            }
            // The org TYPE is the identity record's SHAPE now (community included), so
            // `mintOrg` above already carried it — there is nothing local left to set.
            NSLog("[devlink] org-new OK id=\(id)")
        // THE JOURNEY'S THREE MISSING HINGES (one-pager beats 2–3, driven headlessly
        // across a sim fleet). Each is the SAME call the UI makes; what was missing
        // was only a way to reach it without a finger, and to read the minted id back
        // out of the container the way `event-new`/`new-room` already do.
        case "space-new":
            // Like `org-new`, but it EXPORTS the id. A harness that mints a Space and
            // cannot address it afterwards has to scrape NSLog for it.
            guard let name = q("name"), !name.isEmpty else { return }
            guard let id = core.mintOrg(name: name, shape: q("shape") ?? "organisation",
                                        purpose: q("purpose") ?? "") else {
                NSLog("[devlink] space-new FAILED: \(core.error ?? "no error set")")
                return
            }
            core.exportSpaceId(id)
            NSLog("[devlink] space-new OK id=\(id)")
        case "space-forum-new":
            // A CHANNEL on a Space: a real Forum GroupObject + the `base.setPart`
            // edge, and the connected members with stocked prekeys are pulled in by
            // the same call the ＋ runs. Beat 3's surface.
            guard let object = q("object"), let name = q("name"), !name.isEmpty else { return }
            Task {
                guard let id = await core.newGroupForum(object, name: name) else {
                    NSLog("[devlink] space-forum-new FAILED: \(core.error ?? "no error set")")
                    return
                }
                core.exportForumId(id)
                NSLog("[devlink] space-forum-new OK id=\(id)")
            }
        case "space-affiliate":
            // The publisher's claim — `created` by default, which is the edge beat 2's
            // HOST capsule reads back (`creatorOfObject`). Authored on the SPACE's own
            // log; nobody writes on someone else's.
            guard let object = q("object"), let peer = q("peer"), !peer.isEmpty else { return }
            let rel = q("rel") ?? "created"
            let ok = core.setAffiliation(object, peer: peer, rel: rel, name: q("name") ?? "")
            NSLog("[devlink] space-affiliate \(rel) ok=\(ok) err=\(core.error ?? "none")")
        case "org-role":
            // Fill an office: a real `group.setMemberRole`. The office LABEL is this
            // layer's (core has no committee vocabulary), the AUTHORITY is in the log.
            guard let object = q("object"), let peer = q("peer") else { return }
            let office = q("office").flatMap { Office(rawValue: $0) }
            let role = office?.coreRole ?? (q("role") ?? "member")
            NSLog("[devlink] org-role object=\(object.prefix(12)) peer=\(peer.prefix(16)) role=\(role)")
            if core.setOrgRole(object, space: peer, role: role) {
                if let office {
                    Task { await OrgOffices.setHolder(object, office: office,
                                                      space: peer, core: core) }
                }
                NSLog("[devlink] org-role OK")
            } else {
                NSLog("[devlink] org-role FAILED: \(core.error ?? "no error set")")
            }
        case "org-list":
            // Report what this device actually holds, so the harness can assert
            // convergence from the log rather than from a screenshot.
            for o in core.objects where o.kind == "group" {
                let v = core.orgView(o.id)
                let roles = (v?.roles ?? []).map { "\($0.member.prefix(10))=\($0.role)" }.joined(separator: ",")
                NSLog("[devlink] org-list id=\(o.id.prefix(12)) name=\(v?.displayName ?? o.name) "
                      + "shape=\(v?.shape ?? "?") members=\(core.objectMembers(o.id).count) roles=[\(roles)]")
            }
        case "set-profile":
            // Headless drive of a profile EDIT — the same path the Me editors take,
            // so the two-sim harness proves the real fan-out, not a test-only shim.
            // `photo` is base64 (any bytes will do for a transport proof).
            Task {
                if let name = q("name"), !name.isEmpty { core.displayName = name }
                core.editCard { c in
                    if let org = q("org") { c.org = org }
                    if let title = q("title") { c.title = title }
                    if let note = q("note") { c.note = note }
                    if let color = q("color") { c.avatarColor = color }
                    if let photo = q("photo") {
                        c.photo = photo
                        c.photoMime = "image/jpeg"
                        c.capturedAt = UInt64(Date().timeIntervalSince1970)
                    }
                }
                await core.flushProfile()   // skip the debounce: the script is waiting
            }
        case "dump-profiles":
            // Write what we hold for every connection, so the host can diff the two
            // devices and see the profile actually crossed.
            core.dumpPeerProfiles()
        case "export-contact":
            // Re-export a FRESH bundle (new single-use key package) so a mesh
            // harness can add this device to more than one group. See reexportContact.
            core.reexportContact()
        case "sysiphus-signin":
            // Headless drive of the credentialed Sysiphus sign-in (the SystemsView form
            // otherwise needs a tap). The System pulls its Fields -> feeds the catalogue.
            guard let user = q("u"), !user.isEmpty else { return }
            NotificationCenter.default.post(name: .pacificSysiphusSignIn, object: nil,
                                            userInfo: ["u": user, "p": q("p") ?? "aries26"])
        case "dm-send":
            guard let peer = q("peer"), let text = q("text"), !text.isEmpty else { return }
            Task { await core.sendToDM(peer, text) }
        case "dm-react":
            guard let peer = q("peer"), let emoji = q("emoji"), !emoji.isEmpty else { return }
            let convo = Conversation(id: peer, kind: .dm, title: "", tint: .clear)
            guard let target = targetMsg(core, convo, author: q("author"), gen: q("gen")) else { return }
            Task { _ = await core.react(convo, to: target, emoji: emoji, active: q("active") != "0") }
        case "dm-reply":
            guard let peer = q("peer"), let text = q("text"), !text.isEmpty else { return }
            let convo = Conversation(id: peer, kind: .dm, title: "", tint: .clear)
            guard let target = targetMsg(core, convo, author: q("author"), gen: q("gen")) else { return }
            Task { _ = await core.reply(convo, text: text, to: target); await core.syncNow() }
        case "seed-dm-demo":
            guard let peer = q("peer"), !peer.isEmpty else { return }
            Task { await core.seedDmDemo(peer) }
        case "open-dm":
            guard let peer = q("peer"), !peer.isEmpty else { return }
            NotificationCenter.default.post(name: .pacificOpenDM, object: nil, userInfo: ["peer": peer])
        case "new-chat":
            NotificationCenter.default.post(name: .pacificNewChat, object: nil)
        case "arc-join":
            // Tether THIS device to an Arc headlessly — the same join the Settings row runs
            // (POST /v1/signup → sync the Welcome → verify against a gated plane), so a
            // simulator can become a real member without a tap. `id` selects from the
            // directory; omitted = the currently selected Arc.
            guard let arcs, let membership else {
                NSLog("[devlink] arc-join: no arcs/membership store")
                return
            }
            let wanted = q("id")
            Task {
                // Discovery may not have run yet in a fresh simulator, so the directory can be
                // empty while defaults still point at an Arc. Refresh FIRST, then choose —
                // and say out loud which Arc was picked (a silent early return here is what
                // made this look like the link never fired at all).
                await arcs.refresh()
                let arc = (wanted.map { id in arcs.directory.first { $0.id == id } } ?? arcs.selected)
                    ?? arcs.directory.first
                guard let arc else {
                    NSLog("[devlink] arc-join: no Arc to join (directory=%d, selected=nil)", arcs.directory.count)
                    return
                }
                NSLog("[devlink] arc-join: joining %@ (%@)", arc.name, arc.origin)
                let ok = await membership.join(arc, core: core)
                if ok { arcs.select(arc) }
                NSLog("[devlink] arc-join %@ -> %@", arc.name, ok ? "JOINED" : "FAILED")
            }
        case "note-commit":
            // Author a REAL note through NotesStore — exactly what compose→save does: it
            // persists to the union store, so the note appears in the Notes tab. No
            // shortcut: this is the genuine end-to-end entry, not a bare notification.
            guard let text = q("text"), !text.isEmpty, let notes else { return }
            var note = notes.create()
            note.title = q("title") ?? ""
            note.body = text
            notes.update(note)
        case "seed-events":
            // The two real launch events through the REAL pipe (mint + tickets + media
            // with the bundled posters) — same seam as the -pacificSeedEvents launch arg.
            EventSeeder.seed(core: core)
        case "union-language":
            // The Seoul language lane (F-3): persist the ONE union language authority
            // (`pacific://union-language?lang=ko|en`). The heavy half — the store
            // reindex (`LodeLocalStore.migrateStore`) — runs at the NEXT LAUNCH's
            // composition-root reconcile: a live store handle cannot swap models under
            // the producers holding it. For ko the contextual model's assets (an
            // on-demand OS download) are fetched FIRST, so the relaunch cannot land on
            // a missing model and loudly reset back to en.
            guard let lang = q("lang").flatMap(NLLanguageIdentifier.init(rawValue:)) else {
                NSLog("[devlink] union-language: lang must be one of {%@}",
                      NLLanguageIdentifier.allCases.map(\.rawValue).joined(separator: "|"))
                return
            }
            Task {
                if lang == .korean {
                    let available = (try? await ContextualUnionEmbedder.requestAssets(for: .korean)) ?? false
                    NSLog("[devlink] union-language: ko contextual assets %@",
                          available ? "available" : "UNAVAILABLE (setting saved anyway; launch resets to en if still absent)")
                }
                UnionLanguageSetting.save(lang)
                NSLog("[devlink] union-language saved '%@' — relaunch to reindex the union store", lang.rawValue)
            }

        // ---- Ticketing (the two-sim e2e: list → buy → fulfil → door → receipt) ----
        // Every handler below is the SAME call the tapped surface makes; the seam exists
        // because a simulator has no camera and a host script has no fingers.
        case "event-new":
            // EventComposer.save(), headless: mint then setTickets. price=0 (the default)
            // is the FREE path, which this phone fulfils itself; a priced listing needs
            // acct+delegate and the composer gates on the same rule, so it is refused here
            // rather than minted into a listing nothing can complete.
            guard let title = q("title"), !title.isEmpty else { return }
            let start = Int64(q("start") ?? "") ?? Int64(Date().addingTimeInterval(7200).timeIntervalSince1970 * 1000)
            let price = Int64(q("price") ?? "") ?? 0
            let capacity = Int64(q("capacity") ?? "") ?? 1
            let acct = q("acct") ?? "", delegate = q("delegate") ?? ""
            guard price == 0 || (!acct.isEmpty && !delegate.isEmpty) else {
                NSLog("[devlink] event-new REFUSED: a priced listing needs acct+delegate (Slice 2B)")
                return
            }
            guard let id = core.mintEvent(title: title,
                                          descriptor: q("descriptor") ?? "",
                                          startMs: start,
                                          venue: q("venue") ?? "",
                                          lineup: q("lineup")?.removingPercentEncoding ?? "") else {
                NSLog("[devlink] event-new FAILED: \(core.error ?? "no error set")")
                return
            }
            guard core.setEventTickets(id, priceCents: price,
                                       currency: q("currency") ?? "usd",
                                       capacity: capacity,
                                       acct: acct, delegate: delegate) else {
                NSLog("[devlink] event-new tickets FAILED: \(core.error ?? "no error set")")
                return
            }
            core.exportEventId(id)   // dev-event.txt: id + listing, what the buyer addresses
            NSLog("[devlink] event-new id=\(id) listing=\(core.eventListingId(id) ?? "?") "
                  + "price=\(price) capacity=\(capacity)")
        case "ticket-request":
            // The BUYER's half of GET TICKET: request-first, exactly as Life.buyTicket does.
            guard let peer = q("peer"), let listing = q("listing"), !listing.isEmpty else { return }
            let pi = q("pi") ?? ("free-" + UUID().uuidString.lowercased())
            let qty = Int64(q("qty") ?? "") ?? 1
            NSLog("[devlink] ticket-request listing=\(listing) qty=\(qty) pi=\(pi)")
            Task {
                let ok = await core.requestTicket(peer: peer, listing: listing, qty: qty, pi: pi)
                NSLog("[devlink] ticket-request \(ok ? "OK" : "FAILED: " + (core.error ?? "?")) pi=\(pi)")
            }
        case "door-scan":
            // THE DOOR, without a camera: hand the wallet's verbatim payload to the real
            // DoorScanSheet, which runs the identical check → redeem → admit-receipt path
            // a lens would. `object` is the event whose door is open (the sheet always
            // knows it); omitted, it is resolved from the credential's own listing.
            guard let qr = q("qr"), !qr.isEmpty else { return }
            guard let object = q("object") ?? eventForQR(core, qr) else {
                NSLog("[devlink] door-scan: no event holds this credential's listing")
                return
            }
            NSLog("[devlink] door-scan object=\(object.prefix(12))")
            NotificationCenter.default.post(name: .pacificDoorScan, object: nil,
                                            userInfo: ["object": object, "qr": qr])
        case "show-tickets":
            // The wallet; `ticket=<id>` opens that credential full-screen (where the
            // ADMITTED green lives), since a headless drive cannot tap a row.
            var info: [String: String] = [:]
            if let t = q("ticket"), !t.isEmpty { info["ticket"] = t }
            NotificationCenter.default.post(name: .pacificShowTickets, object: nil, userInfo: info)
        case "dump-wallet":
            core.dumpWallet()
        case "dump-tickets":
            core.dumpTicketRequests()
        case "dump-events":
            core.refreshEvents()
            core.dumpEvents()
        case "load-fixture":
            // Fold a QA fixture into real objects (FixtureLoader): the two-sim feed
            // harness drops fixtures/<name>.json in the container, then drives this.
            guard let name = q("name"), !name.isEmpty else { return }
            FixtureLoader.load(name: name, core: core)
        case "attach-media":
            Task { await FixtureLoader.attachMedia(core: core) }
        case "invite":
            // Author a REAL contact.invite for an event this device holds — the
            // counterparty's RSVP channel (setGoing's precondition).
            // `peer` is the identity HEX (what EventInvite passes from connection
            // rows); a space1-address is normalised so the harness can pass its
            // dev-contact.txt space line verbatim (run-3: the space form reached
            // parse_peer_hex and every invite died with ok=0, unread).
            guard var peer = q("peer"), let event = q("event") else { return }
            peer = IdentityKey.hex(peer)
            let row = core.events.first { $0.id == event }
            let invitePeer = peer
            Task {
                let ok = await core.sendInvite(peer: invitePeer, event: event,
                                               title: row?.title ?? "",
                                               startMs: row?.startMs ?? 0,
                                               venue: row?.venue ?? "")
                NSLog("[devlink] invite %@ -> %@ ok=%d", event, invitePeer, ok ? 1 : 0)
                // The §8 bar: a harness must ASSERT the leg, not trust the exit code.
                devWrite("dev-invite.txt", "invite \(event) ok=\(ok ? 1 : 0)\n")
            }
        case "rsvp":
            guard let event = q("event") else { return }
            // A human reaches RSVP through LIFE, whose enterLife() refreshed the
            // invite channels; a headless drive has no page entry, and the 3s tick's
            // refresh() deliberately skips events/invites — so refresh here or
            // setGoing never finds the channel (run-4 finding: ok=0 forever).
            core.refreshEvents()   // chains refreshInvites()
            Task {
                let ok = await core.setGoing(event: event, going: q("going") != "false")
                NSLog("[devlink] rsvp %@ ok=%d", event, ok ? 1 : 0)
                devWrite("dev-rsvp.txt", "rsvp \(event) ok=\(ok ? 1 : 0)\n")
            }
        case "dump-listings":
            // Every listing this device can see (own + travelled), for harness asserts.
            let dir = FileManager.default.urls(for: .applicationSupportDirectory,
                                               in: .userDomainMask)[0]
                .appendingPathComponent("pacific", isDirectory: true)
            core.refreshListings()
            let rows = core.listings.map {
                // photo = b64 length, so the harness can assert media re-gossip
                // (the §14 DS-1 loose end) without shipping bytes through a dump.
                "\($0.origin.prefix(8))|\($0.posture)|\($0.title)|hops=\($0.hops)|photo=\($0.photo.count)"
            }
            try? rows.joined(separator: "\n")
                .write(to: dir.appendingPathComponent("dev-listings.txt"),
                       atomically: true, encoding: .utf8)
            NSLog("[devlink] dump-listings %d rows", rows.count)
        case "show-life":
            NotificationCenter.default.post(name: .pacificShowLife, object: nil)
        case "show-notes":
            NotificationCenter.default.post(name: .pacificShowNotes, object: nil)
        case "show-account":
            NotificationCenter.default.post(name: .pacificShowAccount, object: nil)
        case "bloom":
            NotificationCenter.default.post(name: .pacificTestBloom, object: nil)
        case "my-qr":
            NotificationCenter.default.post(name: .pacificShowQR, object: nil)

        // ---- Project (Work tab) demo links ----
        case "new-project":
            if let id = core.newProject() { core.exportProjectId(id) }
        case "project-add-member":
            guard let object = q("object"), let bundle = q("bundle"), !bundle.isEmpty else { return }
            Task { await core.groupAddMember(object, bundle: bundle) }
        case "seed-project-demo":
            guard let object = q("object"), !object.isEmpty else { return }
            Task { await core.seedProjectDemo(object) }
        case "dump-board":
            guard let object = q("object"), !object.isEmpty else { return }
            core.dumpBoard(object)
        case "open-project":
            guard let object = q("object"), !object.isEmpty else { return }
            NotificationCenter.default.post(name: .pacificOpenProject, object: nil, userInfo: ["object": object])
        case "project-settings", "project-people", "project-item":
            // Present a new authoring sheet on an OPEN project — verification seam so a
            // headless drive can screenshot controls that otherwise only open on a tap.
            guard let object = q("object"), let host = url.host else { return }
            let sheet = host.replacingOccurrences(of: "project-", with: "")
            NotificationCenter.default.post(name: .pacificProjectSheet, object: nil,
                                            userInfo: ["object": object, "sheet": sheet])
        case "seed-objective":
            // Verification seam: author a canned Objectives spine (headline/goal + a
            // supplier stakeholder + a KPI) so the Objectives tab has content to show.
            guard let object = q("object"), !object.isEmpty else { return }
            Task {
                _ = await core.projectAuthor(object, projectSetObjective(
                    headline: "Land AW26 Drop 1 on time",
                    goal: "Ship the drop by 27 Aug with zero crossed codes and 70% sell-through in 4 weeks."))
                _ = await core.projectAuthor(object, projectAddStakeholder(
                    stakeholderId: UUID().uuidString, name: "Fabric supplier", note: "6-week lead time — order by 1 Jul"))
                _ = await core.projectAuthor(object, projectSetKpi(
                    kpiId: UUID().uuidString, label: "Sell-through", target: "70% in 4 weeks"))
                await core.syncNow()
            }
        case "project-tab":
            guard let object = q("object"), let n = q("n") else { return }
            NotificationCenter.default.post(name: .pacificProjectTab, object: nil,
                                            userInfo: ["object": object, "n": Int(n) ?? 0])

        // ---- ChatRoom (threaded Forum) demo links ----
        case "new-room":
            if let id = core.createGroupConversation() { core.exportRoomId(id) }
        case "room-add-member":
            // Generic owner-add — a ChatRoom is a forum object, same MLS hinge.
            guard let object = q("object"), let bundle = q("bundle"), !bundle.isEmpty else { return }
            Task { await core.groupAddMember(object, bundle: bundle) }
        case "room-post":
            // A top-level post (a new thread root).
            guard let object = q("object"), let text = q("text"), !text.isEmpty else { return }
            Task { _ = await core.sendToGroup(object, text); await core.syncNow() }
        case "room-reply":
            // Reply to the post whose text contains `match`, else the thread root.
            guard let object = q("object"), let text = q("text"), !text.isEmpty else { return }
            let convo = Conversation(id: object, kind: .group, title: "", tint: .clear)
            guard let target = roomTarget(core, object, match: q("match")) else { return }
            Task { _ = await core.reply(convo, text: text, to: target); await core.syncNow() }
        case "dump-thread":
            guard let object = q("object"), !object.isEmpty else { return }
            core.dumpThread(object)
        case "open-room":
            guard let object = q("object"), !object.isEmpty else { return }
            NotificationCenter.default.post(name: .pacificOpenRoom, object: nil, userInfo: ["object": object])

        default:
            break
        }
        #endif
    }

    /// Which Event a scanned credential belongs to, when the caller did not say.
    ///
    /// The QR is `pacific://ticket?c=<base64url core>&s=<sig>`; the core names its LISTING,
    /// and a listing belongs to exactly one Event. Nothing is trusted from this — it only
    /// picks which fold to hand the payload to; the signature and the ledger are still
    /// checked in core by `event_door_check`. A real door never needs it (the sheet is
    /// opened from an event), which is why it lives in the dev seam.
    private static func eventForQR(_ core: CoreService, _ qr: String) -> String? {
        guard let rest = qr.split(separator: "?").last,
              let c = rest.split(separator: "&").first(where: { $0.hasPrefix("c=") })?.dropFirst(2)
        else { return nil }
        var b64 = String(c).replacingOccurrences(of: "-", with: "+")
            .replacingOccurrences(of: "_", with: "/")
        b64 += String(repeating: "=", count: (4 - b64.count % 4) % 4)
        guard let data = Data(base64Encoded: b64),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let listing = json["listing"] as? String
        else { return nil }
        return core.events.first { $0.listing == listing }?.id
    }

    /// Resolve a ChatRoom reply target: the post whose text CONTAINS `match`, or —
    /// when `match` is empty/nil — the thread root (the first depth-0 post). This
    /// lets a headless drive reply to the root or to a specific earlier post (for a
    /// nested, depth-2 reply) without knowing its (author, gen). nil if not found.
    private static func roomTarget(_ core: CoreService, _ object: String, match: String?) -> ChatMsg? {
        let rows = core.objectThread(object)
        if let m = match, !m.isEmpty {
            return rows.first(where: { $0.post.text.contains(m) })?.post
        }
        return rows.first(where: { $0.depth == 0 })?.post
    }

    /// Resolve a react/reply target: an explicit `(author, gen)` MsgRef if given,
    /// else the last INBOUND message (the natural thing to react/reply to in a
    /// headless drive). nil when there's nothing to target.
    private static func targetMsg(_ core: CoreService, _ c: Conversation,
                                  author: String?, gen: String?) -> ChatMsg? {
        let msgs = core.messages(c)
        if let a = author, let g = gen, let gv = UInt64(g) {
            return msgs.first(where: { $0.author == a && $0.gen == gv })
        }
        return msgs.last(where: { !$0.mine })
    }
}

#if DEBUG
/// The OTHER doorway to `DevLinks` — the same URLs, delivered without SpringBoard.
///
/// `simctl openurl` stopped being drivable on iOS 26: every custom-scheme open raises an
/// "Open in Pacific?" confirmation, and a host script has no way to tap it (the simulator
/// takes no synthetic touches from the shell). So the two-sim harness APPENDS its
/// `pacific://…` links, one per line, to `dev-commands.txt` in this app's own container —
/// a directory the host already reads dev files out of — and the app drains them here.
///
/// Nothing is bypassed: each line goes through `DevLinks.handle`, the identical handler
/// `.onOpenURL` calls, so a harness step is still exactly the core call the tapped surface
/// makes. The cursor is a consumed-line count, never a truncation, so the host can append
/// while the app is reading. DEBUG only, and the file lives inside the sandbox — a
/// Release build has neither this drain nor the handlers it would feed.
@MainActor
enum DevCommands {
    private static let file = "dev-commands.txt"
    private static let cursorKey = "pacific.dev.commandCursor"

    /// Poll the command file forever (cheap: one stat + a small read). One second, not the
    /// sync tick, because a command must land whether or not the relay is answering.
    static func watch(_ handle: @escaping (URL) -> Void) async {
        while !Task.isCancelled {
            drain(handle)
            try? await Task.sleep(nanoseconds: 1_000_000_000)
        }
    }

    static func drain(_ handle: (URL) -> Void) {
        let dir = FileManager.default
            .urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("pacific", isDirectory: true)
        guard let text = try? String(contentsOf: dir.appendingPathComponent(file), encoding: .utf8)
        else { return }
        let lines = text.split(separator: "\n", omittingEmptySubsequences: true)
        let cursor = UserDefaults.standard.integer(forKey: cursorKey)
        guard lines.count > cursor else { return }
        for line in lines[cursor...] {
            let s = line.trimmingCharacters(in: .whitespaces)
            guard let url = URL(string: s), url.scheme == "pacific" else {
                NSLog("[devcmd] ignored: %@", s)
                continue
            }
            NSLog("[devcmd] %@", s)
            handle(url)
        }
        UserDefaults.standard.set(lines.count, forKey: cursorKey)
    }
}
#endif
