//  GeohashQueryTests.swift
//  The query layer — search · overlap · closeby.
//
//  The cases that matter are the ones the module's header warns about: a shared prefix
//  implies proximity, but proximity does NOT imply a shared prefix. Every test that
//  crosses a cell boundary is there to pin that, because a naive prefix search passes
//  every other test in this file and still loses the point next door.

import XCTest
import CoreLocation
@testable import WallFlowers

final class GeohashQueryTests: XCTestCase {

    private let soho = CLLocationCoordinate2D(latitude: 51.5136, longitude: -0.1365)

    // MARK: - Search

    func test_coverIsNineCellsAtAPrecisionThatHoldsTheRadius() {
        let keys = Geohash.cover(soho, radius: 500)
        XCTAssertEqual(Set(keys).count, keys.count, "no duplicate cells")
        XCTAssertLessThanOrEqual(keys.count, 9, "a 3×3 patch, never more")
        let p = Geohash.precision(forRadius: 500)
        XCTAssertTrue(keys.allSatisfy { $0.count == p }, "one precision across the patch")
    }

    /// The property that actually matters, tested as a property: whatever precision the
    /// ladder picks, the 3×3 cover must REACH a point at the edge of the radius. Tested
    /// by placing one, rather than by asserting the ladder against itself.
    ///
    /// North and east separately: geohash cells are not square, and a cover that reaches
    /// far enough one way can fall short the other.
    func test_coverReachesTheEdgeOfTheRadiusInBothAxes() {
        for radius in [50.0, 500, 5_000, 50_000] {
            let north = CLLocationCoordinate2D(latitude: soho.latitude + radius / 111_132,
                                               longitude: soho.longitude)
            let east = CLLocationCoordinate2D(
                latitude: soho.latitude,
                longitude: soho.longitude + radius / (111_320 * cos(soho.latitude * .pi / 180)))
            for edge in [north, east] {
                let hash = Geohash.encode(edge, precision: 9)
                // The margin is not slack: `near` measures between precision-9 CELL
                // CENTRES, so both ends are quantised to a ~2.4m cell and a point
                // placed exactly at the radius can measure a few metres over. The
                // module says so itself — `distance` is "accurate to roughly the cell
                // size". Anything tighter than that tests the rounding, not the cover.
                let margin = radius * 1.02 + 6
                XCTAssertTrue(Geohash.near([hash], of: soho, radius: margin).contains(hash),
                              "a point \(radius)m out must be inside the cover")
            }
        }
    }

    // MARK: - Overlap

    func test_overlapIsContainmentNotProximity() {
        let fine = Geohash.encode(soho, precision: 9)
        let coarse = String(fine.prefix(5))
        XCTAssertTrue(Geohash.overlaps(fine, coarse), "the coarse cell contains the fine one")
        XCTAssertTrue(Geohash.overlaps(coarse, fine), "and the test is symmetric")
        XCTAssertFalse(Geohash.overlaps(fine, "u10hb"), "a different cell does not overlap")
        XCTAssertFalse(Geohash.overlaps("", fine), "an empty hash contains nothing")
    }

    func test_commonPrefixCountsSharedCells() {
        let a = Geohash.encode(soho, precision: 9)
        XCTAssertEqual(Geohash.commonPrefix(a, a), a.count)
        XCTAssertEqual(Geohash.commonPrefix(a, String(a.prefix(4))), 4)
        XCTAssertEqual(Geohash.commonPrefix("gcpuv", "u10hb"), 0)
    }

    // MARK: - Closeby

    func test_nearFiltersToTheRadiusAndSortsByDistance() {
        let here = Geohash.encode(soho, precision: 9)
        let near = Geohash.encode(.init(latitude: soho.latitude + 0.0009,   // ~100 m
                                        longitude: soho.longitude), precision: 9)
        let far = Geohash.encode(.init(latitude: soho.latitude + 0.05,      // ~5.5 km
                                       longitude: soho.longitude), precision: 9)

        let hits = Geohash.near([far, near, here], of: soho, radius: 300)
        XCTAssertEqual(hits.first, here, "nearest first")
        XCTAssertTrue(hits.contains(near), "100m is inside a 300m radius")
        XCTAssertFalse(hits.contains(far), "5.5km is not")
    }

    /// THE ONE THAT MATTERS. Two points metres apart across a cell boundary share no
    /// prefix at all — a prefix search drops the neighbour, the 3×3 cover keeps it.
    func test_neighbourAcrossACellBoundaryIsStillFound() {
        // The prime meridian: longitude straddles a top-level cell edge in London.
        let west = CLLocationCoordinate2D(latitude: 51.4779, longitude: -0.0005)
        let east = CLLocationCoordinate2D(latitude: 51.4779, longitude: 0.0005)
        let eastHash = Geohash.encode(east, precision: 9)

        XCTAssertEqual(Geohash.commonPrefix(Geohash.encode(west, precision: 9), eastHash), 0,
                       "precondition: these share NO prefix despite being ~70m apart")
        XCTAssertTrue(Geohash.near([eastHash], of: west, radius: 500).contains(eastHash),
                      "the cover's neighbours must reach across the boundary")
    }

    func test_distanceIsNilOnNonsenseAndZeroOnItself() {
        let h = Geohash.encode(soho, precision: 9)
        XCTAssertEqual(Geohash.distance(h, h) ?? -1, 0, accuracy: 0.001)
        XCTAssertNil(Geohash.distance("", h), "an empty hash has no centre")
    }
}

// MARK: - Paint

final class GeohashPaintTests: XCTestCase {

    private let soho = CLLocationCoordinate2D(latitude: 51.5136, longitude: -0.1365)

    func test_paintFillsAUnitSquareNorthUp() {
        let cells = Geohash.paint(Geohash.patch(around: soho, precision: 6), aspect: 2.5 / 0.8)
        XCTAssertFalse(cells.isEmpty)
        XCTAssertTrue(cells.allSatisfy { $0.x >= 0 && $0.y >= 0 && $0.x + $0.w <= 1 && $0.y + $0.h <= 1 },
                      "every cell stays inside the unit square")

        // North up: the northernmost cell must have the SMALLEST y.
        let byLat = cells.compactMap { c in Geohash.decode(c.hash).map { ($0.latitude, c.y) } }
        let north = byLat.max { $0.0 < $1.0 }!
        let south = byLat.min { $0.0 < $1.0 }!
        XCTAssertLessThan(north.1, south.1, "north sits higher on the card")
    }

    /// Geographic aspect is PRESERVED, not stretched: at London's latitude a cell is
    /// about 0.62 as wide as it is tall, and the painted cell must carry that ratio
    /// through the card's own aspect rather than filling it.
    func test_paintPreservesGeographicAspect() {
        let hash = Geohash.encode(soho, precision: 6)
        guard let size = Geohash.cellSize(hash), let cell = Geohash.paint([hash], aspect: 1).first
        else { return XCTFail("no cell") }
        let ground = size.width / size.height
        let drawn = Double(cell.w / cell.h)
        XCTAssertEqual(drawn, ground, accuracy: ground * 0.05, "drawn shape matches the ground")
    }

    func test_hueIsStableAndScattersNeighbours() {
        let a = Geohash.encode(soho, precision: 7)
        XCTAssertEqual(Geohash.hue(a), Geohash.hue(a), "same cell, same colour, always")
        let others = Geohash.neighbours(a).filter { $0 != a }
        XCTAssertTrue(others.allSatisfy { abs(Geohash.hue($0) - Geohash.hue(a)) > 0.02 },
                      "adjacent cells get visibly different hues, not a gradient")
    }

    func test_bucketsCountByCell() {
        let here = soho
        let alsoHere = CLLocationCoordinate2D(latitude: soho.latitude + 0.00001,
                                              longitude: soho.longitude)
        let far = CLLocationCoordinate2D(latitude: soho.latitude + 0.5, longitude: soho.longitude)
        let counts = Geohash.buckets([here, alsoHere, far], precision: 6)
        XCTAssertEqual(counts.count, 2, "two distinct cells")
        XCTAssertEqual(counts[Geohash.encode(here, precision: 6)], 2)
    }

    func test_paintIsEmptyRatherThanDegenerate() {
        XCTAssertTrue(Geohash.paint([], aspect: 2).isEmpty)
        XCTAssertTrue(Geohash.paint(["!!!"], aspect: 2).isEmpty, "an unparseable hash paints nothing")
    }
}
