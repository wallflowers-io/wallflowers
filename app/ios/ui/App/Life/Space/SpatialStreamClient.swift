//  SpatialStreamClient.swift
//
//  V2 — NOT IN THE v1 BUILD. Orphaned alongside SpatialCaptureView (which is its only
//  caller) and excluded from the target in `ui/project.yml`. See the header of
//  SpatialCaptureView.swift for why, and for how to restore both.
//
//  `StreamedLayout` lives here, which is why Life.swift can no longer name it: the v1
//  Place mint takes a coordinate and optional photos, not a layout.
//
//  Streams ARKit frames to the arc/spatial reconstruction service and receives back
//  geometry and an architectural layout.
//
//  Why iOS is the preferred client, and not just a convenient one: a monocular
//  reconstruction cannot recover absolute scale or which way is up. Both are
//  Sim(3)-ambiguous from pixels alone — the VGGT kitchen sample measures 1.61 m for a
//  scene that is really a table. ARKit's VIO fuses the IMU, so `camera.transform` is
//  METRIC and, with `.gravity` world alignment, GRAVITY-ALIGNED. Sending the pose and
//  gravity alongside each frame is therefore not telemetry — it is the two pieces of
//  information the server cannot derive, and it is what lets the layout stage report
//  `scale_resolved: true` and `up_source: gravity` instead of honestly refusing.
//
//  Wire format mirrors spatial/protocol.py exactly:
//      <UInt32 header_len><UInt32 payload_len><JSON header><payload>
//  little-endian, and `type` is a field inside the JSON header.

import Foundation
import Network
import ARKit
import simd

// MARK: - Protocol

/// One message on the wire. Mirrors `spatial.protocol.Message`.
struct SpatialMessage {
    let type: String
    let header: [String: Any]
    let payload: Data

    func encoded() throws -> Data {
        var h = header
        h["type"] = type
        let hb = try JSONSerialization.data(withJSONObject: h, options: [.sortedKeys])
        var out = Data(capacity: 8 + hb.count + payload.count)
        withUnsafeBytes(of: UInt32(hb.count).littleEndian) { out.append(contentsOf: $0) }
        withUnsafeBytes(of: UInt32(payload.count).littleEndian) { out.append(contentsOf: $0) }
        out.append(hb)
        out.append(payload)
        return out
    }
}

/// The layout the server returns for a finished session — the architectural model.
struct StreamedLayout {
    let backend: String
    /// `gravity` | `camera_trajectory` | `convention` | `none`. Anything but the first
    /// two means the server could not orient the scene and did not pretend otherwise.
    let upSource: String
    let scaleResolved: Bool
    let shellDetected: Bool
    let shellReason: String
    let surfaces: [[String: Any]]

    init?(header: [String: Any]) {
        guard let surfaces = header["surfaces"] as? [[String: Any]] else { return nil }
        self.backend = header["backend"] as? String ?? "unknown"
        self.upSource = header["up_source"] as? String ?? "none"
        self.scaleResolved = header["scale_resolved"] as? Bool ?? false
        self.shellDetected = header["shell_detected"] as? Bool ?? false
        self.shellReason = header["shell_reason"] as? String ?? ""
        self.surfaces = surfaces
    }
}

// MARK: - Client

@MainActor
final class SpatialStreamClient: NSObject, ObservableObject {

    enum State: Equatable {
        case idle, connecting, streaming(frames: Int), finishing
        case finished(points: Int), failed(String)
    }

    @Published private(set) var state: State = .idle
    @Published private(set) var layout: StreamedLayout?
    /// Incremental clouds, newest last — one per reconstruction window.
    @Published private(set) var geometryChunks: [Data] = []
    /// The fused cloud from DONE. This is what gets persisted for the Place; the
    /// per-window chunks are progress, this is the result.
    @Published private(set) var fusedGLB: Data?

    private var connection: NWConnection?
    private var session: ARSession?
    private var frameIndex = 0
    private var lastSent = TimeInterval(0)
    private var inbox = Data()

    /// VGGT needs BASELINE between views, not frame rate — 60 fps of near-identical
    /// frames reconstructs no better than 6 and costs 10x the bandwidth.
    private let targetFPS: Double = 6
    private let jpegQuality: CGFloat = 0.7

    // MARK: Lifecycle

    func start(host: String, port: UInt16, session: ARSession) {
        guard case .idle = state else { return }
        state = .connecting
        self.session = session
        frameIndex = 0
        geometryChunks = []
        layout = nil

        let conn = NWConnection(host: .init(host), port: .init(rawValue: port)!, using: .tcp)
        connection = conn
        conn.stateUpdateHandler = { [weak self] st in
            Task { @MainActor in
                guard let self else { return }
                switch st {
                case .ready:
                    self.send(SpatialMessage(type: "hello",
                                             header: ["session": UUID().uuidString,
                                                      "window": 8, "stride": 4],
                                             payload: Data()))
                    self.state = .streaming(frames: 0)
                    self.receive()
                    session.delegate = self
                case .failed(let e): self.state = .failed(e.localizedDescription)
                case .cancelled: if case .finished = self.state {} else { self.state = .idle }
                default: break
                }
            }
        }
        conn.start(queue: .global(qos: .userInitiated))
    }

    /// Close the stream; the server replies DONE then LAYOUT.
    func finish() {
        guard case .streaming = state else { return }
        state = .finishing
        session?.delegate = nil
        send(SpatialMessage(type: "end", header: [:], payload: Data()))
    }

    func cancel() {
        session?.delegate = nil
        connection?.cancel()
        connection = nil
        state = .idle
    }

    // MARK: Send

    private func send(_ msg: SpatialMessage) {
        guard let conn = connection, let data = try? msg.encoded() else { return }
        conn.send(content: data, completion: .contentProcessed { _ in })
    }

    /// One ARKit frame: JPEG pixels + the metric pose + gravity.
    fileprivate func send(frame: ARFrame) {
        let now = frame.timestamp
        guard now - lastSent >= 1.0 / targetFPS else { return }
        lastSent = now

        guard let jpeg = Self.jpeg(from: frame.capturedImage, quality: jpegQuality) else { return }
        let width = CVPixelBufferGetWidth(frame.capturedImage)
        let height = CVPixelBufferGetHeight(frame.capturedImage)

        // ARKit gives column-major simd; the wire wants 16 floats row-major.
        let t = frame.camera.transform
        let pose: [Float] = (0..<4).flatMap { r in (0..<4).map { c in t[c][r] } }

        // With `.gravity` world alignment ARKit's world Y is up, so gravity is -Y.
        // Sent explicitly rather than assumed, so the server never has to infer it.
        let gravity: [Float] = [0, -1, 0]

        send(SpatialMessage(
            type: "frame",
            header: ["index": frameIndex, "ts_ms": now * 1000,
                     "width": width, "height": height, "format": "jpeg",
                     "pose": pose.map { Double($0) },
                     "gravity": gravity.map { Double($0) }],
            payload: jpeg))
        frameIndex += 1
        state = .streaming(frames: frameIndex)
    }

    private static func jpeg(from buffer: CVPixelBuffer, quality: CGFloat) -> Data? {
        let ci = CIImage(cvPixelBuffer: buffer)
        let ctx = CIContext()
        guard let cg = ctx.createCGImage(ci, from: ci.extent) else { return nil }
        return UIImage(cgImage: cg).jpegData(compressionQuality: quality)
    }

    // MARK: Receive

    private func receive() {
        connection?.receive(minimumIncompleteLength: 1, maximumLength: 1 << 20) {
            [weak self] data, _, isComplete, error in
            Task { @MainActor in
                guard let self else { return }
                if let error { self.state = .failed(error.localizedDescription); return }
                if let data, !data.isEmpty { self.inbox.append(data); self.drain() }
                if isComplete { self.connection?.cancel() } else { self.receive() }
            }
        }
    }

    /// Pull whole messages out of the buffer; a short read simply waits for more.
    private func drain() {
        while inbox.count >= 8 {
            let hLen = Int(inbox.withUnsafeBytes { $0.loadUnaligned(fromByteOffset: 0, as: UInt32.self).littleEndian })
            let pLen = Int(inbox.withUnsafeBytes { $0.loadUnaligned(fromByteOffset: 4, as: UInt32.self).littleEndian })
            let total = 8 + hLen + pLen
            guard inbox.count >= total else { return }

            let headerData = inbox.subdata(in: 8..<(8 + hLen))
            let payload = inbox.subdata(in: (8 + hLen)..<total)
            inbox.removeSubrange(0..<total)

            guard let header = (try? JSONSerialization.jsonObject(with: headerData)) as? [String: Any],
                  let type = header["type"] as? String else { continue }
            handle(type: type, header: header, payload: payload)
        }
    }

    private func handle(type: String, header: [String: Any], payload: Data) {
        switch type {
        case "geometry":
            geometryChunks.append(payload)
        case "done":
            fusedGLB = payload.isEmpty ? nil : payload
            state = .finished(points: header["n_points"] as? Int ?? 0)
        case "layout":
            layout = StreamedLayout(header: header)
        case "error":
            state = .failed(header["message"] as? String ?? "server error")
        default:
            break
        }
    }
}

// MARK: - ARSessionDelegate

extension SpatialStreamClient: ARSessionDelegate {
    nonisolated func session(_ session: ARSession, didUpdate frame: ARFrame) {
        Task { @MainActor in self.send(frame: frame) }
    }
}
