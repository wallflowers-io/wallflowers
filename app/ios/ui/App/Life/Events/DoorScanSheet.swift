//  DoorScanSheet.swift
//  The door, on the organizer's phone: point the camera at a wallet QR and get ONE
//  verdict, full-screen, in the only three colours a doorway reads at arm's length.
//
//  The check is joined in core (`event_door_check`): signature against the signer THIS
//  device's fold names, then ledger ∩ redemptions. GREEN admits — and authors the
//  redemption Delta immediately (OR-set; converges across doors), then tells the HOLDER
//  (`contact.admitTicket`) so their own screen can go green too: the redemption lives in
//  the Event fold, which the buyer is deliberately not a member of, so the receipt on the
//  pairwise channel is the only truthful route. That receipt is a courtesy, never a gate —
//  no connection with the buyer means no receipt and the door still reads VALID.
//  RED refuses: a bad signature, or a ticket already redeemed. AMBER is the honest middle —
//  "unknown" means NOT IN THIS DEVICE'S LEDGER YET, which on a stale fold is a sync gap,
//  not fraud, so it reads UNVERIFIED and never a hard red (D24).

import SwiftUI
import VisionKit
import PacificCore

/// The event whose door is open — an identifiable id for `fullScreenCover(item:)`.
/// `qr` is the DEV seam: a simulator has no camera, so the two-sim harness hands the
/// wallet's verbatim payload in and it runs the SAME `onScanned` path a lens would.
struct DoorLink: Identifiable {
    let id: String
    var qr: String? = nil
}

struct DoorScanSheet: View {
    let objectId: String
    /// Injected payload (DEBUG harness only); nil = the camera is the source.
    var injectedQR: String? = nil
    @EnvironmentObject private var core: CoreService
    @Environment(\.dismiss) private var dismiss

    /// The verdict on screen; nil = scanning. Tapping it resumes the scan.
    @State private var verdict: DoorCheckRow?

    var body: some View {
        NavigationStack {
            ZStack {
                scanGround
                if let verdict {
                    verdictScreen(verdict)
                }
            }
            .navigationTitle("DOOR")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) { Button("Done") { dismiss() } }
            }
        }
        .task {
            // One shot, same door: an injected payload enters through `onScanned`, so
            // nothing about the verdict, the redemption or the receipt is special-cased.
            if let injectedQR, !injectedQR.isEmpty { onScanned(injectedQR) }
        }
        #if DEBUG
        // The NEXT person steps up: a further `door-scan` on an already-open door is the
        // same event the camera raises — clear the standing verdict and check the new
        // credential. DEBUG only; on hardware the lens is the only source.
        .onReceive(NotificationCenter.default.publisher(for: .pacificDoorScan)) { note in
            guard note.userInfo?["object"] as? String == objectId,
                  let qr = note.userInfo?["qr"] as? String, !qr.isEmpty else { return }
            verdict = nil          // the standing verdict clears, as a tap would clear it
            onScanned(qr)
        }
        #endif
    }

    /// The live camera where there is one. `DataScannerViewController` is unsupported on
    /// the simulator (and on hardware without the neural scanner), where it renders a dead
    /// view and still asks for camera access — so state the absence instead.
    @ViewBuilder private var scanGround: some View {
        if DataScannerViewController.isSupported {
            QRScanner { onScanned($0) }
                .ignoresSafeArea(edges: .bottom)
        } else {
            Text("NO CAMERA")
                .font(.system(size: 18, weight: .semibold))
                .tracking(1.4)
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(Theme.bg.ignoresSafeArea())
        }
    }

    private func onScanned(_ qr: String) {
        guard verdict == nil else { return }   // one verdict at a time
        guard let row = core.doorCheck(objectId, qr: qr) else { return }
        verdict = row
        guard row.verdict == "valid" else {
            UINotificationFeedbackGenerator()
                .notificationOccurred(row.verdict == "unknown" ? .warning : .error)
            report(row, receipt: false)
            return
        }
        UINotificationFeedbackGenerator().notificationOccurred(.success)
        let at = Int64(Date().timeIntervalSince1970 * 1000)
        Task {
            _ = await core.redeem(objectId, ticket: row.ticket)
            // The holder's half of the green. Non-fatal by construction: the buyer's pk
            // rides the verified credential core, and with no channel to them
            // `admitReceipt` returns false and logs — the door still reads VALID.
            let sent = await core.admitReceipt(buyer: row.buyer, ticket: row.ticket,
                                               admittedMs: at)
            if !sent { NSLog("[door] admitted %@ but the holder was not told", row.ticket) }
            report(row, receipt: sent)
        }
    }

    /// DEV SEAM: a SwiftUI verdict screen is not something a host script can read, so the
    /// door also states its outcome as one machine-readable line + a file in the container
    /// (`dev-door.txt`). DEBUG only — a shipped door reports to the person holding it.
    private func report(_ row: DoorCheckRow, receipt: Bool) {
        #if DEBUG
        core.dumpDoorVerdict(row, receipt: receipt)
        #endif
    }

    /// The whole screen is the verdict — colour, word, ticket title. Tap to scan the next.
    private func verdictScreen(_ row: DoorCheckRow) -> some View {
        VStack(spacing: 14) {
            Text(word(row.verdict))
                .font(.system(size: 40, weight: .heavy))
                .tracking(1.2)
                .foregroundStyle(.white)
                .accessibilityIdentifier("door-verdict")
            if !row.title.isEmpty {
                Text(row.title)
                    .font(.system(size: 20, weight: .semibold))
                    .foregroundStyle(.white.opacity(0.85))
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(colour(row.verdict).ignoresSafeArea())
        .contentShape(Rectangle())
        .onTapGesture { verdict = nil }
        // The whole ground names the verdict — `door-verdict-valid` is A's green screen —
        // so a test asserts WHICH verdict rather than reading pixels. The word inside
        // keeps the plain `door-verdict` landmark.
        .accessibilityIdentifier("door-verdict-\(row.verdict)")
    }

    private func word(_ v: String) -> String {
        switch v {
        case "valid":    return "VALID"
        case "redeemed": return "REDEEMED"
        case "double":   return "DOUBLE SCAN"
        case "bad_sig":  return "BAD SIGNATURE"
        case "bad_qr":   return "BAD QR"
        default:         return "UNVERIFIED"
        }
    }

    private func colour(_ v: String) -> Color {
        switch v {
        case "valid":   return .green
        case "unknown": return .orange
        default:        return .red
        }
    }
}
