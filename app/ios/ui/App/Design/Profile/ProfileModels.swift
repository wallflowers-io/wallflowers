//  ProfileModels.swift
//  The universal Profile's data contract — the PIE half of the module.
//
//  PURPOSE. One component renders every EGOIC page — People and Spaces (Space =
//  Group). This file is the seam it presents: value types the cards render, and
//  the ONE protocol a host must implement to drive them. The module declares
//  `ProfileSource`; hosts inject concretes (FixtureProfileSource in Previews,
//  RealProfileSource in the app) — the component never imports a store, an FFI
//  type, or the service that feeds it.
//
//  Non-egoic GroupObjects — events, listings, posts — are NOT profiles and do not
//  come through here. They get `Design/Detail/`, a modal that inherits the newitem
//  surface, because what you need from a thing that happened is its media, who
//  made it, and what people said about it — not an identity page.
//
//  Honest-by-default: an unknown value is nil, never a fabricated "" / 0 /
//  stand-in. A card whose data is nil does not render (placebo-control doctrine —
//  no fabricated liveness, no invented "here now").
//
//  Geometry contract:
//  person headline 2.5:1 · space headline 2.5:0.6 · bio 2.5:0.6 · info/map
//  2.5:0.8 · collapsed bar 64pt · cards wear Sig.modalRadius (.continuous) ·
//  one spring: .spring(response: 0.45, dampingFraction: 0.85).

import SwiftUI

// MARK: - Kinds

/// What the profile IS. A Space is a Group — the vocabulary of the Profile spec.
///
/// TWO KINDS, AND ONLY EVER TWO. A profile is for an EGOIC object — someone or
/// something with a self that persists between encounters. An event, a listing and
/// a post are not that: they are things that HAPPEN or are POSTED, they have an
/// author rather than an identity, and they get the detail modal
/// (`Design/Detail/`) instead. An event briefly had a case here; it gave a flyer
/// the geometry of a name plate and its stats a card that could only say
/// "members", which is what a borrowed IA looks like from the inside.
enum ProfileKind {
    case person, space
}

/// What fills the headline's banner slot. Only `block` and `photo` have real
/// data paths today; the motion kinds exist so the contract doesn't churn when
/// their ops land (integration map §5 — banner ops are a pending ruling).
enum ProfileBannerKind {
    case block, photo, video, gif, live
}

// MARK: - Card values

/// The headline card's data — the animating card at the top of every profile.
struct ProfileHeadline {
    let kind: ProfileKind
    let name: String
    let initials: String
    /// Person only — binds `ContactCard.city`, the text field the first-boot BIO
    /// step collects (15 Aug; it was the integration map's §3 gap). A binder with
    /// nothing to say still says nil, and the line simply isn't drawn.
    var city: String? = nil
    let banner: ProfileBannerKind
    /// The block colour — the ground the banner fades INTO on collapse.
    let color: Color
    /// The deep tone — gradient partner and the icon disc's fill.
    let color2: Color
    /// The profile icon, resolve order clip → photo → colour+initials
    /// (group.rs:275-284). Bytes, not URLs — media crosses the seam decoded
    /// from the fold, never fetched by the component.
    var iconPhoto: Data? = nil
    var iconClip: Data? = nil
    /// The banner's REAL pixels, when the fold carries them (`event.setMedia`'s
    /// still, `group.setCover`'s image). Bytes, not URLs — media crosses the seam
    /// decoded from the fold, exactly like the icon. nil falls back to the honest
    /// gradient built from the profile's own colours.
    var bannerPhoto: Data? = nil
    /// A REMOTE banner, for the one profile whose picture does not live in a fold:
    /// a world listing (RA, ArtRabbit) read straight off its source. Only ever set
    /// where there is no GroupObject to have folded bytes instead.
    var bannerURL: URL? = nil
    /// What the banner is CALLED to VoiceOver and to automation. The host names it
    /// because the host is what knows the meaning — "event-hero" is a fact about
    /// events, and a component that also renders people has no business knowing
    /// the word. nil takes the generic name.
    var bannerID: String? = nil
}

/// The info/bio card's data. For a PERSON, `desc` is the bio (binds
/// ContactCard.note) and the counts stay nil; for a SPACE it is the descriptor
/// with its stats beside it. A roster COUNT is deliberately NOT one of them:
/// "n members" on an identity page is an annotation, not a fact you came for
/// (removed 26 Aug) — the roster itself lives on the Space's PEOPLE lane.
struct ProfileInfo {
    let desc: String
    /// No honest primitive exists yet (§5 ruling: defer) — fixtures may carry
    /// it, real binders answer nil and the stat is simply absent.
    var hereNow: String? = nil
    var since: String? = nil

}

/// One geohash cell on the space's map card, in unit fractions of the card.
struct ProfileGeoCell: Identifiable, Hashable {
    let hash: String
    let x: CGFloat, y: CGFloat, w: CGFloat, h: CGFloat
    var id: String { hash }
}

/// Where a pinned bar routes when tapped. Closed: a pin can only point at the
/// kinds of pages the router knows how to open.
enum ProfilePinKind: String {
    case person, space, place, event, listing, post

    /// The bar's trailing label — terse, uppercase, monospaced on the bar.
    var label: String { rawValue.uppercased() }
}

/// One pinned bar under the profile (≤3 by the coming fold sanity — the
/// component renders what it is given and does not enforce the cap).
struct ProfilePin: Identifiable, Hashable {
    let initials: String
    let title: String
    let kind: ProfilePinKind
    /// The object the bar opens — nil renders an inert bar (a pin whose
    /// destination hasn't a page yet never navigates somewhere that isn't it).
    var destinationID: String? = nil
    var color: Color? = nil
    /// Automation/VoiceOver name, host-chosen — same reason as the banner's.
    var accessibilityID: String? = nil

    var id: String { "\(kind.rawValue):\(destinationID ?? title)" }
}

// MARK: - Routes

/// Value-based route to a person's profile page. Typed, like `GroupLink`, so it
/// can never collide with the String route that opens a Project.
struct PersonLink: Hashable {
    let id: String
}

/// Value-based route to a place's profile page.
struct PlaceLink: Hashable {
    let id: String
}

// MARK: - Person pair state (T1 additive)

/// The pair flow's state for a PERSON profile's subject — the core's own
/// vocabulary (PeerStatus: contact | pending | connected | revoked, over the
/// pair_scan/pair_accept double-opt-in). The component renders what the binder
/// answers; a binder that cannot answer says nil and the page shows no status.
enum ProfilePeerStatus: String, Equatable {
    /// Known (discovered/ledgered) but no pair handshake has completed.
    case contact
    /// They scanned us and we have not answered — our accept completes the
    /// double-opt-in (`pair_accept` is purely local; see ChatView's acceptBar).
    case pending
    /// Connected — the 2-member MLS connection group is live.
    case connected
    /// Revoked — the Disconnect residue.
    case revoked
}

// MARK: - The binder seam

/// Everything a Profile page asks of the world. The module declares it; the
/// host injects the concrete (PIE⊗CI — the arrow is module → protocol ← host).
///
/// Async because the real binder folds/reads off-main; each answer is the
/// CURRENT truth, nil where the world can't honestly answer.
@MainActor
protocol ProfileSource {
    /// The headline card. nil = the object is gone/unreadable; the page shows
    /// nothing rather than a fabricated shell.
    func headline() async -> ProfileHeadline?
    /// The bio (person) / info (space) card. nil = no card.
    func info() async -> ProfileInfo?
    /// The space's location card cells. Empty = no map card.
    func geoCells() async -> [ProfileGeoCell]
    /// The pinned bars, in pin order.
    func pins() async -> [ProfilePin]

    // T1 additive members — person actions. Every one carries a default (below)
    // so existing adopters (FixtureProfileSource, T5b's binder-in-progress)
    // keep compiling; a binder that doesn't override simply can't answer.

    /// Person pages only: the subject's pair state. nil = the binder cannot
    /// honestly answer (unknown peer, or not a person) — the Connect circle
    /// renders inert then, and no status value is drawn.
    func peerStatus(id: String) async -> ProfilePeerStatus?
    /// Advance the pair flow for `id` as far as the world truly allows
    /// (accept an inbound `pending`; ask after a known `contact`). Returns the
    /// status AFTER the attempt — nil when nothing could be done.
    func requestConnect(id: String) async -> ProfilePeerStatus?
    /// Where the Issue/Report circle routes: the subject's governing-Arc
    /// surface (`groups.arc_url`), if one exists. nil = no surface — the
    /// circle renders inert (honest; block stays device-local by doctrine).
    func reportSurface(id: String) async -> URL?
}

/// The defaults: an un-overriding binder answers "I can't" — nil everywhere,
/// never a fabricated status or a fake action (placebo-control doctrine).
extension ProfileSource {
    func peerStatus(id: String) async -> ProfilePeerStatus? { nil }
    func requestConnect(id: String) async -> ProfilePeerStatus? { nil }
    func reportSurface(id: String) async -> URL? { nil }
}
