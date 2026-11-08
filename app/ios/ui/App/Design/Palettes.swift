//  Palettes.swift
//  The deliberate "entity palette" — the 5 colours the LIFE categories, the mint menu,
//  and the jewel accents wear. Each palette is one Van Gogh painting's five most-common
//  colours, taken DIRECTLY from Docs/Van_Gogh_Color_Pie_Chart.jpg (hex sampled per pie).
//  Choose one in the account sheet; the whole accent system re-tints live.

import SwiftUI

/// A 5-colour palette. `colors[colorIndex]` is a category's base jewel; the LIFE / mint
/// UIs derive a light sheen + deep shade from it.
struct Palette: Identifiable, Equatable {
    let id: String
    let name: String
    let colors: [Color]   // exactly 5, in category order: events · people · places · groups · projects

    static let all: [Palette] = [cypresses, starryNight, almondBlossom, theSea, wheatfield]
    static let `default` = cypresses

    // Van Gogh — five most-common colours per painting (Docs/Van_Gogh_Color_Pie_Chart.jpg).
    static let cypresses = Palette(id: "cypresses", name: "Cypresses",
        colors: hex(0x476030, 0xDA851A, 0xA8913F, 0x767434, 0x2A2D1A))
    static let starryNight = Palette(id: "starry-night", name: "Starry Night",
        colors: hex(0x6E8595, 0x283687, 0x464C65, 0x253684, 0x1C2252))
    static let almondBlossom = Palette(id: "almond-blossom", name: "Almond Blossom",
        colors: hex(0xA9901B, 0xB25A1E, 0xD4D338, 0xB7AC2A, 0x573416))
    static let theSea = Palette(id: "the-sea", name: "The Sea",
        colors: hex(0x2C5E77, 0x8FAA81, 0x2F6092, 0x66555D, 0x295B78))
    static let wheatfield = Palette(id: "wheatfield", name: "Wheatfield",
        colors: hex(0x486AA4, 0xC19147, 0xD4A45E, 0x9D7E38, 0xBC9552))

    private static func hex(_ v: UInt...) -> [Color] { v.map { Color(hex: $0) } }
}

/// The one active palette, persisted. Views observe it (`@ObservedObject`/`@EnvironmentObject`)
/// so a change re-tints the accents live; the enum colour computeds read `shared.selected`.
final class PaletteStore: ObservableObject {
    static let shared = PaletteStore()
    @Published var selected: Palette {
        didSet { UserDefaults.standard.set(selected.id, forKey: Self.key) }
    }
    private static let key = "pacific.palette.id"
    private init() {
        let id = UserDefaults.standard.string(forKey: Self.key)
        selected = Palette.all.first { $0.id == id } ?? .default
    }
    /// The active palette's base colour for a category slot (0…4).
    static func base(_ index: Int) -> Color { shared.selected.colors[max(0, min(4, index))] }
}

extension Color {
    /// The INVERSE of this colour, for a glyph that has to read against it — the
    /// back chevron on a profile's banner.
    ///
    /// Luminance, not 1-r/1-g/1-b: a literal channel flip turns mid-grey into
    /// mid-grey, which is the one result that cannot be seen. This answers the
    /// question the caller is actually asking — light ground, dark mark; dark
    /// ground, light mark — and it is stable across a banner's whole range.
    var inverse: Color {
        var r: CGFloat = 0, g: CGFloat = 0, b: CGFloat = 0, a: CGFloat = 0
        UIColor(self).getRed(&r, green: &g, blue: &b, alpha: &a)
        let luma = 0.2126 * Double(r) + 0.7152 * Double(g) + 0.0722 * Double(b)
        return luma > 0.55 ? Color(white: 0.08) : .white
    }

    /// A lighter tint (mix toward white) — the jewel's top sheen.
    func lightened(_ amount: Double) -> Color { mixed(with: .white, amount) }
    /// A deeper shade (mix toward black) — the jewel's base.
    func darkened(_ amount: Double) -> Color { mixed(with: .black, amount) }
    private func mixed(with other: Color, _ t: Double) -> Color {
        let a = UIColor(self), b = UIColor(other)
        var ar: CGFloat = 0, ag: CGFloat = 0, ab: CGFloat = 0, aa: CGFloat = 0
        var br: CGFloat = 0, bg: CGFloat = 0, bb: CGFloat = 0, ba: CGFloat = 0
        a.getRed(&ar, green: &ag, blue: &ab, alpha: &aa)
        b.getRed(&br, green: &bg, blue: &bb, alpha: &ba)
        let f = CGFloat(min(max(t, 0), 1))
        return Color(.sRGB,
                     red: Double(ar + (br - ar) * f),
                     green: Double(ag + (bg - ag) * f),
                     blue: Double(ab + (bb - ab) * f))
    }
}
