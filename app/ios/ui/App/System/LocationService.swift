//  LocationService.swift
//  CoreLocation: authorization, the device coordinate, and the compass heading. Feeds the
//  top-down map's "you are here" (UserAnnotation) and the opal compass strip. The heading
//  will also drive a LocationSource.stream("device") self-report once sharing is wired.

import CoreLocation
import Combine

@MainActor
final class LocationService: NSObject, ObservableObject {
    @Published private(set) var authorization: CLAuthorizationStatus = .notDetermined
    @Published private(set) var coordinate: CLLocationCoordinate2D?
    /// Device heading in degrees [0, 360); nil until the compass has a fix.
    @Published private(set) var heading: Double?
    /// When true, the top-down map's rotation is decoupled from the device (tap the bearing to
    /// toggle). UI-owned, so it is a plain published var.
    @Published var headingLocked = false

    private let manager = CLLocationManager()

    override init() {
        super.init()
        manager.delegate = self
        manager.desiredAccuracy = kCLLocationAccuracyNearestTenMeters
        manager.headingFilter = 1                 // fine updates for a smooth compass
        authorization = manager.authorizationStatus
    }

    var isAuthorized: Bool {
        authorization == .authorizedWhenInUse || authorization == .authorizedAlways
    }

    /// Ask for When-In-Use (the map's "you are here" + the compass) and begin updates. Safe
    /// to call repeatedly — the prompt only shows while `.notDetermined`.
    ///
    /// THE PERMISSION BELONGS TO THE MAP. This used to run from the app's launch
    /// task, so a brand-new user met the location alert before seeing anything that
    /// wanted a location. Call it where the blue dot is about to be drawn and the
    /// ask explains itself; everywhere else calls `resume`.
    func request() {
        if authorization == .notDetermined { manager.requestWhenInUseAuthorization() }
        start()
    }

    /// Begin updates IF the user has already granted location — never a prompt.
    /// For launch and for anything that merely benefits from a fix (the listing
    /// Systems' scope), which must not spend a permission to get one.
    func resume() { start() }

    func start() {
        guard isAuthorized else { return }
        manager.startUpdatingLocation()
        if CLLocationManager.headingAvailable() { manager.startUpdatingHeading() }
    }

    func stop() {
        manager.stopUpdatingLocation()
        manager.stopUpdatingHeading()
    }
}

extension LocationService: CLLocationManagerDelegate {
    nonisolated func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
        let status = manager.authorizationStatus
        Task { @MainActor in
            self.authorization = status
            self.start()
        }
    }

    nonisolated func locationManager(_ manager: CLLocationManager, didUpdateLocations locations: [CLLocation]) {
        guard let c = locations.last?.coordinate else { return }
        Task { @MainActor in self.coordinate = c }
    }

    nonisolated func locationManager(_ manager: CLLocationManager, didUpdateHeading newHeading: CLHeading) {
        let deg = newHeading.trueHeading >= 0 ? newHeading.trueHeading : newHeading.magneticHeading
        Task { @MainActor in self.heading = deg }
    }

    nonisolated func locationManager(_ manager: CLLocationManager, didFailWithError error: Error) {}
}
