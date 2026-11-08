//  SpatialCaptureView.swift
//
//  ┌─────────────────────────────────────────────────────────────────────────────┐
//  │ V2 — NOT IN THE v1 BUILD.                                                   │
//  │                                                                             │
//  │ v1 ships no spatial processing, so this file is ORPHANED, not deleted: it   │
//  │ is excluded from the target in `ui/project.yml` and nothing in the      │
//  │ shipping build references it. It stays on disk because the work is sound    │
//  │ and the decision was about scope, not correctness.                          │
//  │                                                                             │
//  │ The island is self-contained — this view, SpatialStreamClient (also         │
//  │ excluded), and `mintScannedPlace` at the foot of this file, which used to   │
//  │ live in Life.swift. It depends only on things v1 still ships: PlaceFix,     │
//  │ PlaceLocator, ScanStore, OutlineModel.                                      │
//  │                                                                             │
//  │ To restore: drop the two excludes from project.yml, run `xcodegen           │
//  │ generate`, and point Life.swift's `mintCover` at this view for `.places`    │
//  │ (v1 mints a Place from a coordinate + optional photos in EntityWizard).     │
//  │                                                                             │
//  │ Blocking v1: reconstruction ran on a LAN Python service (arc/spatial) at a  │
//  │ hardcoded host, so a scan only worked on one Wi-Fi and the pixels left the  │
//  │ device. On-device MapAnything is the intended fix and is a real project —   │
//  │ 1.2B params, int4 to fit, fixed view count.                                 │
//  └─────────────────────────────────────────────────────────────────────────────┘
//
//  Capture a Place: walk the room, frames stream to arc/spatial, geometry and an
//  architectural layout stream back.
//
//  This is the AR toggle in PlacePOV finally doing something. It was a @State bool
//  nothing read; the affordance was already in the right place, it just had nothing
//  behind it.
//
//  `worldAlignment = .gravity` is the load-bearing line. It makes ARKit's world Y the
//  true vertical and `camera.transform` metric, which is what the server needs and
//  cannot derive: a monocular reconstruction is Sim(3)-ambiguous, so without this the
//  layout stage honestly refuses to name a floor. With it, the returned layout reports
//  `up_source: gravity` and `scale_resolved: true`.

import SwiftUI
import ARKit
import RealityKit

// MARK: - AR preview

/// Live camera with ARKit tracking. Non-AR rendering stays the daily driver (see
/// PlacePOV); this view is the capture mode, entered deliberately.
private struct ARPreview: UIViewRepresentable {
    let session: ARSession

    func makeUIView(context: Context) -> ARSCNView {
        let view = ARSCNView(frame: .zero)
        view.session = session
        view.automaticallyUpdatesLighting = true
        return view
    }
    func updateUIView(_ uiView: ARSCNView, context: Context) {}
}

// MARK: - Capture

struct SpatialCaptureView: View {
    /// Minting a NEW place from this scan. A scan is evidence of being somewhere, so
    /// it creates its own Place bound to the live fix — it never attaches to a Place
    /// chosen from a list, which is how a Welsh cottage ends up holding a scan of a
    /// San Francisco living room.
    /// `(fix, layout, points, fusedGLB)` — the caller mints the Place and owns
    /// persistence, so this view never needs to know what a Place is.
    var onMinted: (PlaceFix, StreamedLayout?, Int, Data?) -> Void = { _, _, _, _ in }
    /// Host of the arc/spatial server. Persisted so it survives a relaunch.
    @AppStorage("spatial.host") private var host: String = ""
    @AppStorage("spatial.port") private var port: String = "8799"

    @StateObject private var client = SpatialStreamClient()
    @StateObject private var locator = PlaceLocator()
    @State private var session = ARSession()
    @State private var tracking = "starting…"
    var onExit: () -> Void

    private var tint: Color { LifeFilter.places.color }

    var body: some View {
        ZStack {
            ARPreview(session: session).ignoresSafeArea()

            VStack(spacing: 0) {
                header
                Spacer()
                statusCard
                controls
            }
        }
        .background(.black)
        .statusBarHidden()
        .onAppear { startSession(); locator.start() }
        .onDisappear { session.pause(); client.cancel(); locator.stop() }
    }

    // MARK: Session

    private func startSession() {
        let config = ARWorldTrackingConfiguration()
        // The whole point: metric, gravity-aligned poses.
        config.worldAlignment = .gravity
        config.environmentTexturing = .none
        if ARWorldTrackingConfiguration.supportsFrameSemantics(.sceneDepth) {
            config.frameSemantics.insert(.sceneDepth)   // free on LiDAR devices
        }
        session.run(config, options: [.resetTracking, .removeExistingAnchors])
    }

    // MARK: Chrome

    private var header: some View {
        HStack {
            chrome("xmark") { session.pause(); client.cancel(); onExit() }
            Spacer()
            VStack(spacing: 2) {
                Text("New place").font(.system(size: 16, weight: .bold))
                Text(host + ":" + port)
                    .font(.system(size: 10, weight: .medium).monospacedDigit())
                    .foregroundStyle(.white.opacity(0.55))
            }
            .foregroundStyle(.white)
            .shadow(color: .black.opacity(0.6), radius: 4)
            Spacer()
            chrome("arrow.clockwise") { startSession() }
        }
        .padding(.horizontal, 16).padding(.top, 8)
    }

    /// State, stated plainly. A capture that is silently not streaming is the worst
    /// outcome, so every stage says what it is doing.
    private var statusCard: some View {
        VStack(alignment: .leading, spacing: 6) {
            row("here", fixLabel, warn: locator.state.fix?.isUsable != true)
            row("tracking", tracking)
            row("stream", streamLabel)
            row("geometry", "\(client.geometryChunks.count) window\(client.geometryChunks.count == 1 ? "" : "s")")
            if let l = client.layout {
                Divider().overlay(.white.opacity(0.15))
                row("layout", "\(l.surfaces.count) surfaces · \(l.backend)")
                row("up axis", l.upSource, warn: l.upSource != "gravity" && l.upSource != "camera_trajectory")
                row("scale", l.scaleResolved ? "metric" : "unresolved", warn: !l.scaleResolved)
                row("shell", l.shellDetected ? "room" : "not a room")
                if !l.shellDetected {
                    Text(l.shellReason)
                        .font(.system(size: 10)).foregroundStyle(.orange.opacity(0.9))
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .padding(.horizontal, 16)
    }

    private func row(_ k: String, _ v: String, warn: Bool = false) -> some View {
        HStack {
            Text(k).font(.system(size: 11, weight: .medium)).foregroundStyle(.white.opacity(0.55))
            Spacer()
            Text(v).font(.system(size: 11, weight: .semibold).monospacedDigit())
                .foregroundStyle(warn ? .orange : .white)
        }
    }

    /// The fix, stated with its own error bar — the geohash length follows from it.
    private var fixLabel: String {
        switch locator.state {
        case .idle, .requesting: return "locating…"
        case .denied: return "location denied"
        case .searching: return "searching…"
        case .located(let f):
            return f.isUsable ? "\(f.cellLabel) · \(f.accuracyLabel)"
                              : "too coarse (\(f.accuracyLabel))"
        }
    }

    private var streamLabel: String {
        switch client.state {
        case .idle: return "not started"
        case .connecting: return "connecting…"
        case .streaming(let n): return "\(n) frames sent"
        case .finishing: return "reconstructing…"
        case .finished(let p): return "\(p) points"
        case .failed(let e): return "failed: \(e)"
        }
    }

    private var controls: some View {
        HStack(spacing: 10) {
            switch client.state {
            case .idle, .failed:
                // No usable fix, no scan. Refusing is the point: a scan that cannot say
                // where it was taken is not evidence, and a Place minted from it would
                // be a guess wearing a coordinate.
                let fix = locator.state.fix
                Button { if let f = fix, f.isUsable {
                    client.start(host: host, port: UInt16(port) ?? 8799, session: session) } } label: {
                    Label(fix?.isUsable == true ? "Scan this place" : "Waiting for a location fix",
                          systemImage: fix?.isUsable == true ? "record.circle" : "location.slash")
                        .font(.system(size: 16, weight: .semibold))
                        .foregroundStyle(fix?.isUsable == true ? .black : .white.opacity(0.6))
                        .frame(maxWidth: .infinity).padding(.vertical, 14)
                        .background(fix?.isUsable == true ? AnyShapeStyle(tint)
                                                          : AnyShapeStyle(.ultraThinMaterial), in: Capsule())
                }
                .disabled(fix?.isUsable != true)
            case .streaming:
                Button { client.finish() } label: {
                    Label("Finish", systemImage: "stop.circle")
                        .font(.system(size: 16, weight: .semibold)).foregroundStyle(.white)
                        .frame(maxWidth: .infinity).padding(.vertical, 14)
                        .background(.red, in: Capsule())
                }
            case .finished(let points):
                Button {
                    if let f = locator.state.fix {
                        onMinted(f, client.layout, points, client.fusedGLB)
                    }
                } label: {
                    Label("Save place", systemImage: "checkmark.circle.fill")
                        .font(.system(size: 16, weight: .semibold)).foregroundStyle(.black)
                        .frame(maxWidth: .infinity).padding(.vertical, 14)
                        .background(tint, in: Capsule())
                }
            default:
                Text(streamLabel)
                    .font(.system(size: 14, weight: .semibold)).foregroundStyle(.white.opacity(0.7))
                    .frame(maxWidth: .infinity).padding(.vertical, 14)
            }
        }
        .buttonStyle(.plain)
        .padding(.horizontal, 20).padding(.top, 12).padding(.bottom, 24)
    }

    private func chrome(_ icon: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: icon)
                .font(.system(size: 17, weight: .semibold)).foregroundStyle(.white)
                .frame(width: 40, height: 40)
                .background(.ultraThinMaterial, in: Circle())
        }
        .buttonStyle(.plain)
    }
}

// MARK: - Minting a Place from a scan (v2)

/// Moved here from `Life.swift` when the scan path was orphaned, so the v2 island is
/// complete: restoring capture does not also require reconstructing this function from
/// history. It layers the scan-specific parts — the fused cloud on disk and its
/// ScanStore registration — on top of the ordinary coordinate mint that v1 still uses.
@MainActor
enum ScanMint {
    static func place(core: CoreService, fix: PlaceFix,
                      layout: StreamedLayout?, points: Int, glb: Data?) {
        let surfaces = layout?.surfaces.count ?? 0
        let descriptor = layout.map { l in
            "\(points) pts · \(surfaces) surfaces · \(l.shellDetected ? "room" : l.shellReason)"
        } ?? "\(points) pts"

        // The Place GroupObject + its location facet are NOT scan-specific — a Place minted
        // from a coordinate needs exactly the same thing — so v1 owns that and this reuses
        // it, then layers the scan on top.
        guard let id = PlaceMint.place(core: core, fix: fix, name: fix.geohash,
                                       descriptor: descriptor, note: nil)
        else { return }

        // Persist the fused cloud beside the Place and register it, so walking up to
        // this Place later renders THIS room. Without this the scan dies with the sheet.
        guard let glb, !glb.isEmpty else { return }
        let dir = FileManager.default
            .urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("pacific/scans", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let file = "\(id).glb"
        if (try? glb.write(to: dir.appendingPathComponent(file), options: .atomic)) != nil {
            ScanStore.shared.register(Scan(
                placeID: id, name: fix.geohash, pointCount: points,
                format: "glb", assetRef: file, device: "iPhone",
                fusion: layout?.upSource == "gravity" ? "arkit" : "overlap"))
        }
    }
}
