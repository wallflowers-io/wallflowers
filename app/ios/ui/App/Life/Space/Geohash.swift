//  Geohash.swift
//  Encodes a WGS84 coordinate as a geohash — a short base32 string whose PREFIX is
//  its own spatial index.
//
//  Why geohash and not S2 here: a Space that carries `gcpuvpm3` can answer "what did
//  we scan near here?" with a string prefix comparison, on-device, with no spatial
//  index and no server. Two places in the same ~150 m cell share the first 7
//  characters. For a local-first app holding thousands of venues rather than
//  billions of points, that is the whole query engine. (S2's Hilbert curve has
//  strictly better locality and uniform cells — worth revisiting if this ever needs
//  real proximity search. Geohash cells also narrow with cos(latitude), so a cell in
//  Reykjavik is half the width of one in Nairobi.)
//
//  The prefix property is one-directional and the trap is well known: sharing a
//  prefix implies proximity, but proximity does NOT imply a shared prefix. Two points
//  either side of a cell boundary — the equator, the prime meridian, ±180° — can be
//  metres apart with nothing in common. `neighbours` exists for that reason: a real
//  proximity query checks the cell and its eight neighbours.

import Foundation
import CoreLocation

enum Geohash {
    private static let base32 = Array("0123456789bcdefghjkmnpqrstuvwxyz")

    /// Cell size by precision, at the equator. A venue wants 8–9.
    ///
    ///   6 → ~1.2 km   7 → ~153 m   8 → ~38 m   9 → ~4.8 m   10 → ~1.2 m
    ///
    /// 9 is the default: finer than a GPS fix is honest about (±5 m is a good
    /// outdoor fix, worse indoors), so the cell is not pretending to more precision
    /// than the measurement carries.
    static let venuePrecision = 9

    static func encode(_ c: CLLocationCoordinate2D, precision: Int = venuePrecision) -> String {
        var lat = (min: -90.0, max: 90.0)
        var lon = (min: -180.0, max: 180.0)
        var hash = ""
        var bit = 0, ch = 0, even = true
        hash.reserveCapacity(precision)

        while hash.count < precision {
            if even {
                let mid = (lon.min + lon.max) / 2
                if c.longitude > mid { ch |= (1 << (4 - bit)); lon.min = mid } else { lon.max = mid }
            } else {
                let mid = (lat.min + lat.max) / 2
                if c.latitude > mid { ch |= (1 << (4 - bit)); lat.min = mid } else { lat.max = mid }
            }
            even.toggle()
            if bit < 4 { bit += 1 } else { hash.append(base32[ch]); bit = 0; ch = 0 }
        }
        return hash
    }

    /// The cell's bounds, so a caller can show what a hash actually covers.
    static func bounds(_ hash: String) -> (sw: CLLocationCoordinate2D, ne: CLLocationCoordinate2D)? {
        // An empty string is not a cell. Without this the loop never runs and the whole
        // globe comes back, so `decode("")` answered (0, 0) — a real coordinate off the
        // coast of Ghana — and anything measuring from it got a confident wrong number.
        guard !hash.isEmpty else { return nil }
        var lat = (min: -90.0, max: 90.0)
        var lon = (min: -180.0, max: 180.0)
        var even = true
        for character in hash.lowercased() {
            guard let idx = base32.firstIndex(of: character) else { return nil }
            for bit in stride(from: 4, through: 0, by: -1) {
                let set = (idx >> bit) & 1 == 1
                if even {
                    let mid = (lon.min + lon.max) / 2
                    if set { lon.min = mid } else { lon.max = mid }
                } else {
                    let mid = (lat.min + lat.max) / 2
                    if set { lat.min = mid } else { lat.max = mid }
                }
                even.toggle()
            }
        }
        return (.init(latitude: lat.min, longitude: lon.min),
                .init(latitude: lat.max, longitude: lon.max))
    }

    static func decode(_ hash: String) -> CLLocationCoordinate2D? {
        guard let b = bounds(hash) else { return nil }
        return .init(latitude: (b.sw.latitude + b.ne.latitude) / 2,
                     longitude: (b.sw.longitude + b.ne.longitude) / 2)
    }

    /// Approximate cell size in metres, narrowing with latitude as all lat/lon grids do.
    static func cellSize(_ hash: String) -> (width: Double, height: Double)? {
        guard let b = bounds(hash) else { return nil }
        let mid = (b.sw.latitude + b.ne.latitude) / 2
        return ((b.ne.longitude - b.sw.longitude) * 111_320 * cos(mid * .pi / 180),
                (b.ne.latitude - b.sw.latitude) * 111_132)
    }

    /// The cell plus its eight neighbours. Required for any honest proximity query,
    /// because a point just over a boundary shares no prefix with its neighbour.
    static func neighbours(_ hash: String) -> [String] {
        guard let b = bounds(hash), let centre = decode(hash) else { return [hash] }
        let dLat = b.ne.latitude - b.sw.latitude
        let dLon = b.ne.longitude - b.sw.longitude
        var out: [String] = []
        for i in -1...1 {
            for j in -1...1 {
                let lat = max(-90, min(90, centre.latitude + Double(i) * dLat))
                let lon = centre.longitude + Double(j) * dLon
                let wrapped = lon > 180 ? lon - 360 : (lon < -180 ? lon + 360 : lon)
                out.append(encode(.init(latitude: lat, longitude: wrapped),
                                  precision: hash.count))
            }
        }
        return Array(Set(out)).sorted()
    }

    /// Do two hashes fall in the same cell at `precision`? A cheap "same place?" test.
    static func sameCell(_ a: String, _ b: String, precision: Int) -> Bool {
        guard a.count >= precision, b.count >= precision else { return false }
        return a.prefix(precision) == b.prefix(precision)
    }

    // MARK: - Paint — cells as drawable area
    //
    // Everything above answers questions about cells. This turns a SET of them into
    // something you can fill: a unit-space rectangle per cell, plus a stable colour.
    // Lifted out of the Place page's area cells, which had the projection right and
    // kept it private to one card at one fixed aspect (now `PlaceDetail.areaCells`).

    /// One painted cell — its hash and where it sits in a unit square (0…1, north up).
    struct Painted: Identifiable, Hashable {
        let hash: String
        let x: CGFloat, y: CGFloat, w: CGFloat, h: CGFloat
        var id: String { hash }
    }

    /// Project cells into a unit square of `aspect` (width ÷ height), north up, with
    /// the GEOGRAPHIC aspect preserved and letterboxed rather than stretched — a cell
    /// squashed to fill a card is a lie about the ground.
    ///
    /// `inset` is the hair of gap that makes a patch read as cells rather than a slab.
    /// Returns empty for an empty input or a degenerate span, so a caller can simply
    /// not draw rather than draw nothing-shaped.
    static func paint(_ hashes: [String],
                      aspect: CGFloat,
                      fill: CGFloat = 0.84,
                      inset: CGFloat = 0.04) -> [Painted] {
        let patch = hashes.compactMap { h in bounds(h).map { (h, $0) } }
        guard !patch.isEmpty, aspect > 0 else { return [] }

        var minLat = patch[0].1.sw.latitude, maxLat = patch[0].1.ne.latitude
        var minLon = patch[0].1.sw.longitude, maxLon = patch[0].1.ne.longitude
        for (_, b) in patch {
            minLat = min(minLat, b.sw.latitude); maxLat = max(maxLat, b.ne.latitude)
            minLon = min(minLon, b.sw.longitude); maxLon = max(maxLon, b.ne.longitude)
        }
        var cells = patch
        var dLat = maxLat - minLat, dLon = maxLon - minLon

        // ANTIMERIDIAN. A patch straddling ±180° spans the globe by these numbers, and
        // every cell would smear across the whole card. Fall back to the first cell
        // alone — a smaller truth beats a wide falsehood.
        if let first = patch.first, let fb = bounds(first.0) {
            let cellLon = fb.ne.longitude - fb.sw.longitude
            if dLon > cellLon * Double(patch.count + 3) {
                cells = [first]
                minLon = fb.sw.longitude; dLon = cellLon
                minLat = fb.sw.latitude;  dLat = fb.ne.latitude - fb.sw.latitude
            }
        }
        guard dLat > 0, dLon > 0 else { return [] }

        // Metres, not degrees: longitude narrows with latitude, so a degree box is not
        // a square and projecting it as one tilts every cell in the patch.
        let midLat = (minLat + maxLat) / 2
        let widthM = dLon * 111_320 * cos(midLat * .pi / 180)
        let heightM = dLat * 111_132
        guard widthM > 0, heightM > 0 else { return [] }

        let patchAspect = CGFloat(widthM / heightM)
        var fy = fill, fx = fy * (patchAspect / aspect)
        if fx > 0.92 { fx = 0.92; fy = fx * (aspect / patchAspect) }
        let x0 = (1 - fx) / 2, y0 = (1 - fy) / 2

        return cells.map { hash, b in
            var x = x0 + fx * CGFloat((b.sw.longitude - minLon) / dLon)
            var y = y0 + fy * CGFloat((maxLat - b.ne.latitude) / dLat)   // north up
            var w = fx * CGFloat((b.ne.longitude - b.sw.longitude) / dLon)
            var h = fy * CGFloat((b.ne.latitude - b.sw.latitude) / dLat)
            x += w * inset; y += h * inset
            w *= (1 - 2 * inset); h *= (1 - 2 * inset)
            return Painted(hash: hash, x: x, y: y, w: w, h: h)
        }
    }

    /// The patch to paint around a point: the cell and its eight neighbours.
    static func patch(around c: CLLocationCoordinate2D, precision: Int) -> [String] {
        neighbours(encode(c, precision: precision))
    }

    /// A STABLE hue for a cell, 0…1. The same cell is the same colour on every surface
    /// and every device, because it is derived from the hash and nothing else — no
    /// palette index, no draw order, no random seed to drift.
    ///
    /// FNV-1a: tiny, deterministic across platforms, and it scatters adjacent hashes
    /// (which share long prefixes) into unrelated hues — neighbouring cells read as
    /// distinct areas rather than a gradient.
    static func hue(_ hash: String) -> Double {
        var h: UInt64 = 0xcbf2_9ce4_8422_2325
        for byte in hash.utf8 {
            h ^= UInt64(byte)
            h = h &* 0x1000_0000_01b3
        }
        return Double(h % 3600) / 3600
    }

    /// Group coordinates into cells at `precision`, with a count each — the input to a
    /// DENSITY paint, where a cell's intensity is how much is in it.
    static func buckets(_ points: [CLLocationCoordinate2D],
                        precision: Int) -> [String: Int] {
        points.reduce(into: [:]) { out, c in
            out[encode(c, precision: precision), default: 0] += 1
        }
    }

    // MARK: - Query — search · overlap · closeby
    //
    // Everything above describes ONE cell. Everything below answers a question about a
    // SET of them, and all of it is built on the header's warning: a shared prefix
    // implies proximity, proximity does NOT imply a shared prefix. So nothing here
    // matches on a bare prefix — a query is always the cell AND its eight neighbours,
    // which is what makes a point the far side of a boundary findable.

    /// Cell half-widths in metres by precision, at the equator — the coarse ladder used
    /// to choose a precision for a radius. Latitude narrows cells by cos(lat), so this is
    /// an upper bound on the cell and therefore a SAFE choice: erring coarse over-fetches
    /// and still filters exactly, erring fine would silently miss.
    private static let cellMetres: [Int: Double] = [
        1: 2_500_000, 2: 630_000, 3: 78_000, 4: 20_000, 5: 2_400,
        6: 610, 7: 76, 8: 19, 9: 2.4, 10: 0.6,
    ]

    /// The finest precision whose cell is still at least `radius` across, so the 3×3
    /// patch around it is guaranteed to cover a circle of that radius. Clamped to the
    /// table's ends.
    static func precision(forRadius radius: CLLocationDistance) -> Int {
        for p in stride(from: 10, through: 1, by: -1) where (cellMetres[p] ?? 0) >= radius {
            return p
        }
        return 1
    }

    /// THE SEARCH KEY SET: which cells to look in for anything within `radius` of a
    /// point. Nine cells at a precision chosen to cover the radius — query by
    /// `hasPrefix` against each, then filter exactly with `distance`.
    ///
    /// Returned coarse-first as a set so a caller can index on them directly.
    static func cover(_ c: CLLocationCoordinate2D,
                      radius: CLLocationDistance) -> [String] {
        neighbours(encode(c, precision: precision(forRadius: radius)))
    }

    /// Metres between two cells' CENTRES. Honest about what it is: a cell is an area,
    /// so this is accurate to roughly the cell size — fine for ranking, wrong for
    /// anything that needs the true separation of two fixes.
    static func distance(_ a: String, _ b: String) -> CLLocationDistance? {
        guard let x = decode(a), let y = decode(b) else { return nil }
        return CLLocation(latitude: x.latitude, longitude: x.longitude)
            .distance(from: CLLocation(latitude: y.latitude, longitude: y.longitude))
    }

    /// OVERLAP — does one cell contain the other, at either's precision? True when
    /// the coarser is a prefix of the finer, which is exactly cell containment.
    ///
    /// This is the one honest use of a bare prefix comparison, because containment is
    /// what the prefix property actually states. It is NOT a proximity test: two cells
    /// can be metres apart and overlap by nothing.
    static func overlaps(_ a: String, _ b: String) -> Bool {
        guard !a.isEmpty, !b.isEmpty else { return false }
        return a.hasPrefix(b) || b.hasPrefix(a)
    }

    /// How much two cells share — the length of their common prefix. A coarse ranking
    /// signal when there is no coordinate to measure from, and the tie-break the market
    /// uses on `area` prefixes, which never get finer than 5 (`thing.rs`).
    static func commonPrefix(_ a: String, _ b: String) -> Int {
        zip(a, b).prefix { $0 == $1 }.count
    }

    /// CLOSEBY — the members of `hashes` within `radius` of `centre`, nearest first.
    ///
    /// Two passes on purpose. The cover is the cheap index sweep that decides what is
    /// worth measuring; `distance` is the exact filter that decides what actually
    /// qualifies. Skipping the first makes it O(n) over everything you hold; skipping
    /// the second returns a square patch and calls it a circle.
    static func near(_ hashes: [String],
                     of centre: CLLocationCoordinate2D,
                     radius: CLLocationDistance) -> [String] {
        let keys = cover(centre, radius: radius)
        let here = encode(centre, precision: venuePrecision)
        return hashes
            .filter { h in keys.contains { h.hasPrefix($0) } }
            .compactMap { h in distance(here, h).map { (h, $0) } }
            .filter { $0.1 <= radius }
            .sorted { $0.1 < $1.1 }
            .map(\.0)
    }
}
