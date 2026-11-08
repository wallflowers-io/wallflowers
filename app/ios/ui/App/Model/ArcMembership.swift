//  ArcMembership.swift
//  Joining an Arc — the device half of the Tether.
//
//  Signing up to an Arc IS the Pacific signup (ICD §10). We POST this device's contact bundle
//  to the Arc's entrance — `POST /v1/signup`, the one ungated write — and the Arc pair-scans it,
//  owns the resulting 2-member `member-tether`, seals a Welcome back to us over the relay, and
//  authors our default role ONTO that tether. Folding the Welcome (a sync) is what actually puts
//  us on the roster; until then the Arc has sealed an invitation we have not opened.
//
//  Membership is NOT a token the Arc issues. It is the MLS tether itself, so every gated plane is
//  authorised later by signing with the SAME identity key that established the tether — there is
//  nothing here to store, revoke, or leak. The role rides that same tether, so the device learns
//  its own standing by folding, not by being told.

import Foundation

/// Where this device stands with one Arc. Deliberately has no `.member` case: this device cannot
/// yet PROVE membership (that needs the signed `PacificMember` credential, which is the next
/// increment), so the furthest we honestly claim is that the Welcome was sealed and folded.
enum ArcJoinState: Equatable {
    case idle
    case joining
    /// The Arc sealed a Welcome and we synced to fold it. Membership is established as far as
    /// this device can observe WITHOUT exercising a gated plane. Renders as "Joined".
    case tethered
    /// A gated plane accepted a credential we minted — the Arc's own roster answered yes. This
    /// is the ONLY state that may be shown as "Member", because it is the only one backed by
    /// the authority (the Arc), rather than by a local record of our own intent.
    case verified
    case failed(String)

    var isBusy: Bool { self == .joining }
}

/// Joins Arcs and remembers which ones we have tethered to. One instance for the app.
@MainActor
final class ArcMembership: ObservableObject {
    /// Join state per Arc id — drives the button in the picker.
    @Published private(set) var state: [String: ArcJoinState] = [:]

    /// Arc ids this device has completed a join against, persisted so the UI survives relaunch.
    /// An optimistic local record, NOT proof: the Arc's roster is the authority, and a wiped Arc
    /// or a re-minted identity makes this stale. We re-verify by USING a gated plane, never by
    /// trusting this list.
    @Published private(set) var joined: Set<String> = []

    private let defaults = UserDefaults.standard
    private static let joinedKey = "pacific.joinedArcs"

    init() {
        joined = Set(defaults.stringArray(forKey: Self.joinedKey) ?? [])
    }

    func state(for arc: Arc) -> ArcJoinState {
        state[arc.id] ?? (joined.contains(arc.id) ? .tethered : .idle)
    }

    /// Join `arc`: POST our contact bundle to its entrance, then sync to fold the Welcome.
    ///
    /// Both halves matter. The POST gets us onto the Arc's roster (it owns the tether); the sync
    /// is what brings the Welcome — and the role delta the Arc authored — down to this device.
    /// Skipping the sync would leave us a member the Arc knows about but the device does not.
    @discardableResult
    func join(_ arc: Arc, core: CoreService) async -> Bool {
        guard arc.hasOrigin, let url = URL(string: Arc.join(arc.origin, "/v1/signup")) else {
            state[arc.id] = .failed("This Arc has no origin to join.")
            return false
        }
        state[arc.id] = .joining

        // A fresh signed bundle — the same artefact a peer would scan to connect to us.
        let bundle = core.contactBundle()
        guard !bundle.isEmpty else {
            state[arc.id] = .failed("Could not build this device's contact bundle.")
            return false
        }

        var req = URLRequest(url: url)
        req.httpMethod = "POST"
        req.setValue("text/plain", forHTTPHeaderField: "Content-Type")
        req.httpBody = Data(bundle.utf8)
        req.timeoutInterval = 20

        do {
            let (data, resp) = try await URLSession.shared.data(for: req)
            guard let http = resp as? HTTPURLResponse else {
                state[arc.id] = .failed("No response from the Arc.")
                return false
            }
            guard (200..<300).contains(http.statusCode) else {
                // Surface the Arc's own reason ({"error":{"message":…}}) rather than a bare code.
                state[arc.id] = .failed(Self.reason(data) ?? "The Arc refused the join (HTTP \(http.statusCode)).")
                return false
            }
        } catch {
            state[arc.id] = .failed(error.localizedDescription)
            return false
        }

        // Fold the sealed Welcome (and the role authored onto the tether). Without this the
        // tether exists only on the Arc's side.
        await core.syncNow(silent: true)

        joined.insert(arc.id)
        defaults.set(Array(joined), forKey: Self.joinedKey)
        state[arc.id] = .tethered
        // Signing up is US asserting; being accepted is THE ARC answering. Ask it.
        await verify(arc)
        return true
    }

    /// Prove membership by USING it — the check the old code said it would do and never did.
    ///
    /// `/v1/arcs` is behind the membership gate (`/v1/health` and `/v1/arc` are not), so a 2xx
    /// means this device's minted credential was accepted by the Arc's own roster, and a 401
    /// means it was not. Either way the answer comes from the authority rather than from a
    /// `UserDefaults` set recording our own intent — which is the entire difference between
    /// "Joined" and "Member".
    ///
    /// Non-destructive on a network failure: an unreachable Arc is not a refusal, so we leave
    /// the state alone rather than downgrading someone for being on a train.
    @discardableResult
    func verify(_ arc: Arc) async -> Bool {
        guard arc.hasOrigin, let url = URL(string: Arc.join(arc.origin, "/v1/arcs")) else { return false }
        // The audience we sign for is the ARC's space id, from its own `/v1/arc` document —
        // never a locally cached guess, because signing for the wrong audience would fail in a
        // way indistinguishable from being refused.
        guard let space = await ArcDiscovery.capabilities(origin: arc.origin)?.spaceId,
              !space.isEmpty,
              let mint = ArcCredential.provider,
              let credential = try? await mint(space) else {
            // Nothing to prove membership WITH, so we cannot claim it.
            if state[arc.id] == .verified { state[arc.id] = .tethered }
            return false
        }
        var req = URLRequest(url: url)
        req.timeoutInterval = 12
        req.setValue(credential, forHTTPHeaderField: "Authorization")
        guard let (_, resp) = try? await URLSession.shared.data(for: req),
              let http = resp as? HTTPURLResponse else { return false }   // unreachable ≠ refused

        Trace.shared.log("membership", "\(arc.name): gated plane answered HTTP \(http.statusCode)",
                         ok: (200..<300).contains(http.statusCode))
        switch http.statusCode {
        case 200..<300:
            state[arc.id] = .verified
            return true
        case 401, 403:
            // The Arc says no, and the Arc is the authority — so DROP the local claim
            // rather than keep rendering "Joined" over a refusal.
            //
            // This is the dead end that stranded a device permanently: holding `.tethered`
            // on a 401 left the row's only affordance as "Leave" (see Settings.tap), so a
            // device whose identity the Arc does not recognise — re-minted key, wiped Arc,
            // signup that never folded — could never re-join without first appearing to
            // quit something it was never part of. Clearing the record makes the row read
            // "Join" and one tap re-establishes the tether.
            leave(arc)   // same three writes, one owner
            Trace.shared.log("membership", "\(arc.name): refused this device — local join record cleared", ok: false)
            return false
        default:
            return false
        }
    }

    /// Leave `arc`: drop this device's local membership record so the list flips back to "Join"
    /// at once. The tether on the Arc's side is torn down on its next fold; locally we stop
    /// claiming membership immediately (the same optimistic-local stance as `joined`).
    func leave(_ arc: Arc) {
        joined.remove(arc.id)
        defaults.set(Array(joined), forKey: Self.joinedKey)
        state[arc.id] = .idle
    }

    /// Pull `error.message` out of the Arc's error envelope, if that's what came back.
    private static func reason(_ data: Data) -> String? {
        guard let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let err = obj["error"] as? [String: Any],
              let msg = err["message"] as? String, !msg.isEmpty
        else { return nil }
        return msg
    }
}
