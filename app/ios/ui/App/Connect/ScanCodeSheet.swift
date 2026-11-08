//  ScanCodeSheet.swift
//  The device-only half of pairing that the Simulator could never run: point the
//  camera at another device's Me-tab QR (a real signed contact bundle) and form the
//  encrypted 1:1. VisionKit's DataScannerViewController does the live capture; the
//  decoded payload runs the SAME `core.pairScan` handshake as "Enter a code".
//
//  The QR we scan is either a raw contact bundle (what QRView encodes) or a
//  `pacific://pair-scan?bundle=…` invite link — extractBundle handles both.

import SwiftUI
import VisionKit
import AVFoundation
import UIKit
import PacificCore

// MARK: - Live camera QR scanner (VisionKit)

/// Wraps DataScannerViewController for QR-only capture and hands the first
/// non-empty payload up exactly once per detection edge.
struct QRScanner: UIViewControllerRepresentable {
    var onScan: (String) -> Void

    func makeUIViewController(context: Context) -> DataScannerViewController {
        let vc = DataScannerViewController(
            recognizedDataTypes: [.barcode(symbologies: [.qr])],
            qualityLevel: .balanced,
            recognizesMultipleItems: false,
            isHighFrameRateTrackingEnabled: false,
            isHighlightingEnabled: true
        )
        vc.delegate = context.coordinator
        return vc
    }

    func updateUIViewController(_ vc: DataScannerViewController, context: Context) {
        // Idempotent: startScanning throws if already scanning — harmless to ignore.
        try? vc.startScanning()
    }

    static func dismantleUIViewController(_ vc: DataScannerViewController, coordinator: Coordinator) {
        vc.stopScanning()
    }

    func makeCoordinator() -> Coordinator { Coordinator(onScan: onScan) }

    final class Coordinator: NSObject, DataScannerViewControllerDelegate {
        let onScan: (String) -> Void
        init(onScan: @escaping (String) -> Void) { self.onScan = onScan }

        func dataScanner(_ s: DataScannerViewController, didAdd added: [RecognizedItem], allItems: [RecognizedItem]) {
            emit(added)
        }
        func dataScanner(_ s: DataScannerViewController, didTapOn item: RecognizedItem) {
            emit([item])
        }
        private func emit(_ items: [RecognizedItem]) {
            for case let .barcode(code) in items {
                if let value = code.payloadStringValue, !value.isEmpty {
                    onScan(value)
                    return
                }
            }
        }
    }
}

// MARK: - The scan sheet

/// Camera scan → real pairing. Reuses the exact `pairScan` → open-DM flow that
/// PasteCodeSheet uses, so a scanned code and a pasted code are indistinguishable
/// downstream. The in-flight `pairing` guard keeps a lingering QR from firing twice.
struct ScanCodeSheet: View {
    @EnvironmentObject private var core: CoreService
    @Environment(\.dismiss) private var dismiss
    var onOpen: (Conversation) -> Void

    @State private var pairing = false
    @State private var note: String?
    @State private var camStatus = AVCaptureDevice.authorizationStatus(for: .video)

    private var canScan: Bool {
        DataScannerViewController.isSupported && camStatus == .authorized
    }

    var body: some View {
        NavigationStack {
            Group {
                if canScan { scanner } else { fallback }
            }
            .navigationTitle("Scan a code")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .topBarLeading) { Button("Cancel") { dismiss() } } }
        }
        .task { await ensureCameraAccess() }
    }

    private var scanner: some View {
        ZStack(alignment: .bottom) {
            QRScanner { onScanned($0) }
                .ignoresSafeArea(edges: .bottom)
            Group {
                if pairing {
                    Label("Pairing…", systemImage: "arrow.triangle.2.circlepath")
                        .foregroundStyle(Color.primary)
                } else {
                    Text(note ?? "Point at a WallFlowers QR code — their Me tab.")
                        .foregroundStyle(note == nil ? Color.primary : Color.red)
                        .multilineTextAlignment(.center)
                }
            }
            .font(.subheadline.weight(.medium))
            .padding(.horizontal, 16).padding(.vertical, 11)
            .background(.ultraThinMaterial, in: Capsule())
            .padding(.bottom, 36)
            .padding(.horizontal, 24)
        }
    }

    private var fallback: some View {
        ContentUnavailableView {
            Label(fallbackTitle, systemImage: "camera.metering.unknown")
        } description: {
            Text(fallbackMessage)
        } actions: {
            if camStatus == .denied || camStatus == .restricted {
                Button("Open Settings") {
                    if let url = URL(string: UIApplication.openSettingsURLString) {
                        UIApplication.shared.open(url)
                    }
                }
                .buttonStyle(.pixel)
            }
        }
    }

    private var fallbackTitle: String {
        DataScannerViewController.isSupported ? "Camera access needed" : "Scanning unavailable"
    }
    private var fallbackMessage: String {
        DataScannerViewController.isSupported
            ? "Allow camera access to scan a WallFlowers QR code. You can also use “Enter a code” instead."
            : "This device can’t scan QR codes. Use “Enter a code” to paste a connect code instead."
    }

    private func ensureCameraAccess() async {
        if camStatus == .notDetermined {
            _ = await AVCaptureDevice.requestAccess(for: .video)
            camStatus = AVCaptureDevice.authorizationStatus(for: .video)
        }
    }

    private func onScanned(_ raw: String) {
        guard !pairing else { return }

        // A PUBLIC PLACE's poster is a pairing QR with a place named on it. It has to be
        // checked FIRST: `extractBundle` accepts any `pacific://` URL carrying a bundle,
        // so a place link would otherwise pair silently and drop the place — the scanner
        // would join nothing and never be told why.
        if let link = PublicPlace.joinLink(from: raw) {
            pairing = true
            note = nil
            Task {
                defer { pairing = false }
                guard await PublicPlace.knock(link, core: core) else {
                    note = core.error ?? "Couldn’t knock — that sign may have been retired."
                    return
                }
                // Deliberately NOT "you're in": knocking is not joining. A member of the
                // place admits us on their next pass, which needs one of their phones to
                // be about. Saying otherwise would promise something we cannot see land.
                note = "Knocked. You’ll be let in when someone there is around."
                // Try to answer our own doors too — if we are already in other places,
                // this is a good moment to let anyone waiting at those in.
                _ = await core.answerDoors()
            }
            return
        }

        let bundle = Self.extractBundle(raw)
        guard !bundle.isEmpty else {
            note = "That QR isn’t a WallFlowers connect code."
            return
        }
        pairing = true
        note = nil
        Task {
            defer { pairing = false }
            guard let peer = await core.pairScan(bundle) else {
                note = core.error ?? "Couldn’t pair — the code may be stale. Ask for a fresh QR."
                return
            }
            let title = core.connection(peer)?.displayName ?? String(peer.prefix(12)) + "…"
            dismiss()
            onOpen(Conversation(id: peer, kind: .dm, title: title, tint: tintFor(peer)))
        }
    }

    /// Accept both a raw contact bundle (what QRView encodes) and a
    /// `pacific://pair-scan?bundle=<percent-encoded>` invite link.
    private static func extractBundle(_ raw: String) -> String {
        let s = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        if let comps = URLComponents(string: s),
           comps.scheme == "pacific",
           let b = comps.queryItems?.first(where: { $0.name == "bundle" })?.value {
            return b
        }
        return s
    }
}
