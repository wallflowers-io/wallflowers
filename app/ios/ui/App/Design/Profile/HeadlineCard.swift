//  HeadlineCard.swift
//  The profile's animating card — the one that collapses into the 64pt bar.
//
//  Geometry/animation contract, ported from the profile prototype EXACTLY:
//    expanded height: person = width / 2.5 · space = width * 0.6 / 2.5
//    collapsed: the 64pt bar
//    collapse = icon slides LEFT and shrinks · banner fades to the block colour ·
//    name slides to CENTRE · the action circles fade
//    one spring drives it all: .spring(response: 0.45, dampingFraction: 0.85) —
//    owned by the CALLER (ProfileStack); this card is a pure function of `collapsed`.
//
//  The icon is the app's ONE avatar atom (Design/Signal.swift `Avatar`), never a
//  local re-implementation.

import SwiftUI

/// Shared profile geometry tokens — the numbers the contract names.
enum ProfileGeometry {
    /// The collapsed bar — also the directory row and the pin bar (one bar, three uses).
    static let bar: CGFloat = DesignTokens.Profile.bar
    /// Expanded headline height for a card `width` wide.
    static func headlineHeight(_ kind: ProfileKind, width: CGFloat) -> CGFloat {
        width * (kind == .person ? DesignTokens.Profile.headlinePerson
                                 : DesignTokens.Profile.headlineSpace)
    }
    /// Info and map card height (2.5:0.8). The bio has no ratio: it is a band
    /// that takes the height of its words.
    static func infoHeight(width: CGFloat) -> CGFloat { width * DesignTokens.Profile.info }
    /// THE collapse spring — the whole stack moves on this one curve.
    static let spring = Animation.spring(response: DesignTokens.Motion.profileResponse,
                                        dampingFraction: DesignTokens.Motion.profileDamping)
    /// Every profile card wears the Note-modal radius.
    static var shape: RoundedRectangle {
        RoundedRectangle(cornerRadius: Sig.modalRadius, style: .continuous)
    }
}

struct HeadlineCard: View {
    let headline: ProfileHeadline
    let collapsed: Bool
    /// The card's concrete width — the stack measures once and hands it down, so
    /// the fractional positions inside the ZStack have a real number to stand on.
    /// It is the WHOLE screen's width now: the banner is a wall, not a card.
    let width: CGFloat
    /// The status-bar strip the wall runs UP into. The media covers it; the
    /// icon, name and circles stay below it, so nothing is drawn under the
    /// clock. A host that is not full-bleed passes 0 and nothing moves.
    var safeTop: CGFloat = 0
    /// The circles this profile carries, each in its own corner. Empty = a card
    /// with no actions at all, which is a Space today.
    ///
    /// This was three named closures (onConnect/onIssue/onMessage) for exactly as
    /// long as `person` was the only kind with actions. An event's circles are not
    /// a person's — DISCUSS is not a DM, GOING is not Connect — and naming the
    /// slots after one kind's verbs would have meant either a fourth and fifth
    /// closure or an event borrowing labels that lie about what it does. The card
    /// owns the GEOMETRY of the corners; the caller owns what sits in them.
    var actions: [ProfileAction] = []

    /// True where the banner is REAL — folded bytes or a listing's remote poster.
    private var hasBanner: Bool {
        headline.bannerPhoto != nil || headline.bannerURL != nil
    }

    private var expandedHeight: CGFloat {
        ProfileGeometry.headlineHeight(headline.kind, width: width)
    }
    private var height: CGFloat { collapsed ? ProfileGeometry.bar : expandedHeight }

    var body: some View {
        // TWO LAYERS, ALWAYS THE SAME TWO (26 Aug). The ground and the banner fill
        // the WHOLE wall, status-bar strip included; everything the reader touches
        // sits in a second box of the card's own height, held below the strip. The
        // banner's own branches (real pixels · remote poster · honest gradient) all
        // return an opaque fill, so the stack is identical whether a cover is
        // published or not — with one set, only the pixels change.
        ZStack(alignment: .top) {
            headline.color                                  // the block colour beneath
            // A PUBLISHED banner is never hidden. Only the honest gradient — which
            // is a stand-in for a picture that does not exist — fades to the block
            // colour on collapse; real pixels stay, because a cover that vanishes
            // when you scroll reads as a rendering fault.
            ProfileBanner(headline: headline)
                .opacity(hasBanner || !collapsed ? 1 : 0)
            content
                .frame(width: width, height: height)
                .padding(.top, safeTop)
        }
        .frame(width: width, height: height + safeTop)
        // No rounded rectangle: a wall has no corners. `.clipped()` takes over the
        // job the clipShape was quietly doing — holding `scaledToFill` media in.
        .clipped()
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("profile-headline")
    }

    /// Everything above the banner, in the card's own coordinate box.
    private var content: some View {
        ZStack {
            // profile icon — slides left and shrinks on collapse. The app's one
            // avatar atom: photo when published, block colour + initials otherwise.
            Avatar(initials: headline.initials,
                   tint: headline.color2,
                   size: iconSize,
                   photo: headline.iconPhoto,
                   color: headline.color2)
                .position(iconPos)

            // name — slides to centre on collapse
            Text(headline.name)
                .font(.system(size: nameSize, weight: .bold))
                .foregroundStyle(.white)
                .shadow(color: .black.opacity(0.5), radius: 6, y: 1)
                .position(namePos)

            if headline.kind == .person, let city = headline.city {
                Text(city)
                    .font(.system(size: 13))
                    .foregroundStyle(.white.opacity(0.75))
                    .position(x: width / 2, y: expandedHeight * 0.76)
                    .opacity(collapsed ? 0 : 1)
            }
            ForEach(actions) { action in
                ProfileCircleButton(symbol: action.symbol, action: action.run, tint: action.tint,
                                    identifier: action.accessibilityID)
                    .position(position(action.slot))
                    .opacity(collapsed ? 0 : 1)
            }
        }
    }

    // MARK: The contract's positions (prototype-exact)

    private var iconSize: CGFloat {
        collapsed ? 36 : (headline.kind == .person ? 72 : 44)
    }

    private var nameSize: CGFloat {
        collapsed ? 15 : (headline.kind == .person ? 20 : 19)
    }

    private var iconPos: CGPoint {
        if collapsed { return CGPoint(x: 32, y: ProfileGeometry.bar / 2) }
        return headline.kind == .person
            ? CGPoint(x: width / 2, y: expandedHeight * 0.38)
            : CGPoint(x: 42, y: expandedHeight / 2)
    }

    private var namePos: CGPoint {
        if collapsed { return CGPoint(x: width / 2, y: ProfileGeometry.bar / 2) }
        return headline.kind == .person
            ? CGPoint(x: width / 2, y: expandedHeight * 0.62)
            : CGPoint(x: 76 + nameHalfWidth, y: expandedHeight / 2)
    }

    /// Inline-left layout aid for the space name (prototype-exact).
    private var nameHalfWidth: CGFloat { CGFloat(headline.name.count) * 5.4 }

    /// The three corners, prototype-exact — the same numbers the person circles
    /// were written at, now named rather than repeated at each call.
    private func position(_ slot: ProfileActionSlot) -> CGPoint {
        switch slot {
        case .topTrailing:    CGPoint(x: width - 33, y: 31)
        case .bottomTrailing: CGPoint(x: width - 33, y: height - 31)
        case .bottomLeading:  CGPoint(x: 33, y: height - 31)
        }
    }
}

// MARK: - The actions

/// Which corner a circle sits in. Top-LEADING is deliberately absent: it is the
/// close/back affordance's corner, and that one belongs to the host presenting
/// the page, not to the card.
enum ProfileActionSlot {
    case topTrailing, bottomTrailing, bottomLeading
}

/// One circle on the headline. `run == nil` still draws it — inert, which is the
/// honest rendering of an action the world cannot currently perform (a Connect
/// with no pair flow to advance, a DISCUSS on a listing with no thread behind it).
struct ProfileAction: Identifiable {
    let slot: ProfileActionSlot
    /// SF Symbol.
    let symbol: String
    /// nil = the glass circle; a tint fills it instead (the DM circle's gold).
    var tint: Color? = nil
    /// Automation/VoiceOver name. A circle's MEANING is the host's ("event-discuss"),
    /// and the symbol it wears is not a name — two kinds could reasonably both use
    /// a speech bubble for different acts. nil falls back to the symbol.
    var accessibilityID: String? = nil
    var run: (() -> Void)? = nil

    var id: String { "\(slot)-\(symbol)" }
}

// MARK: - The banner

/// The banner slot. REAL PIXELS FIRST: a fold that carries banner bytes, or a
/// world listing that carries a flyer URL, is drawn as the picture it is — this
/// is the whole of an event page, where the flyer is the object.
///
/// It did not always do that. `.photo` used to render a gradient stand-in for a
/// photo that existed, which read as a design until you noticed the picture was
/// never shown. Where no pixels exist the gradient stays, and stays HONEST: it is
/// built from the profile's own colours and claims to be nothing else. The chip
/// labels the motion kinds.
struct ProfileBanner: View {
    let headline: ProfileHeadline

    private var kind: ProfileBannerKind { headline.banner }
    private var bannerID: String { headline.bannerID ?? "profile-banner-photo" }
    private var color: Color { headline.color }
    private var color2: Color { headline.color2 }

    var body: some View {
        ZStack(alignment: .topLeading) {
            if let photo = headline.bannerPhoto, let image = UIImage(data: photo) {
                Image(uiImage: image)
                    .resizable()
                    .scaledToFill()
                    .accessibilityIdentifier(bannerID)
            } else if let url = headline.bannerURL {
                // The one remote case: a listing whose picture lives at its source.
                // Stripes while it loads or fails — the app's own empty-media ground,
                // never a spinner pretending something is on its way.
                PosterImage(url: url, imageID: bannerID) { FlyerStripes() }
            } else {
                gradient
            }
            if let label = chip {
                Text(label)
                    .font(.system(size: 10, weight: .semibold, design: .monospaced))
                    .kerning(1.2)
                    .foregroundStyle(kind == .live
                                     ? Color(red: 1.0, green: 0.42, blue: 0.34)
                                     : Theme.textPrimary)
                    .padding(.horizontal, 8).padding(.vertical, 3)
                    .background(Capsule().fill(.black.opacity(0.55)))
                    .padding(12)
            }
        }
    }

    /// The honest stand-in, per kind — what a profile with no pixels looks like.
    @ViewBuilder private var gradient: some View {
        ZStack {
            switch kind {
            case .block:
                LinearGradient(colors: [color, color2],
                               startPoint: .topLeading, endPoint: .bottomTrailing)
            case .photo:
                ZStack {
                    LinearGradient(colors: [Color(white: 0.10), color2],
                                   startPoint: .top, endPoint: .bottom)
                    RadialGradient(colors: [color2, .clear],
                                   center: .init(x: 0.2, y: 0.1), startRadius: 0, endRadius: 420)
                    RadialGradient(colors: [color.opacity(0.9), .clear],
                                   center: .init(x: 0.85, y: 0.85), startRadius: 0, endRadius: 380)
                }
            case .video:
                LinearGradient(colors: [Color(white: 0.09), color2],
                               startPoint: .top, endPoint: .bottom)
            case .gif:
                LinearGradient(colors: [color2, color, color2],
                               startPoint: .leading, endPoint: .trailing)
            case .live:
                ZStack {
                    LinearGradient(colors: [Color(white: 0.08), Color(white: 0.12)],
                                   startPoint: .top, endPoint: .bottom)
                    RadialGradient(colors: [color2, .clear],
                                   center: .init(x: 0.8, y: 0.0), startRadius: 0, endRadius: 420)
                }
            }
        }
    }

    private var chip: String? {
        switch kind {
        case .video: "VIDEO"
        case .gif: "GIF"
        case .live: "LIVE"
        default: nil
        }
    }
}

// MARK: - The action circle

/// The headline's 38pt glass circle (Connect · Issue). Inert until a host wires
/// the action — a circle that does nothing is still the geometry contract.
struct ProfileCircleButton: View {
    let symbol: String
    var action: (() -> Void)? = nil
    /// nil = the glass circle. A tint fills it instead — the DM circle's gold.
    var tint: Color? = nil
    /// Host-chosen name; nil falls back to the symbol.
    var identifier: String? = nil

    var body: some View {
        Button { action?() } label: {
            Image(systemName: symbol)
                .font(.system(size: 15, weight: .bold))
                .foregroundStyle(tint == nil ? .white : Color.black.opacity(0.82))
                .frame(width: 38, height: 38)
                .background(Circle().fill(tint.map { AnyShapeStyle($0) }
                                          ?? AnyShapeStyle(.black.opacity(0.45))))
                .overlay(Circle().strokeBorder(.white.opacity(0.7), lineWidth: 1.5))
        }
        .buttonStyle(.plain)
        .disabled(action == nil)
        .accessibilityIdentifier(identifier ?? "profile-circle-\(symbol)")
    }
}
