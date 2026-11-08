//  PlaceFix.swift
//  Where a scan was actually taken — acquired from CoreLocation at capture time,
//  never chosen by the user.
//
//  This is the integrity rule behind the capture flow: a scan is EVIDENCE OF BEING
//  SOMEWHERE. Letting someone open a Place called "Wales" while standing in San
//  Francisco and attach a scan to it produces a record that is silently false. So
//  scanning does not decorate an existing Place — it MINTS one, bound to the fix that
//  was live when the frames were captured.
//
//  Precision follows the measurement. A 9-character geohash describes a ~4.8 m cell;
//  emitting one from a ±65 m fix claims an accuracy the hardware did not deliver.
//  `PlaceFix` therefore derives its precision from `horizontalAccuracy`, so the length
//  of the hash is itself an honest statement about how well the position is known.

import Foundation
import CoreLocation

struct PlaceFix: Equatable {
    let coordinate: CLLocationCoordinate2D
    /// Metres. CoreLocation's own estimate; negative means invalid.
    let accuracy: Double
    let capturedAt: Date
    /// Derived from `accuracy` — never longer than the fix justifies.
    let geohash: String

    init(location: CLLocation) {
        coordinate = location.coordinate
        accuracy = location.horizontalAccuracy
        capturedAt = location.timestamp
        geohash = Geohash.encode(location.coordinate,
                                 precision: PlaceFix.precision(for: location.horizontalAccuracy))
    }

    /// Cell size should not undercut the fix's own error bar.
    ///   ≤5 m → 9 (~4.8 m) · ≤40 m → 8 (~38 m) · ≤150 m → 7 (~153 m) · else 6 (~1.2 km)
    static func precision(for accuracy: Double) -> Int {
        guard accuracy > 0 else { return 6 }
        switch accuracy {
        case ..<5:   return 9
        case ..<40:  return 8
        case ..<150: return 7
        default:     return 6
        }
    }

    /// A fix good enough to bind a scan to. Indoors GPS degrades badly, and a scan
    /// pinned to a 500 m cell is not evidence of anything.
    var isUsable: Bool { accuracy > 0 && accuracy <= 150 }

    var accuracyLabel: String {
        accuracy > 0 ? String(format: "±%.0f m", accuracy) : "no fix"
    }
    var cellLabel: String {
        guard let s = Geohash.cellSize(geohash) else { return geohash }
        return String(format: "%@ · %.0f×%.0f m", geohash, s.width, s.height)
    }

    static func == (a: PlaceFix, b: PlaceFix) -> Bool { a.geohash == b.geohash && a.accuracy == b.accuracy }
}

/// Acquires and publishes the current fix. Owned by the capture flow only.
@MainActor
final class PlaceLocator: NSObject, ObservableObject {
    enum State: Equatable {
        case idle, requesting, denied, searching
        case located(PlaceFix)

        var fix: PlaceFix? { if case .located(let f) = self { return f } else { return nil } }
    }

    @Published private(set) var state: State = .idle

    private let manager = CLLocationManager()

    override init() {
        super.init()
        manager.delegate = self
        manager.desiredAccuracy = kCLLocationAccuracyBest
    }

    func start() {
        switch manager.authorizationStatus {
        case .notDetermined:
            state = .requesting
            manager.requestWhenInUseAuthorization()
        case .denied, .restricted:
            state = .denied
        default:
            state = .searching
            manager.startUpdatingLocation()
        }
    }

    func stop() { manager.stopUpdatingLocation() }
}

extension PlaceLocator: CLLocationManagerDelegate {
    nonisolated func locationManagerDidChangeAuthorization(_ m: CLLocationManager) {
        Task { @MainActor in
            switch m.authorizationStatus {
            case .denied, .restricted: state = .denied
            case .notDetermined: break
            default: state = .searching; m.startUpdatingLocation()
            }
        }
    }

    nonisolated func locationManager(_ m: CLLocationManager, didUpdateLocations locs: [CLLocation]) {
        guard let best = locs.last else { return }
        Task { @MainActor in
            let fix = PlaceFix(location: best)
            // Keep the tightest fix seen; accuracy generally improves as GPS settles.
            if let current = state.fix, current.accuracy > 0, current.accuracy <= fix.accuracy { return }
            state = .located(fix)
        }
    }

    nonisolated func locationManager(_ m: CLLocationManager, didFailWithError error: Error) {
        Task { @MainActor in if state.fix == nil { state = .searching } }
    }
}
