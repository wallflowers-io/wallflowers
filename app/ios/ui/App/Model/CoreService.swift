//  CoreService.swift
//  The one seam from the SwiftUI app into the real Rust core (pacific-ffi).
//  Points pacific-core at the app sandbox and exposes the live GroupObject surface —
//  the same primitives the desktop studio drives via the `pacific` CLI.
//
//  Relay resolution: PACIFIC_RELAY_URL env (simulator/dev runs — set via
//  SIMCTL_CHILD_PACIFIC_RELAY_URL) → the "pacific.relayUrl" default → the
//  canonical production relay baked into the core. PACIFIC_IDENTITY_NAME names
//  the identity on FIRST mint only (so two demo simulators aren't both "Me").

import Foundation
import SwiftUI
import UIKit        // event media: downscale/compress stills under the fold's b64 caps
import PacificCore

@MainActor
final class CoreService: ObservableObject {
    private let core: Core

    @Published var spaceId = ""
    @Published var sas = ""
    /// The read-aloud three-word "verify you match" rendering of the SAS. Same
    /// identity → same words on every device (deterministic in the core).
    @Published var sasWords = ""
    /// The SAS words dot-separated for display — the eye reads three distinct
    /// tokens ("buzz · payment · pottery") rather than a phrase making a claim.
    var sasWordsDotted: String { sasWords.replacingOccurrences(of: " ", with: " · ") }
    /// This device's REAL identity display name (from the core, chosen at first
    /// mint). The self avatar + Me profile read this, so two devices show their
    /// own identities rather than a shared hardcoded mock.
    @Published var displayName = ""
    @Published var objects: [ObjectRow] = []

    /// The legacy slot: a bare message with no idea what was being attempted. Still
    /// written by call sites that have not been given their operation yet, and promoted
    /// into `failure` on the way through so nothing stops surfacing mid-migration.
    @Published var error: String? {
        didSet {
            guard !naming, let error, !error.isEmpty else { return }
            failure = Failure(operation: nil, detail: error)
            Trace.shared.log("core", error, ok: false)
        }
    }

    /// True while `fail(_:_:)` is writing the legacy slot, so the promotion above does
    /// not overwrite the named failure with an anonymous one, or log it twice.
    private var naming = false

    /// What actually failed, and at what. This is what the UI presents.
    @Published var failure: Failure?

    /// A BACKGROUND READ that failed. Traced, never presented: nobody asked for this
    /// read, so nothing about it may interrupt them. `refresh` alone runs every 3
    /// seconds — routing it through the shared slot is what made the old alert
    /// un-dismissable, because it re-armed faster than it could be tapped away.
    ///
    /// A read that a person IS waiting on is not this — it is `fail`, with a name.
    func readFailed(_ what: String, _ error: Error, object: String = "") {
        Trace.shared.log("read", object: object, "\(what) — \(error)", ok: false)
    }

    /// Record a failure against the operation that failed. `operation` completes
    /// "Couldn't …" and is the sentence a person reads, so write it as one:
    /// `fail("add them to the Space", error)`, not `fail("addContact", error)`.
    /// Every call also lands in Trace, which reaches both the simulator's log stream
    /// and a device console.
    func fail(_ operation: String, _ error: Error, object: String = "") {
        let f = Failure(operation: operation, error: error, object: object)
        naming = true
        self.error = f.detail          // keep the legacy readers alive
        naming = false
        failure = f
        Trace.shared.log("core", object: object, f.logLine, ok: false)
    }

    // MARK: - My profile (name · avatar · the whole bio card)
    //
    // THE CORE OWNS THIS NOW. It used to live half in the core (`displayName`, set
    // once at first mint and never editable) and half in iOS UserDefaults (the
    // avatar) — which is exactly why it could never be shared: half of it had no way
    // to cross the FFI. Every field below is read from and written back through
    // `Core.myProfile` / `Core.setMyProfile`, and every write FANS OUT to all
    // connections as a signed Delta (see `commitProfile`).
    //
    // `@Published` mirrors of the core's state, so SwiftUI redraws on edit.

    /// Indigo (the Life "people" tint) — the default block colour before any pick.
    static let defaultAvatarColorHex = "7A6FF0"

    /// THE avatar block-colour swatches. One list, two consumers: the first-boot
    /// FACE step and the Avatar editor in Settings/Profile — so the colour a user
    /// picks at boot is one of the ones they can pick again later.
    static let avatarSwatches: [UInt] = [
        0x7A6FF0, 0x49A6F2, 0x33C08C, 0xE7A83C, 0xF0669E, 0xE5788A, 0x98A2AE
    ]

    /// The swatch this identity gets before anyone chooses one — derived from the
    /// identity's own public key, so a user who taps straight through first boot
    /// still has a face that is THEIRS rather than the shared default purple.
    ///
    /// Deterministic and cosmetic: it is a view of a public key, so it leaks
    /// nothing, and it is a WEAK identity signal at 7 swatches — never a thing to
    /// recognise someone by (the three fingerprint words are that).
    static func derivedAvatarColorHex(forIdentityKey key: String) -> String {
        // The key's own bytes, folded. `identityKey` is "ed25519:<hex>"; anything
        // unexpected falls back rather than crashing on a first launch.
        let hexPart = key.split(separator: ":").last.map(String.init) ?? key
        guard !hexPart.isEmpty else { return defaultAvatarColorHex }
        let fold = hexPart.unicodeScalars.reduce(UInt32(0)) { ($0 &* 31) &+ $1.value }
        let swatch = avatarSwatches[Int(fold % UInt32(avatarSwatches.count))]
        return String(format: "%06X", swatch)
    }

    /// The whole card as the core holds it. Editors mutate this and call
    /// `commitProfile()`; nothing writes it behind the UI's back.
    @Published private(set) var card: ProfileCard = CoreService.emptyCard

    /// The self avatar PHOTO (a downscaled JPEG picked in Me), or nil for the
    /// block-colour disc.
    var avatarPhoto: Data? {
        get { Data(base64Encoded: card.photo) }
        set {
            card.photo = newValue?.base64EncodedString() ?? ""
            card.photoMime = newValue == nil ? "" : "image/jpeg"
            card.capturedAt = newValue == nil ? 0 : UInt64(Date().timeIntervalSince1970)
            commitProfile()
        }
    }

    /// The self avatar BLOCK COLOUR as "RRGGBB" — the disc fill when there is no
    /// photo (and the tint Me's swatches match).
    var avatarColorHex: String {
        get { card.avatarColor.isEmpty ? Self.defaultAvatarColorHex : card.avatarColor }
        set { card.avatarColor = newValue; commitProfile() }
    }

    /// The block colour as a SwiftUI Color, for the avatar disc + Me's swatches.
    var avatarColor: Color { Color(hex: avatarColorHex) }

    /// An all-empty card — the shape every default and reset starts from.
    static var emptyCard: ProfileCard {
        ProfileCard(org: "", title: "", note: "", emails: [], phones: [], urls: [], tags: [],
                    photo: "", photoMime: "", clip: "", clipMime: "", capturedAt: 0,
                    avatarColor: defaultAvatarColorHex, city: "")
    }

    // MARK: - Editing + publishing the profile

    /// Coalesces a burst of edits into ONE fan-out. Every keystroke in a bio field
    /// would otherwise author a Delta per connection; the profile is not a chat
    /// message and nobody needs to watch you type. Cancelled + rescheduled on each
    /// edit, and flushed immediately when the app backgrounds so a pending change is
    /// never left only in memory.
    private var profileCommit: Task<Void, Never>?
    private static let profileDebounce: Duration = .milliseconds(800)

    /// Rename yourself. The display name is a profile field, not the identity — the
    /// MLS credential keys on the identity pubkey — so a rename never disturbs a
    /// group or a pairing. It just has to reach everyone.
    func rename(to name: String) {
        displayName = name
        commitProfile()
    }

    /// A two-way binding onto one card field, for the Me editors. Reading is free;
    /// writing schedules the debounced publish, so a text field just binds and the
    /// fan-out takes care of itself.
    func cardBinding<T>(_ keyPath: WritableKeyPath<ProfileCard, T>) -> Binding<T> {
        Binding(
            get: { self.card[keyPath: keyPath] },
            set: { self.card[keyPath: keyPath] = $0; self.commitProfile() }
        )
    }

    /// Mutate the card directly (list add/remove, where a keypath binding won't do).
    func editCard(_ mutate: (inout ProfileCard) -> Void) {
        mutate(&card)
        commitProfile()
    }

    /// Persist the profile to the core and TRANSMIT it to every connection as a
    /// signed Delta — debounced. Idempotent by construction: the core compares a
    /// digest per connection, so a commit that changes nothing authors nothing.
    private func commitProfile() {
        profileCommit?.cancel()
        profileCommit = Task { [weak self] in
            try? await Task.sleep(for: Self.profileDebounce)
            guard !Task.isCancelled else { return }
            await self?.flushProfile()
        }
    }

    /// Write + publish NOW, skipping the debounce. Call on app background so an
    /// in-flight edit is committed rather than lost with the process.
    func flushProfile() async {
        profileCommit?.cancel()
        profileCommit = nil
        let c = core
        let (name, card) = (displayName, self.card)
        do {
            let reached = try await Task.detached {
                try c.setMyProfile(displayName: name, shape: "individual",
                                   card: card)
            }.value
            if reached > 0 { profileReach = Int(reached) }
        } catch { fail("publish your profile", error) }
    }

    /// How many connections the last publish actually reached — surfaced so the UI
    /// can say so rather than implying a silent success.
    @Published private(set) var profileReach = 0

    /// Load my profile out of the core, migrating the pre-core avatar on first run.
    ///
    /// The photo + block colour used to live in UserDefaults and never crossed the
    /// FFI. Anyone upgrading has one there and none in the core, so we lift it in
    /// once — otherwise upgrading would silently blank the avatar the user chose.
    private func loadProfile() {
        guard let row = try? core.myProfile() else { return }
        card = row.card
        if card.avatarColor.isEmpty { card.avatarColor = Self.defaultAvatarColorHex }

        let d = UserDefaults.standard
        let legacyPhoto = d.data(forKey: "pacific.avatarPhoto")
        let legacyColor = d.string(forKey: "pacific.avatarColorHex")
        guard legacyPhoto != nil || legacyColor != nil else { return }
        // The core wins where it already has a value — a migration must never
        // clobber something published since.
        if card.photo.isEmpty, let legacyPhoto {
            card.photo = legacyPhoto.base64EncodedString()
            card.photoMime = "image/jpeg"
        }
        if let legacyColor, !legacyColor.isEmpty, row.card.avatarColor.isEmpty {
            card.avatarColor = legacyColor
        }
        d.removeObject(forKey: "pacific.avatarPhoto")
        d.removeObject(forKey: "pacific.avatarColorHex")
        commitProfile()
    }

    // MARK: - Peer profiles (what our connections published to us)

    /// The profile `peerId` published to us, or nil if they have not published one.
    /// Nil is NOT an empty card: callers fall back to the pairing-bundle name rather
    /// than rendering a blank one.
    func peerProfile(_ peerId: String) -> ProfileRow? {
        (try? core.peerProfile(peerId: peerId)) ?? nil
    }

    /// A peer's avatar photo bytes, if they published one.
    func peerPhoto(_ peerId: String) -> Data? {
        guard let p = peerProfile(peerId), !p.card.photo.isEmpty else { return nil }
        return Data(base64Encoded: p.card.photo)
    }

    /// A peer's chosen block colour, or nil to keep the neutral fallback disc.
    func peerColor(_ peerId: String) -> Color? {
        guard let p = peerProfile(peerId), !p.card.avatarColor.isEmpty else { return nil }
        return Color(hex: p.card.avatarColor)
    }

    /// Two-letter initials from the real display name, for the avatar badge.
    var initials: String {
        let parts = displayName.split(separator: " ")
        guard let f = parts.first else { return "?" }
        if parts.count >= 2 { return (f.prefix(1) + parts[1].prefix(1)).uppercased() }
        return f.prefix(2).uppercased()
    }

    /// The relay every networked call uses. Env override wins (dev/demo), then
    /// a user default, then the canonical production URL from the core.
    /// A persisted relay is honoured UNLESS this build has retired it. Without that
    /// check the precedence is upside down on an upgrade: the device's stored value
    /// (written by an older build, or adopted from an Arc that has since moved) beats
    /// the URL we ship, so messaging keeps dialling a host that is gone and every
    /// send fails silently. `isRetiredRelayUrl` is the core's own list, so the app
    /// and the engine can never disagree about which relays are dead.
    nonisolated static var relayUrl: String {
        if let env = ProcessInfo.processInfo.environment["PACIFIC_RELAY_URL"], !env.isEmpty { return env }
        if let d = UserDefaults.standard.string(forKey: "pacific.relayUrl"), !d.isEmpty,
           !isRetiredRelayUrl(url: d) {
            return d
        }
        return defaultRelayUrl()
    }

    /// The RAIL origin — the money tier (`/v1/box/*`). Env override for dev, else the
    /// shipped default. Separate host from the relay on purpose: this one holds
    /// secrets, the relay holds nothing.
    nonisolated static var railUrl: String {
        if let env = ProcessInfo.processInfo.environment["PACIFIC_RAIL_URL"], !env.isEmpty { return env }
        return defaultRailUrl()
    }
    var railUrl: String { Self.railUrl }
    var relayUrl: String { Self.relayUrl }

    /// Whether the device-to-device mesh is in the transport set. Off by default: it turns on
    /// Bluetooth scanning and advertising, which is a battery and permission decision the user
    /// makes, not one we make for them.
    static var meshEnabled: Bool { UserDefaults.standard.bool(forKey: "pacific.meshEnabled") }

    // MARK: - The account channel, and why there is no longer one here

    /// IT WAS DERIVED FROM THE PASSKEY, AND THAT WAS THE BUG. ~45 lines lived
    /// here to record a rendezvous: a tag in UserDefaults, a seal in the
    /// Keychain, both from a second PRF evaluation of the same credential. The
    /// reasoning was that two devices holding one synced passkey derive the same
    /// 32 bytes and so find each other without meeting.
    ///
    /// That has a hole which only shows on real hardware: whether a platform
    /// carries a credential's PRF secret between a person's devices is up to the
    /// platform. Where it does not, the two devices derived DIFFERENT channels
    /// and silently never met — both of them apparently working.
    ///
    /// The channel comes from the SEED now, in the core, and every device that
    /// can open the account has it by construction. The webapp made the same
    /// move on the same day. Nothing read this store anyway — its own comment
    /// said so — so it goes rather than being carried as a second, wrong answer
    /// to a question the seed already answers.

    /// The transport set, as the core's route spec. This is the ONE thing that decides where a
    /// message goes — the core no longer takes a relay on each call, so anything that changes the
    /// relay or the mesh switch must push through here.
    static var routeSpec: String {
        var routes = [relayUrl]
        if meshEnabled { routes.append("mesh:") }
        return routes.joined(separator: ",")
    }

    /// Hand the core this device's transport set. Called at launch and whenever the relay or the
    /// mesh switch changes; cheap and idempotent, so callers need not track whether it is needed.
    func applyRoutes() {
        do { try core.setRoutes(spec: Self.routeSpec) }
        catch { self.error = String(describing: error) }
    }

    /// The relay tags this device currently SUBs, lowercase hex — what the WAKE tier is told
    /// to watch (RINGFENCE.txt §6, PushRegistrar).
    ///
    /// Straight from the core (`wake_tags`, added 4 Aug): the intro mailbox, every RETAINED
    /// epoch tag of every group — the current one plus the window `sync` still re-drains, so a
    /// message that raced a rotation still wakes us — and each open Place's doorbell. It is a
    /// pure directory read on that side (no MLS group loaded, no epoch advanced, no relay
    /// touched), which is what makes it safe to call on every sync tick.
    ///
    /// Tags ROTATE per epoch, so this is a snapshot with a short life; PushRegistrar
    /// re-registers whenever the set changes. A failure yields NO tags rather than stale ones:
    /// registering mailboxes we are not really listening to would promise wakes that never come.
    ///
    /// `PACIFIC_WAKE_TAGS` remains a DEV SEAM (explicit comma-separated hex) so a harness can
    /// drive the wake plane without a real device; it overrides, and is never a source of real tags.
    func wakeTags() -> [String] {
        if let raw = ProcessInfo.processInfo.environment["PACIFIC_WAKE_TAGS"], !raw.isEmpty {
            return raw.split(separator: ",")
                .map { $0.trimmingCharacters(in: .whitespaces).lowercased() }
                .filter { !$0.isEmpty }
        }
        do { return try core.wakeTags() }
        catch {
            readFailed("read the wake tags", error)
            return []
        }
    }

    init() {
        // iOS has no ~/.pacific; hand the core the app sandbox (Application Support).
        let base = FileManager.default
            .urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("pacific", isDirectory: true)
        try? FileManager.default.createDirectory(at: base, withIntermediateDirectories: true)
        // Provision + inject the device master key BEFORE the core touches identity/DB,
        // so id_ed25519 and the secrets pacific-core seals in pacific.db seal under it
        // at rest (S5). No key means a sealed store cannot be opened — which is the point.
        //
        // FAIL CLOSED on a keychain failure: inject nothing rather than minting a
        // replacement key. A fresh key would orphan the existing sealed store forever;
        // injecting nothing leaves it shut but INTACT, so a merely unavailable keychain
        // (a background launch before first unlock) recovers on the next launch. The
        // core still opens — it will simply report that it cannot read the sealed store,
        // which is the honest, recoverable symptom rather than a silent wipe.
        var keyFailure: String?
        do {
            try DeviceKey.activate()
        } catch {
            keyFailure = (error as? LocalizedError)?.errorDescription ?? String(describing: error)
        }
        core = Core(stateDir: base.path)
        setLogSink(sink: CoreLog())
        if let keyFailure { self.error = keyFailure }
        // PERSIST the relay: env wins at launch but doesn't survive a home-screen
        // relaunch, so a user-launched app would silently fall back to the (dead)
        // production relay. Sticking the env URL into the "pacific.relayUrl"
        // default makes later launches keep using it — no more "message vanished
        // because we were talking to the wrong relay".
        if let env = ProcessInfo.processInfo.environment["PACIFIC_RELAY_URL"], !env.isEmpty {
            UserDefaults.standard.set(env, forKey: "pacific.relayUrl")
        }
        // MIGRATE OFF A RETIRED RELAY, once, at launch. Reading past a dead stored
        // value (above) keeps this launch working; CLEARING it is what stops the
        // Arc-adoption path from writing it straight back and stops the user seeing
        // a host we no longer run in the relay field. A pinned relay is left alone —
        // if someone pinned it by hand, that is a choice, not staleness.
        if let stored = UserDefaults.standard.string(forKey: "pacific.relayUrl"),
           !stored.isEmpty, isRetiredRelayUrl(url: stored),
           !UserDefaults.standard.bool(forKey: "pacific.relayPinned") {
            UserDefaults.standard.removeObject(forKey: "pacific.relayUrl")
        }
        // Transport is CORE state now, not an argument on every call — so hand the core the set
        // before anything tries to send. Every later change (an Arc picked, a relay pinned, the
        // mesh switched on) pushes through `applyRoutes` the same way. On a FIRST session this
        // call fails (the node needs an identity) — finishBootstrap re-applies once one exists.
        do { try core.setRoutes(spec: Self.routeSpec) }
        catch { NSLog("[routes] pre-identity setRoutes deferred: %@", String(describing: error)) }
        bootstrap()
    }

    /// True while the FIRST BOOT SEQUENCE owns the screen (`BootFlow`) instead of
    /// the shell. It covers more than "no identity": the sequence continues past
    /// the mint (face, bio, recovery key, fingerprint), and an app killed halfway
    /// through must come back to where it was rather than dropping a half-set-up
    /// user into the shell. So the gate is "the sequence has not been walked to
    /// its end" — recorded under `firstBootCompleteKey` and closed by
    /// `finishFirstBoot()`. A harness/dev run passes `PACIFIC_IDENTITY_NAME` and
    /// never sees it.
    @Published private(set) var needsOnboarding = false

    /// Set once the first-boot sequence has been walked to its end. Absent on an
    /// identity minted before the sequence existed — those are marked complete at
    /// launch rather than sent back through an onboarding they already passed.
    private static let firstBootCompleteKey = "pacific.firstBootComplete"

    /// THE STAMP THAT TELLS THOSE TWO APART, and it exists because of the worst
    /// bug this app has had.
    ///
    /// The gate used to read: identity present, complete-key absent ⇒ this is an
    /// identity that predates the sequence, mark it complete. That is true of an
    /// old install. It is also true of an identity minted ten seconds ago by a
    /// sequence that then stopped — and for that person the migration closed
    /// first boot on a run that had never reached the RECOVERY KEY step. Their
    /// account's only offline backup had never been on screen, and now never
    /// could be: the words are generated, shown once, and stored by us nowhere.
    ///
    /// So the sequence stamps itself the moment it opens. Complete-key absent AND
    /// this stamp present means an interrupted run, which RESUMES. Complete-key
    /// absent and no stamp means a genuinely pre-sequence identity, which is
    /// migrated as before.
    private static let firstBootStartedKey = "pacific.firstBootStarted"

    /// Mint the on-device identity on first launch, then load it. A first launch
    /// with no supplied name defers the mint to onboarding — nothing below the
    /// mint exists until the identity does.
    func bootstrap() {
        let d = UserDefaults.standard
        // TEST SEAM (`-pacificResetFirstBoot`): re-open the sequence so a UI suite
        // can walk it from the door. It resets the GATE only — the key file is
        // never touched, because a test must not be able to destroy an identity.
        // A device that still holds one resumes at the collecting steps, exactly
        // as an interrupted run does.
        let reopened = ProcessInfo.processInfo.arguments.contains("-pacificResetFirstBoot")
        if reopened {
            d.removeObject(forKey: Self.firstBootCompleteKey)
            // The seam re-opens the sequence, so from here on this IS a run of it —
            // stamp it, or an interrupted reopened run would be migrated shut by
            // the branch below on the very next launch.
            d.set(true, forKey: Self.firstBootStartedKey)
        }
        var minted = false
        if !core.hasIdentity() {
            if let name = ProcessInfo.processInfo.environment["PACIFIC_IDENTITY_NAME"], !name.isEmpty {
                do { _ = try core.initIdentity(displayName: name) }
                catch { self.error = String(describing: error); return }
                minted = true
                // A CI/dev run that names its identity has said it is not testing
                // onboarding; the sequence is marked walked so it never appears.
                d.set(true, forKey: Self.firstBootCompleteKey)
                d.removeObject(forKey: Self.firstBootStartedKey)
            } else {
                // THE SEQUENCE IS ABOUT TO OPEN. Stamp it before it does, so that a
                // run killed at any point after this — including between the mint
                // and the recovery-key step — is recognisable as an interrupted
                // walk rather than mistaken for a pre-sequence identity.
                d.set(true, forKey: Self.firstBootStartedKey)
                needsOnboarding = true
                return
            }
        } else if !reopened, d.object(forKey: Self.firstBootCompleteKey) == nil {
            if d.bool(forKey: Self.firstBootStartedKey) {
                // AN INTERRUPTED WALK, not an old install. Leave the gate open: the
                // sequence resumes below at its collecting steps, and the recovery
                // key this person has never seen is still ahead of them.
                NSLog("[boot] first boot was started and never finished — resuming the sequence")
            } else {
                // An identity that predates the sequence. It never walked it and does
                // not need to — sending an established user back through onboarding
                // would be the worse of the two wrong answers. Skipped when the seam
                // above deliberately re-opened the sequence, or the migration would
                // close it again in the same breath.
                d.set(true, forKey: Self.firstBootCompleteKey)
            }
        }
        finishBootstrap()
        // AFTER the fold is up: an edit made before `finishBootstrap` is clobbered
        // when the stored card loads (11 Aug — the fleet's colours silently never
        // landed). A CI mint gets the same key-derived face a real one does.
        if minted { seedDerivedFace() }
        // A sequence interrupted AFTER the identity was created (the app was
        // killed between the mint and the last step) resumes rather than
        // stranding a half-set-up user in the shell.
        if !d.bool(forKey: Self.firstBootCompleteKey) { needsOnboarding = true }
    }

    // THE HARNESS PROFILE SEAM IS GONE (15 Aug). `PACIFIC_IDENTITY_COLOR` and
    // `PACIFIC_IDENTITY_NOTE` were added on 11 Aug for one reason: the fleet had
    // to give itself a complete profile and first boot collected only a name, so
    // there was no UI path to drive. The boot sequence is that path — sim-fleet
    // now walks the real screens through `FleetIdentityUITests` — and a seam that
    // writes the card behind the app's own edit path is exactly what lets a bug in
    // that path go unnoticed. `PACIFIC_IDENTITY_NAME` stays, demoted to what it
    // should always have been: a CI skip for suites that are not about onboarding.

    /// THE MINT — first boot's NEW door. The Ed25519 identity genuinely does not
    /// exist until this call, which is what makes that screen honest rather than
    /// theatre. Brings the rest of the core up exactly as a normal launch would,
    /// then seeds the block colour from the key that was just minted so a user
    /// who taps straight through still leaves with a face of their own.
    ///
    /// The sequence stays on screen afterwards (`needsOnboarding` is closed by
    /// `finishFirstBoot`) — the face, bio, recovery-key and fingerprint steps
    /// follow, and they need the core up to write anything.
    @discardableResult
    func completeOnboarding(name: String) -> Bool {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return false }
        do {
            _ = try core.initIdentity(displayName: trimmed)
            finishBootstrap()
            seedDerivedFace()
            return true
        } catch {
            self.error = String(describing: error)
            return false
        }
    }

    /// The block colour derived from the identity's own key, written through the
    /// real card edit path. Only ever seeds — a colour already chosen is left
    /// alone, so this is safe on a resumed sequence.
    private func seedDerivedFace() {
        let derived = Self.derivedAvatarColorHex(forIdentityKey: spaceId)
        guard card.avatarColor.isEmpty || card.avatarColor == Self.defaultAvatarColorHex else { return }
        editCard { $0.avatarColor = derived }
    }

    /// This identity's public key as the 32 RAW BYTES, which is what a passkey's
    /// user handle carries and what the Arc keys an account on. `spaceId` is the
    /// same key wearing its `ed25519:` prefix — that prefix is part of what gets
    /// signed, so it is stripped here and nowhere else.
    var identityKeyBytes: Data {
        let hex = spaceId.contains(":") ? String(spaceId.split(separator: ":").last!) : spaceId
        var out = Data(capacity: hex.count / 2)
        var i = hex.startIndex
        while i < hex.endIndex, let j = hex.index(i, offsetBy: 2, limitedBy: hex.endIndex) {
            if let b = UInt8(hex[i..<j], radix: 16) { out.append(b) }
            i = j
        }
        return out
    }

    /// THE ACCOUNT SERVICE, built here because it needs the core and the core is
    /// private. A view that could reach the `Core` directly could mint an
    /// identity outside this class's knowledge, and `finishBootstrap` would
    /// never run for it — which is the shape of bug where the app holds a key
    /// and shows an empty profile.
    func makeSignIn() -> SignInService { SignInService(core: core) }

    /// Test opt-OUT for the passkey ceremony — see `enrolPasskey`. Named, rather
    /// than an inline string, so the suites and the code cannot disagree about it.
    static let noPasskeyArg = "-pacificNoPasskey"

    /// WHAT THIS DEVICE'S PASSKEY IS DOING. Not a gate and never one: the seed is
    /// the account, a passkey only WRAPS it, and an identity with no passkey is a
    /// working identity whose owner restores it with their twenty-four words.
    enum PasskeyEnrolment: Equatable {
        case none, enrolling, enrolled
        /// It did not happen, and this is what the ceremony or the Arc said.
        case failed(String)

        var isFailed: Bool { if case .failed = self { return true }; return false }
        var fault: String? { if case .failed(let m) = self { return m }; return nil }
    }

    @Published private(set) var passkeyEnrolment: PasskeyEnrolment = .none

    /// Owned by the SERVICE, not by a view, and that is the whole point. This used
    /// to be a `Task` started inside the NAME step with `done()` after the
    /// `await` — so a ceremony that never called back (a simulator with no
    /// associated domain presents no sheet and reports no failure) stopped the
    /// first-boot sequence dead on the name field, with an identity already minted
    /// and its owner's twenty-four words never shown. The identity is what the tap
    /// makes; enrolment is an errand run beside the person, not in front of them.
    private var passkeyTask: Task<Void, Never>?
    private var signInForEnrolment: SignInService?

    /// Make this device a passkey and store the wrap it opens. Returns
    /// immediately. Every outcome lands in `passkeyEnrolment`, where the recovery
    /// key step reads it — because "this device has no passkey" is precisely the
    /// fact that makes those twenty-four words the only way back.
    func enrolPasskey(label: String) {
        guard passkeyTask == nil else { return }
        guard !spaceId.isEmpty else {
            NSLog("[account] enrol: no identity to enrol — skipped")
            return
        }
        // THE ONE THING A UI SUITE CANNOT TAP (the `-pacificNoWake` idiom).
        //
        // `registerPasskey` runs an `ASAuthorizationController` ceremony, and that
        // sheet is a REMOTE VIEW owned by another process. It is full-screen, it
        // sits above this app's window, and XCUITest cannot reach into it — so
        // every synthesised tap after BEGIN lands on the sheet instead of on the
        // step behind it. Measured, 15 Sep 2026: the first-boot suite got as far
        // as FACE (the state machine DID advance), then tapped a swatch and NEXT
        // into a sheet, and sat on FACE until the assertion timed out.
        //
        // AND IT IS A FINDING, NOT ONLY A TEST PROBLEM. The claim this flow makes
        // — "BEGIN mints and advances; the next screen is not gated on a passkey"
        // — is true of `step` and FALSE OF THE SCREEN. A real person on a real
        // device is shown FACE and then has a system sheet put over it before they
        // can touch anything. Whether the ceremony belongs at BEGIN at all is a
        // product decision and is NOT settled by this flag; the flag only stops a
        // suite asserting on a surface it is not being allowed to drive.
        if ProcessInfo.processInfo.arguments.contains(Self.noPasskeyArg) {
            NSLog("[account] enrol: %@ — the ceremony is not run, the identity stands",
                  Self.noPasskeyArg)
            passkeyEnrolment = .failed("Passkey enrolment was skipped by a test seam.")
            return
        }
        let pk = identityKeyBytes
        let service = makeSignIn()
        signInForEnrolment = service
        passkeyEnrolment = .enrolling
        passkeyTask = Task { @MainActor [weak self] in
            defer { self?.passkeyTask = nil }
            do {
                try await service.registerPasskey(pk: pk, label: label)
                self?.passkeyEnrolment = .enrolled
                NSLog("[account] enrol: this device has a passkey")
            } catch SignInError.cancelled {
                NSLog("[account] enrol: cancelled — the identity stands, with no passkey")
                self?.passkeyEnrolment = .failed("You skipped making a passkey.")
            } catch {
                NSLog("[account] enrol: FAILED — %@", error.localizedDescription)
                self?.passkeyEnrolment = .failed(error.localizedDescription)
            }
        }
    }

    /// The core already holds an identity that arrived from OUTSIDE this class —
    /// a sign-in (passkey or words) brought it back. Everything that
    /// normally follows a mint still has to happen: routes re-applied, the key
    /// and SAS read back, the profile loaded, the credential minter registered.
    ///
    /// It is deliberately NOT a second mint path. The identity exists; this only
    /// catches the app up to it.
    func adoptRestoredIdentity(_ identityKey: String) {
        guard !identityKey.isEmpty else { return }
        finishBootstrap()
        seedDerivedFace()
    }

    /// A SIGN-IN LANDS, by either door: this device IS the person, so no mint, no
    /// name step, nothing to collect; straight to the shell. An object the spine
    /// names and this device could not join is said, never dropped: the shell's
    /// failure surface, since the boot screen closes.
    func finishSignIn(_ result: SignInResult) {
        adoptRestoredIdentity(result.identityKey)
        finishFirstBoot()
        let missed = result.objects.filter { $0.outcome != "Joined" && $0.outcome != "AlreadyHeld" }
        if !missed.isEmpty {
            let why = missed.map { "\($0.kind) \($0.object.prefix(8)): \($0.outcome) \($0.detail)" }
            fail("rejoin \(missed.count) of \(result.objects.count) objects",
                 SignInError.service(why.joined(separator: "; ")))
        }
    }

    /// The last step's ENTER: the sequence has been walked, so the shell takes
    /// over and never asks again.
    func finishFirstBoot() {
        UserDefaults.standard.set(true, forKey: Self.firstBootCompleteKey)
        UserDefaults.standard.removeObject(forKey: Self.firstBootStartedKey)
        needsOnboarding = false
    }

    /// This identity's RECOVERY KEY — the 24 words that bring it back on a device
    /// that has lost everything. Empty when there is none to show (an identity
    /// minted before seeds existed): the step is omitted rather than faked.
    /// Read live from the core each time; nothing caches it.
    func recoveryKey() -> String {
        (try? core.recoveryKey()) ?? ""
    }

    /// Everything after the identity exists: load it, the profile, the credential
    /// minter, and the first fold.
    private func finishBootstrap() {
        // ROUTES, RE-APPLIED NOW THE IDENTITY EXISTS. The init-time setRoutes fails
        // on a first session (the node opens with the identity) and used to be
        // swallowed whole — so a fresh device spent its ENTIRE first session syncing
        // against the DEFAULT relay while LiveSync SUBbed the configured one, and
        // the push path never fired (Latency harness: 2 SUB conns, 0 publishes).
        applyRoutes()
        do {
            spaceId = try core.identityKey()
            sas = try core.sas()
            sasWords = (try? core.sasWords()) ?? ""
            displayName = (try? core.displayName()) ?? ""
            loadProfile()
            registerArcCredentialMinter()
            refresh()
            #if DEBUG
            exportDevContact()   // DEBUG-only: writes dev-contact.txt for host pairing scripts
            #endif
        } catch { self.error = String(describing: error) }
    }

    /// Give the rest of the app the ability to prove membership, without handing it the core.
    ///
    /// The whole credential — payload, signature, encoding — is assembled in Rust
    /// (`arc_member_credential`), so the format stays pinned to the verifier that reads it and
    /// no general signing primitive ever crosses the FFI. What a caller receives through
    /// `ArcCredential.provider` is the narrow capability "mint a membership credential for THIS
    /// Arc", nothing more.
    ///
    /// Registered only after the identity loads, because minting requires a key to sign with.
    private func registerArcCredentialMinter() {
        let c = core
        ArcCredential.provider = { arcSpaceID in
            membershipLog.debug("[mint] arc_member_credential(arcSpaceId: \(String(arcSpaceID.prefix(20)), privacy: .public))…")
            // Off the main actor: signing is cheap but this is called on the request path.
            do {
                let cred = try await Task.detached { try c.arcMemberCredential(arcIdentityKey: arcSpaceID) }.value
                membershipLog.debug("[mint] FFI returned \(cred.count, privacy: .public) chars")
                return cred
            } catch {
                membershipLog.error("[mint] FFI arc_member_credential THREW: \(error.localizedDescription, privacy: .public)")
                throw error
            }
        }
        membershipLog.debug("[mint] minter registered (ArcCredential.provider set)")
    }

    /// Seed the demo conversation with `peer` through REAL ops: this device
    /// authors only its own scripted lines (resolved from its identity name) and
    /// drains the peer's over the relay. Returns whether it ran (this identity is
    /// a scripted actor). Reaches the relay — runs off the main thread.
    @discardableResult
    func seedDmDemo(_ peerId: String) async -> Bool {
        let c = core
        let relay = relayUrl
        do { return try await Task.detached { try c.seedDmDemo(peerId: peerId) }.value }
        catch { self.error = String(describing: error); return false }
    }

    /// DEV SEAM: drop the (public, signed) contact bundle + space id where host
    /// tooling can read them out of the simulator container — this is what lets
    /// a script pair two simulators without driving the UI. The bundle is the
    /// same public payload the QR shows; no secret material leaves the core.
    private func exportDevContact() {
        let dir = FileManager.default
            .urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("pacific", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let payload = "\(spaceId)\n\(contactBundle())\n"
        try? payload.write(to: dir.appendingPathComponent("dev-contact.txt"), atomically: true, encoding: .utf8)
    }

    /// DEV SEAM: re-export a FRESH contact bundle on demand. `contactBundle()` mints
    /// a NEW key package each call, and MLS key packages are single-use — so a mesh
    /// harness that adds this device to several groups must fetch a fresh bundle
    /// before each add (otherwise the 2nd add reuses a consumed key package and the
    /// join can't complete). The real app never hits this: every QR pairing shows a
    /// freshly-built bundle. Callable via the `export-contact` dev link.
    func reexportContact() { exportDevContact() }

    func refresh() {
        // Publish only on CHANGE. @Published fires on every assignment, and this runs
        // on every 3-second sync tick — an ungated write here re-fires every $objects
        // subscriber (the graph projection among them) on a steady-state tick.
        do {
            let fresh = try core.objects()
            if fresh != objects { objects = fresh }
        } catch { readFailed("re-read your objects", error) }
        // Places are folded per-object (each has its own log), so they are not derivable
        // from `objects` — they need their own read, exactly like Things.
        refreshPlaces()
        // Posts are folded per-object too — same reason as Places and Things. The FEED
        // reads them, so a Post minted anywhere reaches the scroll on the next pass.
        refreshPosts()
        // The forum→host map rides every re-list: the CHAT list joins on it, and a
        // room minted or detached anywhere must re-dress its row on the next pass.
        refreshForumHosts()
        // Exteriors arrive by sync from connections, so they are re-read on every pass.
        refreshExteriors()
    }

    /// THE EXTERIORS THAT REACHED ME (#76): objects I am not in, carried by their members.
    /// Equality-gated like every tick read.
    @Published private(set) var exteriors: [ExteriorRow] = []

    func refreshExteriors() {
        do {
            let fresh = try core.exteriors()
            if fresh != exteriors { exteriors = fresh }
        } catch { readFailed("re-read exteriors", error) }
    }

    /// forum object id → its hosting Group, the whole map in one FFI call. What
    /// makes a hosted room render as "Group / Room" rather than posing as a
    /// free-standing chat. Empty for a device with no group-hosted rooms.
    @Published private(set) var forumHosts: [String: ForumHostRow] = [:]

    /// room object id → its hosting Channel (parent forum) — forumHosts one
    /// level down. The CHAT list joins on it so a channel's room renders as
    /// "Channel / Room"; ThreadView reads the forward set per channel for tabs.
    @Published private(set) var roomHosts: [String: RoomHostRow] = [:]

    /// forum object id → the object it is a part of, and in what role, from the
    /// forum's OWN fold. A forum in the comments role is its parent's comments
    /// section: reached from its object, never listed as a conversation.
    @Published private(set) var forumParents: [String: ForumParentRow] = [:]

    /// The comments role, as the core's ICD-pinned table names it.
    private static let commentsPart = commentsRole()

    func isCommentsSection(_ forumId: String) -> Bool {
        forumParents[forumId]?.role == Self.commentsPart
    }

    /// The object's comments section, by the forum's own back-edge: nil until the
    /// device holds it.
    func commentsForum(of objectId: String) -> String? {
        forumParents.values.first { $0.parent == objectId && $0.role == Self.commentsPart }?.forum
    }

    func refreshForumHosts() {
        do {
            let fresh = Dictionary(try core.forumHosts().map { ($0.forum, $0) },
                                   uniquingKeysWith: { a, _ in a })
            if fresh != forumHosts { forumHosts = fresh }
            let rooms = Dictionary(try core.roomHosts().map { ($0.room, $0) },
                                   uniquingKeysWith: { a, _ in a })
            if rooms != roomHosts { roomHosts = rooms }
            let parents = Dictionary(try core.forumParents().map { ($0.forum, $0) },
                                     uniquingKeysWith: { a, _ in a })
            if parents != forumParents { forumParents = parents }
        } catch { readFailed("re-read channel hosts", error) }
    }

    /// A fresh signed contact bundle for the pairing QR. Fails loudly (surfaces
    /// via `error`) rather than silently returning a blank bundle.
    func contactBundle() -> String {
        do { return try core.contactBundle() }
        catch { fail("build your connect code", error); return "" }
    }

    // MARK: - Pairing (the real MLS handshake)

    /// Scan/paste a peer's contact bundle: forms the 2-party connection and seals
    /// the Welcome to them over the relay. Returns the peer's space id, or nil on
    /// failure (surfaced via `error`). Runs off-main — it reaches the relay.
    @discardableResult
    func pairScan(_ bundle: String) async -> String? {
        let c = core
        let relay = relayUrl
        do {
            let peer = try await Task.detached { try c.pairScan(bundle: bundle) }.value
            NSLog("[pair] scanned ok → %@", peer)
            refresh()
            return peer
        } catch { NSLog("[pair] error: %@", String(describing: error)); fail("connect with that code", error); return nil }
    }

    /// Accept a pending connection (double-opt-in): marks the peer Connected.
    ///
    /// ALSO the first moment a wake is genuinely wanted: from here on someone can
    /// message you, so this is where iOS is asked for notifications — not at
    /// launch, in front of a user with nobody to hear from. Idempotent; a user who
    /// has already answered is not asked again.
    func pairAccept(_ peer: String) {
        do {
            try core.pairAccept(peer: peer)
            WakePermission.requestNow()
            refresh()
        }
        catch { fail("accept the connection", error) }
    }

    /// The connection row for one peer (nil if unknown).
    func connection(_ peerId: String) -> ConnectionRow? {
        connectionRows().first { $0.peerId == peerId }
    }

    /// Mint a new local (group-of-1) Forum object.
    func newForum() {
        do { _ = try core.objectNew(kind: "forum", name: ""); refresh() }
        catch { self.error = String(describing: error) }
    }

    /// Author one real `forum.post` delta. relayUrl is unused for a group-of-1.
    func post(_ objectId: String, _ text: String) {
        guard !text.isEmpty else { return }
        do { _ = try core.objectPost(objectId: objectId, text: text, media: nil) }
        catch { fail("post to the channel", error) }
    }

    /// The folded, deterministic transcript of a Forum object.
    func transcript(_ objectId: String) -> [String] {
        (try? core.objectTranscript(objectId: objectId)) ?? []
    }

    // MARK: - Chats seam (groups = forum objects, DMs = connections)

    /// My own author token as it appears in folded transcripts: the first 8 hex
    /// chars of my identity pubkey (short(pk) = hex(pk[..4])). Deterministic — no
    /// heuristics — so outgoing bubbles align correctly.
    var myAuthorToken: String { String(myIdentityHex.prefix(8)) }

    /// My identity pubkey as full 64-char hex (the `space1` prefix stripped) — the
    /// same value the core stamps as `ChatMsg.author`, so the UI can tell whether a
    /// reply's parent was authored by me ("You") without another FFI hop.
    var myIdentityHex: String {
        IdentityKey.hex(spaceId)
    }

    /// Group conversations are forum objects (objects() already excludes the
    /// 1:1 'connection' groups).
    var groupObjects: [ObjectRow] { objects.filter { $0.kind == "forum" } }

    /// The user's 1:1 connections (peers), for the DM list. Blocked peers are absent by
    /// default — this is THE enforcement seam for the blocklist, so every people surface
    /// inherits it. Ledger-reading paths (wallet, invite folds, name maps) pass
    /// `includeBlocked: true`: a block hides a person, it does not rewrite history.
    func connectionRows(includeBlocked: Bool = false) -> [ConnectionRow] {
        do {
            let rows = try core.connections()
            return includeBlocked ? rows : rows.filter { !BlockStore.shared.contains($0.peerId) }
        }
        catch { readFailed("read your connections", error); return [] }
    }

    func groupTranscript(_ objectId: String) -> [String] { transcript(objectId) }

    func dmTranscript(_ peerId: String) -> [String] {
        do { return try core.dmTranscript(peerId: peerId) }
        catch { readFailed("read the conversation", error); return [] }
    }

    /// Mint a new local (group-of-1) forum object to use as a group conversation.
    /// `name` (may be empty) rides the MLS GroupContext extension, so every member —
    /// including late joiners, via their Welcome — reads the same current title.
    @discardableResult
    func createGroupConversation(name: String = "") -> String? {
        do { let id = try core.objectNew(kind: "forum", name: name); refresh(); return id }
        catch { self.error = String(describing: error); return nil }
    }

    /// Author one message into a group (forum) conversation. Reaches the relay for
    /// a multi-member group, so the blocking FFI call runs OFF the main thread.
    /// Returns whether it succeeded; failures surface via `error` (never silent).
    @discardableResult
    func sendToGroup(_ objectId: String, _ text: String) async -> Bool {
        let c = core
        do {
            let t0 = TimingProbe.nowMs
            _ = try await Task.detached { try c.objectPost(objectId: objectId, text: text, media: nil) }.value
            TimingProbe.shared.send(convo: objectId, kind: "group", chars: text.count,
                                    ackMs: TimingProbe.nowMs - t0)
            return true
        } catch { fail("send your message", error); return false }
    }

    /// The THREADED (Reddit-style) projection of a Forum/ChatRoom object — each
    /// post with its nesting `depth` and `replyCount`, pre-order flattened. The
    /// tree twin of `messages`; the ChatRoom surface binds to this. Sync read
    /// (a pure fold), reloaded on `syncTick` like every other open surface.
    func objectThread(_ objectId: String) -> [ForumThreadRow] {
        do { return try core.objectThread(objectId: objectId) }
        catch { readFailed("read the thread", error); return [] }
    }

    // MARK: - Project seam (the live Work-tab GroupObject)

    /// Mint a new local (group-of-1) Project object. Local until members join.
    @discardableResult
    func newProject() -> String? {
        do { let id = try core.objectNew(kind: "project", name: ""); refresh(); return id }
        catch { self.error = String(describing: error); return nil }
    }

    /// Our real Project objects (`objects()` already excludes 1:1 connection groups).
    var projectObjects: [ObjectRow] { objects.filter { $0.kind == "project" } }

    // MARK: - Places (somewhere you have been)

    /// Every Place we own, folded from its delta log. Like Things, these are the DURABLE
    /// truth — a Place is a GroupObject (`ObjectKind::Place`, type_id 28) carrying the
    /// common location facet, so it syncs, can be shared, and has a signed log. It is NOT
    /// an OutlineModel entity row: those are device-local and cannot be sent to anyone.
    @Published private(set) var places: [PlaceRow] = []

    func refreshPlaces() {
        do {
            let fresh = try core.places()
            if fresh != places { places = fresh }
        } catch { readFailed("re-read your places", error) }
    }

    /// Mint a Place from a live location fix. A group of 1: local, no relay, until shared.
    ///
    /// `placed: false` mints the Place UNPLACED rather than refusing — GPS is slow indoors
    /// and losing the user's typing to a missing fix is worse than a Place that says it has
    /// no coordinate yet. `placeSetLocation` fills it in later.
    @discardableResult
    func mintPlace(name: String, descriptor: String = "",
                   placed: Bool, lat: Double, lng: Double) -> String? {
        do {
            let id = try core.placeMint(name: name, descriptor: descriptor,
                                        placed: placed, lat: lat, lng: lng)
            refreshPlaces()
            return id
        } catch {
            self.error = String(describing: error)
            return nil
        }
    }

    // -- Discovery (ask the graph) --------------------------------------------

    /// The hops-aware place listing: what I hold (`holder == ""`, hops 0) merged
    /// with every live answer my connections (and theirs, at 2 hops) gave a
    /// discovery request — reference rows, not objects. ONE list drives both of
    /// the publish sheet's shelves; answers accrue over sync ticks after
    /// `discoverPlaces`.
    @Published private(set) var discovered: [DiscoveredRow] = []

    /// The app-load pass: warm the n-2 gossip caches for every discoverable kind
    /// (Trade warms itself — listings are push gossip riding every sync).
    /// Idempotent: a still-live question is reused, never re-flooded, so calling
    /// this on every launch costs nothing once the graph has been asked.
    /// Fire-and-forget; answers stream into `discovered` as syncs complete.
    func warmCaches() {
        let c = core
        Task.detached { [weak self] in
            do { _ = try c.warmCaches() }
            catch { await MainActor.run { self?.error = String(describing: error) } }
        }
    }

    /// Ask the graph for nearby Places. Fire on opening the publish sheet; answers
    /// stream into `discovered` as syncs complete. Re-asking a live question reuses
    /// its request id, so an impatient sheet never re-floods. Detached: this
    /// authors one delta per connection, each a relay round trip.
    func discoverPlaces(tags: [String] = [], budget: UInt32 = 2) async {
        let c = core
        do { _ = try await Task.detached { try c.discoverStart(kind: "place", tags: tags, budget: budget) }.value }
        catch { readFailed("look for nearby places", error) }
    }

    /// Re-read the merged listing (pure fold read; asks nobody anything).
    func refreshDiscovered(hops: UInt32 = 2) {
        do {
            let fresh = try core.discoverResults(kind: "place", hops: hops)
            if fresh != discovered { discovered = fresh }
        }
        catch { readFailed("re-read discovered places", error) }
    }

    /// Adopt a discovered place as a held Place I can publish into. Minted-wins:
    /// the same discovery adopts once, and the remote row stops being listed.
    func adoptPlace(_ row: DiscoveredRow) -> String? {
        do {
            let id = try core.placeAdopt(name: row.name, descriptor: row.descriptor,
                                         placed: row.placed, lat: row.lat, lng: row.lng,
                                         remoteId: row.id)
            refreshPlaces()
            refreshDiscovered()
            return id
        } catch {
            self.error = String(describing: error)
            return nil
        }
    }

    /// Publish a note AT a Place. `noteId` is the note's own UUID — publishing the
    /// same note again replaces the copy rather than stacking a duplicate.
    /// Detached: for a shared Place the append flushes over the relay.
    func publishNote(to placeId: String, noteId: String, title: String, text: String) async {
        let c = core
        let at = Int64(Date().timeIntervalSince1970 * 1000)
        do {
            _ = try await Task.detached {
                try c.placePost(objectId: placeId, note: noteId, title: title, text: text, at: at)
            }.value
        } catch { fail("publish your note here", error) }
    }

    /// The notes published AT a Place, folded — the Place profile's Posts lane
    /// (`place.post` is the only wall in the schema today; profile-integration-map
    /// §3). Synchronous fold read, like `transcript`.
    func placePosts(_ objectId: String) -> [PlacePostRow] {
        do { return try core.placePosts(objectId: objectId) }
        catch { readFailed("read the place's posts", error); return [] }
    }

    // -- Public places (request to join) --------------------------------------

    /// Set how far the fact of a Place's EXISTENCE may travel — the visibility
    /// dial, not the door: "private" | "connections" | "network". Orthogonal to
    /// `setPlaceAccess` (joinability) and to membership.
    @discardableResult
    func setPlaceVisibility(_ objectId: String, _ visibility: String) -> Bool {
        do {
            _ = try core.placeSetVisibility(objectId: objectId, visibility: visibility)
            refreshPlaces()
            refreshDiscovered()
            return true
        } catch {
            self.error = String(describing: error)
            return false
        }
    }

    /// Open a Place to the public, or close it again.
    ///
    /// OPENING IS THE CONSENT — whoever scans the QR is admitted automatically by this
    /// device on its next sync, with no per-person approval. Opening publishes nothing:
    /// joiners become members of the group, they do not read it from outside. Closing
    /// stops future admissions and evicts nobody.
    @discardableResult
    func setPlaceAccess(_ objectId: String, public isPublic: Bool) -> Bool {
        do {
            _ = try core.placeSetAccess(objectId: objectId, access: isPublic ? "public" : "private")
            refreshPlaces()
            return true
        } catch {
            self.error = String(describing: error)
            return false
        }
    }

    /// The string behind a public Place's QR — what goes on the poster.
    ///
    /// Carries the place id AND a fresh pairing bundle for us, because a stranger has no
    /// other route to the owner: no directory, no server-side identity lookup. Scanning
    /// pairs with the owner and then asks. Returns nil on a private Place (the core
    /// refuses rather than handing out a pairing route to somewhere unopened).
    func placeJoinCard(_ objectId: String) -> String? {
        do { return try core.placeJoinCard(objectId: objectId) }
        catch { fail("build the place's join code", error); return nil }
    }

    /// Record whose land a place sits on — a CLAIM with its provenance, never a verified
    /// fact.
    ///
    /// NOTHING IN THE APP DOES THE LOOKUP. HM Land Registry's free products are monthly
    /// bulk files (INSPIRE Index Polygons for the parcel, CCOD/OCOD for a corporate
    /// proprietor), not a point-query API, and Scotland is a separate register again. So
    /// resolution is an ingest job elsewhere; this stores the answer signed, with the
    /// dataset vintage, so a reader can judge how stale it is.
    ///
    /// `permission` is a self-report. A register tells you who to ask — never what they
    /// said — so knowing the owner must not be mistaken for having asked them.
    @discardableResult
    func setPlaceLand(_ objectId: String, parcel: String, proprietor: String = "",
                      companyNo: String = "", source: String, asOf: Date,
                      permission: String = "unknown") -> Bool {
        do {
            _ = try core.placeSetLand(objectId: objectId, parcel: parcel,
                                      proprietor: proprietor, companyNo: companyNo,
                                      source: source,
                                      asOf: Int64(asOf.timeIntervalSince1970 * 1000),
                                      permission: permission)
            refreshPlaces()
            return true
        } catch {
            self.error = String(describing: error)
            return false
        }
    }

    /// Retract a land claim — the lookup was wrong, or the land changed hands.
    @discardableResult
    func clearPlaceLand(_ objectId: String) -> Bool {
        do {
            _ = try core.placeClearLand(objectId: objectId)
            refreshPlaces()
            return true
        } catch {
            self.error = String(describing: error)
            return false
        }
    }

    /// Fit (or ROTATE) this place's doorbell. Returns the new capability as hex.
    ///
    /// Rotating retires every printed sign carrying the old one — the only revocation
    /// there is, and the only way to stop an ex-member reading arrivals.
    @discardableResult
    func fitDoorbell(_ objectId: String) -> String? {
        do {
            let db = try core.placeFitDoorbell(objectId: objectId)
            refreshPlaces()
            return db
        } catch {
            self.error = String(describing: error)
            return nil
        }
    }

    /// Knock at a place's door — what scanning the sign does.
    ///
    /// Deposits our own signed bundle in the place's mailbox. No pairing with anyone.
    /// Returns once the knock is published; MEMBERSHIP LANDS when a member answers.
    func knockAtPlace(placeID: String, doorbell: String) async -> Bool {
        let c = core
        do {
            try await Task.detached { try c.placeKnock(placeId: placeID, doorbellHex: doorbell) }.value
            return true
        } catch {
            self.error = String(describing: error)
            return false
        }
    }

    /// Answer the door at every open place we are in. Runs on EVERY member, not just the
    /// owner — that is what keeps a place joinable without one specific phone.
    @discardableResult
    func answerDoors() async -> [String] {
        let c = core
        do {
            let ids = try await Task.detached { try c.placeAnswerDoors() }.value
            if !ids.isEmpty { refreshPlaces() }
            return ids
        } catch {
            self.error = String(describing: error)
            return []
        }
    }

    /// Put a group on a place: the agreement to admit whoever scans it and picks you.
    @discardableResult
    func anchorGroup(_ groupId: String, at placeId: String) -> Bool {
        do {
            _ = try core.groupAnchorAtPlace(groupId: groupId, placeId: placeId,
                                            atMs: Int64(Date().timeIntervalSince1970 * 1000))
            return true
        } catch {
            self.error = String(describing: error)
            return false
        }
    }

    /// Our groups that answer for this place — what the scan picker lists.
    func groupsAnchored(at placeId: String) -> [String] {
        (try? core.groupsAnchoredAt(placeId: placeId)) ?? []
    }

    /// Admit everyone pending on our public places now, rather than on the next sync tick.
    /// The sync loop does this itself; this is for making it immediate.
    @discardableResult
    func reconcilePlaceJoins() -> [String] {
        do {
            let places = try core.placeReconcileJoins()
            if !places.isEmpty { refreshPlaces() }
            return places
        } catch {
            self.error = String(describing: error)
            return []
        }
    }

    /// Admit one joiner explicitly. Public places do this themselves; this is the manual
    /// escape hatch — adding someone who never scanned.
    @discardableResult
    func admitToPlace(_ objectId: String, peerId: String) -> Bool {
        do {
            _ = try core.placeAdmit(objectId: objectId, peerId: peerId)
            refreshPlaces()
            return true
        } catch {
            self.error = String(describing: error)
            return false
        }
    }

    /// Place (or move) an existing Place. Owner-sequenced at the core.
    @discardableResult
    func setPlaceLocation(_ objectId: String, lat: Double, lng: Double) -> Bool {
        do {
            _ = try core.placeSetLocation(objectId: objectId, lat: lat, lng: lng)
            refreshPlaces()
            return true
        } catch {
            self.error = String(describing: error)
            return false
        }
    }

    // MARK: - Things (the market objects)

    /// Every Thing we own, folded from its delta log. These are the DURABLE truth the LIFE
    /// market list reads; the resolver graph is the derived read-index that proposed them.
    /// Published so a mint re-lists without a manual refresh.
    @Published private(set) var things: [ThingRow] = []

    /// THE BUY SIDE: every live listing others have published or relayed to me, deduped
    /// across channels. `things` is my own inventory; this is everyone else's market.
    /// Published so a sync that discovers something re-lists it without a manual refresh.
    @Published private(set) var listings: [ListingRow] = []

    // Equality-gated like `refresh()`: an unchanged read must not republish — with the
    // feed subscribed to the tick, an ungated assignment is the 4 Aug storm again.
    func refreshThings() {
        do {
            let fresh = try core.things()
            if fresh != things { things = fresh }
        }
        catch { readFailed("re-read your things", error) }
        refreshListings()
    }

    func refreshListings() {
        do {
            let fresh = try core.listings()
            if fresh != listings { listings = fresh }
        }
        catch { readFailed("re-read listings", error) }
    }

    /// Fan my postures out to every connection and relay onward what I hold. The sync loop
    /// already does this on its own tick; call it to make a just-edited listing travel now
    /// rather than on the next tick.
    func publishMarket() {
        do { _ = try core.publishMarket() }
        catch { self.error = String(describing: error) }
    }

    /// THE mint — the one call behind every `+` in the app (Design/New.swift).
    ///
    /// `kind` is the CORE's own wire name, which is why `NewKind.rawValue` is that name:
    /// the UI cannot ask for a species the core does not have, and a rename on either
    /// side fails here rather than minting into the void. `pacific_core::mint` maps the
    /// one shared draft onto each kind's own op-0 arg vocabulary — `name` becomes
    /// `displayName` on a Group and `title` on an Event — so this layer never restates
    /// arg names, which is what `mintOrg` used to do by hand.
    ///
    /// Mints the object and its profile delta, nothing else. Posture, tickets, media and
    /// venue are separate ops the caller authors after, so a failure is reportable
    /// against the step that failed.
    @discardableResult
    func mint(kind: String, draft: MintDraft) -> String? {
        do {
            let id = try core.objectMint(kind: kind, draft: draft)
            NSLog("[mint] \(kind) → \(id)")
            return id
        } catch {
            self.error = String(describing: error)
            NSLog("[mint] \(kind) FAILED: \(self.error ?? "?")")
            return nil
        }
    }

    /// Mint a Thing — what the swipe deck does on ACCEPT. A group of 1: local, no relay.
    /// `category` is the market-face axis (artifact | skill | job).
    /// Returns the new object id, or nil after surfacing the failure.
    @discardableResult
    func mintThing(name: String, descriptor: String = "", category: String = "artifact") -> String? {
        do {
            let id = try core.thingMint(name: name, descriptor: descriptor, category: category)
            refreshThings()
            return id
        } catch {
            self.error = String(describing: error)
            return nil
        }
    }

    /// Put a thing in a market posture. `price` only means anything on an ACTIVE posture
    /// (buying/selling) — core rejects it on a standing intent rather than dropping it.
    /// `reach` and `area` scope who the posture is visible to. Both are optional at the core —
    /// an empty string means "unset" — so the defaults keep every existing caller's behaviour.
    func setPosture(_ objectId: String, posture: String, price: String = "", deadline: Int64 = 0,
                    reach: String = "", area: String = "") {
        do {
            _ = try core.thingSetPosture(objectId: objectId, posture: posture,
                                         price: price, deadline: deadline,
                                         reach: reach, area: area)
            refreshThings()
        } catch {
            self.error = String(describing: error)
        }
    }

    /// Re-title an existing thing — the profile half of a listing edit. The profile op is
    /// a full replace, so the category must be restated (pass the row's own).
    func renameThing(_ objectId: String, name: String, descriptor: String,
                     category: String = "artifact") {
        do {
            _ = try core.thingSetProfile(objectId: objectId, name: name,
                                         descriptor: descriptor, category: category)
            refreshThings()
        } catch {
            self.error = String(describing: error)
        }
    }

    /// Set (or clear) a group's cover banner. Stills are downscaled/re-compressed under
    /// the fold's cap BEFORE authoring (the setEventMedia rule); a gif or clip that
    /// cannot fit is refused loudly here — never clamped, so two devices always agree.
    /// Pass nil data to clear.
    @discardableResult
    func setGroupCover(_ objectId: String, data: Data?, mime: String) -> Bool {
        var payload = ""
        var outMime = ""
        if let data {
            switch mime {
            case "image/jpeg", "image/png":
                guard let fit = Self.jpegUnderCap(data, capB64: 700_000) else {
                    self.error = "cover image exceeds the media cap"
                    return false
                }
                payload = fit.base64EncodedString()
                outMime = "image/jpeg"
            case "image/gif":
                payload = data.base64EncodedString()
                guard payload.count <= 700_000 else {
                    self.error = "gif exceeds the cover cap"
                    return false
                }
                outMime = mime
            case "video/mp4":
                payload = data.base64EncodedString()
                guard payload.count <= 1_500_000 else {
                    self.error = "clip exceeds the cover cap"
                    return false
                }
                outMime = mime
            default:
                self.error = "unsupported cover type \(mime)"
                return false
            }
        }
        do {
            _ = try core.groupSetCover(objectId: objectId, data: payload, mime: outMime)
            return true
        } catch {
            self.error = String(describing: error)
            return false
        }
    }

    /// Set a group's icon — the avatar STILL on its contact card. `setProfile` replaces
    /// the whole card, so the current card is restated verbatim with only the photo
    /// slots changed; losing the org/emails/colour to an icon change would be a bug.
    @discardableResult
    func setGroupIcon(_ objectId: String, jpeg: Data) -> Bool {
        guard let v = orgView(objectId) else {
            self.error = "no folded view for \(objectId.prefix(12))"
            return false
        }
        guard let fit = Self.jpegUnderCap(jpeg, capB64: 500_000) else {
            self.error = "icon exceeds the photo cap"
            return false
        }
        let card: [String: Any] = [
            "org": v.org, "title": v.title, "note": v.note, "tags": v.tags,
            "photo": fit.base64EncodedString(), "photo_mime": "image/jpeg",
            "clip": v.clip, "clip_mime": v.clipMime, "captured_at": v.capturedAt,
            "avatar_color": v.avatarColor,
            "emails": v.emails.map { ["label": $0.label, "value": $0.value] },
            "phones": v.phones.map { ["label": $0.label, "value": $0.value] },
            "urls": v.urls.map { ["label": $0.label, "value": $0.value] },
        ]
        guard let json = (try? JSONSerialization.data(withJSONObject: card))
            .flatMap({ String(data: $0, encoding: .utf8) }) else {
            self.error = "could not encode the card"
            return false
        }
        do {
            _ = try core.groupAuthor(objectId: objectId, draft: DeltaDraft(
                kind: .group, opId: 0,
                args: [ArgEntry(key: "displayName", value: .text(value: v.displayName)),
                       ArgEntry(key: "shape", value: .text(value: v.shape)),
                       ArgEntry(key: "card", value: .text(value: json))]))
            return true
        } catch {
            self.error = String(describing: error)
            return false
        }
    }

    /// Record that a group CREATED an object it hosts — an Event or a listing Thing.
    /// Owner-only (the core refuses otherwise). `refresh()` republishes `objects`, the
    /// delta→graph trigger, so the authorship edge lands in the graph immediately.
    @discardableResult
    func attachCreated(_ groupId: String, objectId: String, name: String) -> Bool {
        do {
            _ = try core.groupAttachCreated(groupId: groupId, objectId: objectId,
                                            name: name, at: 0)
            refresh()
            return true
        } catch {
            self.error = String(describing: error)
            return false
        }
    }

    /// Truncate a position to a publishable coarse area. Delegates to core so there is ONE
    /// geohash implementation, and the precision floor is enforced where the reducer enforces
    /// it rather than being re-derived (and drifting) on this side.
    func coarseArea(lat: Double, lon: Double, precision: Int) -> String {
        (try? core.coarseArea(lat: lat, lon: lon, precision: UInt8(clamping: precision))) ?? ""
    }

    func clearPosture(_ objectId: String) {
        do {
            _ = try core.thingClearPosture(objectId: objectId)
            refreshThings()
        } catch {
            self.error = String(describing: error)
        }
    }

    /// Set (or clear, with nil) a Thing's photo — the listing's face. Fitted under
    /// the shared photo cap by the ONE helper (`jpegUnderCap`), authored as a real
    /// `thing.setPhoto` delta, and carried by the next market publish to every
    /// connection. Reaches the relay — runs off-main.
    @discardableResult
    func setThingPhoto(_ objectId: String, jpeg: Data?) async -> Bool {
        var photo = ""
        var mime = ""
        if let jpeg {
            guard let fit = Self.jpegUnderCap(jpeg, capB64: 500_000) else {
                self.error = "photo exceeds the media cap"
                return false
            }
            photo = fit.base64EncodedString()
            mime = "image/jpeg"
        }
        let c = core
        do {
            _ = try await Task.detached {
                try c.thingSetPhoto(objectId: objectId, photo: photo, mime: mime)
            }.value
            refreshThings()
            return true
        } catch { self.error = String(describing: error); return false }
    }

    // MARK: - Events (the ticketed GroupObjects)

    /// Every Event we hold, folded from its delta log. Like Things and Places these are the
    /// DURABLE truth — an Event is a GroupObject (kind 29) carrying profile + ticket listing +
    /// sales ledger. Published so a mint re-lists without a manual refresh.
    /// Every Post we hold, folded. Republished only on a real change — the equality gate
    /// is what keeps the 3s sync tick from re-rendering the feed over unchanged rows.
    @Published private(set) var posts: [PostRow] = []

    func refreshPosts() {
        do {
            let fresh = try core.posts()
            if fresh != posts { posts = fresh }
        }
        catch { readFailed("re-read your posts", error) }
    }

    @Published private(set) var events: [EventRow] = []

    func refreshEvents() {
        do {
            let fresh = try core.events()
            if fresh != events { events = fresh }
        }
        catch { readFailed("re-read your events", error) }
        refreshInvites()
    }

    /// Events I have said I am GOING to — the delta-backed commitment that replaced
    /// the device-local going-set.
    ///
    /// Derived, never stored: an entry exists because a signed `contact.inviteReply`
    /// exists on some channel. That is what makes it true for the organiser too,
    /// rather than only on this phone.
    @Published private(set) var goingEvents: Set<String> = []

    /// eventId -> the peer whose channel carries the invitation. An answer has to be
    /// authored to SOMEBODY; this is who.
    private var inviteChannels: [String: String] = [:]

    /// Re-fold the invitations on every connection. Cheap enough to run on the sync
    /// tick (one fold per connection) and it must not be lazy — a row asking "am I
    /// going" mid-scroll cannot afford a per-row fold.
    func refreshInvites() {
        var going: Set<String> = []
        var channels: [String: String] = [:]
        let me = myIdentityHex
        // includeBlocked: a commitment you made stays true after you block the inviter.
        for c in connectionRows(includeBlocked: true) {
            for inv in invites(peer: c.peerId) {
                // An invitation I RECEIVED — mine to answer, so this channel is where
                // the answer goes.
                if inv.author != me {
                    channels[inv.event] = c.peerId
                    if inv.reply == "going" { going.insert(inv.event) }
                }
            }
        }
        if going != goingEvents { goingEvents = going }
        inviteChannels = channels
    }

    /// Commit, or withdraw. Returns false when there is nobody to tell — an event
    /// nobody invited you to has no channel, and a commitment with no counterparty is
    /// exactly the local-only mark this replaced.
    @discardableResult
    func setGoing(event: String, going: Bool) async -> Bool {
        guard let peer = inviteChannels[event] else { return false }
        let ok = await replyInvite(peer: peer, event: event, going: going)
        if ok { refreshInvites() }
        return ok
    }

    /// True when this event can carry a real commitment — i.e. somebody invited you,
    /// so there is a channel to answer on.
    func canCommit(event: String) -> Bool { inviteChannels[event] != nil }

    /// Who has said they are GOING to this event, as (peer id, display name).
    ///
    /// NOT the MLS roster. An Event GroupObject's members are the organiser and the
    /// door staff — buyers and guests are deliberately kept OUT of it (the ticketing
    /// model) — so a page that showed `members` as attendance would state a confident
    /// falsehood: two people at a sold-out night of four hundred.
    ///
    /// What is true instead is the set of signed `contact.inviteReply` legs on the
    /// channels this device can see: invitations I SENT (`author == me`) that came
    /// back "going". That is a partial view by construction — someone else's guests
    /// answered on channels I do not hold, and no total exists on this device — so
    /// this is a list of people, never a headcount of the night.
    func attendees(of event: String) -> [(id: String, name: String)] {
        let me = myIdentityHex
        var out: [(id: String, name: String)] = []
        var seen = Set<String>()
        // includeBlocked: they still answered. Blocking hides a person from you; it
        // does not un-say what they said to the organiser.
        for c in connectionRows(includeBlocked: true) {
            let going = invites(peer: c.peerId).contains {
                $0.event == event && $0.author == me && $0.reply == "going"
            }
            guard going, !seen.contains(c.peerId) else { continue }
            seen.insert(c.peerId)
            out.append((c.peerId,
                        c.displayName.isEmpty ? String(c.peerId.prefix(8)) : c.displayName))
        }
        return out.sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    }

    // ---- INVITES: an event invitation rides the pairwise channel ------------
    // A guest never joins the Event GroupObject (the ticketing model deliberately
    // keeps buyers out), so an invitation has nowhere to live except the Contact
    // channel the two of you already share — the same place the ticket legs ride.

    /// Invite `peer` to an event. The leg carries title/when/where, NOT just the id,
    /// because the invitee does not hold the Event object: an id-only invite would
    /// render as an opaque handle and be unanswerable offline. Reaches the relay —
    /// runs off-main.
    @discardableResult
    func sendInvite(peer: String, event: String, title: String, startMs: Int64,
                    venue: String = "", note: String = "") async -> Bool {
        let c = core
        let hex = peerHex(peer)   // connections() hands out space-form ids
        let at = Int64(Date().timeIntervalSince1970 * 1000)
        do {
            _ = try await Task.detached {
                try c.sendInvite(peer: hex, event: event, title: title,
                                 startMs: startMs, venue: venue, note: note, at: at)
            }.value
            return true
        } catch { self.error = String(describing: error); return false }
    }

    /// The invite/ticket FFI family (`sendInvite`/`replyInvite`/`invites`) parses
    /// peers with the STRICT hex parser, while `connections()` emits SPACE-form
    /// ids ("space1" + hex) — so every peer that flows connections → invites died
    /// in `parse_peer_hex` and the RSVP loop read as permanently empty (10 Aug
    /// two-sim run 4 finding; the DM family already tolerates both via
    /// `parse_space_id`). Normalise at this boundary until the FFI parsers are
    /// unified — that regen (`just ffi`) needs a quiet tree.
    private func peerHex(_ peer: String) -> String {
        IdentityKey.hex(peer)
    }

    /// Answer an invitation. Changing your mind is just calling this again — core
    /// keys the reply per (author, event) and the later gen wins.
    @discardableResult
    func replyInvite(peer: String, event: String, going: Bool) async -> Bool {
        let c = core
        let hex = peerHex(peer)
        let at = Int64(Date().timeIntervalSince1970 * 1000)
        do {
            _ = try await Task.detached {
                try c.replyInvite(peer: hex, event: event, going: going, at: at)
            }.value
            return true
        } catch { self.error = String(describing: error); return false }
    }

    /// Every invitation on the channel with `peer` — sent and received, newest first,
    /// each joined to its answer. Synchronous read; the thread card renders from this.
    func invites(peer: String) -> [InviteRow] {
        (try? core.invites(peer: peerHex(peer))) ?? []
    }

    /// A human line for a repeat rule ("Every Thursday"). Core's own labeller, so the
    /// words can never drift from the RRULE that produced them.
    func recurrenceLabel(_ rrule: String) -> String {
        rrule.isEmpty ? "" : PacificCore.recurrenceSummary(rrule: rrule)
    }

    /// Mint an Event with its profile in one call. A group of 1: local, no relay.
    /// `recurrence` is an RFC 5545 RRULE for a repeating event; "" = one-off.
    @discardableResult
    func mintEvent(title: String, descriptor: String = "", startMs: Int64,
                   endMs: Int64 = 0, venue: String = "", recurrence: String = "",
                   lineup: String = "") -> String? {
        do {
            let id = try core.eventMint(title: title, descriptor: descriptor,
                                        startMs: startMs, endMs: endMs, venue: venue,
                                        recurrence: recurrence, lineup: lineup)
            refreshEvents()
            return id
        } catch {
            fail("create the event", error)
            return nil
        }
    }

    /// Point (or clear) the event's venue at a Place GroupObject — the `happens_at`
    /// edge the graph projector folds. Empty `placeId` clears it. `refresh()`
    /// republishes `objects`, the app's delta→graph trigger, so the edge lands in
    /// the graph immediately rather than on the next sync.
    @discardableResult
    func setEventVenue(_ objectId: String, placeId: String, name: String) -> Bool {
        do {
            _ = try core.eventSetVenue(objectId: objectId, place: placeId, name: name, at: 0)
            refreshEvents()
            refresh()
            return true
        } catch {
            self.error = String(describing: error)
            return false
        }
    }

    /// Set (or update) the ticket listing. `priceCents` 0 = free; a priced listing REQUIRES
    /// `acct` (the rail's connected account) and `delegate` (the fulfilment Arc's pk, hex) —
    /// the core refuses loudly otherwise, and the composer gates on the same rule.
    @discardableResult
    func setEventTickets(_ objectId: String, priceCents: Int64, currency: String,
                         capacity: Int64, open: Bool = true, acct: String = "",
                         delegate: String = "", terms: String = "", rev: Int64 = 0) -> Bool {
        do {
            _ = try core.eventSetTickets(objectId: objectId, priceCents: priceCents,
                                         currency: currency, capacity: capacity, open: open,
                                         acct: acct, delegate: delegate, terms: terms, rev: rev)
            refreshEvents()
            return true
        } catch {
            fail("set the tickets", error)
            return false
        }
    }

    /// The stable listing key of one event (what buyers and the box office use).
    func eventListingId(_ objectId: String) -> String? {
        do { return try core.eventListingId(objectId: objectId) }
        catch { readFailed("read the event's listing", error); return nil }
    }

    /// The fold's media caps, in BASE64 characters (event.rs). Oversize is REFUSED by
    /// the core — never clamped — so this side compresses to fit BEFORE authoring.
    static let bannerCapB64 = 700_000
    static let photoCapB64 = 500_000
    static let clipCapB64 = 1_500_000

    /// Set (or wholesale replace) an event's media facet. Stills are downscaled to
    /// 1080px and re-compressed down to quality 0.5 until they fit their cap; a still
    /// that cannot fit even then is refused loudly here — the same refusal the fold
    /// would give, before a delta is ever authored. The clip arrives already exported
    /// (the composer's AVAssetExportSession pass) so it is only cap-checked.
    @discardableResult
    func setEventMedia(objectId: String, bannerJPEG: Data?, photos: [Data], clipMP4: Data?) -> Bool {
        var banner = ""
        if let data = bannerJPEG {
            guard let fit = Self.jpegUnderCap(data, capB64: Self.bannerCapB64) else {
                self.error = "banner exceeds the media cap"
                return false
            }
            banner = fit.base64EncodedString()
        }
        var rows: [EventPhotoRow] = []
        for data in photos {
            guard let fit = Self.jpegUnderCap(data, capB64: Self.photoCapB64) else {
                self.error = "photo exceeds the media cap"
                return false
            }
            rows.append(EventPhotoRow(data: fit.base64EncodedString(), mime: "image/jpeg"))
        }
        var clip = ""
        if let mp4 = clipMP4 {
            clip = mp4.base64EncodedString()
            guard clip.count <= Self.clipCapB64 else {
                self.error = "clip exceeds the media cap"
                return false
            }
        }
        do {
            _ = try core.eventSetMedia(objectId: objectId,
                                       banner: banner,
                                       bannerMime: banner.isEmpty ? "" : "image/jpeg",
                                       photos: rows,
                                       clip: clip,
                                       clipMime: clip.isEmpty ? "" : "video/mp4")
            refreshEvents()
            return true
        } catch {
            self.error = String(describing: error)
            return false
        }
    }

    /// The full media facet of one event — the detail page's read.
    func eventMedia(_ objectId: String) -> EventMediaRow? {
        do { return try core.eventMedia(objectId: objectId) }
        catch { readFailed("read the event's media", error); return nil }
    }

    /// Downscale to 1080px (the Me.swift avatar helper's pattern, at flyer size) and
    /// walk quality 0.8 → 0.5 until the BASE64 rendering fits `capB64`. nil = it never
    /// fit — the caller refuses rather than authoring a delta the fold must reject.
    private static func jpegUnderCap(_ data: Data, capB64: Int) -> Data? {
        guard let image = UIImage(data: data) else { return nil }
        let longest = max(image.size.width, image.size.height)
        let scale = longest > 1080 ? 1080 / longest : 1
        let target = CGSize(width: image.size.width * scale, height: image.size.height * scale)
        let format = UIGraphicsImageRendererFormat.default()
        format.scale = 1
        let resized = UIGraphicsImageRenderer(size: target, format: format).image { _ in
            image.draw(in: CGRect(origin: .zero, size: target))
        }
        for quality: CGFloat in [0.8, 0.7, 0.6, 0.5] {
            guard let jpeg = resized.jpegData(compressionQuality: quality) else { return nil }
            if (jpeg.count + 2) / 3 * 4 <= capB64 { return jpeg }
        }
        return nil
    }

    /// BUYER: author the request Delta into the connection with `peer` (the organizer on
    /// free events) BEFORE any checkout opens — request-first, so a paid-then-killed
    /// session always has a request. Reaches the relay → runs off the main thread.
    @discardableResult
    func requestTicket(peer: String, listing: String, qty: Int64, pi: String) async -> Bool {
        let c = core
        let hex = Self.identityHex(peer)
        do {
            _ = try await Task.detached {
                try c.ticketRequest(peer: hex, listing: listing, qty: qty, pi: pi)
            }.value
            return true
        } catch { fail("request the ticket", error); return false }
    }

    /// SELLER: pending ticket requests folded off our connection with `peer`.
    func pendingTicketRequests(peer: String) -> [TicketRequestRow] {
        do { return try core.ticketRequests(peer: Self.identityHex(peer)) }
        catch { readFailed("read pending ticket requests", error); return [] }
    }

    /// The free-path fulfilment tail: record the sale on the event ledger (unit 0 / fee 0,
    /// fresh ticket ids) and issue + deliver the signed credentials into the buyer's
    /// connection, in one pass. Reaches the relay → runs off the main thread.
    @discardableResult
    func recordFreeSale(objectId: String, pi: String, buyer: String, qty: Int64) async -> Bool {
        let c = core
        let buyerHex = Self.identityHex(buyer)
        let currency = events.first { $0.id == objectId }?.currency ?? "usd"
        let tickets = (0..<max(qty, 1)).map { _ in UUID().uuidString.lowercased() }
        do {
            try await Task.detached {
                _ = try c.eventRecordSale(objectId: objectId, pi: pi, buyer: buyerHex,
                                          qty: qty, unitCents: 0, feeCents: 0,
                                          currency: currency, tickets: tickets)
                _ = try c.eventIssueAndDeliver(objectId: objectId, pi: pi)
            }.value
            refreshEvents()
            return true
        } catch { self.error = String(describing: error); return false }
    }

    /// BUYER: the ticket wallet folded off the connection with `peer` (who delivered).
    func walletTickets(peer: String) -> [WalletTicketRow] {
        do { return try core.wallet(peer: Self.identityHex(peer)) }
        catch { return [] }   // a not-yet-formed channel is not an error worth an alert
    }

    /// DOOR: one scanned QR → one verdict, joined in core (signature ∩ ledger ∩ redemptions).
    func doorCheck(_ objectId: String, qr: String) -> DoorCheckRow? {
        do { return try core.eventDoorCheck(objectId: objectId, qr: qr) }
        catch { self.error = String(describing: error); return nil }
    }

    /// DOOR: admit — author the redemption Delta (OR-set; converges across doors).
    /// Reaches the relay for a multi-member event → runs off the main thread.
    @discardableResult
    func redeem(_ objectId: String, ticket: String) async -> Bool {
        let c = core
        do {
            _ = try await Task.detached { try c.eventRedeem(objectId: objectId, ticket: ticket) }.value
            return true
        } catch { fail("redeem the ticket", error); return false }
    }

    /// DOOR → HOLDER: tell the buyer their ticket was admitted (`contact.admitTicket`).
    ///
    /// The redemption lands in the EVENT fold, which the buyer is deliberately NOT a member
    /// of, so this receipt on the pairwise channel is the only way their device can ever
    /// truthfully say they were let in. It is a COURTESY, not a gate: with no connection to
    /// the buyer there is nothing to author, and the door must still read green. Failure is
    /// therefore returned, never thrown into `error` — the door's own verdict stands.
    /// Reaches the relay → runs off the main thread.
    @discardableResult
    func admitReceipt(buyer: String, ticket: String, admittedMs: Int64) async -> Bool {
        let c = core
        let hex = Self.identityHex(buyer)
        guard !hex.isEmpty else { return false }
        do {
            _ = try await Task.detached {
                try c.ticketAdmitReceipt(buyer: hex, ticket: ticket, admittedMs: admittedMs)
            }.value
            return true
        } catch {
            NSLog("[door] admit receipt not sent (%@): %@", String(hex.prefix(12)), String(describing: error))
            return false
        }
    }

    /// OWNER: restate current event state at the current epoch so a late-joining door-staff
    /// device converges (state re-emission — the no-backfill doctrine's answer). Call after
    /// adding a member. Reaches the relay → runs off the main thread.
    @discardableResult
    func reemitEvent(_ objectId: String) async -> Int {
        let c = core
        do { return Int(try await Task.detached { try c.eventReemit(objectId: objectId) }.value) }
        catch { self.error = String(describing: error); return 0 }
    }

    /// The folded timeline rows (the vertical Gantt) — structured, honest-absent.
    func projectItems(_ objectId: String) -> [ProjectItemRow] {
        (try? core.projectItems(objectId: objectId)) ?? []
    }

    /// The folded deliverables (child-Project subscriptions), collapsed-view rows.
    func projectDeliverables(_ objectId: String) -> [DeliverableRow] {
        (try? core.projectDeliverables(objectId: objectId)) ?? []
    }

    /// The Project card header (title/status + derived piece count).
    func projectHeader(_ objectId: String) -> ProjectHeaderRow? {
        try? core.projectHeader(objectId: objectId)
    }

    /// The folded dependency edges — what the item editor lists and removes; the
    /// derived `blocked` state comes from these unresolved edges.
    func projectDependencies(_ objectId: String) -> [DependencyRow] {
        (try? core.projectDependencies(objectId: objectId)) ?? []
    }

    /// The current MLS member roster of an object (identity hex ids) — the Project
    /// assignee picker's options and the "Add people" screen's current members.
    func objectMembers(_ objectId: String) -> [String] {
        (try? core.objectMembers(objectId: objectId)) ?? []
    }

    // MARK: - Graph projection inputs

    /// Everything `ObjectProjection` reads through the FFI, gathered in ONE detached
    /// pass: connections, each object's roster, each group's folded view. Off-main
    /// because this is ~2N FFI calls and its triggers fire on the sync cadence — on
    /// the main actor it was a per-tick UI stall proportional to the object count.
    struct ProjectionInputs {
        let connections: [ConnectionRow]
        let members: [String: [String]]   // object id → roster identity hexes
        let groups: [String: GroupRow]    // group object id → folded view
    }

    func projectionInputs(for objects: [ObjectRow]) async -> ProjectionInputs {
        let core = self.core
        let wanted = objects
            .filter { ["group", "project", "thing", "place", "event"].contains($0.kind) }
            .map { (id: $0.id, kind: $0.kind) }
        return await Task.detached(priority: .utility) {
            var members: [String: [String]] = [:]
            var groups: [String: GroupRow] = [:]
            for o in wanted {
                members[o.id] = (try? core.objectMembers(objectId: o.id)) ?? []
                if o.kind == "group" { groups[o.id] = try? core.groupView(objectId: o.id) }
            }
            return ProjectionInputs(connections: (try? core.connections()) ?? [],
                                    members: members, groups: groups)
        }.value
    }

    // MARK: - Orgs (the LIFE ▸ GROUPS ▸ ＋ hot path)

    /// Constitute a body: a real Group GroupObject (a group of 1 — local, no relay)
    /// carrying a real `group.setProfile` delta. Returns the object id, or nil after
    /// surfacing the failure.
    ///
    /// `shape` is core's `GroupShape` (individual|team|organisation). "Community" is not
    /// a core shape — `OrgWizard` mints it as `organisation` and keeps the refinement in
    /// its own layer (see OrgWizard.swift's header).
    ///
    /// `purpose` rides the ContactCard's `note`, which is the only free-text slot on the
    /// profile op. That is where a community interest statement belongs until the op
    /// catalog carries one of its own.
    @discardableResult
    func mintOrg(name: String, shape: String, purpose: String = "") -> String? {
        do {
            let id = try core.objectNew(kind: "group", name: name)
            // setProfile is owner/sequenced — we are the owner of a group of 1, so it lands.
            var args = [ArgEntry(key: "displayName", value: .text(value: name)),
                        ArgEntry(key: "shape", value: .text(value: shape))]
            if !purpose.trimmingCharacters(in: .whitespaces).isEmpty,
               let card = try? JSONSerialization.data(withJSONObject: ["note": purpose]),
               let json = String(data: card, encoding: .utf8) {
                args.append(ArgEntry(key: "card", value: .text(value: json)))
            }
            _ = try core.groupAuthor(objectId: id,
                                     draft: DeltaDraft(kind: .group, opId: 0, args: args))
            // The founder's tenure is written by core at mint (membership-through-mls.md
            // §10.3) — the app no longer records it.
            NSLog("[org] minted \(id) shape=\(shape)")
            return id
        } catch {
            fail("create the Space", error)
            NSLog("[org] mintOrg FAILED: \(self.error ?? "?")")
            return nil
        }
    }

    /// Author a real `group.setMemberRole` (op 4, owner/sequenced).
    ///
    /// `space` must be a SpaceID and the holder must ALREADY be on the MLS roster — core
    /// rejects with PreconditionFailed otherwise, because a role overlays a member and
    /// Author this Space's half of an affiliation — `group.setAffiliation` (op 5),
    /// which nothing in the app reached until now despite the op, the FFI draft
    /// and the ICD all existing.
    ///
    /// `rel` is the arm, and it decides what the projector mints:
    ///   peer     -> affiliated_with   (endpoints normalised, so both halves fold to one)
    ///   parent   -> member_of  self -> peer
    ///   child    -> member_of  peer -> self
    ///   created  -> created           (this Space published that object)
    ///   anchored -> nothing yet
    ///
    /// The peer need not be another Group: `created` already points at an Event
    /// or a Thing, and `anchored` at a Place. So a Space may be `peer` to an
    /// EVENT it did not create — "we are affiliated with that night" — which is
    /// how a Space links to a promoter's event without claiming to have made it.
    ///
    /// Only ever this side's half. Rosters never merge (the ICD says so in the
    /// op's own summary); the other party records their own half or does not.
    @discardableResult
    func setAffiliation(_ objectId: String, peer: String, rel: String,
                        name: String, tether: String? = nil,
                        atMs: Int64 = Int64(Date().timeIntervalSince1970 * 1000)) -> Bool {
        do {
            _ = try core.groupAuthor(objectId: objectId,
                                     draft: groupSetAffiliation(peer: peer, rel: rel, name: name,
                                                                tether: tether, at: atMs))
            refresh()
            NSLog("[org] setAffiliation \(rel) → \(peer.prefix(12))… on \(objectId.prefix(12))…")
            return true
        } catch {
            self.error = String(describing: error)
            return false
        }
    }

    /// never confers membership. Returns false (and sets `error`) on rejection so the
    /// caller can decline to show an office the log does not back.
    @discardableResult
    func setOrgRole(_ objectId: String, space: String, role: String) -> Bool {
        do {
            _ = try core.groupAuthor(
                objectId: objectId,
                draft: DeltaDraft(kind: .group, opId: 4,
                                  args: [ArgEntry(key: "space", value: .text(value: space)),
                                         ArgEntry(key: "role", value: .text(value: role))]))
            NSLog("[org] setMemberRole \(role) → \(space.prefix(16))… on \(objectId.prefix(12))…")
            return true
        } catch {
            self.error = String(describing: error)
            NSLog("[org] setMemberRole FAILED: \(self.error ?? "?")")
            return false
        }
    }

    /// The object's roster as SpaceIDs. `objectMembers` returns bare identity hex; the
    /// role op wants `space1<hex>`, so convert here rather than at every call site.
    func orgMembers(_ objectId: String) -> [String] {
        objectMembers(objectId).map { IdentityKey.display($0) }
    }

    /// A human label for a SpaceID, falling back to a truncated address. Never renders
    /// bare hex as if it were a name.
    func displayName(forSpace space: String) -> String {
        let hex = Self.identityHex(space)
        if let n = peerNamesByIdentity()[hex], !n.isEmpty { return n }
        if hex == myIdentityHex { return displayName }
        return String(hex.prefix(10)) + "…"
    }

    /// Which Space published this object — the `created` affiliation, read
    /// BACKWARDS.
    ///
    /// A Group records `created` against each Event or Thing it publishes, on
    /// its OWN log. So the event carries no pointer to its host; the host
    /// carries a pointer to the event, and finding the host means asking every
    /// Group we hold whether it claims this one. That is a scan, and it is the
    /// honest cost of one-sided edges: nobody may write on someone else's log.
    ///
    /// Cheap in practice — a device holds tens of Groups, not thousands — and
    /// exact: a Space that never claimed the event cannot appear.
    func creatorOfObject(_ objectId: String) -> (id: String, name: String)? {
        for g in orgs {
            guard let v = orgView(g.id) else { continue }
            if v.affiliations.contains(where: { $0.rel == "created" && $0.peer == objectId }) {
                return (id: g.id, name: v.displayName.isEmpty ? g.name : v.displayName)
            }
        }
        return nil
    }

    /// The folded Group state — display name, shape, roster roles, credentials.
    func orgView(_ objectId: String) -> GroupRow? {
        try? core.groupView(objectId: objectId)
    }

    /// Every Group identity record, folded — the LIFE ▸ GROUPS listing in one FFI
    /// call. Published so a mint re-lists without a manual refresh (mirrors
    /// `events`/`places`/`things`).
    @Published private(set) var orgs: [GroupListRow] = []

    /// THE SPACES — bodies you belong to, without the identity RECORDS this device
    /// keeps for its contacts (one per pairing, per imported vCard, per accepted
    /// Person proposal). `orgs` is every Group object; this is the half a Space
    /// surface means.
    ///
    /// The CORE decides which is which (`is_space`, from the peer edge and the
    /// folded shape). It was decided per-caller before: two surfaces wrote
    /// `shape != "individual"` by hand and the GROUPS lane wrote nothing, so every
    /// contact listed as a Space — nameless, because a seeded record carries its
    /// name in the MLS GroupContext and not in its folded profile.
    var spaces: [GroupListRow] { orgs.filter(\.isSpace) }

    func refreshOrgs() {
        do {
            let fresh = try core.groups()
            if fresh != orgs { orgs = fresh }
        }
        catch { readFailed("re-read your Spaces", error) }
    }

    /// ONE group's LIFE listing row — roster count (MLS, the single source) +
    /// avatar slots, addressed singly for the Profile binder. Reads the published
    /// cache first; folds fresh on a miss so a deep link ahead of the first
    /// `refreshOrgs` still resolves.
    func groupListRow(_ objectId: String) -> GroupListRow? {
        if let hit = orgs.first(where: { $0.id == objectId }) { return hit }
        return (try? core.groups())?.first { $0.id == objectId }
    }

    /// The Group's rooms, folded from ITS log (attach order). `held` rows open in
    /// ThreadView; the rest are rooms this device knows of but is not in yet.
    func groupForums(_ objectId: String) -> [GroupForumRow] {
        do { return try core.groupForums(groupId: objectId) }
        catch { readFailed("read the Space's channels", error); return [] }
    }

    /// Mint a room FOR a group: a real forum object plus its `part_of` edge (`base.setPart`)
    /// every member folds; connected members with stocked prekeys are pulled in
    /// immediately. Owner-only. Reaches the relay — runs off-main.
    @discardableResult
    func newGroupForum(_ objectId: String, name: String) async -> String? {
        let c = core
        let at = Int64(Date().timeIntervalSince1970 * 1000)
        do {
            let id = try await Task.detached {
                try c.groupForumNew(groupId: objectId, name: name, atMs: at)
            }.value
            refresh()
            refreshOrgs()
            return id
        } catch { fail("create the channel", error); return nil }
    }

    /// Up/downvote a post in a forum object — a real `forum.vote` delta (dir 1 up,
    /// -1 down, 0 clears; one vote per member per post, LWW). Reaches the relay —
    /// runs off-main. Failures surface via `error`, never a silent no-op.
    @discardableResult
    func votePost(_ objectId: String, on msg: ChatMsg, dir: Int8) async -> Bool {
        let c = core
        do {
            try await Task.detached {
                try c.objectVotePost(objectId: objectId, targetAuthor: msg.author,
                                     targetGen: msg.gen, dir: dir)
            }.value
            return true
        } catch { self.error = String(describing: error); return false }
    }

    /// Detach a room from a group — clears the edge; the room object, its log and
    /// its roster are untouched. Owner-only. Reaches the relay — runs off-main.
    @discardableResult
    func detachGroupForum(_ objectId: String, forumId: String) async -> Bool {
        let c = core
        do {
            _ = try await Task.detached {
                try c.groupForumDetach(groupId: objectId, forumId: forumId)
            }.value
            refreshOrgs()
            return true
        } catch { self.error = String(describing: error); return false }
    }

    /// A Channel's constituent Rooms, folded from ITS log (attach order = the
    /// tab order). `held` rows open; the rest are tabs this device knows of but
    /// is not in yet.
    func forumRooms(_ objectId: String) -> [ForumRoomRow] {
        do { return try core.forumRooms(forumId: objectId) }
        catch { readFailed("read the channel's rooms", error); return [] }
    }

    /// Mint a Room FOR a channel: a real forum object plus its `part_of` edge
    /// (`base.setPart`), which every member folds; connected members with stocked prekeys are
    /// pulled in immediately. Owner-only. Reaches the relay — runs off-main.
    @discardableResult
    func newForumRoom(_ objectId: String, name: String) async -> String? {
        let c = core
        let at = Int64(Date().timeIntervalSince1970 * 1000)
        do {
            let id = try await Task.detached {
                try c.forumRoomNew(forumId: objectId, name: name, atMs: at)
            }.value
            refresh()
            return id
        } catch { self.error = String(describing: error); return nil }
    }

    /// Detach a Room from its channel — clears the edge; the room object, its
    /// log and its roster are untouched. Owner-only. Reaches the relay — runs
    /// off-main.
    @discardableResult
    func detachForumRoom(_ objectId: String, roomId: String) async -> Bool {
        let c = core
        do {
            _ = try await Task.detached {
                try c.forumRoomDetach(forumId: objectId, roomId: roomId)
            }.value
            refresh()
            return true
        } catch { self.error = String(describing: error); return false }
    }

    /// The Objectives spine of a Project (Tab 1) — headline, goal, participants+roles,
    /// external stakeholders, key locations, KPIs.
    func projectObjectives(_ objectId: String) -> ObjectivesRow? {
        try? core.projectObjectives(objectId: objectId)
    }

    /// The Project's linked ChatRoom forum id (Tab 3), or nil if none is linked yet.
    func projectChatRoom(_ objectId: String) -> String? {
        (try? core.projectChatroom(objectId: objectId)) ?? nil
    }

    /// Mint a ChatRoom (forum) and link it to the Project via a `forum`-kind
    /// subscription edge. Returns the new forum id. Reaches the relay — runs off-main.
    @discardableResult
    func linkChatRoom(_ objectId: String) async -> String? {
        guard let forumId = try? core.objectNew(kind: "forum", name: "") else {
            self.error = "couldn't mint the Channel"
            return nil
        }
        refresh()
        let ok = await projectAuthor(objectId, projectSubscribe(
            subId: UUID().uuidString, target: forumId, kind: "forum", disclosure: "full", originItem: nil))
        return ok ? forumId : nil
    }

    /// Author one Project delta (built via a `project*` DeltaDraft constructor).
    /// Reaches the relay for a multi-member project, so it runs OFF the main thread.
    /// Failures surface via `error`, never a silent no-op.
    @discardableResult
    func projectAuthor(_ objectId: String, _ draft: DeltaDraft) async -> Bool {
        let c = core
        do {
            _ = try await Task.detached { try c.projectAuthor(objectId: objectId, draft: draft) }.value
            return true
        } catch { self.error = String(describing: error); return false }
    }

    /// The owner-set group title of a Forum object ("" until named), read live from
    /// the MLS GroupContext. `ObjectRow.name` already carries this for every row in
    /// `objects` — prefer that in list code; this is the single-object read.
    func objectTitle(_ objectId: String) -> String {
        (try? core.objectTitle(objectId: objectId)) ?? ""
    }

    /// The object's owner as 64-char identity hex ("" if unknown).
    func objectOwner(_ objectId: String) -> String {
        (try? core.objectOwner(objectId: objectId)) ?? ""
    }

    /// Whether WE own this object — only the owner may add members or rename, so
    /// the UI hides both affordances for everyone else rather than letting them
    /// tap into a guaranteed throw.
    func isGroupOwner(_ objectId: String) -> Bool {
        objectOwner(objectId) == myIdentityHex
    }

    /// MAY I MINT INTO THIS SPACE — a Channel, an Event, a Listing authored AS the Space?
    ///
    /// Asks the CORE (`GroupRole::may_mint`) rather than deciding here, so the `+` a
    /// member sees cannot disagree with what the fold will accept. Today that resolves to
    /// the owner, because the two ops that make a thing BELONG to a Space
    /// (`base.setPart`, `group.setAffiliation` 5) are both Owner-only and `Authority`
    /// has no Admin rung — see the rule's own doc comment.
    func groupMayMint(_ objectId: String) -> Bool {
        (try? core.groupMayMint(objectId: objectId)) ?? false
    }

    // ---- offices: an appointment is a delta, not a device dictionary --------

    /// Appoint a member to an office. Owner-only and owner-sequenced (core rejects
    /// anyone else), and the holder must already be on the roster — an office is an
    /// overlay on membership, never a grant of it.
    @discardableResult
    func setOffice(_ objectId: String, office: String, holderSpace: String,
                   status: String) async -> Bool {
        // The wire wants the identity pubkey; the UI carries SpaceIDs.
        let holder = IdentityKey.hex(holderSpace)
        let c = core
        let at = Int64(Date().timeIntervalSince1970 * 1000)
        do {
            _ = try await Task.detached {
                try c.groupSetOffice(groupId: objectId, office: office,
                                     holder: holder, status: status, at: at)
            }.value
            refreshOrgs()
            return true
        } catch { self.error = String(describing: error); return false }
    }

    /// Vacate an office. Removes the appointment — vacancy is absence, not a value.
    @discardableResult
    func clearOffice(_ objectId: String, office: String) async -> Bool {
        let c = core
        do {
            _ = try await Task.detached {
                try c.groupClearOffice(groupId: objectId, office: office)
            }.value
            refreshOrgs()
            return true
        } catch { self.error = String(describing: error); return false }
    }

    /// Re-author the identity record's shape — used by the offices migration to turn
    /// a device-local "community" refinement into the real `community` shape.
    @discardableResult
    func setOrgShape(_ objectId: String, shape: String) async -> Bool {
        guard let view = orgView(objectId) else { return false }
        let c = core
        let args = [ArgEntry(key: "displayName", value: .text(value: view.displayName)),
                    ArgEntry(key: "shape", value: .text(value: shape))]
        do {
            _ = try await Task.detached {
                try c.groupAuthor(objectId: objectId,
                                  draft: DeltaDraft(kind: .group, opId: 0, args: args))
            }.value
            refreshOrgs()
            return true
        } catch { self.error = String(describing: error); return false }
    }

    // ---- membership: leaving, removing and handing over are MLS COMMITS --------
    // The roster IS the access control. Each of these
    // calls a core door that performs the MLS operation AND writes the membership
    // record beside it — the app never writes a membership record itself, and core
    // refuses one written any other way. A refusal comes back as a thrown error whose
    // message is meant for a person ("the owner must hand the object over before
    // leaving"), and lands in `error` like every other failure here.

    /// Leave a group. Core records the departure and proposes this person's removal —
    /// every one of their devices. Another member completes it on their next sync (MLS
    /// forbids committing your own removal, RFC 9420 §12.2), and from that commit on
    /// this person can read nothing new. Until then the group shows as leaving.
    ///
    /// The owner cannot leave: ask `mustHandOverBeforeLeaving` first and offer the
    /// transfer instead.
    @discardableResult
    func leaveGroup(_ objectId: String) async -> Bool {
        await membershipCall("leave") { c in try c.groupLeave(objectId: objectId) }
    }

    /// Remove someone else — every device of theirs. Owner-only. `reason`, when given,
    /// is one of `requested`, `lost_device`, `inactive`, `conduct`, `other`.
    @discardableResult
    func removeMember(_ objectId: String, memberHex: String, reason: String? = nil) async -> Bool {
        await membershipCall("remove") { c in
            try c.groupRemoveMember(objectId: objectId, memberId: memberHex, reason: reason)
        }
    }

    /// Hand the group to another member. Owner-only; the successor must already be
    /// in it.
    @discardableResult
    func handOverGroup(_ objectId: String, toMemberHex: String) async -> Bool {
        await membershipCall("hand over") { c in
            try c.groupHandOver(objectId: objectId, memberId: toMemberHex)
        }
    }

    /// True when leaving is blocked pending a handover — i.e. you own this group.
    func mustHandOverBeforeLeaving(_ objectId: String) -> Bool {
        isGroupOwner(objectId) && orgMembers(objectId).count > 1
    }

    /// Is a leave in flight — proposed, and waiting for another member to complete it?
    func isLeaving(_ objectId: String) -> Bool {
        (try? core.isLeaving(objectId: objectId)) ?? false
    }

    /// Has a commit removed this device from the object? Its history stays readable.
    func isDeparted(_ objectId: String) -> Bool {
        (try? core.isDeparted(objectId: objectId)) ?? false
    }

    /// One path for the three doors — each reaches the relay, so it runs off-main.
    private func membershipCall(_ what: String, _ body: @escaping (Core) throws -> Void) async -> Bool {
        let c = core
        do {
            try await Task.detached { try body(c) }.value
            NSLog("[membership] %@ ok", what)
            refresh()
            refreshOrgs()
            return true
        } catch {
            NSLog("[membership] %@ refused: %@", what, String(describing: error))
            self.error = String(describing: error)
            return false
        }
    }

    /// RENAME a group — an owner-only MLS GroupContextExtensions commit. Every
    /// existing member folds it off the epoch tag on their next sync, and anyone
    /// added later reads it from their Welcome, so the name has ONE home at every
    /// epoch. Reaches the relay for a multi-member group → runs off-main.
    @discardableResult
    func renameGroup(_ objectId: String, to name: String) async -> Bool {
        let c = core
        let relay = relayUrl
        do {
            try await Task.detached {
                try c.objectRename(objectId: objectId, name: name)
            }.value
            refresh()
            return true
        } catch { fail("rename the Space", error); return false }
    }

    /// Peer display names keyed by 64-char identity hex — the roster form
    /// (`objectMembers`, `ChatMsg.author`) rather than the `space1…` address that
    /// `connections()` returns. Built once per list rebuild, not once per row.
    func peerNamesByIdentity() -> [String: String] {
        var out: [String: String] = [:]
        // includeBlocked: a label map, not a people surface — a blocked member's group
        // messages still render, and they should carry a name rather than raw hex.
        for c in connectionRows(includeBlocked: true) where !c.displayName.isEmpty {
            out[Self.identityHex(c.peerId)] = c.displayName
        }
        return out
    }

    /// A space id (`space1<hex>`) reduced to the bare 64-char identity hex.
    static func identityHex(_ spaceId: String) -> String {
        IdentityKey.hex(spaceId)
    }

    /// A human label for a group that carries no name of its own — the other
    /// members, the way every messenger does it ("Nye, Tom & Lauren"). NEVER the
    /// object id: that is an address, not a name. Members we have no name for are
    /// counted ("+2") rather than rendered as hex.
    func derivedGroupName(_ objectId: String, names: [String: String]? = nil) -> String {
        let names = names ?? peerNamesByIdentity()
        let others = objectMembers(objectId).filter { $0 != myIdentityHex }
        guard !others.isEmpty else { return "Just you" }
        let known = others.compactMap { names[$0] }
        let unknown = others.count - known.count
        guard !known.isEmpty else { return "Group" }

        var label: String
        switch known.count {
        case 1: label = known[0]
        case 2: label = "\(known[0]) & \(known[1])"
        case 3: label = "\(known[0]), \(known[1]) & \(known[2])"
        default: label = "\(known[0]), \(known[1]) & \(known.count - 2) others"
        }
        if unknown > 0, known.count <= 3 { label += " +\(unknown)" }
        return label
    }

    /// Owner-add a peer (by their contact bundle) into an EXISTING project group —
    /// the two-person hinge. Reaches the relay; runs off-main. Returns the peer id.
    @discardableResult
    func groupAddMember(_ objectId: String, bundle: String) async -> String? {
        let c = core
        let relay = relayUrl
        do {
            let peer = try await Task.detached { try c.groupAddMember(objectId: objectId, bundle: bundle) }.value
            refresh()
            // Core recorded the arrival alongside the Add it committed (§10.3).
            return peer
        } catch { self.error = String(describing: error); return nil }
    }

    /// Add an EXISTING contact to `objectId` by CONSUMING one of their stocked prekeys
    /// — no code exchange (the tap-to-add path). Reaches the relay; runs off-main.
    /// Returns whether it succeeded; a failure (e.g. no prekeys left) surfaces via `error`.
    @discardableResult
    func addContact(_ objectId: String, peerId: String) async -> Bool {
        let c = core
        let relay = relayUrl
        do {
            _ = try await Task.detached { try c.addContactToObject(objectId: objectId, peerId: peerId) }.value
            refresh()
            // Same MLS Add, prekey path — core recorded the arrival (§10.3).
            return true
        } catch { fail("add them to the Space", error); return false }
    }

    /// This device authors its half of the AW26 Project demo through real ops.
    @discardableResult
    func seedProjectDemo(_ objectId: String) async -> Bool {
        let c = core
        let relay = relayUrl
        do { return try await Task.detached { try c.seedProjectDemo(objectId: objectId) }.value }
        catch { self.error = String(describing: error); return false }
    }

    /// DEV SEAM: dump what we hold for every connection — `name|org|title|photoSHA|
    /// color|gen` per line — so the host can prove a peer's profile actually crossed.
    /// The photo is reduced to a short digest: the point is that the bytes match, and
    /// a base64 JPEG would swamp the file.
    func dumpPeerProfiles() {
        let lines = connectionRows(includeBlocked: true).map { c -> String in
            guard let p = peerProfile(c.peerId) else { return "\(c.peerId)|<none>" }
            let photoTag = p.card.photo.isEmpty ? "-" : String(p.card.photo.hashValue, radix: 16)
            return [p.displayName, p.card.org, p.card.title, photoTag,
                    p.card.avatarColor, String(p.gen)].joined(separator: "|")
        }
        writeDevFile("dev-profiles.txt", lines.joined(separator: "\n") + "\n")
    }

    /// DEV SEAM: write the minted project id where the host two-sim script reads it.
    func exportProjectId(_ id: String) {
        let dir = FileManager.default
            .urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("pacific", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try? "\(id)\n".write(to: dir.appendingPathComponent("dev-project.txt"), atomically: true, encoding: .utf8)
    }

    /// DEV SEAM: write the folded board (deterministic lines) so the host can diff
    /// two sims' boards for byte-identical convergence.
    func dumpBoard(_ objectId: String) {
        let board = (try? core.projectBoard(objectId: objectId)) ?? []
        writeDevFile("dev-board.txt", board.joined(separator: "\n") + "\n")
    }

    // MARK: - ChatRoom (threaded Forum) dev seam

    /// DEV SEAM: write the minted ChatRoom (forum) id where the host 3-sim script
    /// reads it — the twin of `exportProjectId`.
    func exportRoomId(_ id: String) {
        writeDevFile("dev-room.txt", "\(id)\n")
    }

    /// DEV SEAM: the Space and the Channel a fleet harness has just minted, so the
    /// host can address them on the OTHER devices too. Same contract as
    /// `dev-room.txt` / `dev-event.txt` — a public object id, nothing secret.
    func exportSpaceId(_ id: String) { writeDevFile("dev-space.txt", "\(id)\n") }
    func exportForumId(_ id: String) { writeDevFile("dev-forum.txt", "\(id)\n") }

    /// DEV SEAM: write the folded thread as deterministic `depth|author8|text` lines
    /// (author + order are replica-independent, so the host can diff two devices'
    /// threads for byte-identical convergence).
    func dumpThread(_ objectId: String) {
        let lines = objectThread(objectId).map { "\($0.depth)|\($0.post.author.prefix(8))|\($0.post.text)" }
        writeDevFile("dev-thread.txt", lines.joined(separator: "\n") + "\n")
    }

    // MARK: - Ticketing dev seam (the two-sim e2e harness reads these)

    /// DEV SEAM: write the minted Event's `id` and its stable `listing` key — the two
    /// handles the buyer's device and the door both address. Twin of `exportProjectId`.
    func exportEventId(_ id: String) {
        writeDevFile("dev-event.txt", "\(id)\n\(eventListingId(id) ?? "")\n")
    }

    /// DEV SEAM: every Event this device folds, one deterministic line each —
    /// `id|listing|priceCents|currency|capacity|sold|listed|open|owner`. The host asserts
    /// the listing exists and the sale landed on the organizer's ledger.
    func dumpEvents() {
        let lines = events.map { r in
            [r.id, r.listing, String(r.priceCents), r.currency, String(r.capacity),
             String(r.sold), r.listed ? "1" : "0", r.open ? "1" : "0",
             isGroupOwner(r.id) ? "mine" : "theirs"].joined(separator: "|")
        }
        writeDevFile("dev-events.txt", lines.joined(separator: "\n") + "\n")
    }

    /// DEV SEAM: the wallet as the host must read it — `ticket|listing|title|verified|
    /// admittedMs|qr`. The QR is verbatim (never re-serialized): it is what the harness
    /// feeds the door in lieu of a camera, and `admittedMs` is the holder's green.
    func dumpWallet() {
        var seen = Set<String>()
        let rows = connectionRows(includeBlocked: true)
            .flatMap { walletTickets(peer: $0.peerId) }
            .filter { seen.insert($0.ticket).inserted }
        let lines = rows.map { t in
            [t.ticket, t.listing, t.title, t.verified ? "1" : "0",
             String(t.admittedMs), t.qr].joined(separator: "|")
        }
        writeDevFile("dev-wallet.txt", lines.joined(separator: "\n") + "\n")
    }

    /// DEV SEAM: pending ticket REQUESTS folded off every connection —
    /// `pi|listing|qty|author`. Lets the host see the buyer's request reach the seller
    /// before blaming the sweep for not fulfilling it.
    func dumpTicketRequests() {
        let lines = connectionRows(includeBlocked: true).flatMap { c in
            pendingTicketRequests(peer: c.peerId).map { r in
                [r.pi, r.listing, String(r.qty), r.author].joined(separator: "|")
            }
        }
        writeDevFile("dev-tickets.txt", lines.joined(separator: "\n") + "\n")
    }

    /// DEV SEAM: the door's own verdict, as one line — `verdict|ticket|buyer|receipt`.
    /// Written by `DoorScanSheet` after the real check, because a SwiftUI verdict screen
    /// is not something a host script can read. `receipt` is whether the holder was told.
    func dumpDoorVerdict(_ row: DoorCheckRow, receipt: Bool) {
        let line = [row.verdict, row.ticket, row.buyer, receipt ? "sent" : "-"].joined(separator: "|")
        NSLog("[door] verdict=%@", line)
        writeDevFile("dev-door.txt", line + "\n")
    }

    /// Write a small file into the app's `pacific/` support dir the host tooling
    /// reads out of the simulator container. Dev seam only — no secret material.
    private func writeDevFile(_ name: String, _ contents: String) {
        let dir = FileManager.default
            .urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("pacific", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try? contents.write(to: dir.appendingPathComponent(name), atomically: true, encoding: .utf8)
    }

    /// Author one 1:1 message to a connected peer. Reaches the relay, so it runs
    /// off the main thread. Loud on an unknown/not-connected peer — surfaced via
    /// `error` and a false return, never a silent no-op.
    @discardableResult
    func sendToDM(_ peerId: String, _ text: String) async -> Bool {
        let c = core
        do {
            let t0 = TimingProbe.nowMs
            _ = try await Task.detached { try c.dmPost(peerId: peerId, text: text) }.value
            TimingProbe.shared.send(convo: peerId, kind: "dm", chars: text.count,
                                    ackMs: TimingProbe.nowMs - t0)
            return true
        } catch { fail("send your message", error); return false }
    }

    // MARK: - Structured chat (timestamps · reactions · replies)

    /// The STRUCTURED transcript of a conversation — messages carrying a display
    /// timestamp, reply linkage and emoji-reaction groups. ONE code path for DMs
    /// (connections) and group objects (forums); the fold is the same core
    /// Coordinator, just a richer projection than the string `*Transcript`.
    func messages(_ c: Conversation) -> [ChatMsg] {
        do {
            switch c.kind {
            case .dm:    return try core.dmMessages(peerId: c.id)
            case .group: return try core.objectMessages(objectId: c.id)
            }
        } catch { readFailed("read the transcript", error); return [] }
    }

    /// Conversations whose read-receipt author is currently in flight — a guard so
    /// two overlapping opens (open + a sync tick) don't both author the same read
    /// receipts. The core is already idempotent; this just avoids the redundant work.
    private var markingRead: Set<String> = []

    /// Emit READ receipts for every message we hold from OTHERS in `c` — the
    /// ✓✓-blue the sender sees. Called when the user is viewing the conversation
    /// (on open and on each sync tick while it's open). Idempotent: the core only
    /// authors a receipt for a message not yet acknowledged, so a conversation with
    /// nothing new owes nothing and never touches the relay. Best-effort — a read
    /// receipt is courtesy telemetry, so a failure (e.g. a not-yet-connected DM) is
    /// swallowed, never surfaced as an alert. Reaches the relay → runs off-main.
    func markConversationRead(_ c: Conversation) async {
        let key = c.kind.rawValue + ":" + c.id
        if markingRead.contains(key) { return }
        markingRead.insert(key)
        defer { markingRead.remove(key) }
        let core = self.core
        let (id, kind) = (c.id, c.kind)
        _ = try? await Task.detached {
            switch kind {
            case .dm:    return try core.dmMarkRead(peerId: id)
            case .group: return try core.objectMarkRead(objectId: id)
            }
        }.value
    }

    /// Toggle THIS device's emoji reaction on `target` (one reaction per person,
    /// LWW — Signal semantics). `active=true` sets/replaces it with `emoji`,
    /// `false` clears it. A real `forum.react` Delta — reaches the relay for a
    /// multi-member conversation, so it runs OFF the main thread.
    @discardableResult
    func react(_ c: Conversation, to target: ChatMsg, emoji: String, active: Bool) async -> Bool {
        let core = self.core
        let (id, kind, author, gen) = (c.id, c.kind, target.author, target.gen)
        do {
            try await Task.detached {
                switch kind {
                case .dm:
                    try core.reactDm(peerId: id, targetAuthor: author, targetGen: gen,
                                     emoji: emoji, active: active)
                case .group:
                    try core.objectReact(objectId: id, targetAuthor: author, targetGen: gen,
                                         emoji: emoji, active: active)
                }
            }.value
            return true
        } catch { fail("add your reaction", error); return false }
    }

    /// Post a reply that quotes `parent`. A real `forum.post` Delta carrying the
    /// parent ref — reaches the relay for a multi-member conversation, so it runs
    /// OFF the main thread.
    @discardableResult
    func reply(_ c: Conversation, text: String, to parent: ChatMsg) async -> Bool {
        let core = self.core
        let (id, kind, author, gen) = (c.id, c.kind, parent.author, parent.gen)
        do {
            try await Task.detached {
                switch kind {
                case .dm:
                    _ = try core.replyDm(peerId: id, text: text, replyToAuthor: author,
                                         replyToGen: gen, media: nil)
                case .group:
                    _ = try core.objectReply(objectId: id, text: text, replyToAuthor: author,
                                             replyToGen: gen, media: nil)
                }
            }.value
            return true
        } catch { fail("send your reply", error); return false }
    }

    private var syncing = false

    /// Bumped after every successful sync so open surfaces (Chats list, an open
    /// conversation) reload and inbound messages appear without a manual pull.
    @Published private(set) var syncTick = 0

    /// Drain inbound Welcomes/commits/deltas across all groups. Blocks on the
    /// core's tokio runtime, so it runs off the main thread. A reentrancy guard
    /// prevents two syncs driving the same FFI handle concurrently.
    ///
    /// `silent` (the periodic background driver) swallows transport errors so a
    /// blip doesn't spam an alert every few seconds; a user-initiated sync
    /// (pull-to-refresh / ⟳) surfaces failures loudly via `error`.
    func syncNow(silent: Bool = false) async {
        if syncing { return }
        syncing = true
        defer { syncing = false }
        let c = core
        do {
            _ = try await Task.detached { try c.sync() }.value
            syncTick &+= 1
        } catch { if !silent { self.error = String(describing: error) } }
        refresh()
        TimingProbe.shared.drain(core: self)   // no-op unless PACIFIC_TIMING=1
    }

    /// Periodic background sync while the app is foregrounded — makes 1:1 and
    /// group messages arrive on their own (no push infra yet; a simple poll).
    /// Cancelled automatically when the driving view disappears.
    func autoSyncLoop(every seconds: UInt64 = 3) async {
        while !Task.isCancelled {
            await syncNow(silent: true)
            try? await Task.sleep(nanoseconds: seconds * 1_000_000_000)
        }
    }
}

// MARK: - The exterior (#76, ICD 2.6.0)
// What a non-member holds of an object, and its door. Every write and every door read is a
// relay round trip, so each runs off the main thread.

extension CoreService {
    /// The ICD's inline ceiling for an exterior's still, base64 characters.
    var exteriorPhotoMax: Int { Int(core.exteriorPhotoMax()) }

    /// Write an object's visibility and, when it reaches outward, its exterior. The name is the
    /// object's own; the still is cut to the ICD's ceiling or refused.
    @discardableResult
    func setOutside(_ objectId: String, name: String, visibility: String,
                    description: String, photo: Data, door: DoorChoice) async -> Bool {
        let c = core
        var photoB64 = ""
        if !photo.isEmpty {
            guard let jpeg = Self.jpegUnderCap(photo, capB64: exteriorPhotoMax) else {
                self.error = "That picture is too large for what outsiders see."
                return false
            }
            photoB64 = jpeg.base64EncodedString()
        }
        // At Only Me there is no outside to write: the dial alone.
        let outsideName = visibility == "private" ? "" : name
        do {
            // One Save: only what changed is written, and sent once.
            _ = try await Task.detached {
                try c.setOutside(objectId: objectId, visibility: visibility, name: outsideName,
                                 description: description, photo: photoB64,
                                 photoMime: photoB64.isEmpty ? "" : "image/jpeg", door: door.rawValue)
            }.value
            return true
        } catch { fail("write what outsiders see", error, object: objectId); return false }
    }

    /// After a mint: what the creator's Visibility face said, written when the dial was moved
    /// outward. Only Me is the mint's own state and writes nothing.
    func setOutside(_ objectId: String, from draft: NewDraft) async {
        guard draft.audience == .connections || draft.audience == .network else { return }
        await setOutside(objectId, name: draft.name, visibility: draft.audience.rawValue,
                         description: draft.outside, photo: draft.outsidePhoto, door: draft.door)
    }

    /// Ask to join an object whose exterior reached me.
    func requestJoin(_ objectId: String) async -> Bool {
        let c = core
        do {
            _ = try await Task.detached { try c.requestJoin(objectId: objectId, note: "") }.value
            refreshExteriors()
            return true
        } catch { fail("ask to join", error, object: objectId); return false }
    }

    func withdrawRequest(_ objectId: String) async -> Bool {
        let c = core
        do {
            try await Task.detached { try c.withdrawRequest(objectId: objectId) }.value
            refreshExteriors()
            return true
        } catch { fail("withdraw your ask", error, object: objectId); return false }
    }

    /// The requests waiting for me, from what this device's syncs have drained from its doors.
    func pendingRequests() async -> [RequestRow] {
        let c = core
        do { return try await Task.detached { try c.pendingRequests() }.value }
        catch { readFailed("read who is asking to join", error); return [] }
    }

    func decideRequest(_ r: RequestRow, admit: Bool) async -> Bool {
        let c = core
        do {
            try await Task.detached { try c.decideRequest(objectId: r.object, request: r.request, admit: admit) }.value
            return true
        } catch { fail(admit ? "admit" : "decline", error, object: r.object); return false }
    }
}
