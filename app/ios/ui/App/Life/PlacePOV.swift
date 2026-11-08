//  PlacePOV.swift
//  Walking up to a Place now puts you INSIDE it.
//
//  What changed and why: the previous version projected neighbouring places onto a
//  local ENU ground plane and drew each as a cylinder, viewed from a camera 9 m up.
//  That is a map rendered in 3D — a third-person diorama. The product is "enter a
//  virtual representation of a real place", which is a first-person interior at 1:1.
//  Different camera, different content, different scale.
//
//  So this view renders the REAL reconstruction: a VGGT point cloud captured by
//  arc/spatial (25 frames → 6 windows → 22,309 fused points), shipped as `kitchen.pspc`,
//  built into a splat mesh at runtime, and placed around the viewer.
//
//  SKINS are literal here: the same real geometry, re-materialised. That is the whole
//  thesis — the place is real, the skin is ours, and swapping one does not touch the
//  other.
//
//  Camera-AR stays OFF and opt-in, as before: AR roughly doubles battery draw and the
//  rendered world is the daily driver. When it lands it attaches to a relocaliser map
//  (see RelocMapRef) rather than ARGeoAnchor, because the venues that matter here are
//  interiors and geo-anchors are street-facing.
//
//  RealityKit does not render in Previews or reliably in the Simulator — this needs a
//  device build to evaluate.

import SwiftUI
import RealityKit
import simd

// MARK: - Skins

/// A skin is a re-materialisation of a Space's real geometry. In the model a skin is a
/// child Space (`Space.Role.skin`) so skins nest and compose; these are the built-in
/// material skins that ship with the build.
enum SpaceSkin: String, CaseIterable, Identifiable {
    case captured = "Captured"
    case mono     = "Mono"
    case neon     = "Neon"

    var id: String { rawValue }
    var glyph: String {
        switch self {
        case .captured: return "camera.macro"
        case .mono:     return "circle.lefthalf.filled"
        case .neon:     return "sparkles"
        }
    }
    /// nil = use the captured colour atlas untouched.
    var tint: UIColor? {
        switch self {
        case .captured: return nil
        case .mono:     return .white
        case .neon:     return UIColor(red: 0.35, green: 1.0, blue: 0.85, alpha: 1)
        }
    }
}

// MARK: - The interior

/// Where a Place's geometry comes from. A Place with no scan says so; it does not
/// silently show someone else's room, which is what the hardcoded bundle fixture did.
enum CloudSource {
    case scan(URL)        // this Place's own capture
    case fixture(String)  // the bundled demo cloud, for Places that were never scanned
}

private struct SpaceInterior: View {
    let source: CloudSource
    let skin: SpaceSkin
    /// Estimated metres-per-unit to bring an unscaled reconstruction to human size.
    let metricScale: Float
    @Binding var status: String
    @Binding var inside: Bool

    /// ORBIT, not free-look. A reconstruction from a walkthrough is a SURFACE, not a
    /// sealed room — this kitchen was captured facing one way, so a camera parked at
    /// its centroid sits buried in the point shell with nothing to see but splats. So
    /// the view opens framed on the whole space and orbits it; pinching dollies in,
    /// and crossing the shell radius is what "entering" actually is.
    @State private var yaw: Float = 0
    @State private var pitch: Float = -0.12
    @State private var dragStart: SIMD2<Float> = .zero

    /// Orbit radius, and the framing distance it started at.
    @State private var distance: Float = 4
    @State private var framedDistance: Float = 4
    @State private var pinchStart: Float?
    /// Radius of the cloud's bounding sphere, in scaled metres.
    @State private var shellRadius: Float = 1

    private let fov: Float = 68

    var body: some View {
        RealityView { content in
            let rig = Entity()
            rig.name = "rig"
            let camera = PerspectiveCamera()
            camera.camera.fieldOfViewInDegrees = fov
            camera.camera.near = 0.01
            camera.name = "camera"
            rig.addChild(camera)
            content.add(rig)

            do {
                let cloud: PointCloud
                switch source {
                case .scan(let url):     cloud = try PointCloud.load(contentsOf: url)
                case .fixture(let name): cloud = try PointCloud.load(resource: name)
                }
                // Splats sized just under the cloud's own sample spacing: big enough to
                // read as surface, small enough that structure survives. Too large and
                // the 3-quad cross of every point fuses into plaid.
                let model = try PointCloudMesh.build(cloud, splat: 0.0022)
                let holder = Entity()
                holder.addChild(model)
                holder.scale = .init(repeating: metricScale)
                holder.name = "cloud"
                content.add(holder)

                let s = cloud.size * metricScale
                // Frame the bounding sphere: the distance at which it just fills the FOV.
                let r = simd_length(s) / 2
                let d = r / sin(fov * .pi / 180 / 2) * 1.15
                shellRadius = r
                framedDistance = d
                distance = d

                let origin: String
                if case .fixture = source { origin = " · demo cloud" } else { origin = "" }
                status = String(format: "%d pts · %.1f × %.1f × %.1f m%@",
                                cloud.count, s.x, s.y, s.z, origin)
            } catch {
                status = "scan unavailable — \(error.localizedDescription)"
            }
        } update: { content in
            guard let rig = content.entities.first(where: { $0.name == "rig" }),
                  let camera = rig.findEntity(named: "camera") else { return }

            // Orbit the cloud's centre (the cloud is already centroid-centred at origin).
            let orientation = simd_quatf(angle: yaw, axis: [0, 1, 0])
                * simd_quatf(angle: pitch, axis: [1, 0, 0])
            rig.orientation = orientation
            camera.position = .init(0, 0, distance)

            if let holder = content.entities.first(where: { $0.name == "cloud" }),
               let model = holder.children.first as? ModelEntity {
                apply(skin, to: model)
            }
        }
        .gesture(
            DragGesture()
                .onChanged { v in
                    let t = SIMD2<Float>(Float(v.translation.width), Float(v.translation.height))
                    let d = t - dragStart
                    dragStart = t
                    yaw   -= d.x * 0.006
                    pitch  = max(-1.35, min(1.35, pitch - d.y * 0.006))
                }
                .onEnded { _ in dragStart = .zero }
        )
        .simultaneousGesture(
            MagnifyGesture()
                .onChanged { v in
                    let base = pinchStart ?? distance
                    if pinchStart == nil { pinchStart = distance }
                    // Dolly from well inside the shell out to 2.5× the framing distance.
                    distance = max(shellRadius * 0.12,
                                   min(framedDistance * 2.5, base / Float(v.magnification)))
                    inside = distance < shellRadius
                }
                .onEnded { _ in pinchStart = nil }
        )
        .background(.black)
    }

    /// Re-materialise in place — the geometry is never rebuilt, only re-skinned.
    private func apply(_ skin: SpaceSkin, to model: ModelEntity) {
        guard var material = model.model?.materials.first as? UnlitMaterial else { return }
        if let tint = skin.tint {
            // Keep the atlas as the alpha/structure source, drive colour from the tint.
            material.color = .init(tint: tint, texture: material.color.texture)
        } else {
            material.color = .init(tint: .white, texture: material.color.texture)
        }
        model.model?.materials = [material]
    }
}

// MARK: - The POV screen

struct PlacePOVView: View {
    let focus: PlaceAnnotation
    let neighbours: [PlaceAnnotation]
    var avatarColor: Color = .indigo
    var onExit: () -> Void

    // NOTE: there is deliberately no capture entry point here. Scanning MINTS a
    // place from the live location fix (LIFE -> PLACES -> +); it never attaches a
    // scan to a place you happen to be looking at, because you can look at Wales
    // from San Francisco.
    @State private var skin: SpaceSkin = .captured
    @State private var status = "loading scan…"
    @ObservedObject private var scanStore = ScanStore.shared

    /// This Place's own scan, or nothing. Resolving here keeps SpaceInterior ignorant of
    /// Places and of ScanStore.
    ///
    /// NO FIXTURE FALLBACK. v1 cannot produce a scan at all (capture is orphaned — see
    /// Life/Space/SpatialCaptureView.swift), so falling back to the bundled demo cloud
    /// would show a stranger's kitchen for EVERY Place, every time. That is precisely the
    /// behaviour `CloudSource` was introduced to stop. Places minted from a coordinate
    /// have no interior, and the view says so rather than inventing one.
    ///
    /// The `.fixture` case is kept for v2, when a real scan can exist to compare against.
    private var cloudSource: CloudSource? {
        guard let scan = scanStore.scans(forPlace: focus.id).first, !scan.assetRef.isEmpty
        else { return nil }
        let url = FileManager.default
            .urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("pacific/scans", isDirectory: true)
            .appendingPathComponent(scan.assetRef)
        return FileManager.default.fileExists(atPath: url.path) ? .scan(url) : nil
    }
    @State private var inside = false
    @State private var showPublic = false
    @EnvironmentObject private var core: CoreService

    private var isPublicPlace: Bool {
        core.places.first { $0.id == focus.id }?.access == "public"
    }

    /// The reconstruction is metric only up to a similarity transform, so this is an
    /// ESTIMATE and the UI says so. ARKit VIO resolves it in the capture path.
    private var metricScale: Float { isScanned ? 1.0 : 2.5 }

    /// A Place minted from a coordinate has no interior to walk. v1 captures no geometry,
    /// so this is the normal case, not an error — it reads as a plain statement rather
    /// than a failure, and does not offer a button that cannot do anything yet.
    private var noInterior: some View {
        ZStack {
            LinearGradient(colors: [Theme.bg, Theme.bg.opacity(0.86)],
                           startPoint: .top, endPoint: .bottom)
                .ignoresSafeArea()
            VStack(spacing: 10) {
                Image(systemName: "mappin.and.ellipse")
                    .font(.system(size: 34, weight: .light))
                    .foregroundStyle(tint.opacity(0.7))
                Text("No interior")
                    .font(.system(size: 15, weight: .semibold)).foregroundStyle(.white.opacity(0.9))
                Text("This place is a coordinate, not a scan.")
                    .font(.system(size: 12)).foregroundStyle(.white.opacity(0.5))
            }
        }
    }

    private var tint: Color { LifeFilter.places.color }
    private var isScanned: Bool { if case .scan? = cloudSource { return true } else { return false } }

    var body: some View {
        ZStack {
            if let source = cloudSource {
                SpaceInterior(source: source, skin: skin,
                              metricScale: metricScale, status: $status, inside: $inside)
                    .id(focus.id)   // a different Place rebuilds the scene rather than mutating it
                    .ignoresSafeArea()
            } else {
                // Nothing else writes `status` on this branch, so it would otherwise sit
                // on "loading scan…" forever for a Place that is not loading anything.
                noInterior.onAppear { status = "coordinate only" }
            }

            VStack(spacing: 0) {
                HStack {
                    chromeButton("xmark", action: onExit)
                    Spacer()
                    VStack(spacing: 2) {
                        Text(focus.name)
                            .font(.system(size: 17, weight: .bold)).foregroundStyle(.white)
                        Text(status)
                            .font(.system(size: 10, weight: .medium).monospacedDigit())
                            .foregroundStyle(.white.opacity(0.55))
                    }
                    .shadow(color: .black.opacity(0.6), radius: 4)
                    Spacer()
                    // Opening a place to the public lives HERE because this is where a
                    // place actually opens from: PLACES renders a map, and its pins come
                    // straight to this view. The same control in EntityDetail is only
                    // reachable from a list PLACES never shows.
                    chromeButton(isPublicPlace ? "qrcode" : "person.badge.plus") { showPublic = true }
                }
                .padding(.horizontal, 16).padding(.top, 8)

                Spacer()

                // Scale is unresolved until a metric anchor lands. Say it rather than
                // quietly presenting an estimate as truth.
                Label(inside ? "inside · pinch to pull back" : "pinch to step inside · drag to orbit",
                      systemImage: inside ? "arrow.down.right.and.arrow.up.left" : "hand.pinch")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(.white.opacity(0.75))
                    .padding(.bottom, 6)

                Label(isScanned ? "metric · ARKit anchored" : "scale estimated · no metric anchor yet",
                      systemImage: isScanned ? "ruler.fill" : "ruler")
                    .font(.system(size: 10, weight: .medium))
                    .foregroundStyle(.white.opacity(0.5))
                    .padding(.bottom, 10)

                skinPicker
                    .padding(.horizontal, 20)
                    .padding(.bottom, 24)
            }
        }
        .background(Color.black)
        .statusBarHidden()
        .sheet(isPresented: $showPublic) {
            PlacePublicSheet(placeID: focus.id, placeName: focus.name)
                .environmentObject(core)
        }
    }

    /// One dominant bottom-thumb control — swapping the skin on a real place.
    private var skinPicker: some View {
        HStack(spacing: 8) {
            ForEach(SpaceSkin.allCases) { s in
                Button {
                    withAnimation(.spring(response: 0.3, dampingFraction: 0.85)) { skin = s }
                } label: {
                    Label(s.rawValue, systemImage: s.glyph)
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(skin == s ? .black : .white)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 11)
                        .background(skin == s ? AnyShapeStyle(tint) : AnyShapeStyle(.ultraThinMaterial),
                                    in: Capsule())
                }
                .buttonStyle(.plain)
            }
        }
    }

    private func chromeButton(_ icon: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: icon)
                .font(.system(size: 17, weight: .semibold)).foregroundStyle(.white)
                .frame(width: 40, height: 40)
                .background(.ultraThinMaterial, in: Circle())
        }
        .buttonStyle(.plain)
    }
}
