//  Settings.swift
//  The account/Settings tree, reached from the top-left avatar. FOUR category cards —
//  PROFILE · SYSTEMS · WAKE · TRACE — and, beneath them, the two things that are not
//  categories at all: the theme switch and SIGN OUT. Every other control lives one level
//  down, as its own card in that category's submenu. The card geometry is the same
//  throughout (a List section per card), so the tree reads identically at every depth.
//
//  SIGN OUT sits at the root deliberately. It is the one control a user goes looking for
//  when something is wrong, and burying an escape hatch two menus deep is how it stops
//  being one. It erases (see Wipe.swift) — on a device that holds the only copy of the
//  identity there is no weaker sign-out that would mean anything.
//
//  Type: one word per title, set in caps, CENTRED, no icons and no chevrons — the word IS the
//  row. Navigation is a plain button + one `navigationDestination` per menu (a NavigationLink
//  would draw the system disclosure chevron back in).
//
//  Cut for v1 (4 Aug): ARCS (the backend picker is not a user surface yet), MODELS (nothing
//  read the placement), LOCATION (audience + hidden places had no enforcement — a legible
//  control that does nothing teaches the user false beliefs about their own safety).

import SwiftUI

// MARK: - Shared row

/// One settings card row: the title, centred, with an optional value centred beneath it.
struct SettingsRow: View {
    let label: String
    var value: String? = nil
    /// A row whose action cannot be undone says so in COLOUR, not in a caption
    /// underneath it. Only SIGN OUT uses it; nil leaves the inherited ink alone.
    var tone: Color? = nil

    var body: some View {
        VStack(spacing: 3) {
            let title = Text(label)
                .font(.subheadline.weight(.semibold))
                .tracking(1.0)
            if let tone { title.foregroundStyle(tone) } else { title }
            if let value, !value.isEmpty {
                Text(value)
                    .font(.caption).foregroundStyle(.secondary)
                    .lineLimit(1).truncationMode(.middle)
            }
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 8)
        .contentShape(Rectangle())
    }
}

/// A tappable settings card — centred title, no chevron. Fully-circular ends: each card is a
/// SLOT, so the row reads as a capsule cut out of the ground.
private struct SettingsCard: View {
    let label: String
    var value: String? = nil
    var tone: Color? = nil
    let action: () -> Void

    var body: some View {
        Section {
            Button(action: action) { SettingsRow(label: label, value: value, tone: tone) }
                .buttonStyle(.plain)
                .slotCard()
        }
    }
}

extension View {
    /// The settings "slot": a bedrock capsule row background with the separators out of the way.
    func slotCard() -> some View {
        listRowBackground(Color.clear.pixelCard(Brand.card, in: Capsule(style: .circular)))
            .listRowSeparator(.hidden)
    }
}

/// Standard chrome for every screen in the tree — one card geometry, one ground.
private struct SettingsList<Content: View>: View {
    let title: String
    @ViewBuilder var content: Content

    var body: some View {
        List { content }
            .scrollContentBackground(.hidden)
            .background(Theme.bg)
            .navigationTitle(title)
            .navigationBarTitleDisplayMode(.inline)
    }
}

// MARK: - Root

struct AccountSheet: View {
    private enum Route: Hashable { case profile, systems, wake, trace }
    @State private var route: Route?
    /// SIGN OUT is the one row here that is not navigation, so it is the one row
    /// that has to stop and ask. See `Wipe` for what "sign out" means on a device
    /// that holds the only copy of the identity.
    @State private var confirmingSignOut = false
    @State private var signingOut = false

    var body: some View {
        NavigationStack {
            SettingsList(title: "SETTINGS") {
                SettingsCard(label: "PROFILE") { route = .profile }
                SettingsCard(label: "SYSTEMS") { route = .systems }
                SettingsCard(label: "WAKE")    { route = .wake }
                SettingsCard(label: "TRACE")   { route = .trace }
                ThemeSwitch()
                // Last, and in danger ink: an ACTION among categories, and the only
                // irreversible one in the tree.
                SettingsCard(label: "SIGN OUT", tone: Theme.danger) { confirmingSignOut = true }
            }
            .navigationDestination(item: $route) { r in
                switch r {
                case .profile: ProfileMenu()
                case .systems: SystemsMenu()
                case .wake:    WakeMenu()
                case .trace:   TraceView()
                }
            }
        }
        // AN ALERT, NOT A CONFIRMATION DIALOG. On iPhone a confirmationDialog now presents
        // as a popover anchored near the tap, and SwiftUI DROPS the .cancel button from it
        // — dismissal is "tap somewhere else", which is discoverable by accident and by
        // nothing else. For the one control that destroys an identity, the way out has to
        // be as visible as the way through, so this is a modal alert with both.
        //
        // The message carries the whole consequence — an identity, a history and a set of
        // memberships — and names BOTH ways back, because there are two now and saying
        // one would be a lie of omission at the worst possible moment. A passkey wraps
        // the seed at the Arc, so signing in on this device again unwraps it; the 24
        // words (PROFILE → RECOVERY) restore it with no passkey at all. No caption on
        // the row itself: the row says what it does, the alert says what it costs.
        .alert("Sign out of WallFlowers?", isPresented: $confirmingSignOut) {
            Button("Cancel", role: .cancel) { }
            Button("Erase this device", role: .destructive) {
                signingOut = true
                Wipe.eraseAndQuit()
            }
        } message: {
            Text("This erases everything on this device: your identity, your messages, "
               + "your groups and every connected System. Your account itself survives — "
               + "your passkey or your twenty-four words bring it back. What is erased "
               + "here is this device's copy, and anything only this device held. "
               + "WallFlowers will close.")
        }
        .overlay { if signingOut { SigningOutVeil() } }
    }
}

/// The last frame before the process exits — so the app going away reads as the thing
/// the user just asked for rather than a crash.
private struct SigningOutVeil: View {
    var body: some View {
        ZStack {
            Theme.bg.ignoresSafeArea()
            Text("SIGNED OUT")
                .font(.footnote.weight(.semibold))
                .tracking(1.2)
                .foregroundStyle(.secondary)
        }
        .transition(.opacity)
    }
}

/// THEME — one switch, two keys: P (Pacific, light) / K (Kenjin, dark). Drives
/// the whole palette: WallFlowersApp pins `preferredColorScheme` from the same
/// store, and every Theme colour is a dynamic light/dark pair. The flip itself
/// hides behind `ThemeCrossfade`: a snapshot of the outgoing key fades on a
/// logarithmic curve while a glow blooms through the seam.
private struct ThemeSwitch: View {
    @AppStorage("themeMode") private var mode = "kenjin"

    var body: some View {
        Section {
            HStack {
                Spacer()
                Switch(options: ["pacific", "kenjin"],
                       word: { $0 == "pacific" ? "Light" : "Dark" },
                       selection: crossfaded)
                    .frame(width: 170)
                Spacer()
            }
            .slotCard()
        }
    }

    /// The mode, with the crossfade wrapped around every set.
    private var crossfaded: Binding<String> {
        Binding(get: { mode }, set: { new in
            guard new != mode else { return }
            ThemeCrossfade.run { mode = new }
        })
    }
}

/// The theme flip, staged: snapshot the window, flip the key beneath it, then
/// fade the snapshot out on a LOGARITHMIC curve — a fast first reveal that
/// settles asymptotically, sampled as keyframes of 1 − log10(1 + 9t) — while a
/// radial glow blooms and dies through the crossfade. Reduce Motion skips the
/// theatre and just flips.
@MainActor private enum ThemeCrossfade {
    static func run(_ flip: @escaping () -> Void) {
        guard !UIAccessibility.isReduceMotionEnabled,
              let window = UIApplication.shared.connectedScenes
                  .compactMap({ ($0 as? UIWindowScene)?.keyWindow }).first,
              let snap = window.snapshotView(afterScreenUpdates: false)
        else { flip(); return }

        snap.frame = window.bounds
        window.addSubview(snap)

        // The glow: a radial bloom rising from the switch's seat, screen-
        // composited so it lightens whichever key is showing through it.
        let glow = UIView(frame: window.bounds)
        glow.isUserInteractionEnabled = false
        let grad = CAGradientLayer()
        grad.type = .radial
        grad.frame = window.bounds
        grad.colors = [UIColor.white.withAlphaComponent(0.55).cgColor,
                       UIColor.white.withAlphaComponent(0).cgColor]
        grad.startPoint = CGPoint(x: 0.5, y: 0.62)
        grad.endPoint = CGPoint(x: 1.25, y: 1.4)
        glow.layer.addSublayer(grad)
        glow.layer.compositingFilter = "screenBlendMode"
        glow.alpha = 0
        window.addSubview(glow)

        flip()   // the new key renders beneath the snapshot

        UIView.animateKeyframes(withDuration: 0.9, delay: 0, options: [.calculationModeLinear]) {
            // 1 − log10(1 + 9t): most of the new key arrives in the first
            // quarter, the rest breathes in on the long tail.
            let steps = 14
            for i in 1...steps {
                let t = Double(i) / Double(steps)
                UIView.addKeyframe(withRelativeStartTime: Double(i - 1) / Double(steps),
                                   relativeDuration: 1.0 / Double(steps)) {
                    snap.alpha = CGFloat(1 - log10(1 + 9 * t))
                }
            }
            // The glow swells fast and dies on the same tail.
            UIView.addKeyframe(withRelativeStartTime: 0, relativeDuration: 0.18) {
                glow.alpha = 1
                glow.transform = CGAffineTransform(scaleX: 1.06, y: 1.06)
            }
            UIView.addKeyframe(withRelativeStartTime: 0.18, relativeDuration: 0.82) {
                glow.alpha = 0
                glow.transform = CGAffineTransform(scaleX: 1.3, y: 1.3)
            }
        } completion: { _ in
            snap.removeFromSuperview()
            glow.removeFromSuperview()
        }
    }
}

// MARK: - Wake

/// WAKE — can this device actually be woken by a message?
///
/// This screen exists so "notifications don't work" is DIAGNOSABLE instead of mysterious. Four
/// things must all be true, they fail independently, and until now none of them was visible:
///   TOKEN    APNs issued this install a device token (needs the aps-environment entitlement).
///   WAKER    the wake tier is reachable AND holds an APNs auth key (RINGFENCE.txt §6).
///   TAGS     the core told us which relay mailboxes to have watched.
///   WATCHING the waker accepted the (token, tags) registration.
/// Each row shows what it knows and nothing else — no captions, no advice.
struct WakeMenu: View {
    @ObservedObject private var push = PushRegistrar.shared

    var body: some View {
        SettingsList(title: "WAKE") {
            Section { SettingsRow(label: "TOKEN", value: tokenValue).slotCard() }
            Section { SettingsRow(label: "TAGS", value: "\(push.tags.count)").slotCard() }
            Section { SettingsRow(label: "WATCHING", value: push.state.label).slotCard() }
            SettingsCard(label: "WAKER", value: WakerEndpoint.host) {
                Task { await push.checkHealth() }
            }
            Section { SettingsRow(label: "HEALTH", value: push.health.label).slotCard() }
        }
        .task { await push.checkHealth() }
    }

    /// The token's first bytes, not the whole thing — enough to tell one install from another
    /// in a log, not enough to be worth copying out of a screenshot.
    private var tokenValue: String {
        guard let t = push.token, !t.isEmpty else { return "None" }
        return String(t.prefix(12)) + "…"
    }
}

// MARK: - Profile

struct ProfileMenu: View {
    private enum Route: Hashable { case name, photo, bio, contact, palette, recovery, privacy }
    @State private var route: Route?

    var body: some View {
        SettingsList(title: "PROFILE") {
            SettingsCard(label: "NAME")    { route = .name }
            SettingsCard(label: "PHOTO")   { route = .photo }
            SettingsCard(label: "BIO")     { route = .bio }
            SettingsCard(label: "CONTACT") { route = .contact }
            SettingsCard(label: "PALETTE") { route = .palette }
            // The first-boot recovery step, re-openable — the words are shown once
            // at the mint and a phrase you cannot look up is a phrase you lose.
            SettingsCard(label: "RECOVERY") { route = .recovery }
            SettingsCard(label: "PRIVACY") { route = .privacy }
        }
        .navigationDestination(item: $route) { r in
            switch r {
            case .name:     ProfileEditorScreen(title: "NAME")     { NameEditor() }
            case .photo:    ProfileEditorScreen(title: "PHOTO")    { AvatarEditor() }
            case .bio:      ProfileEditorScreen(title: "BIO")      { BioEditor() }
            case .contact:  ProfileEditorScreen(title: "CONTACT")  { ContactEditor() }
            case .palette:  ProfileEditorScreen(title: "PALETTE")  { PaletteEditor() }
            case .recovery: ProfileEditorScreen(title: "RECOVERY") { RecoveryKeyPanel() }
            case .privacy:  PrivacyMenu()
            }
        }
    }
}

/// One profile editor on its own Settings screen. The editors themselves live in
/// Me.swift and are shared with `MeView` — ONE implementation, two hosts.
private struct ProfileEditorScreen<Content: View>: View {
    var title: String
    @ViewBuilder var content: Content
    var body: some View {
        ScrollView { content.padding() }
            .background(Brand.groupBG)
            .navigationTitle(title)
            .navigationBarTitleDisplayMode(.inline)
    }
}

// MARK: - Privacy

/// PRIVACY — one control and the standing facts. BLOCKED is the device-local blocklist
/// (see BlockStore): a blocked person leaves the chat list, PEOPLE and every picker, and
/// their messages no longer wake this device. The statement panel below is the same one
/// MeView shows — what is true of every account, not a setting.
struct PrivacyMenu: View {
    @ObservedObject private var blocks = BlockStore.shared
    private enum Route: Hashable { case blocked }
    @State private var route: Route?

    var body: some View {
        SettingsList(title: "PRIVACY") {
            SettingsCard(label: "BLOCKED",
                         value: blocks.blocked.isEmpty ? nil : "\(blocks.blocked.count)") {
                route = .blocked
            }
            Section {
                PrivacyPanel()
                    .listRowBackground(Color.clear)
                    .listRowSeparator(.hidden)
                    .listRowInsets(EdgeInsets())
            }
        }
        .navigationDestination(item: $route) { _ in BlockedMenu() }
    }
}

/// One person on the blocklist screens — the peer id and the word shown for them.
private struct BlockedPerson: Identifiable, Hashable {
    let id: String
    let label: String

    init(_ peerId: String, name: String) {
        id = peerId
        label = name.isEmpty ? String(peerId.prefix(12)) + "…" : name
    }
}

/// The blocklist: each blocked person a card (tap → unblock, confirmed), and BLOCK to
/// pick from your connections. A blocked peer who has since left the roster still shows,
/// by id — the block outlives the connection.
struct BlockedMenu: View {
    @EnvironmentObject private var core: CoreService
    @ObservedObject private var blocks = BlockStore.shared
    @State private var unblocking: BlockedPerson?
    @State private var picking = false

    var body: some View {
        SettingsList(title: "BLOCKED") {
            ForEach(blockedRows) { row in
                SettingsCard(label: row.label) { unblocking = row }
            }
            SettingsCard(label: "BLOCK") { picking = true }
        }
        .navigationDestination(isPresented: $picking) { BlockPicker() }
        .confirmationDialog("Unblock \(unblocking?.label ?? "")?",
                            isPresented: Binding(get: { unblocking != nil },
                                                 set: { if !$0 { unblocking = nil } }),
                            titleVisibility: .visible) {
            Button("Unblock", role: .destructive) {
                if let row = unblocking { blocks.unblock(row.id) }
                unblocking = nil
            }
            Button("Cancel", role: .cancel) { unblocking = nil }
        }
    }

    private var blockedRows: [BlockedPerson] {
        let all = core.connectionRows(includeBlocked: true)
        return blocks.blocked.sorted().map { id in
            BlockedPerson(id, name: all.first { $0.peerId == id }?.displayName ?? "")
        }
    }
}

/// Everyone this device knows and has not blocked. Tapping a name blocks — after the
/// dialog says what that means, which is the one place the consequence is spelled out.
struct BlockPicker: View {
    @EnvironmentObject private var core: CoreService
    @ObservedObject private var blocks = BlockStore.shared
    @Environment(\.dismiss) private var dismiss
    @State private var blocking: BlockedPerson?

    var body: some View {
        SettingsList(title: "BLOCK") {
            ForEach(candidates) { row in
                SettingsCard(label: row.label) { blocking = row }
            }
        }
        .confirmationDialog("Block \(blocking?.label ?? "")? They leave your chats and people, and their messages stop reaching this device.",
                            isPresented: Binding(get: { blocking != nil },
                                                 set: { if !$0 { blocking = nil } }),
                            titleVisibility: .visible) {
            Button("Block", role: .destructive) {
                if let row = blocking { blocks.block(row.id) }
                blocking = nil
                dismiss()
            }
            Button("Cancel", role: .cancel) { blocking = nil }
        }
    }

    private var candidates: [BlockedPerson] {
        core.connectionRows()
            .map { BlockedPerson($0.peerId, name: $0.displayName) }
            .sorted { $0.label.localizedCaseInsensitiveCompare($1.label) == .orderedAscending }
    }
}

// MARK: - Systems

struct SystemsMenu: View {
    private enum Route: Hashable { case manage, music, art, calendar, connect, gmail }
    @State private var route: Route?

    var body: some View {
        SettingsList(title: "SYSTEMS") {
            SettingsCard(label: "MANAGE")   { route = .manage }
            SettingsCard(label: "MUSIC")    { route = .music }
            SettingsCard(label: "ART")      { route = .art }
            SettingsCard(label: "CALENDAR") { route = .calendar }
            SettingsCard(label: "CONNECT")  { route = .connect }
            SettingsCard(label: "GMAIL")    { route = .gmail }
        }
        .navigationDestination(item: $route) { r in
            switch r {
            case .manage:   SystemsView()
            case .music:    MusicView()
            case .art:      ArtView()
            case .calendar: CalendarConnectView()
            case .connect:  NotBuiltYet(title: "CONNECT")
            case .gmail:    NotBuiltYet(title: "GMAIL")
            }
        }
    }
}

// MARK: - Google Calendar (on-device OAuth System)

/// Connect Google Calendar as a live, on-device System: OAuth runs here, tokens are sealed in the
/// credential vault, and the calendar feeds the EVENTS ribbon (attendees → PEOPLE, location →
/// PLACES). Mirrors `SystemsView`'s connect/connected/error shape.
struct CalendarConnectView: View {
    @EnvironmentObject private var google: GoogleService

    private var connecting: Bool { google.status == .connecting }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Label("Google Calendar", systemImage: "calendar")
                    .font(.title3.bold()).foregroundStyle(Theme.textPrimary)
                Text("Your calendar becomes the EVENTS ribbon — each event's attendees join PEOPLE and its location joins PLACES. Sign-in happens on this device; the tokens are sealed in the credential vault and never synced.")
                    .font(.subheadline).foregroundStyle(.secondary)

                if !google.isConfigured {
                    setupNeeded
                } else if google.isSignedIn {
                    connected
                } else {
                    connectButton
                }

                if case .error(let e) = google.status {
                    Label(e, systemImage: "exclamationmark.triangle")
                        .font(.caption).foregroundStyle(.orange)
                }
            }
            .padding()
        }
        .background(Theme.bg)
        .navigationTitle("CALENDAR")
        .navigationBarTitleDisplayMode(.inline)
    }

    private var connectButton: some View {
        Button {
            Task { await google.connect() }
        } label: {
            HStack {
                if connecting { ProgressView().tint(Theme.onAccent) }
                Text(connecting ? "Connecting…" : "Connect Google Calendar")
            }
            .frame(maxWidth: .infinity)
        }
        .buttonStyle(.pixel)
        .foregroundStyle(Theme.onAccent)
        .disabled(connecting)
    }

    private var connected: some View {
        VStack(alignment: .leading, spacing: 14) {
            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 10) {
                    Image(systemName: "calendar").foregroundStyle(Brand.accent)
                    VStack(alignment: .leading, spacing: 1) {
                        Text(google.account ?? "Google Calendar").font(.headline)
                        Text("Connected · tokens sealed on this device")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    Spacer()
                }
                Text("\(google.events.count) upcoming event\(google.events.count == 1 ? "" : "s") feeding the EVENTS ribbon")
                    .font(.caption).foregroundStyle(.secondary)
            }
            .padding()
            .background(Brand.card, in: RoundedRectangle(cornerRadius: 14))

            Button { Task { await google.refreshEvents() } } label: {
                Label(connecting ? "Refreshing…" : "Refresh events", systemImage: "arrow.clockwise")
            }
            .disabled(connecting)

            Button(role: .destructive) { google.disconnect() } label: {
                Label("Disconnect", systemImage: "rectangle.portrait.and.arrow.right")
            }
            .padding(.top, 4)
        }
    }

    private var setupNeeded: some View {
        VStack(alignment: .leading, spacing: 8) {
            Label("Setup needed", systemImage: "wrench.and.screwdriver")
                .font(.subheadline.weight(.semibold)).foregroundStyle(.orange)
            Text("Add an iOS OAuth client id as GIDClientID in the app config, and register its reversed-client-id URL scheme. Then Connect runs Google sign-in on this device.")
                .font(.caption).foregroundStyle(.secondary)
        }
        .padding()
        .background(Brand.card, in: RoundedRectangle(cornerRadius: 14))
    }
}

// MARK: - Honest placeholder

/// A surface that is designed but not built. Says so plainly rather than showing controls that
/// do nothing — the house rule: never present a capability the app does not have.
struct NotBuiltYet: View {
    let title: String

    var body: some View {
        Text("NOT BUILT YET")
            .font(.footnote.weight(.semibold))
            .tracking(1.2)
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Theme.bg)
            .navigationTitle(title)
            .navigationBarTitleDisplayMode(.inline)
    }
}
