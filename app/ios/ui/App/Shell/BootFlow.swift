//  BootFlow.swift
//  FIRST BOOT — the sequence between install and the shell.
//
//  WHY IT EXISTS. First boot used to be ONE field: a name, a mint, and straight
//  into the shell. Everything else about a person — their face, their line, where
//  they are — was reachable only by going looking for Settings ▸ PROFILE, so a new
//  user was a default purple disc with no bio, and the three-sim fleet had to be
//  identified through LAUNCH ENVIRONMENT VARIABLES because no UI path to a complete
//  profile existed. This is that path: the same screens a user walks, drivable by
//  the harness, so the env seam can go back to being a CI skip.
//
//  SHAPE. Slate throughout: black ground, ONE thing per
//  screen, a field that says what it needs, no captions and no explainers
//  ([[ui-less-verbose]]). Every step after the name is SKIPPABLE and re-openable
//  from the profile page — first boot collects, it never gates. The one screen with
//  a teaching job is the last: it shows what the identity IS, so that a pairing
//  screen's word comparison later is a recognition rather than a puzzle.
//
//  ONE CEREMONY, TWO OUTCOMES (13 Sep). The first screen asks one question, and
//  the person should not have to know which half of it applies to them: PASSKEY
//  runs a discoverable assertion, and what comes back either unwraps a seed this
//  account already has or says there was nothing to choose. RECOVERY KEY is the
//  door that always opens. NEW ACCOUNT mints.
//
//  APPLE AND AI PASSPORT ARE GONE, and not only because their routes were
//  deleted. They authenticate a PERSON TO A SERVICE, which under this model is a
//  fact about nothing: what signs someone in is holding a seed, and an OAuth
//  provider cannot hand one over. They were two doors that could never carry a
//  key. Their entitlement went with them; `webcredentials:app.wallflowers.io` stays,
//  because that is what passkeys need.
//
//  THE SEED IS THE ACCOUNT. A passkey WRAPS it — the Arc stores 76 bytes of
//  ciphertext whose key never leaves the authenticator — and the 24 words
//  restore it when no passkey will. So signing in on a second device is not a
//  new identity: it is the same person, with the same key, and every peer's pin
//  still matching. That is why both sign-in doors skip the whole collecting
//  ladder below, which exists only to furnish an identity that has just been
//  minted.
//
//  WHAT SIGNING IN BRINGS is the account's objects: either door walks the spine
//  (`signInFromWrap`, `signInFromWords`) and joins every object it names before
//  any self record is made. A device that restored the identity alone and then
//  synced would mint a second one; neither door can.
//
//  A11Y IDS are part of the contract, not an afterthought (§4): the fleet drives
//  these screens through XCUITest, so every step is addressable from the first
//  commit — boot-door-{kenjin,restore,new}, boot-door-error, boot-restore-key,
//  welcome-name,
//  boot-face, boot-word, boot-city, boot-key, boot-fingerprint, boot-next,
//  boot-skip, boot-back.

import SwiftUI
import UIKit
import PhotosUI

struct BootFlow: View {
    @EnvironmentObject private var core: CoreService

    /// Where the sequence is. The order is the doc's: door → (restore) → name →
    /// face → bio → recovery key → fingerprint.
    enum Step: Hashable { case door, restore, name, face, bio, key, fingerprint }

    @State private var step: Step = .door

    var body: some View {
        ZStack {
            Color.black.ignoresSafeArea()
            Group {
                switch step {
                case .door:        DoorStep(go: { advance(to: $0) }, core: core)
                case .restore:     RestoreStep(back: { advance(to: .door) }, core: core)
                case .name:        NameStep(done: { advance(to: .face) })
                case .face:        FaceStep(nav: nav(back: nil, next: .bio))
                case .bio:         BioStep(nav: nav(back: .face, next: afterBio))
                case .key:         RecoveryKeyStep(nav: nav(back: .bio, next: .fingerprint))
                case .fingerprint: FingerprintStep(back: { advance(to: .key) }, enter: core.finishFirstBoot)
                }
            }
            .transition(.opacity)
        }
        // An identity that already exists when the sequence opens means a run that
        // was interrupted after the mint. Resume at the first COLLECTING step
        // rather than at a door that would only refuse.
        .onAppear { if step == .door, !core.spaceId.isEmpty { step = .face } }
    }

    /// The recovery-key step is only shown when there is a key to show and the
    /// user has not just typed it in: a RESTORE arrival already holds the words,
    /// and an identity minted before seeds existed has none.
    ///
    /// THE SKIP IS SAID OUT LOUD. There are exactly two honest reasons to walk
    /// past the only screen that shows a person their offline backup, and both
    /// are visible above. Anything else reaching this branch — an empty key on a
    /// freshly minted identity, say — is a fault, and a fault that silently
    /// skipped this step is how an account ends up with a backup nobody has read.
    private var afterBio: Step {
        guard core.recoveryKey().isEmpty else { return .key }
        Trace.shared.log("boot", "NO RECOVERY KEY to show — this identity has no words, so the "
              + "step is omitted rather than faked. It cannot be restored offline.", ok: false)
        return .fingerprint
    }

    private func advance(to next: Step) {
        // Put the keyboard away with the step that raised it. Without this it
        // stays up across the transition and sits over the next screen — which on
        // the recovery-key step covers the thing the user is being asked to copy.
        UIApplication.shared.sendAction(#selector(UIResponder.resignFirstResponder),
                                        to: nil, from: nil, for: nil)
        withAnimation(.easeInOut(duration: 0.22)) { step = next }
    }

    private func nav(back: Step?, next: Step) -> BootNav {
        BootNav(back: back.map { s in { advance(to: s) } }, next: { advance(to: next) })
    }
}

/// What a collecting step can do with the sequence: go back (nil on the first
/// step after the mint, where back would mean un-minting), or move on. SKIP and
/// NEXT are the same move — the step has already written whatever it collected.
struct BootNav {
    var back: (() -> Void)?
    var next: () -> Void
}

// MARK: - 0 · DOOR

/// ONE CEREMONY, TWO OUTCOMES. A passkey exists for this account or it does
/// not, and the person should not have to know which — so SIGN IN tries the
/// discoverable assertion first and mints only when the platform says there is
/// nothing to choose. The third word is the door that always opens.
private struct DoorStep: View {
    var go: (BootFlow.Step) -> Void
    @EnvironmentObject private var core: CoreService
    @StateObject private var signIn: SignInService

    init(go: @escaping (BootFlow.Step) -> Void, core: CoreService) {
        self.go = go
        _signIn = StateObject(wrappedValue: core.makeSignIn())
    }

    var body: some View {
        // NOTHING BUT THE DOORS. No wordmark, no caption — a first screen that
        // explains itself is a first screen that did not need to.
        BootSlate {
            Text("Sign In:")
                .font(.system(size: 15, design: .monospaced))
                .foregroundStyle(.white)
        } actions: {
            VStack(spacing: 12) {
                // THE PASSKEY DOOR. It unwraps a seed that is already this
                // person's; it does not prove them to a server. Apple and AI
                // Passport used to sit under this one and are gone: both
                // authenticate a PERSON TO A SERVICE, which under this model is
                // a fact about nothing, and both called routes the Arc deleted.
                BootWord("Passkey", enabled: signIn.busy == nil) { passkey() }
                    .accessibilityIdentifier("boot-door-kenjin")
                // THE DOOR THAT ALWAYS OPENS, and it is not a fallback for
                // failure: it is the only way in on a device whose platform does
                // not carry a passkey's secret across, and after a lost phone.
                BootWord("Recovery key", enabled: signIn.busy == nil) { go(.restore) }
                    .accessibilityIdentifier("boot-door-restore")
                BootWord("New account", enabled: signIn.busy == nil) { go(.name) }
                    .accessibilityIdentifier("boot-door-new")

                if let busy = signIn.busy {
                    Text(signIn.progress ?? busy.uppercased())
                        .font(.system(size: 9, design: .monospaced))
                        .tracking(1.6)
                        .foregroundStyle(Color(white: 0.5))
                }
                // THE ONE PIECE OF TEXT THAT STAYS, and it is not copy: it is
                // only ever on screen when something failed. A sign-in that fails
                // silently is the failure this surface exists to not have.
                if let err = signIn.error {
                    Text(err)
                        .font(.system(size: 10, design: .monospaced))
                        .foregroundStyle(Color(red: 0.88, green: 0.42, blue: 0.40))
                        .multilineTextAlignment(.center)
                        .padding(.horizontal, 28)
                        .accessibilityIdentifier("boot-door-error")
                }
            }
        }
    }

    /// The passkey door. Every outcome lands somewhere visible: signed in, or a
    /// message that says whose problem it is and what the other door is for.
    private func passkey() {
        signIn.error = nil
        Task {
            do {
                core.finishSignIn(try await signIn.signInWithPasskey())
            } catch SignInError.cancelled {
                // Not an error on screen. Logged anyway: "the person cancelled"
                // and "the sheet closed itself" are indistinguishable in a bug
                // report, and only one of them is a bug.
                Trace.shared.log("account", "passkey: cancelled by the person")
            } catch {
                Trace.shared.log("account", "passkey: FAILED — \(error.localizedDescription)", ok: false)
                signIn.error = error.localizedDescription
            }
        }
    }
}

// MARK: - 0b · RESTORE

/// The recovery key, typed back in. The phrase is checked by the core the moment
/// it is submitted — a mistyped word fails the mnemonic checksum HERE, at the
/// field where it can be fixed, rather than silently reconstituting a different
/// identity.
private struct RestoreStep: View {
    @EnvironmentObject private var core: CoreService
    var back: () -> Void
    @StateObject private var signIn: SignInService

    init(back: @escaping () -> Void, core: CoreService) {
        self.back = back
        _signIn = StateObject(wrappedValue: core.makeSignIn())
    }

    @State private var phrase = ""
    @State private var failed = false
    @FocusState private var focused: Bool

    private var wordCount: Int { phrase.split(whereSeparator: \.isWhitespace).count }
    private var ready: Bool { wordCount == 24 }

    var body: some View {
        BootSlate(back: back) {
            TextField("", text: $phrase, axis: .vertical)
                .font(.system(size: 17, design: .monospaced))
                .multilineTextAlignment(.center)
                .lineLimit(4...8)
                .textFieldStyle(.plain)
                .foregroundStyle(.white)
                .tint(.gray)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .focused($focused)
                .padding(.horizontal, 8)
                .overlay(alignment: .bottom) {
                    Rectangle().fill(failed ? Color.orange : Color(white: 0.23)).frame(height: 1)
                        .offset(y: 10)
                }
                .accessibilityIdentifier("boot-restore-key")
                .onChange(of: phrase) { _, _ in failed = false }
            // The only number on the screen, and the only one that helps: 24
            // words is the shape of the thing, and a person typing them needs
            // to know when they are done.
            Text(signIn.busy == nil ? "\(wordCount)/24" : (signIn.progress ?? "SIGNING IN"))
                .font(.system(size: 12, design: .monospaced))
                .foregroundStyle(Color(white: 0.42))
            if let err = signIn.error {
                Text(err)
                    .font(.system(size: 12, design: .monospaced))
                    .foregroundStyle(Color(red: 0.86, green: 0.42, blue: 0.40))
                    .multilineTextAlignment(.center)
                    .accessibilityIdentifier("boot-door-error")
            }
        } actions: {
            BootWord("RESTORE", enabled: ready && signIn.busy == nil, action: restore)
                .accessibilityIdentifier("boot-next")
        }
        .onAppear { focused = true }
    }

    /// The words sign in as the passkey does: the spine is walked before anything
    /// is made, and a restored account goes straight to the shell.
    private func restore() {
        guard ready else { return }
        signIn.error = nil
        Task {
            do {
                core.finishSignIn(try await signIn.restoreWithWords(phrase))
            } catch SignInError.badWords {
                failed = true
                UINotificationFeedbackGenerator().notificationOccurred(.error)
            } catch {
                Trace.shared.log("account", "words: FAILED — \(error.localizedDescription)", ok: false)
                signIn.error = error.localizedDescription
            }
        }
    }
}

// MARK: - 1 · NAME — the mint

/// One field, one word, and the keys are minted on the tap — the Ed25519 pair
/// genuinely does not exist until then, which is what makes this screen honest
/// rather than theatre. On the RESTORE path the identity is already back, so the
/// same screen is a plain rename.
private struct NameStep: View {
    @EnvironmentObject private var core: CoreService
    var done: () -> Void

    @State private var name = ""
    /// What the core said when the mint was refused. The RESTORE step's dress:
    /// the rule under the field turns, and the fault is stated.
    @State private var fault: String?
    @FocusState private var focused: Bool

    private var ready: Bool { !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }

    var body: some View {
        BootSlate {
            Wordmark()
            BootField("name", text: $name, focused: $focused)
                .submitLabel(.go)
                .onSubmit(begin)
                .accessibilityIdentifier("welcome-name")
                .overlay(alignment: .bottom) {
                    Rectangle().fill(fault == nil ? Color.clear : Color.orange)
                        .frame(height: 1).offset(y: 10)
                }
                .onChange(of: name) { _, _ in fault = nil }
            // THE FAULT, STATED. A mint can be refused — most often by a key file
            // that is present but unreadable, which fails `load` (so the app shows
            // this gate) and then fails `init` too (`IdentityExists`), leaving BEGIN
            // as a button that does nothing at all. Silence there is unfixable from
            // the outside; the core's own words are at least a thread to pull.
            if let fault {
                Text(fault)
                    .font(.system(size: 12, design: .monospaced))
                    .foregroundStyle(.orange)
                    .multilineTextAlignment(.center)
                    .padding(.horizontal, 24)
                    .accessibilityIdentifier("welcome-fault")
            }
        } actions: {
            BootWord("BEGIN", enabled: ready, action: begin)
                .accessibilityIdentifier("welcome-begin")
        }
        .onAppear { focused = true }
    }

    private func begin() {
        guard ready else { return }
        // An identity already here means the RESTORE door came first: the name is
        // a profile field, so this is a rename, not a second mint.
        guard core.spaceId.isEmpty else {
            core.rename(to: name.trimmingCharacters(in: .whitespacesAndNewlines))
            done()
            return
        }
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard core.completeOnboarding(name: trimmed) else {
            fault = core.error ?? "the identity could not be minted"
            UINotificationFeedbackGenerator().notificationOccurred(.error)
            return
        }
        // THE MINT IS ONLY HALF OF IT. An identity that exists on this phone and
        // nowhere else is one a second device can never reach and a lost phone
        // takes with it — so the same act makes the passkey, wraps the seed
        // under it, and stores those 76 bytes at the Arc.
        //
        // BESIDE THE PERSON, NEVER IN FRONT OF THEM. This used to be an `await`
        // with `done()` after it, which made a passkey ceremony the gate on the
        // whole sequence — and a ceremony that never calls back (no associated
        // domain to validate, no sheet, no error) left BEGIN as a tap that minted
        // an identity and then stopped, with the twenty-four words never shown.
        // A FAILURE HERE IS NOT A FAILED MINT and must not read as one, and
        // neither is a silence. The identity is real the instant `completeOnboarding`
        // returns true, so the sequence moves on that fact alone; the enrolment
        // runs on the CoreService (which outlives this screen) and reports itself
        // on the RECOVERY KEY step, where "this device has no passkey" is exactly
        // the thing that makes those words matter.
        core.enrolPasskey(label: trimmed)
        done()
    }
}

// MARK: - 2 · FACE — the block colour, then the photo

/// The swatch row comes pre-selected from the identity's own public key (seeded
/// at the mint, `CoreService.derivedAvatarColorHex`), so a user who taps straight
/// through still leaves with a face that is theirs rather than a shared purple.
/// Both controls write the REAL card through the same edit path Settings uses.
private struct FaceStep: View {
    @EnvironmentObject private var core: CoreService
    var nav: BootNav

    @State private var picked: PhotosPickerItem?

    var body: some View {
        BootSlate(back: nav.back) {
            Avatar(initials: core.initials, tint: .white, size: 96,
                   photo: core.avatarPhoto, color: core.avatarColor)
                // ONE element, so the harness can find the face it is editing —
                // an Avatar is a ZStack of shapes and would otherwise expose none.
                .accessibilityElement(children: .ignore)
                .accessibilityIdentifier("boot-face")

            HStack(spacing: 12) {
                ForEach(CoreService.avatarSwatches, id: \.self) { hex in
                    let swatch = Color(hex: hex)
                    Button { core.avatarColorHex = swatch.hexString } label: {
                        Circle().fill(swatch).frame(width: 22, height: 22)
                            .overlay(Circle().strokeBorder(.white.opacity(selected(swatch) ? 0.9 : 0),
                                                           lineWidth: 2))
                    }
                    .buttonStyle(.plain)
                    .accessibilityIdentifier("boot-face-swatch-\(String(format: "%06X", hex))")
                }
            }

            PhotosPicker(selection: $picked, matching: .images) {
                Text("photo")
                    .font(.system(size: 15))
                    .foregroundStyle(Color(white: 0.55))
                    .frame(maxWidth: .infinity)
                    .padding(.bottom, 6)
                    .overlay(alignment: .bottom) {
                        Rectangle().fill(Color(white: 0.23)).frame(height: 1)
                    }
            }
            .accessibilityIdentifier("boot-face-photo")
            .padding(.horizontal, 40)
        } actions: {
            BootWord("NEXT", action: nav.next).accessibilityIdentifier("boot-next")
        }
        .task(id: picked) {
            guard let item = picked,
                  let data = try? await item.loadTransferable(type: Data.self),
                  let jpeg = downscaledAvatarJPEG(data) else { return }
            core.avatarPhoto = jpeg
        }
    }

    private func selected(_ swatch: Color) -> Bool {
        core.avatarColorHex.caseInsensitiveCompare(swatch.hexString) == .orderedSame
    }
}

// MARK: - 3 · BIO — one line, and where you are

/// `card.note` (the bio the Person profile renders) and `card.city`. The city is
/// a TEXT FIELD, not a GPS grab: first boot must not spend the location permission
/// to fill a display string — the map's permission belongs to the map.
private struct BioStep: View {
    @EnvironmentObject private var core: CoreService
    var nav: BootNav

    @FocusState private var focused: Bool

    var body: some View {
        BootSlate(back: nav.back) {
            BootField("one line", text: core.cardBinding(\.note), focused: $focused)
                .accessibilityIdentifier("boot-word")
            BootField("city", text: core.cardBinding(\.city), focused: nil)
                .accessibilityIdentifier("boot-city")
        } actions: {
            BootWord("NEXT", action: nav.next).accessibilityIdentifier("boot-next")
        }
        .onAppear { focused = true }
    }
}

// MARK: - 4 · RECOVERY KEY — the words only you know

/// The 24 words that bring this identity back on a device that has lost
/// everything. Generated, never chosen; shown once; stored by us NOWHERE.
///
/// Twenty-four words, and the count is the safeguard: the three FINGERPRINT words
/// on the next screen are PUBLIC — derived from the public key, meant to be read
/// aloud during pairing — and these are secret and must never be spoken. Identical
/// shape would invite the worst possible mistake, so they do not share one.
private struct RecoveryKeyStep: View {
    @EnvironmentObject private var core: CoreService
    var nav: BootNav

    @State private var words: [String] = []

    var body: some View {
        BootSlate(back: nav.back) {
            Text("never say these")
                .font(.system(size: 12))
                .tracking(0.6)
                .foregroundStyle(Color(white: 0.42))

            // Three numbered columns of eight — a shape a person can copy onto
            // paper without losing their place, and one nothing else in the app
            // resembles, which is half the reason it cannot be confused with the
            // three fingerprint words.
            LazyVGrid(columns: Array(repeating: GridItem(.flexible(), alignment: .leading),
                                     count: 3), spacing: 9) {
                ForEach(Array(words.enumerated()), id: \.offset) { i, w in
                    Text("\(i + 1) \(w)")
                        .font(.system(size: 13, design: .monospaced))
                        .foregroundStyle(.white)
                        .lineLimit(1)
                        .minimumScaleFactor(0.7)
                }
            }
            .padding(.horizontal, 6)
            .accessibilityIdentifier("boot-key")

            // WHY THIS SCREEN IS NOT DECORATION, said only when it is the whole
            // truth. A passkey wraps the seed and is the convenient way back; when
            // this device could not make one — the ceremony failed, or the Arc
            // refused the wrap — these words are the ONLY way back, and the person
            // holding them deserves to know that while they are still on screen.
            if let fault = core.passkeyEnrolment.fault {
                Text("This device has no passkey, so these words are the only way "
                     + "back to this account. \(fault)")
                    .font(.system(size: 11))
                    .foregroundStyle(Color(red: 0.88, green: 0.62, blue: 0.36))
                    .multilineTextAlignment(.center)
                    .padding(.horizontal, 10)
                    .accessibilityIdentifier("boot-key-no-passkey")
            }
        } actions: {
            BootWord("WRITTEN DOWN", action: nav.next).accessibilityIdentifier("boot-next")
        }
        .onAppear {
            words = core.recoveryKey().split(separator: " ").map(String.init)
            Trace.shared.log("boot", "recovery key shown · \(words.count) words")
        }
    }
}

// MARK: - 5 · YOU ARE — the fingerprint, shown once

/// The only screen with a teaching job: it shows the user what their identity IS,
/// so the pairing screen's word comparison later is a recognition rather than a
/// puzzle. It costs one screen and it is honest — the words already exist in core
/// (`fingerprint_words`), and they are PUBLIC: these are the ones you say aloud.
private struct FingerprintStep: View {
    @EnvironmentObject private var core: CoreService
    var back: () -> Void
    var enter: () -> Void

    var body: some View {
        BootSlate(back: back) {
            Text("say these")
                .font(.system(size: 12))
                .tracking(0.6)
                .foregroundStyle(Color(white: 0.42))

            Text(core.sasWords.isEmpty ? "—" : core.sasWordsDotted)
                .font(.system(size: 20, weight: .semibold))
                .multilineTextAlignment(.center)
                .foregroundStyle(.white)
                .accessibilityIdentifier("boot-fingerprint")

            Text(core.spaceId)
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(Color(white: 0.42))
                .multilineTextAlignment(.center)
                .lineLimit(2)
                .truncationMode(.middle)
                .padding(.horizontal, 20)
                .accessibilityIdentifier("boot-identity-key")
        } actions: {
            BootWord("ENTER", action: enter).accessibilityIdentifier("boot-enter")
        }
    }
}

// MARK: - Slate

/// The one container every step wears: the step's content centred, its action
/// word beneath, and a back chevron where going back is meaningful. Nothing else
/// — no title bar, no progress rail, no captions.
///
/// There is no separate SKIP control because there is nothing to skip PAST: every
/// collecting step writes as you type, so its action word carries a filled field
/// and an empty one identically. The step is skipped by tapping NEXT.
private struct BootSlate<Content: View, Actions: View>: View {
    var back: (() -> Void)?
    @ViewBuilder var content: Content
    @ViewBuilder var actions: Actions

    init(back: (() -> Void)? = nil,
         @ViewBuilder content: () -> Content,
         @ViewBuilder actions: () -> Actions) {
        self.back = back
        self.content = content()
        self.actions = actions()
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                if let back {
                    Button(action: back) {
                        Image(systemName: "chevron.left")
                            .font(.system(size: 15, weight: .medium))
                            .foregroundStyle(Color(white: 0.42))
                    }
                    .buttonStyle(.plain)
                    .accessibilityIdentifier("boot-back")
                }
                Spacer()
            }
            .frame(height: 24)
            .padding(.horizontal, 22)

            Spacer()
            VStack(spacing: 26) { content }
            Spacer()
            actions
            Spacer().frame(height: 60)
        }
        .padding(.horizontal, 32)
    }
}

/// The wordmark, at the size the first two screens wear it.
private struct Wordmark: View {
    var body: some View {
        Text("PACIFIC")
            .font(.system(size: 26, weight: .semibold))
            .tracking(10)
            .foregroundStyle(.white)
    }
}

/// A slate field: the placeholder IS the label, and there is nothing else.
private struct BootField: View {
    var placeholder: String
    @Binding var text: String
    var focused: FocusState<Bool>.Binding?

    init(_ placeholder: String, text: Binding<String>, focused: FocusState<Bool>.Binding?) {
        self.placeholder = placeholder
        self._text = text
        self.focused = focused
    }

    var body: some View {
        let field = TextField(placeholder, text: $text)
            .font(.title3)
            .multilineTextAlignment(.center)
            .textFieldStyle(.plain)
            .foregroundStyle(.white)
            .tint(.gray)
            .autocorrectionDisabled()
            .padding(.bottom, 7)
            .overlay(alignment: .bottom) {
                Rectangle().fill(Color(white: 0.23)).frame(height: 1)
            }
        if let focused { field.focused(focused) } else { field }
    }
}

/// A slate action word — the pill the sketch draws. Disabled reads as grey; there
/// is no explanation of why, because the field above it is the explanation.
private struct BootWord: View {
    var label: String
    var enabled: Bool
    var action: () -> Void

    init(_ label: String, enabled: Bool = true, action: @escaping () -> Void) {
        self.label = label
        self.enabled = enabled
        self.action = action
    }

    var body: some View {
        Button(action: action) {
            Text(label)
                .font(.system(size: 11, design: .monospaced))
                .tracking(1.4)
                .foregroundStyle(enabled ? .white : Color(white: 0.35))
                .padding(.horizontal, 16).padding(.vertical, 8)
                .overlay(Capsule().strokeBorder(Color(white: enabled ? 0.32 : 0.18), lineWidth: 1))
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
    }
}

#Preview("door") { BootFlow().environmentObject(CoreService()) }
