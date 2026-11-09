//  GroupDetailView.swift
//  The SPACE PROFILE — one Group GroupObject's page on the universal Profile
//  grammar (Design/Profile): the headline WALL
//  (icon inline-left with the name, the group's COVER — still, gif or clip —
//  full-bleed behind them, collapsing to the 64pt bar on HeadlineCard's
//  contract), the description band, and the component's six jewels (PEOPLE ·
//  SPACES · CHANNELS · EVENTS · LISTINGS · POSTS) selecting the typed lanes
//  beneath — roster, affiliations, and the objects this Space carries,
//  re-expressed in the one identity bar.
//
//  NO PIN SLOTS (26 Aug). The first three held channels used to sit as bars
//  between the description and the rail, from the Profile spec's pin slots. With
//  a CHANNELS jewel below the rail listing the same rooms, every one of them was
//  on screen twice — the pins were not a summary of the lane, they were a second
//  copy of its first three rows. The lane is the one that can show them all, so
//  the lane is the one that stays.
//
//  A LANE SORTS BY WHAT THE OBJECT IS, not by the edge that reached it. An event
//  shared into this Space arrives as a `peer` affiliation (EventDetail.share) and
//  belongs under EVENTS; reading the rel alone once listed it as a Space.
//
//  Forums are real `forum`-kind GroupObjects; the group's own log carries the
//  edge (`base.setPart`, role room), so every member folds the same room list. A held
//  room opens in the same ThreadView every chat surface uses; a room this
//  device is not in yet renders dimmed — known of, not enterable, honestly.
//
//  Minting/creating/cover-setting are owner-only (the ops are owner/sequenced) —
//  the affordances hide for everyone else rather than offering a guaranteed throw.

import SwiftUI
import PhotosUI
import AVFoundation
import ImageIO
import UniformTypeIdentifiers
import PacificCore

struct GroupDetailView: View {
    @EnvironmentObject private var core: CoreService
    /// The shell hosts the `New` surface — a lane's ＋ raises the same component the
    /// LIFE ＋ does, as an overlay UNDER the disc row, so those three circles become its
    /// buttons here exactly as they do there.
    @EnvironmentObject private var shell: ShellState
    /// The event modal's binder reads the live `happens_at` edge for its venue fact.
    @EnvironmentObject private var graph: GraphService
    /// The read-watermark ledger the CHAT list keeps — a room's unread count is the
    /// same number on both surfaces or it is a second source of truth.
    @EnvironmentObject private var convos: ConversationStore
    let objectId: String

    @State private var view: GroupRow?
    @State private var rooms: [GroupForumRow] = []
    @State private var naming = false
    @State private var roomName = ""
    @State private var minting = false
    @State private var confirmLeave = false
    @State private var transferring = false
    @State private var busy = false
    /// The create flow a lane's ＋ opened — an Event or one of the market faces.
    /// `ThingRow` is not Identifiable, so the editor rides a Bool + row pair — the
    /// MarketplaceView pattern.
    @State private var editingThing: ThingRow?
    @State private var thingEditorOpen = false
    /// The maximised lane — which typed list the ground shows, selected by the
    /// profile jewels row.
    @State private var lane: EventCategory = .events
    /// The headline card's collapse state — HeadlineCard's contract, tap-driven
    /// (the scroll-driven collapse waits on CoverScroll's offset tracking, T3).
    @State private var collapsed = false
    @State private var adminOpen = false
    /// MAY I MINT INTO THIS SPACE — the answer the jewels' ＋ hangs on. Read from the
    /// CORE (`GroupRole::may_mint`) so the affordance cannot disagree with what the
    /// fold will accept, and held in state because it is read once per reload rather
    /// than six times per render (one FFI call per jewel, every frame, was the shape
    /// of the 3s sync-tick storm).
    ///
    /// It resolves to the OWNER today. `Authority` has two rungs, Owner and
    /// AnyMember, so an Admin has none to stand on: they could mint the object and
    /// then fail to attach it — see the rule's own note in pacific-core/src/group.rs.
    /// Widening this to Admin is a core change, and the ＋ follows it for free.
    @State private var mayMint = false
    /// Unread per room, by forum object id — read once per reload, never per render
    /// (a count is a folded transcript, and there is one per room).
    @State private var unreadByRoom: [String: Int] = [:]
    @State private var addingPeople = false
    /// Cover/icon pickers (owner-only affordances).
    @State private var coverItem: PhotosPickerItem?
    @State private var iconItem: PhotosPickerItem?
    /// The cover clip decoded to a temp file once per change — never per frame.
    @State private var coverClipURL: URL?
    /// The cover DECODED, once per change. `banner` used to do base64 -> Data ->
    /// UIImage inside `body`, which during the collapse spring is a decode per
    /// frame — that is what let the ground show through as a flash.
    @State private var coverImage: UIImage?
    @State private var coverGIF: Data?
    /// Where the wall row is on the GLASS, live. The only thing the fade needs: the
    /// row reports its own top edge, and the strip it is falling into is `safeTop`.
    ///
    /// OPTIONAL, not a sentinel. It was seeded at -1 to mean "not measured yet", and
    /// a scrolled row's top edge is LEGITIMATELY negative — so the moment the name
    /// crossed the top of the glass it read as unmeasured and snapped back to full
    /// opacity. nil is the only value that cannot also be a position.
    @State private var rowY: CGFloat?
    @Environment(\.dismiss) private var dismiss

    /// The GROUPS channel's identity — the jewel's own tint, via the one mapping.
    /// (Moved here from the retired GroupsPage; the Space profile is the
    /// surviving Groups surface.)
    static let accent = EventsSeed.tint(.groups)
    static let accentSoft = EventsSeed.tint(.groups).lightened(0.35)

    /// What a group can create: an Event, or a listing on each market face. The raw
    /// value seeds the Thing's category; events mint a real Event GroupObject.
    enum CreateKind: String, Identifiable, CaseIterable {
        case event, job, skill, inventory, trade
        var id: String { rawValue }
        var label: String {
            switch self {
            case .event: return "Event"; case .job: return "Job"; case .skill: return "Skill"
            case .inventory: return "Inventory"; case .trade: return "Trade"
            }
        }
        var glyph: String {
            switch self {
            case .event:     return "calendar"
            case .job:       return MarketFaceCategory.jobs.glyph
            case .skill:     return MarketFaceCategory.skills.glyph
            case .inventory: return MarketFaceCategory.inventory.glyph
            case .trade:     return MarketFaceCategory.trade.glyph
            }
        }
        /// The Thing category this face mints (events aside).
        var category: String {
            switch self {
            case .job: return "job"; case .skill: return "skill"
            default: return "artifact"
            }
        }
        /// The posture a fresh listing opens in: a job is capability WANTED, a skill is
        /// capability OFFERED, inventory is what the group HAS, trade opens on OFFERS.
        var posture: MarketIntent {
            switch self {
            case .job: return .wants
            case .skill: return .offers
            case .inventory: return .has
            case .trade: return .offers
            case .event: return .offers   // unused — events take the composer
            }
        }
    }

    var body: some View {
        // TWO EDGES GOVERN THIS WHOLE STRIP: the top of the glass, and the bottom of
        // the iOS bar. `safeTop` is the second one, read ONCE, here — outside anything
        // that ignores the safe area, which is the only place the number is true — and
        // handed down. Everything in the strip is stated against those two edges:
        //
        //   the fade   runs 0 → safeTop          (screen top, to the bar's bottom)
        //   the row    starts at safeTop         (and fades as it falls back toward 0)
        //   the wall   runs up behind both       (CoverScroll's contract, unchanged)
        //
        // IT USED TO BE INFERRED, TWICE. The chevron was placed by the system and its
        // measured position was read back as the strip's height — a number that is
        // only right if you also know which frame the reader sits in. An overlay in
        // this same ZStack silently read a different one, drew the fade 62pt low, and
        // covered the row it was supposed to sit above. One stated inset beats two
        // inferred ones.
        //
        // The chevron stays a plain sibling, placed by the system rather than padded.
        // That is the CHECK: if `safeTop` is right, the system's chevron and the
        // padded row land on one line, and the screenshot says so.
        GeometryReader { geo in
            let safeTop = geo.safeAreaInsets.top
            ZStack(alignment: .topLeading) {
                page(width: geo.size.width, safeTop: safeTop)
                backChevron
                statusFade(safeTop)
            }
        }
        .background(Theme.bg.ignoresSafeArea())
        .toolbar(.hidden, for: .navigationBar)
        .onAppear(perform: reload)
        .onChange(of: core.syncTick) { _, _ in reload() }
        .alert("New channel", isPresented: $naming) {
            TextField("Name", text: $roomName)
            Button("Create") { mint() }
            Button("Cancel", role: .cancel) { roomName = "" }
        }
        .navigationDestination(for: GroupForumLink.self) {
            ThreadView(objectId: $0.id, title: $0.name).shellBottomBarClearance()
        }
        .sheet(isPresented: $adminOpen) { adminSheet }
        // The lanes' ＋ used to raise EventComposer / ListingEditor as SHEETS here. They
        // now raise the universal `New` surface as a shell overlay, so the three circles
        // are its buttons — which a sheet, sitting above the shell, could never borrow.
        .sheet(isPresented: $thingEditorOpen, onDismiss: { core.refreshThings() }) {
            ListingEditor(editingThing)
        }
        .sheet(isPresented: $addingPeople) {
            AddPeopleSheet(objectId: objectId, label: view?.displayName ?? "this group") { reload() }
        }
        .task(id: coverItem) { await loadPickedCover() }
        .task(id: iconItem) { await loadPickedIcon() }
    }

    /// The scrolling page — the wall and everything under it. It is the sibling that
    /// DOES ignore the top safe area, so the banner runs to the screen's edge.
    private func page(width glass: CGFloat, safeTop: CGFloat) -> some View {
        // Near-full-width cards, 16pt gutters — the ProfileStack rule.
        let width = max(0, glass - 2 * DesignTokens.Life.gutter)
        return CoverScroll {
                // SPACING IS PER-ELEMENT, not the stack's. The jewels row has to sit
                // FLUSH under the card above it — the strip it stands in measures one
                // slot square, and a stack gap would make that strip taller than the
                // square it is supposed to be. Everything else carries its own gap.
                VStack(spacing: 0) {
                    if let v = view {
                        // ONE SURFACE, NO SEAM: the wall and the description band
                        // meet edge to edge, so they read as the top of the page
                        // rather than two stacked cards.
                        //
                        // CoverScroll ignores the top safe area (its wall contract),
                        // and the headline IS the wall: it takes the whole width and
                        // runs to the top edge. The chevron is not in here at all —
                        // it is the safe-area sibling in `body`.
                        VStack(spacing: 0) {
                            headlineCard(v, width: glass, safeTop: safeTop)
                            infoCard(v)
                        }
                    }
                    jewelsRow(width: glass)
                    VStack(spacing: 12) { laneGround(width: width) }
                        .padding(.top, 4)
                }
                .frame(maxWidth: .infinity)
                .padding(.bottom, 24)
        }
    }

    /// The group publishes the fresh object as its own — the `created` edge on the
    /// group's log, which the graph projector folds into (group)-[created]->(object).
    private func attach(_ id: String, name: String) {
        _ = core.attachCreated(objectId, objectId: id, name: name)
        reload()
    }

    private func reload() {
        view = core.orgView(objectId)
        rooms = core.groupForums(objectId)
        mayMint = core.groupMayMint(objectId)
        // The SAME watermark the chat list reads (ConversationStore) — one ledger,
        // so a room you have read in CHAT is read here too. Rooms this device does
        // not hold have no transcript to count.
        unreadByRoom = Dictionary(uniqueKeysWithValues: rooms.filter(\.held).map { room in
            let count = core.messages(Conversation(id: room.id, kind: .group,
                                                   title: "", tint: Self.accent)).count
            return (room.id, convos.unread(room.id, count: count))
        })
        // The lanes' rows resolve against the folded Events/Things they point at.
        core.refreshEvents()
        core.refreshThings()
        decodeCover()
    }

    /// THE ONE PLACE THE COVER IS DECODED. Every branch lands in state, so `body`
    /// only ever reads a ready view — no base64, no UIImage, nothing per frame.
    private func decodeCover() {
        coverClipURL = nil; coverImage = nil; coverGIF = nil
        guard let v = view, !v.cover.isEmpty,
              let data = Data(base64Encoded: v.cover) else { return }
        switch v.coverMime {
        case "video/mp4":
            let url = FileManager.default.temporaryDirectory
                .appendingPathComponent("group-cover-\(objectId.prefix(16)).mp4")
            if (try? data.write(to: url, options: .atomic)) != nil { coverClipURL = url }
        case "image/gif":
            coverGIF = data
        default:
            coverImage = UIImage(data: data)
        }
    }

    private func mint() {
        let name = roomName.trimmingCharacters(in: .whitespaces)
        roomName = ""
        guard !name.isEmpty else { return }
        minting = true
        Task {
            _ = await core.newGroupForum(objectId, name: name)
            reload()
            minting = false
        }
    }

    // MARK: HEADLINE — the animating card (HeadlineCard's space contract, with
    // the REAL cover in the banner slot)

    /// The component's `HeadlineCard` has no banner-content slot (its banner is
    /// an honest gradient until the media ops land), and the group's cover is
    /// real folded media — still, gif or mp4. So the card is composed HERE on
    /// the component's own tokens (`ProfileGeometry`: 2.5:0.6 expanded, 64pt
    /// bar, the one spring, the modal radius) and HeadlineCard's exact collapse
    /// geometry — icon 44→36 sliding to x 32, name 19→15 sliding to centre,
    /// banner fading to the block colour.
    private func headlineCard(_ v: GroupRow, width: CGFloat, safeTop: CGFloat) -> some View {
        let expandedH = ProfileGeometry.headlineHeight(.space, width: width)
        let h = collapsed ? ProfileGeometry.bar : expandedH
        // ONE GROUND, NOT TWO. The block colour used to be its own layer under the
        // cover, and the cover was faded on top of it — so every frame the cover was
        // not fully opaque, the block colour showed through. `banner` is opaque on
        // every branch (still · gif · clip · the colour ground), so it IS the wall's
        // ground and there is nothing behind it left to flash.
        return ZStack(alignment: .top) {
            // THE BANNER IS SIZED, NOT JUST CLIPPED. `scaledToFill` REPORTS the size it
            // would need to fill — for a landscape cover on a 402pt glass that is ~268pt
            // against a 161pt wall — and an unsized child sets the ZStack's height. The
            // outer `.frame` then CENTRED that oversized stack and took the row up with
            // it, ~55pt, which is what put the face, the name and the manage ellipsis
            // under the clock. Giving the banner the wall's own frame keeps the stack
            // exactly one wall tall, so `padding(.top, safeTop)` lands the row where it
            // says it does — on the chevron's line. `.clipped()` still holds the
            // overflowing pixels in.
            banner
                .frame(width: width, height: h + safeTop)
                .clipped()
                .allowsHitTesting(false)                 // media never eats the collapse tap
            // Face · name · manage SCROLL with the wall; only the chevron is pinned.
            // They start at the iOS bar's bottom edge and REPORT where they are, which
            // is the whole of the fade: no scroll offset, no baseline, no second
            // number — the row's own top edge, falling from safeTop toward 0.
            wallRow(v)
                // MEASURED BEFORE IT IS PADDED. Read after the padding, this reports the
                // padded box's top — the glass edge, 0 — so the row was fully faded at
                // rest. The row has to report the ROW.
                .onGeometryChange(for: CGFloat.self) { $0.frame(in: .global).minY } action: {
                    rowY = $0
                }
                .opacity(rowOpacity(safeTop))
                .padding(.top, safeTop)
        }
        .frame(width: width, height: h + safeTop)
        // No rounded rectangle: a wall has no corners. `.clipped()` takes over the job
        // the clipShape was quietly doing — holding `scaledToFill` media in.
        .clipped()
        .contentShape(Rectangle())
        .onTapGesture {
            withAnimation(ProfileGeometry.spring) { collapsed.toggle() }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("group-cover")
    }

    /// THE FADE, over the iOS bar and nothing else. TWO EDGES: full ground at the
    /// top of the glass, gone by the bottom of the bar. That is the whole control —
    /// one gradient, two stops, one height.
    ///
    /// It is a SIBLING that ignores the top safe area, exactly as the scroll does,
    /// because that is what puts a view's origin on the glass. As an `.overlay` on
    /// the page it inherited the page's SAFE-AREA frame instead, started 62pt down,
    /// and drew itself across the row it was meant to sit above.
    ///
    /// The wall still runs to the physical top — CoverScroll's contract is intact and
    /// right; a cover that stopped at the safe area would show the seam that component
    /// exists to remove. This only stops it COMPETING with the clock and the battery.
    private func statusFade(_ safeTop: CGFloat) -> some View {
        LinearGradient(colors: [Theme.bg, Theme.bg.opacity(0)],
                       startPoint: .top, endPoint: .bottom)
            .frame(height: safeTop)
            .frame(maxWidth: .infinity, alignment: .top)
            .ignoresSafeArea(edges: .top)
            .allowsHitTesting(false)          // never eats the wall's collapse tap
            .accessibilityHidden(true)
    }

    /// THE PINNED HALF — the back chevron alone.
    ///
    /// It is the page's own overlay rather than a toolbar item: a button in the real
    /// toolbar is dressed by the system, and on iOS 26 that dressing is a glass disc
    /// which `.buttonStyle(.plain)` does not take off. Being an overlay OUTSIDE
    /// CoverScroll is also what places it — the scroll view ignores the top safe
    /// area, so anything inside it lands under the clock however the inset is read.
    private var backChevron: some View {
        Button { dismiss() } label: {
            Image(systemName: "chevron.left")
                .font(.system(size: 19, weight: .semibold))
                .frame(width: 30, height: Self.chevron, alignment: .leading)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .foregroundStyle(ink)
        .shadow(color: .black.opacity(0.35), radius: 5, y: 1)
        .padding(.leading, 14)
        .accessibilityLabel("Back")
        .accessibilityIdentifier("group-back")
    }

    /// THE SCROLLING HALF — name · manage. These belong to the wall, so they leave
    /// with it; only the chevron stays, because leaving a page is not something you
    /// should have to scroll back up to do.
    ///
    /// NO FACE (26 Aug ruling). The Space's icon came off this row: the wall IS the
    /// Space's face, and a 32pt disc of the same identity sitting on top of it was
    /// the identity said twice. The icon is still the Space's — it dresses its
    /// directory rows and its bars, and it is still set from Manage — it just is not
    /// drawn over its own cover.
    ///
    /// THE NAME TAKES THE CENTRE, and takes it from the WHOLE row, not from what is
    /// left after the slots: a title that recentres depending on whether you happen
    /// to own the Space is a title that moves for reasons the reader cannot see. So
    /// the slots are a layer BESIDE the name rather than boxes in a line with it, and
    /// the name's padding — the two slots plus their clearance — is what keeps it off
    /// them when it is long.
    private func wallRow(_ v: GroupRow) -> some View {
        ZStack {
            Text(v.displayName)
                .font(.system(size: 17, weight: .semibold))
                .lineLimit(1)
                .truncationMode(.tail)
                .padding(.horizontal, 44)
                .frame(maxWidth: .infinity)

            HStack(spacing: 10) {
                // The chevron's slot, held open so a long name cannot run under it.
                // The chevron itself is drawn by the pinned sibling in `body`.
                Color.clear.frame(width: 30, height: Self.chevron)

                Spacer(minLength: 8)

                // Manage stays owner-only: a non-owner has nothing to manage, and the
                // ops behind the sheet are owner-sequenced. What it no longer depends
                // on is the collapse — it is always there for those who can use it.
                if core.isGroupOwner(objectId) {
                    Button { adminOpen = true } label: {
                        Image(systemName: "ellipsis")
                            .font(.system(size: 19, weight: .bold))
                            .frame(width: 30, height: Self.chevron, alignment: .trailing)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Manage")
                    .accessibilityIdentifier("group-admin")
                }
            }
        }
        // ONE ink for the whole row — the banner's inverse — with a shadow so it
        // still reads where a busy photo cover runs under it.
        .foregroundStyle(ink)
        .shadow(color: .black.opacity(0.35), radius: 5, y: 1)
        .padding(.horizontal, 14)
        .frame(height: Self.chevron)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// The row's height — the chevron's, the face's and the ellipsis's alike.
    private static let chevron: CGFloat = 44

    /// THE ROW GOES OUT BEFORE IT GOES UNDER — on the same two edges as the fade.
    /// It rests with its top on the bar's bottom edge and is gone by the time that
    /// edge reaches the top of the glass, so it never shares the strip with the clock.
    /// `rowY` is the row's own reported top; -1 means "not measured yet", which draws
    /// it solid rather than blinking it out on the first frame.
    ///
    /// The chevron does NOT take this: it is pinned, and leaving a page is not
    /// something you should have to scroll back up to do.
    private func rowOpacity(_ safeTop: CGFloat) -> Double {
        guard let rowY, safeTop > 0 else { return 1 }
        return Double(max(0, min(1, rowY / safeTop)))
    }


    /// The wall's ink: whatever is drawn ON the banner reads in its inverse.
    private var ink: Color { (color ?? Self.accent).inverse }


    /// The wall's ground: the group's folded cover — a still, an animated gif, or a
    /// looping clip — or the quiet colour ground when none is published. Reads the
    /// DECODED state (`decodeCover`), never the fold, so a render costs nothing.
    @ViewBuilder private var banner: some View {
        if let coverClipURL {
            LoopingClipPlayer(url: coverClipURL)
        } else if let coverGIF {
            AnimatedGIFView(data: coverGIF)
        } else if let coverImage {
            Image(uiImage: coverImage).resizable().scaledToFill()
        } else {
            bannerGround
        }
    }

    /// No cover yet: the Space's own colours as a quiet ground, never a grey hole.
    ///
    /// It used to fade to `Theme.bg`, which made the wall dissolve into the page
    /// exactly where a published cover would have been solid — the layering changed
    /// depending on whether a cover was set. This is OPAQUE, like every other
    /// branch of `banner`, and it is built from the profile's own colours so it
    /// still claims to be nothing but a stand-in.
    private var bannerGround: some View {
        LinearGradient(colors: [(color ?? Self.accent),
                                (color ?? Self.accent).opacity(0.55)],
                       startPoint: .topLeading, endPoint: .bottomTrailing)
    }

    private func coverButton(_ system: String) -> some View {
        Image(systemName: system)
            .font(.system(size: 14, weight: .semibold))
            .foregroundStyle(.white)
            .frame(width: 32, height: 32)
            .background(Color.black.opacity(0.45), in: Circle())
    }

    // The Space's face used to be composed here for the wall row. It came off the
    // row (see `wallRow`), and with it the base64 decode that fed it — the icon is
    // still folded, still set from Manage, and still drawn wherever a Space is
    // LISTED; this page simply no longer draws it over its own cover.

    private var color: Color? {
        guard let c = view?.avatarColor, !c.isEmpty else { return Self.accent.opacity(0.7) }
        return Color(hex: c)
    }

    /// A picked cover lands as the delta right away: images (gif detected by magic
    /// bytes) go as they are, movies get one low-preset export try to fit the fold's
    /// cap — refused loudly past that, never clamped.
    private func loadPickedCover() async {
        guard let item = coverItem else { return }
        coverItem = nil
        if item.supportedContentTypes.contains(where: { $0.conforms(to: .movie) }) {
            guard let picked = try? await item.loadTransferable(type: PickedCover.self),
                  let data = await Self.fitCoverClip(picked.url) else {
                core.error = "clip exceeds the cover cap"
                return
            }
            if core.setGroupCover(objectId, data: data, mime: "video/mp4") { reload() }
        } else if let data = try? await item.loadTransferable(type: Data.self) {
            let isGif = data.starts(with: Array("GIF8".utf8))
            if core.setGroupCover(objectId, data: data,
                                  mime: isGif ? "image/gif" : "image/jpeg") { reload() }
        }
    }

    private func loadPickedIcon() async {
        guard let item = iconItem else { return }
        iconItem = nil
        guard let data = try? await item.loadTransferable(type: Data.self) else { return }
        if core.setGroupIcon(objectId, jpeg: data) { reload() }
    }

    /// ≤ the fold's 1.5M-b64 cap: the raw pick if it already fits, else ONE
    /// low-preset, 5-second export try (the EventComposer clip discipline).
    private static func fitCoverClip(_ url: URL) async -> Data? {
        let cap = 1_500_000
        if let d = try? Data(contentsOf: url), d.base64EncodedString().count <= cap { return d }
        let asset = AVURLAsset(url: url)
        guard let session = AVAssetExportSession(asset: asset,
                                                 presetName: AVAssetExportPresetLowQuality) else { return nil }
        let duration = (try? await asset.load(.duration)) ?? .zero
        session.timeRange = CMTimeRange(start: .zero,
                                        duration: CMTimeMinimum(duration, CMTime(seconds: 5, preferredTimescale: 600)))
        let out = FileManager.default.temporaryDirectory
            .appendingPathComponent("cover-export-\(UUID().uuidString).mp4")
        defer { try? FileManager.default.removeItem(at: out) }
        guard (try? await session.export(to: out, as: .mp4)) != nil,
              let d = try? Data(contentsOf: out),
              d.base64EncodedString().count <= cap else { return nil }
        return d
    }

    // MARK: INFO — the description, and only when there is one to give

    @ViewBuilder private func infoCard(_ v: GroupRow) -> some View {
        if !v.note.isEmpty {
            InfoCard(info: ProfileInfo(desc: v.note))
        }
    }

    /// A ROOM BAR: its name, and what is waiting in it.
    ///
    /// No disc — a Forum has no icon op, so a circle of initials would invent a
    /// face the object cannot have. No trailing word either: every bar on the lane
    /// said "CHANNEL" under a jewel that already says CHANNEL. The slot carries the
    /// one thing that differs per room and changes minute to minute — your unread
    /// count — and stays empty when there is nothing waiting.
    private func channelBar(_ room: GroupForumRow, width: CGFloat) -> some View {
        let name = room.name.isEmpty ? "Channel" : room.name
        let unread = unreadByRoom[room.id] ?? 0
        return spaceBar(title: name,
                        label: "",
                        avatar: false,
                        unread: unread,
                        width: width)
            .accessibilityElement(children: .ignore)
            // SPOKEN, the pill is a bare number after a name and means nothing. The
            // count says what it counts here, which is also what makes it assertable
            // from the two-simulator harness — the number on this bar is the thing
            // under test when a room is read on another device.
            .accessibilityLabel(unread > 0 ? "\(name) \(unread) unread" : name)
            .accessibilityIdentifier("group-forum-\(room.id.prefix(12))")
    }

    // MARK: JEWELS — the shared rail, selecting the lane on the ground

    private func jewelsRow(width: CGFloat) -> some View {
        ProfileJewelRail(isActive: { lane == $0 },
                         onTap: { cat in
                             withAnimation(.spring(response: 0.36, dampingFraction: 0.78)) {
                                 lane = cat
                             }
                         },
                         idPrefix: "group-jewel",
                         width: width,
                         canMint: { mayMint && mintKind(for: $0) != nil },
                         onMint: { if let kind = mintKind(for: $0) { beginNew(kind) } })
    }

    /// WHAT THIS SPACE MINTS ON A LANE — nil where the lane names something a Space
    /// cannot author, and those three are not oversights:
    ///
    ///   PEOPLE  — the MLS roster. You ADD a member (AddPeopleSheet); a person is
    ///             not an object a Space mints.
    ///   SPACES  — a federation EDGE (`group.setAffiliation`), authored between two
    ///             Groups that already exist.
    ///   POSTS   — a delta INSIDE a channel, so the ＋ for one belongs in the room.
    private func mintKind(for cat: EventCategory) -> NewKind? {
        switch cat {
        case .channels: return .channel
        case .events:   return .event
        case .trade:    return .listing
        default:        return nil
        }
    }

    // MARK: LANES — the maximised jewel's typed list

    @ViewBuilder private func laneGround(width: CGFloat) -> some View {
        // NO HEAD ROW (26 Aug). The word repeated what the lit jewel above
        // already says, and the `+` beside it was a per-lane mint on a page
        // whose species the universal New surface owns. The space is content's.
        switch lane {
        case .people:
            memberBars(width: width)
        case .groups:
            affiliationBars(width: width)
        case .events, .trade:
            createdBars(width: width)
        case .channels:
            // The full channel list (pins carry only the first three). Ghost
            // rooms render dimmed: known from the log, not yet joined.
            //
            // This used to live under POSTS, on the reasoning that "the group's
            // posts ARE its channels' activity". They are not the same thing: a
            // channel is a Forum GroupObject, a post is a delta inside one, and
            // one lane cannot name both. CHANNELS is its own jewel now.
            if rooms.isEmpty {
                emptyLane(.channels)
            }
            ForEach(rooms, id: \.id) { room in
                if room.held {
                    NavigationLink(value: GroupForumLink(id: room.id, name: room.name)) {
                        channelBar(room, width: width)
                    }
                    .buttonStyle(.plain)
                    .contextMenu {
                        if core.isGroupOwner(objectId) {
                            Button(role: .destructive) {
                                Task { _ = await core.detachGroupForum(objectId, forumId: room.id); reload() }
                            } label: {
                                Label("Detach", systemImage: "minus.circle")
                            }
                        }
                    }
                } else {
                    channelBar(room, width: width).opacity(0.4)
                }
            }
        case .posts:
            // A post is a delta INSIDE a channel. Nothing rolls posts up across a
            // Space's channels yet, so the honest word — not the channel list
            // wearing the wrong name, which is what this lane used to show.
            emptyLane(.posts)
        default:
            EmptyView()
        }
    }

    /// UNHOSTED (26 Aug). `beginNew`, `attach` and the add-people sheet are the
    /// Space's mint machinery, and they are correct — the two-delta mint+attach
    /// doctrine below is the part worth keeping. What they lost is their BUTTON:
    /// the lane head row that carried the ＋ is gone, and where the create
    /// affordance re-lands (the universal New surface is the standing answer) is
    /// the next ruling. They are deliberately not deleted with the row.
    ///
    /// Raise the universal `New` surface for something belonging to THIS Space.
    private func beginNew(_ kind: NewKind) {
        let space = objectId
        shell.beginNew(NewSession(kind: kind, parent: space) { draft in
            // A CHANNEL is minted and attached by ONE core call — `group_forum_new` does
            // `object_new` + both halves of `part_of` together, and it is the only kind
            // with a purpose-built op for belonging. Minting it through the generic seam
            // and linking after would create the object twice.
            if kind == .channel {
                guard await core.newGroupForum(space, name: draft.name) != nil else { return nil }
                reload()
                return .saved
            }
            // Everything else: mint, then ATTACH — the SAME `attach` the EventComposer
            // and ListingEditor sheets already used, which publishes the fresh object as
            // the Space's own (`created` edge, folded to (group)-[created]->(object)).
            //
            // Two deltas on purpose: they carry different authority, and folding them
            // into one call would make a failure to attach indistinguishable from a
            // failure to mint. An object that exists but did not attach is visible and
            // recoverable; a silent half-mint is not.
            guard let id = core.mint(kind: kind.rawValue, draft: draft.mint(for: kind))
            else { return nil }
            attach(id, name: draft.name)
            return .saved
        })
    }

    // MARK: PEOPLE — the MLS roster in the identity bar, routing to the person page

    @ViewBuilder private func memberBars(width: CGFloat) -> some View {
        // UNDER THEIR ROLE. The role used to ride each bar's trailing slot, which
        // wrote the same word on every card of a run and left it BLANK for the many
        // members nobody has roled — the overlay is optional, so blank is the common
        // case. A role is a property of the RUN, so it heads the run instead, and the
        // bar's trailing slot goes quiet rather than repeating the word above it.
        ForEach(memberSections) { section in
            roleHeading(section.role)
            ForEach(section.members, id: \.self) { space in
                memberBar(space, width: width)
            }
        }
    }

    /// One run of the roster: the members playing `role`, under its word.
    struct RoleRun: Identifiable {
        let role: String
        let members: [String]
        var id: String { role }
    }

    /// The roster grouped by the role each member plays, in standing order.
    ///
    /// TWO DEFAULTS RESOLVE HERE, once. The role overlay is OPTIONAL by design —
    /// the core answers `None` for anyone nobody has roled, and that means "no
    /// overlay", NOT "no access" (`Node::member_role`) — so an unroled roster member
    /// is a Member. And the OWNER is the object's owner, not a `setMemberRole`
    /// overlay: ownership is carried by the object from genesis, while most groups
    /// carry no overlay at all, so reading `roles` alone would head a Space with no
    /// owner and file its founder under Member.
    ///
    /// A role string this build does not know still gets a run of its own, last,
    /// under its own word: the CORE owns the vocabulary (`GroupRole`,
    /// pacific-core/src/group.rs) and may widen it, and a member must never fall out
    /// of PEOPLE because the app has not caught up.
    ///
    /// Pure, and `static`, so the bucketing can be tested without a CoreService.
    static func roleRuns(roster: [String], ownerHex: String,
                         roles: [GroupRoleRow]) -> [RoleRun] {
        // Compare hex, never the written form — IdentityKey's rule. The roster
        // arrives as `ed25519:…`, `objectOwner` as bare hex, and a role row carries
        // whichever rendering its fold was written in.
        let owner = IdentityKey.hex(ownerHex)
        let overlay = Dictionary(roles.map { (IdentityKey.hex($0.member), $0.role) },
                                 uniquingKeysWith: { first, _ in first })
        var runs: [String: [String]] = [:]
        for space in roster {
            let h = IdentityKey.hex(space)
            let role = h == owner ? "owner" : (overlay[h] ?? "member")
            runs[role, default: []].append(space)
        }
        let known = ["owner", "admin", "member", "viewer", "guest"]
        let unknown = runs.keys.filter { !known.contains($0) }.sorted()
        return (known + unknown).compactMap { role in
            runs[role].map { RoleRun(role: role, members: $0) }
        }
    }

    private var memberSections: [RoleRun] {
        Self.roleRuns(roster: core.orgMembers(objectId),
                      ownerHex: core.objectOwner(objectId),
                      roles: view?.roles ?? [])
    }

    /// The word over a run. Uppercase — how a bar's trailing slot has always written
    /// a role — and quiet, a hair inside the cards it heads. The extra top pad is
    /// what binds it to the run BELOW it rather than floating between two.
    private func roleHeading(_ role: String) -> some View {
        Text(role.uppercased())
            .font(.system(size: 12, weight: .semibold))
            .tracking(0.6)
            .foregroundStyle(Theme.textTertiary)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 20)
            .padding(.top, 8)
            .accessibilityIdentifier("group-role-\(role)")
    }

    private func memberBar(_ space: String, width: CGFloat) -> some View {
        let isMe = hex(of: space) == core.myIdentityHex
        // WHICH ONE IS ME. The roster renders every member the same way — same bar,
        // same face, same name — so on your own Space you are one card among many
        // with nothing saying which. The marker rides the NAME rather than the
        // trailing slot, because it is part of naming the person, not a second fact
        // about them (and the trailing slot now belongs to nothing at all). It is
        // the ROW's mark, not a string spliced into the name: dressed as something
        // the app wrote, it cannot be misread as a name someone chose.
        return NavigationLink(value: PersonLink(id: space)) {
            spaceBar(title: core.displayName(forSpace: space),
                     label: "",
                     photo: isMe ? core.avatarPhoto : core.index.photo(space),
                     color: isMe ? core.avatarColor : core.index.color(space),
                     mark: isMe ? "(me)" : nil,
                     width: width)
                .accessibilityIdentifier("group-member-\(hex(of: space).prefix(12))")
        }
        .buttonStyle(.plain)
        .contextMenu {
            // Removing is the owner's alone, and never of themselves —
            // that is a handover, offered at the leave flow instead.
            if core.isGroupOwner(objectId), !isMe {
                Button(role: .destructive) {
                    Task { _ = await core.removeMember(objectId, memberHex: hex(of: space)) }
                } label: { Label("Remove", systemImage: "person.badge.minus") }
            }
        }
    }

    // MARK: SPACES — the federation edges (this group's half of each affiliation)

    @ViewBuilder private func affiliationBars(width: CGFloat) -> some View {
        // The rel is NOT the lane. `EventDetail.share` writes a `peer` edge for an
        // event shared into this Space — deliberately, so the promoter's `created`
        // claim and ours can coexist — and reading rel alone listed that event as a
        // Space. A lane sorts by what the object IS: anything `bucket` can name is
        // an event or a listing and belongs in its own lane. An object this device
        // does not hold has no readable kind, so it stays here, dimmed below.
        let rows = affiliationRows.filter {
            ["parent", "child", "peer"].contains($0.rel) && bucket($0) == nil
        }
        if rows.isEmpty { emptyLane(.groups) }
        ForEach(rows, id: \.peer) { row in
            let bar = spaceBar(title: row.name.isEmpty ? String(row.peer.prefix(8)) : row.name,
                               label: row.rel.uppercased(), width: width)
                .accessibilityIdentifier("group-affiliate-\(row.peer.prefix(12))")
            if core.objects.contains(where: { $0.id == row.peer }) {
                NavigationLink(value: GroupLink(id: row.peer)) { bar }
                    .buttonStyle(.plain)
            } else {
                bar.opacity(0.4)
            }
        }
    }

    // MARK: EVENTS · LISTINGS — the objects this Space carries, minted or shared

    @ViewBuilder private func createdBars(width: CGFloat) -> some View {
        // EVERY affiliation whose object is of this lane's kind, not just the
        // `created` ones: an event shared into the Space arrives as a `peer` edge
        // and is still an event.
        let rows = affiliationRows.filter { bucket($0) == lane }
        let ghosts = lane == .events ? unresolvedCreatedRows : []
        if rows.isEmpty && ghosts.isEmpty { emptyLane(lane) }
        ForEach(rows, id: \.peer) { row in
            // AN EVENT IS A CARD, the FEED's card — the square with its media, its
            // venue and its stamp — not a collapsed bar. The bar is the identity
            // shape (a person, a Space, a channel: things you OPEN); an event is a
            // thing that HAPPENS, and the feed already says what that looks like.
            // `ObjectCard` is that component and this page is now one of its
            // adopters, reusing the frame rather than re-drawing it.
            if lane == .events, let card = eventCard(row) {
                Button { open(row) } label: { ObjectCard(row: card) }
                    .buttonStyle(.plain)
                    .accessibilityIdentifier("group-created-\(row.peer.prefix(12))")
            } else {
                Button { open(row) } label: { createdBar(row, width: width) }
                    .buttonStyle(.plain)
            }
        }
        // Created objects the group's log names but this device does not hold
        // yet — rendered, not enterable, exactly like an unheld forum room.
        ForEach(ghosts, id: \.peer) { row in
            createdBar(row, width: width).opacity(0.4)
        }
    }

    /// This affiliation's Event as the feed's row, or nil where the device does not
    /// hold the object. The projection is `FeedAssembler.item(for:)` — the SAME one
    /// the feed and the typed lists use — so a card here can never drift from a card
    /// there. The date rule is deliberately not applied: a Space's page lists the
    /// events that Space carries, and a night that already happened still happened.
    private func eventCard(_ row: AffiliationRow) -> FeedItem? {
        guard let event = core.events.first(where: { $0.id == row.peer }) else { return nil }
        return FeedAssembler.item(for: event, maker: core.index.maker(of: event.id))
    }

    private func createdBar(_ row: AffiliationRow, width: CGFloat) -> some View {
        spaceBar(title: row.name.isEmpty ? String(row.peer.prefix(8)) : row.name,
                 label: createdLabel(row), width: width)
            .accessibilityIdentifier("group-created-\(row.peer.prefix(12))")
    }

    /// Every affiliation this Space carries, whatever the edge. The lanes sort
    /// them by the KIND of the object at the far end (`bucket`).
    private var affiliationRows: [AffiliationRow] {
        view?.affiliations ?? []
    }

    /// The Space's own mints — the `created` half, which is the only edge whose
    /// object we can name before holding it (we wrote it).
    private var createdRows: [AffiliationRow] {
        affiliationRows.filter { $0.rel == "created" }
    }

    /// Created edges whose object this device does not hold — shown as ghosts.
    private var unresolvedCreatedRows: [AffiliationRow] {
        createdRows.filter { row in
            !core.events.contains(where: { $0.id == row.peer })
                && !core.things.contains(where: { $0.id == row.peer })
        }
    }

    /// Which jewel a created object belongs to: events by kind; every market
    /// face folds into LISTINGS (the bar's trailing label keeps the face).
    private func bucket(_ row: AffiliationRow) -> EventCategory? {
        if core.events.contains(where: { $0.id == row.peer }) { return .events }
        guard core.things.contains(where: { $0.id == row.peer }) else { return nil }
        return .trade
    }

    /// The bar's trailing word — the object's face, kept legible now the four
    /// market lanes share one LISTINGS jewel.
    private func createdLabel(_ row: AffiliationRow) -> String {
        if core.events.contains(where: { $0.id == row.peer }) { return "EVENT" }
        if let thing = core.things.first(where: { $0.id == row.peer }) {
            switch thing.category {
            case "skill": return "SKILL"
            case "job":   return "JOB"
            default:      return thing.posture == "has" ? "INVENTORY" : "TRADE"
            }
        }
        return "CREATED"
    }

    private func open(_ row: AffiliationRow) {
        if core.events.contains(where: { $0.id == row.peer }) {
            EventDetail.open(row.peer, core: core, graph: graph, shell: shell)
        } else if let thing = core.things.first(where: { $0.id == row.peer }) {
            editingThing = thing
            thingEditorOpen = true
        }
    }

    // MARK: The one identity bar — collapsed headline ≡ directory row ≡ pin bar

    /// `ConversationRow` worn thin inside the 64pt card: the component PinBar's
    /// exact chrome, composed here because `ProfilePinKind` has no channel/rel
    /// vocabulary for the trailing word — and WITHOUT children-combine, so the
    /// title stays a queryable staticText (the UITest oracle).
    private func spaceBar(title: String, label: String,
                          photo: Data? = nil, color: Color? = nil,
                          mark: String? = nil,
                          avatar: Bool = true, unread: Int = 0,
                          width: CGFloat) -> some View {
        ConversationRow(title: title,
                        snippet: "",
                        time: label,
                        unread: unread,
                        tint: Self.accent,
                        initials: Avatar.initials(title),
                        photo: photo,
                        blockColor: color,
                        enrichSnippet: false,
                        avatarSize: 36,
                        avatar: avatar,
                        mark: mark)
            .padding(.horizontal, 14)
            .frame(width: width, height: ProfileGeometry.bar)
            .background(Theme.surface)
            .clipShape(ProfileGeometry.shape)
            .overlay(ProfileGeometry.shape.strokeBorder(Theme.hairline))
            .contentShape(ProfileGeometry.shape)
    }

    /// WHAT A LANE WITH NOTHING IN IT SAYS: the jewel's own word, in the type every
    /// empty surface in the app wears (`EmptyWord`). It used to say "Nothing yet",
    /// "No channels" or "No posts yet" — three phrasings of one condition, on one
    /// page, none of them the lane's name.
    ///
    /// The word takes a third of the GLASS and sits a fifth of that above its own
    /// centre: CoverScroll's house rule (never read an inset under
    /// `ignoresSafeArea`), and the same placement NOTE and CHAT use.
    private func emptyLane(_ cat: EventCategory) -> some View {
        EmptyWord(cat.title, scale: .lane)
            .containerRelativeFrame(.vertical) { h, _ in h / 3 }
    }

    // MARK: MANAGE — admin off the profile surface (11 Aug order): the face,
    // the cover, and the way out live behind the headline's one circle.

    private var adminSheet: some View {
        NavigationStack {
            VStack(spacing: 4) {
                PhotosPicker(selection: $iconItem, matching: .images) {
                    manageRow("person.crop.circle", "Icon")
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("group-icon-set")
                PhotosPicker(selection: $coverItem, matching: .any(of: [.images, .videos])) {
                    manageRow("photo.on.rectangle.angled", "Cover")
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("group-cover-set")
                if SiteDraft.kindHasAPage(NewKind.space.rawValue) {
                    NavigationLink { SitePageView(objectId: objectId) } label: { manageRow("globe", "Page") }
                        .buttonStyle(.plain)
                        .accessibilityIdentifier("group-page")
                }
                if let v = view, !v.cover.isEmpty {
                    Button {
                        _ = core.setGroupCover(objectId, data: nil, mime: "")
                        reload()
                    } label: { manageRow("xmark.circle", "Remove cover") }
                    .buttonStyle(.plain)
                }
                Spacer(minLength: 12)
                leaving
            }
            .padding(.top, 14)
            .padding(.bottom, 18)
            .background(Theme.bg.ignoresSafeArea())
            .navigationTitle("Manage")
            .navigationBarTitleDisplayMode(.inline)
            .confirmationDialog("Leave group?", isPresented: $confirmLeave, titleVisibility: .visible) {
                Button("Leave", role: .destructive) { Task { await leave() } }
                Button("Cancel", role: .cancel) {}
            }
            .sheet(isPresented: $transferring) { transfer }
        }
        .presentationDetents([.medium])
    }

    private func manageRow(_ glyph: String, _ label: String) -> some View {
        HStack(spacing: 14) {
            Image(systemName: glyph)
                .font(.system(size: 16, weight: .semibold))
                .frame(width: 36)
            Text(label)
                .font(.system(size: 17))
            Spacer(minLength: 0)
        }
        .foregroundStyle(Theme.textPrimary)
        .padding(.horizontal, 20)
        .padding(.vertical, 11)
        .contentShape(Rectangle())
    }

    // MARK: LEAVING — a departure is an event, never a silent roster overwrite

    /// Leaving authors `base.memberLeft` about yourself, so it needs neither the
    /// owner nor a network round-trip to be true. The owner is routed through a
    /// handover first: the spine only accepts the owner's sequenced deltas, so an
    /// object whose owner walked could never be renamed, re-roled or re-affiliated
    /// again. A group of one skips it — nobody to hand it to, nothing to freeze.
    private var leaving: some View {
        Button(role: .destructive) {
            if core.mustHandOverBeforeLeaving(objectId) { transferring = true } else { confirmLeave = true }
        } label: {
            HStack(spacing: 14) {
                Image(systemName: "rectangle.portrait.and.arrow.right")
                    .font(.system(size: 16, weight: .semibold))
                    .frame(width: 36)
                Text("Leave group")
                    .font(.system(size: 17))
                Spacer(minLength: 0)
            }
            .foregroundStyle(Theme.danger)
            .padding(.horizontal, 20)
            .padding(.vertical, 11)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(busy)
        .accessibilityIdentifier("group-leave")
        .overlay(alignment: .top) {
            Rectangle().fill(Theme.hairline.opacity(0.6)).frame(height: 0.5).padding(.horizontal, 20)
        }
    }

    /// The successor picker. Tapping a member hands the group over and then leaves,
    /// in that order — the second delta is what the first one makes legal.
    private var transfer: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 0) {
                    ForEach(others, id: \.self) { space in
                        Button {
                            Task { await handOverThenLeave(to: hex(of: space)) }
                        } label: {
                            HStack(spacing: 14) {
                                Avatar(initials: Avatar.initials(core.displayName(forSpace: space)),
                                       tint: Self.accent, size: 36)
                                Text(core.displayName(forSpace: space))
                                    .font(.system(size: 17))
                                    .foregroundStyle(Theme.textPrimary)
                                    .lineLimit(1)
                                Spacer(minLength: 0)
                            }
                            .padding(.horizontal, 20)
                            .padding(.vertical, 9)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .accessibilityIdentifier("group-successor-\(hex(of: space).prefix(12))")
                        .overlay(alignment: .top) {
                            Rectangle().fill(Theme.hairline.opacity(0.6)).frame(height: 0.5)
                                .padding(.horizontal, 20)
                        }
                    }
                }
                .padding(.top, 8)
            }
            .scrollIndicators(.hidden)
            .background(Theme.bg.ignoresSafeArea())
            .navigationTitle("Transfer ownership")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { transferring = false }
                }
            }
        }
        .disabled(busy)
    }

    /// Everyone but me — the candidates for succession.
    private var others: [String] {
        core.orgMembers(objectId).filter { hex(of: $0) != core.myIdentityHex }
    }

    private func hex(of space: String) -> String {
        IdentityKey.hex(space)
    }

    private func leave() async {
        busy = true
        defer { busy = false }
        if await core.leaveGroup(objectId) { dismiss() }
    }

    /// Hand over, then leave. If the handover is rejected we stop: leaving would be
    /// rejected too, and stranding the group ownerless is the one outcome worth
    /// refusing outright.
    private func handOverThenLeave(to memberHex: String) async {
        busy = true
        defer { busy = false }
        guard await core.handOverGroup(objectId, toMemberHex: memberHex) else { return }
        transferring = false
        if await core.leaveGroup(objectId) { dismiss() }
    }

}

// MARK: - Animated GIF (a cover format Image cannot animate)

/// ImageIO frames into a UIImageView-driven animation — no web view, no third-party
/// decoder. A single-frame "gif" degrades to its still.
struct AnimatedGIFView: UIViewRepresentable {
    let data: Data

    func makeUIView(context: Context) -> UIImageView {
        let v = UIImageView()
        v.contentMode = .scaleAspectFill
        v.clipsToBounds = true
        v.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        v.setContentCompressionResistancePriority(.defaultLow, for: .vertical)
        v.image = Self.animated(from: data)
        return v
    }

    func updateUIView(_ v: UIImageView, context: Context) {
        v.image = Self.animated(from: data)
    }

    private static func animated(from data: Data) -> UIImage? {
        guard let src = CGImageSourceCreateWithData(data as CFData, nil) else { return nil }
        let count = CGImageSourceGetCount(src)
        guard count > 1 else { return UIImage(data: data) }
        var frames: [UIImage] = []
        var duration = 0.0
        for i in 0..<count {
            guard let cg = CGImageSourceCreateImageAtIndex(src, i, nil) else { continue }
            frames.append(UIImage(cgImage: cg))
            duration += delay(src, i)
        }
        guard !frames.isEmpty else { return UIImage(data: data) }
        return UIImage.animatedImage(with: frames, duration: max(duration, 0.5))
    }

    private static func delay(_ src: CGImageSource, _ index: Int) -> Double {
        let props = CGImageSourceCopyPropertiesAtIndex(src, index, nil) as? [CFString: Any]
        let gif = props?[kCGImagePropertyGIFDictionary] as? [CFString: Any]
        let unclamped = gif?[kCGImagePropertyGIFUnclampedDelayTime] as? Double
        let clamped = gif?[kCGImagePropertyGIFDelayTime] as? Double
        let d = unclamped ?? clamped ?? 0.1
        return d < 0.011 ? 0.1 : d
    }
}

/// A picked cover movie as a temp-file URL — `Data` would buffer the original in
/// memory just to hand it to an exporter that wants a file anyway.
private struct PickedCover: Transferable {
    let url: URL

    static var transferRepresentation: some TransferRepresentation {
        FileRepresentation(contentType: .movie) {
            SentTransferredFile($0.url)
        } importing: { received in
            let dst = FileManager.default.temporaryDirectory
                .appendingPathComponent("picked-cover-\(UUID().uuidString).\(received.file.pathExtension)")
            try FileManager.default.copyItem(at: received.file, to: dst)
            return PickedCover(url: dst)
        }
    }
}
