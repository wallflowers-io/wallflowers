//  PlaceMint.swift
//  Minting a Place from where you actually are.
//
//  v1 ships no spatial processing, so a Place is not a scan — it is a COORDINATE, taken
//  from CoreLocation at mint time, plus optional photos. That is enough to be a Place:
//  somewhere you have been, with the fix that proves it and pictures of what was there.
//
//  A PLACE IS A GROUPOBJECT. `ObjectKind::Place` (type_id 28) in pacific-core, carrying
//  the COMMON location facet (`geo.rs`) as a `Fixed` point. That is the whole reason this
//  file exists in this shape: an OutlineModel entity row is device-local — no log, no
//  signature, nothing to send — so a Place built that way could never be shared with
//  anyone, and the point of knowing where somewhere is, is telling someone else.
//
//  The integrity rule survives the loss of scanning intact, because it never depended on
//  geometry. A Place is bound to the fix that was live when it was minted — it is not a
//  pin the user drops on a map somewhere they have never stood. `PlaceFix` derives its
//  geohash precision from `horizontalAccuracy`, so the cell is itself an honest statement
//  of how well the position is known (see PlaceFix.swift).
//
//  Coordinates cross the FFI in DEGREES and are stored as e7 fixed-point integers by the
//  core, so log replay folds deterministically on every replica. That conversion happens
//  once, at the FFI boundary, and nowhere else.
//
//  Also called by the orphaned v2 scan path (SpatialCaptureView.swift → ScanMint), which
//  layers a fused point cloud on top of exactly this. Shared deliberately: restoring
//  capture should not fork how a Place comes into being.

import Foundation
import UIKit
import PacificCore   // PlaceRow — the folded Place GroupObject off the FFI

@MainActor
enum PlaceMint {

    // MARK: - The Place itself

    /// Mint a Place GroupObject bound to `fix`. Returns the new object id, or nil after
    /// surfacing the failure through `core.error`.
    ///
    /// `name` is the caller's — the wizard passes what the user typed, the v2 scan path
    /// passes the geohash (honest, and unique per cell, rather than "Untitled Place").
    ///
    /// The fix is folded into the `descriptor` as provenance: it is what makes the Place
    /// evidence of having been somewhere rather than an assertion, and the descriptor is
    /// the one field that travels with the object to everyone it is shared with.
    @discardableResult
    static func place(core: CoreService, fix: PlaceFix?, name: String,
                      descriptor: String? = nil, note: String? = nil) -> String? {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let title = trimmed.isEmpty ? (fix?.geohash ?? "Place") : trimmed

        var parts: [String] = []
        if let descriptor, !descriptor.isEmpty { parts.append(descriptor) }
        if let note, !note.isEmpty { parts.append(note) }
        if let fix { parts.append("\(fix.geohash) · \(fix.accuracyLabel)") }

        // A fix that is missing or too coarse mints an UNPLACED Place rather than none:
        // GPS is slow indoors, and losing what the user typed to a fix that never
        // arrived is worse than an object that honestly has no coordinate yet.
        let usable = fix?.isUsable == true ? fix : nil
        return core.mintPlace(
            name: title,
            descriptor: parts.joined(separator: " · "),
            placed: usable != nil,
            lat: usable?.coordinate.latitude ?? 0,
            lng: usable?.coordinate.longitude ?? 0)
    }

    // MARK: - Photos

    /// Persist photos beside the Place and register them with `MediaStore`.
    ///
    /// Failures are per-photo and silent by design: a photo that will not write must not
    /// take the Place down with it. The Place is the thing being minted; pictures are the
    /// optional part of the sentence.
    ///
    /// NOTE — photos are still device-local. `MediaStore` writes `media.json`; it does not
    /// project into the graph, so a shared Place travels without its pictures. Making a
    /// photo a first-class object (or a facet of the Place) is its own piece of work.
    static func attach(photos: [Data], to placeID: String, fix: PlaceFix?) {
        guard !photos.isEmpty else { return }
        let dir = FileManager.default
            .urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("pacific/media", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)

        for data in photos {
            let file = "\(UUID().uuidString).jpg"
            guard (try? data.write(to: dir.appendingPathComponent(file), options: .atomic)) != nil
            else { continue }
            MediaStore.shared.add(MediaAsset(
                kind: .photo,
                assetRef: file,
                capturedAt: Date(),
                // The Place's fix, not the photo's EXIF: these were taken here, now, by
                // the person minting. Claiming an EXIF coordinate we did not read would
                // be inventing provenance.
                coordinate: fix.map { GeoCoordinate(lat: $0.coordinate.latitude,
                                                    lng: $0.coordinate.longitude) },
                contributorID: contributorID,
                placeID: placeID))
        }
    }

    /// A stable per-install author id. `MediaAsset.contributorID` exists for federation —
    /// several people's captures of one subject accumulating (see Media.swift) — which is
    /// v2. Until there is a real identity to use, this is honest about being local: it
    /// identifies this install and claims nothing about who is holding it.
    private static let contributorIDKey = "place.contributor.id"
    static var contributorID: String {
        if let existing = UserDefaults.standard.string(forKey: contributorIDKey) { return existing }
        let fresh = UUID().uuidString
        UserDefaults.standard.set(fresh, forKey: contributorIDKey)
        return fresh
    }
}

// MARK: - Reading Places back into the LIFE shape

extension PlaceRow {
    /// Project a Place GroupObject onto the `EntityVM` the LIFE rows, EntityDetail and
    /// `PlaceAnnotation` already render. The object id IS the entity id, so a Place is
    /// addressable by the same id everywhere.
    ///
    /// `placed` is honoured rather than trusting `lat`/`lng`: an unplaced Place has no
    /// location facet, and (0, 0) is a real point in the Gulf of Guinea. Unplaced Places
    /// are listed but never pinned, which is what `PlaceAnnotation.init(entity:)` already
    /// does with a nil location.
    var entityVM: EntityVM {
        EntityVM(
            id: id,
            name: name,
            kind: .place,
            descriptor: descriptor.isEmpty ? nil : descriptor,
            location: placed ? .fixed(GeoCoordinate(lat: lat, lng: lng)) : nil)
    }
}
