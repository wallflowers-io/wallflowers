//  ProfileBinding.swift
//  The REAL `ProfileSource` — the CoreService-backed binder over the FFI read
//  models (T5b). The Design/Profile module declares the seam; this file is the
//  host's concrete, injected at M1 in place of `FixtureProfileSource`.
//
//  Bindings:
//    name       — per-kind: person `displayName` · group `displayName` (roster-
//                 derived label when unset) · place `name` · event `title`.
//                 Five field names, unified HERE, never at the schema.
//    bio/desc   — person/group bind `ContactCard.note`; place/event bind their
//                 one-line `descriptor`.
//    icon       — resolve order clip → photo → colour+initials (group.rs:275-284).
//    banner     — group `cover` / event `banner` where they exist; HONEST-NIL
//                 where they don't (person and place have no banner primitive —
//                 §5 decision 6 is a pending ruling, so the kind stays `.block`
//                 and no placebo bytes are invented).
//    since      — honest-nil: memberJoined is written by core's Add door and at
//                 mint (membership-through-mls.md §10.3), but no tenure read model crosses the
//                 FFI yet (`MembershipLog::tenures` stays core-side), so the
//                 binder cannot honestly answer.
//    connect    — `ProfilePeerStatus` from `ConnectionRow` (see the gap note on
//                 `RealProfileSource.peerStatus`).
//    posts      — `place.post` rows, the only wall today (map §3).
//
//  Split for testability: `ProfileBind` is the PURE half — total functions from
//  FFI rows to card values, unit-tested against constructed rows with no core —
//  and `RealProfileSource` is the thin fetch-and-delegate that conforms to
//  `ProfileSource`.

import SwiftUI
import PacificCore

// MARK: - Subject

/// What the profile page is ABOUT — the host-side address the binder resolves.
/// `ProfileKind` (person|space) is what the page RENDERS AS; a place or an
/// event renders as a Space (map surfaces C/D: the located-Space profile and
/// the event shell), so the five subjects fold onto the two kinds.
enum ProfileSubject: Hashable {
    case me
    /// A connection, by peer id — space-form ("space1…") or bare identity hex
    /// both resolve (rosters carry hex, `connections()` carries space-form).
    case peer(String)
    case group(String)
    case place(String)
    case event(String)
}

// The connect circle's state type is `ProfilePeerStatus` from the component
// seam (Design/Profile/ProfileModels.swift) — four-valued to match core's
// semantics. Only `ConnectionRow.connected: Bool` crosses the FFI today, so
// this binder can honestly produce just nil / .pending / .connected of it.

// MARK: - The pure bindings

/// Total functions FFI rows → card values. No core handle, no I/O — the half
/// `ProfileBindingTests` drives with constructed rows.
enum ProfileBind {

    // ---- icon: clip → photo → colour+initials (group.rs:275-284) ----------

    /// The ONE resolved face for surfaces that need a single answer (the
    /// collapsed bar, a pin). A slot that is present but undecodable falls
    /// through to the next — a corrupt clip must not blank the face.
    enum Icon: Equatable {
        case clip(Data)
        case photo(Data)
        case color(String)   // "RRGGBB"
    }

    static func icon(clipB64: String, photoB64: String, colorHex: String) -> Icon {
        if let d = decode(clipB64) { return .clip(d) }
        if let d = decode(photoB64) { return .photo(d) }
        return .color(blockHex(colorHex))
    }

    /// "" → nil (absent), non-base64 → nil (undecodable), else the bytes.
    private static func decode(_ b64: String) -> Data? {
        guard !b64.isEmpty else { return nil }
        return Data(base64Encoded: b64)
    }

    /// The block colour with the documented default when the card never chose
    /// one — the same fallback the self avatar uses (Avatar atom behaviour,
    /// not a fabricated datum: the disc must be SOME colour to exist).
    static func blockHex(_ hex: String) -> String {
        hex.isEmpty ? CoreService.defaultAvatarColorHex : hex
    }

    // ---- banner ------------------------------------------------------------

    /// The banner slot's kind from real media — nil when no banner media
    /// exists (honest-nil; the headline then renders `.block`, the ground
    /// built from the profile's own colours).
    static func bannerKind(b64: String, mime: String) -> ProfileBannerKind? {
        guard !b64.isEmpty else { return nil }
        switch mime {
        case "image/gif":                          return .gif
        case let m where m.hasPrefix("video/"):    return .video
        case let m where m.hasPrefix("image/"):    return .photo
        default:                                   return nil
        }
    }

    /// Two-letter initials from a display name — `CoreService.initials`'
    /// algorithm, hoisted so every kind derives the badge the same way.
    static func initials(_ name: String) -> String {
        let parts = name.split(separator: " ")
        guard let f = parts.first else { return "?" }
        if parts.count >= 2 { return (f.prefix(1) + parts[1].prefix(1)).uppercased() }
        return f.prefix(2).uppercased()
    }

    // ---- colours -----------------------------------------------------------

    /// The headline pair: the block colour and its deep tone (the gradient
    /// partner ≈ the fixtures' ~0.55 ratio).
    static func colors(hex: String) -> (color: Color, color2: Color) {
        let v = rgb(blockHex(hex))
        return (Color(.sRGB, red: v.r, green: v.g, blue: v.b, opacity: 1),
                Color(.sRGB, red: v.r * 0.55, green: v.g * 0.55, blue: v.b * 0.55, opacity: 1))
    }

    private static func rgb(_ hex: String) -> (r: Double, g: Double, b: Double) {
        let cleaned = hex.trimmingCharacters(in: .whitespaces)
            .replacingOccurrences(of: "#", with: "")
        let v = UInt(cleaned, radix: 16) ?? 0x1F2731
        return (Double((v >> 16) & 0xFF) / 255,
                Double((v >> 8) & 0xFF) / 255,
                Double(v & 0xFF) / 255)
    }

    // ---- headlines (names per-kind) ----------------------------------------

    /// Person: `displayName` + the ContactCard's avatar slots. No banner
    /// primitive exists for a person (§5 decision 6 pending) → `.block`.
    /// City binds `ContactCard.city`, collected at the first-boot BIO step (it was
    /// the map's §3 gap until 15 Aug); an empty one stays nil, so the line is
    /// simply not drawn rather than drawn blank.
    static func headline(person name: String, card: ProfileCard?) -> ProfileHeadline {
        let (c1, c2) = colors(hex: card?.avatarColor ?? "")
        let city = card?.city ?? ""
        return ProfileHeadline(
            kind: .person, name: name, initials: initials(name),
            city: city.isEmpty ? nil : city,
            banner: .block, color: c1, color2: c2,
            iconPhoto: card.flatMap { decode($0.photo) },
            iconClip: card.flatMap { decode($0.clip) })
    }

    /// Group: the folded `displayName`, or the roster-derived label when the
    /// profile was never set (never id hex). Banner = the group `cover` where
    /// one exists.
    static func headline(group view: GroupRow, fallbackName: String = "") -> ProfileHeadline {
        let name = view.displayName.isEmpty ? fallbackName : view.displayName
        let (c1, c2) = colors(hex: view.avatarColor)
        return ProfileHeadline(
            kind: .space, name: name, initials: initials(name),
            banner: bannerKind(b64: view.cover, mime: view.coverMime) ?? .block,
            color: c1, color2: c2,
            iconPhoto: decode(view.photo),
            iconClip: decode(view.clip))
    }

    /// Place: `name`. A Place has no card, no colour slot and no banner
    /// primitive — colour+initials on the default block, honestly.
    static func headline(place row: PlaceRow) -> ProfileHeadline {
        let (c1, c2) = colors(hex: "")
        return ProfileHeadline(
            kind: .space, name: row.name, initials: initials(row.name),
            banner: .block, color: c1, color2: c2)
    }

    /// Event: `title`. Banner = `event.setMedia`'s still where one exists.
    static func headline(event row: EventRow) -> ProfileHeadline {
        let (c1, c2) = colors(hex: "")
        return ProfileHeadline(
            kind: .space, name: row.title, initials: initials(row.title),
            banner: bannerKind(b64: row.banner, mime: row.bannerMime) ?? .block,
            color: c1, color2: c2)
    }

    // ---- info (bio / descriptor + stats) -----------------------------------

    /// Person: the bio card binds `ContactCard.note`; empty note = no card.
    /// Counts stay nil by contract (a person has no roster).
    static func info(person card: ProfileCard?) -> ProfileInfo? {
        guard let note = card?.note, !note.isEmpty else { return nil }
        return ProfileInfo(desc: note)
    }

    /// Group: descriptor = the card `note`, the one free-text slot. An empty
    /// note is nothing to say, so there is no card. `since` stays nil — no
    /// tenure read model crosses the FFI yet (the author side is wired; the
    /// read is the next core PR). `hereNow` stays nil by doctrine (§5 decision
    /// 7: no primitive, defer).
    static func info(group view: GroupRow?) -> ProfileInfo? {
        let desc = view?.note ?? ""
        guard !desc.isEmpty else { return nil }
        return ProfileInfo(desc: desc)
    }

    /// Place: its one-line `descriptor`.
    static func info(place row: PlaceRow) -> ProfileInfo? {
        guard !row.descriptor.isEmpty else { return nil }
        return ProfileInfo(desc: row.descriptor)
    }

    /// Event: its one-line `descriptor`.
    static func info(event row: EventRow) -> ProfileInfo? {
        guard !row.descriptor.isEmpty else { return nil }
        return ProfileInfo(desc: row.descriptor)
    }
}

// MARK: - The real source

/// The CoreService-backed `ProfileSource`. Fetches the current fold through
/// CoreService's read seams and delegates every shape to `ProfileBind`; each
/// answer is the CURRENT truth, nil where the world can't honestly answer.
@MainActor
struct RealProfileSource: ProfileSource {
    let core: CoreService
    let subject: ProfileSubject

    func headline() async -> ProfileHeadline? {
        switch subject {
        case .me:
            return ProfileBind.headline(person: core.displayName, card: core.card)
        case .peer(let id):
            // My own id routed as a peer is still me.
            if CoreService.identityHex(id) == core.myIdentityHex {
                return ProfileBind.headline(person: core.displayName, card: core.card)
            }
            guard let row = connectionRow(id) else { return nil }   // unknown, or blocked (a block hides a person)
            if let p = core.peerProfile(row.peerId) {
                return ProfileBind.headline(person: p.displayName, card: p.card)
            }
            // No published profile is NOT an empty card: fall back to the
            // pairing-bundle name, colour+initials on the default block.
            return ProfileBind.headline(person: row.displayName, card: nil)
        case .group(let id):
            guard let v = core.orgView(id) else { return nil }
            return ProfileBind.headline(group: v, fallbackName: core.derivedGroupName(id))
        case .place(let id):
            guard let r = placeRow(id) else { return nil }
            return ProfileBind.headline(place: r)
        case .event(let id):
            guard let r = eventRow(id) else { return nil }
            return ProfileBind.headline(event: r)
        }
    }

    func info() async -> ProfileInfo? {
        switch subject {
        case .me:
            return ProfileBind.info(person: core.card)
        case .peer(let id):
            if CoreService.identityHex(id) == core.myIdentityHex {
                return ProfileBind.info(person: core.card)
            }
            guard let row = connectionRow(id) else { return nil }
            return ProfileBind.info(person: core.peerProfile(row.peerId)?.card)
        case .group(let id):
            return ProfileBind.info(group: core.orgView(id))
        case .place(let id):
            guard let r = placeRow(id) else { return nil }
            return ProfileBind.info(place: r)
        case .event(let id):
            guard let r = eventRow(id) else { return nil }
            return ProfileBind.info(event: r)
        }
    }

    /// Empty for every subject today, honestly: a Group has no location facet
    /// at all, and no geohash SET is persisted anywhere (map §3). §5 decision 4
    /// (derive area cells client-side from an ANCHORED Place's GeoPoint) is the
    /// v1 mechanism — it lands with the anchor read, not by inventing cells here.
    func geoCells() async -> [ProfileGeoCell] { [] }

    /// Empty, honestly: no pin concept exists in core/FFI (map §3); pins are
    /// §5 decision 2 (a `card.pinned` JSON field + fold sanity, not yet minted).
    func pins() async -> [ProfilePin] { [] }

    // ---- beyond the protocol (the M1 wiring surface) -----------------------
    // The `ProfileSource` protocol does not yet carry these lanes; they live
    // on the concrete so the pages can adopt them the moment the seam grows.

    /// Protocol bridge (M1): the seam's per-id members reach the concrete —
    /// without these the pages would hit the protocol's nil defaults.
    func peerStatus(id: String) async -> ProfilePeerStatus? { peerStatus() }

    /// The connect circle's state. Collapsed from `ConnectionRow.connected` —
    /// core's five-valued `PeerStatus` does not cross the FFI, so `contact`
    /// and `revoked` are indistinguishable from `pending` here (seam gap);
    /// nil = no row / self, the seam's honest absence.
    func peerStatus() -> ProfilePeerStatus? {
        guard case .peer(let id) = subject,
              CoreService.identityHex(id) != core.myIdentityHex,
              let row = connectionRow(id) else { return nil }
        return row.connected ? .connected : .pending
    }

    /// The banner MEDIA where it exists (group cover / event still) — nil is
    /// honest absence, never placeholder bytes. `ProfileHeadline` has no data
    /// slot for these bytes yet (protocol gap, reported); this is where the
    /// page will read them from when it does.
    func banner() -> (kind: ProfileBannerKind, data: Data)? {
        switch subject {
        case .group(let id):
            guard let v = core.orgView(id),
                  let kind = ProfileBind.bannerKind(b64: v.cover, mime: v.coverMime),
                  let data = Data(base64Encoded: v.cover) else { return nil }
            return (kind, data)
        case .event(let id):
            guard let r = eventRow(id),
                  let kind = ProfileBind.bannerKind(b64: r.banner, mime: r.bannerMime),
                  let data = Data(base64Encoded: r.banner) else { return nil }
            return (kind, data)
        case .me, .peer, .place:
            return nil   // no banner primitive for person/place (§5 decision 6 pending)
        }
    }

    /// The Posts lane: `place.post` rows for a Place — the ONLY wall today.
    /// Person posts over connection GroupObjects have no op yet (map §3,
    /// pending `contact.publishPost` ruling), so every other subject is empty.
    func posts() -> [PlacePostRow] {
        guard case .place(let id) = subject else { return [] }
        return core.placePosts(id)
    }

    // ---- lookups -----------------------------------------------------------

    /// The connection for `id`, matched on identity hex so rosters (bare hex)
    /// and `connections()` (space-form) both resolve. Blocked peers are absent
    /// by the seam's default — a block hides a person.
    private func connectionRow(_ id: String) -> ConnectionRow? {
        let hex = CoreService.identityHex(id)
        return core.connectionRows().first { CoreService.identityHex($0.peerId) == hex }
    }

    /// The Place row, from the published fold — re-folded on a miss so a
    /// deep link ahead of the first refresh still resolves.
    private func placeRow(_ id: String) -> PlaceRow? {
        if let hit = core.places.first(where: { $0.id == id }) { return hit }
        core.refreshPlaces()
        return core.placesNow().first { $0.id == id }
    }

    private func eventRow(_ id: String) -> EventRow? {
        if let hit = core.events.first(where: { $0.id == id }) { return hit }
        core.refreshEvents()
        return core.eventsNow().first { $0.id == id }
    }

}
