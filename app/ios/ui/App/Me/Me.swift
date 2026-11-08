//  Me.swift
//  Your profile pieces. The Settings tree (Settings.swift) pushes each of these as its own
//  submenu card — `AvatarEditor` (photo + inline block colour), `PaletteEditor`, `PrivacyPanel` —
//  and `MeView` composes the same components into one page (QR + connections alongside them),
//  so there is ONE implementation of each editor, not two.
//
//  EVERY editor here writes through `CoreService`, and every write is TRANSMITTED to
//  all your connections as a signed Delta (debounced, so a burst of typing is one
//  fan-out). Nothing on this screen is device-local any more: what you see is what
//  your connections will hold.

import SwiftUI
import PhotosUI
import PacificCore
#if canImport(UIKit)
import UIKit
#endif

struct MeView: View {
    @EnvironmentObject private var core: CoreService
    @State private var showQR = false
    private var connectionNames: [String] { core.connectionRows().map(\.displayName) }

    var body: some View {
        ScrollView {
            VStack(spacing: 14) {
                qrRow            // QR at the top of Profile
                header
                AvatarEditor()   // Photo · inline colour
                NameEditor()     // Display name
                BioEditor()      // Org · title · note
                ContactEditor()  // Emails · phones · links
                TagsEditor()
                PaletteEditor()
                RecoveryKeyPanel()

                ProfileSection(title: "Connections", systemImage: "person.2.fill",
                               hint: "\(connectionNames.count)") {
                    if connectionNames.isEmpty {
                        Text("No connections yet — pair with someone to start.")
                            .font(.subheadline).foregroundStyle(.secondary)
                    } else {
                        Text(connectionNames.joined(separator: ", "))
                            .font(.subheadline).foregroundStyle(.secondary)
                    }
                }

                PrivacyPanel()
            }
            .padding()
        }
        .background(Brand.groupBG)
        .navigationTitle("Profile")
        .navigationBarTitleDisplayMode(.inline)
        .sheet(isPresented: $showQR) { MyQRSheet() }
    }

    private var header: some View {
        VStack(spacing: 8) {
            // The real self avatar — the photo (if set) or the block-colour disc,
            // the same face the top-left account button shows.
            Avatar(initials: core.initials, tint: Brand.accent, size: 76,
                   photo: core.avatarPhoto, color: core.avatarColor)
            Text(core.displayName.isEmpty ? "You" : core.displayName).font(.title2.bold())
        }
        .frame(maxWidth: .infinity)
        .padding(.top, 4)
    }

    private var qrRow: some View {
        Button { showQR = true } label: {
            Label("My QR", systemImage: "qrcode").frame(maxWidth: .infinity)
        }
        .buttonStyle(.pixel).controlSize(.large)
        .foregroundStyle(Theme.onAccent)
    }
}

// MARK: - Profile editors (shared by MeView and the Settings tree)

/// Pick/replace the profile photo and choose the inline block colour. Writes straight through
/// to CoreService (persisted there), so the account button + any header update together.
struct AvatarEditor: View {
    @EnvironmentObject private var core: CoreService
    @State private var pickedPhoto: PhotosPickerItem?

    /// The block-colour swatches — `CoreService.avatarSwatches`, the one list, so
    /// the colour first boot offers and the colour this editor offers are the same
    /// set. The ColorPicker beside them covers anything else.
    private static var swatches: [UInt] { CoreService.avatarSwatches }

    var body: some View {
        ProfileSection(title: "Avatar", systemImage: "person.crop.circle") {
            VStack(alignment: .leading, spacing: 12) {
                HStack(spacing: 12) {
                    Avatar(initials: core.initials, tint: Brand.accent, size: 52,
                           photo: core.avatarPhoto, color: core.avatarColor)
                    VStack(alignment: .leading, spacing: 8) {
                        PhotosPicker(selection: $pickedPhoto, matching: .images) {
                            Label(core.avatarPhoto == nil ? "Choose photo" : "Replace photo",
                                  systemImage: "photo")
                        }
                        .font(.subheadline.weight(.semibold))
                        if core.avatarPhoto != nil {
                            Button(role: .destructive) { core.avatarPhoto = nil } label: {
                                Label("Remove photo", systemImage: "trash")
                            }
                            .font(.subheadline)
                        }
                    }
                    Spacer()
                }

                // Block colour — the fallback disc (the photo is optional). A row of
                // swatches for the common picks + a ColorPicker for anything else.
                Text("Block colour").font(.caption).foregroundStyle(.secondary)
                HStack(spacing: 10) {
                    ForEach(Self.swatches, id: \.self) { hex in
                        let swatch = Color(hex: hex)
                        Button { core.avatarColorHex = swatch.hexString } label: {
                            Circle().fill(swatch).frame(width: 28, height: 28)
                                .overlay(
                                    Circle().strokeBorder(
                                        .white.opacity(isSelected(swatch) ? 0.9 : 0), lineWidth: 2)
                                )
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel("Avatar colour")
                    }
                    ColorPicker("", selection: Binding(
                        get: { core.avatarColor },
                        set: { core.avatarColorHex = $0.hexString }
                    ), supportsOpacity: false)
                    .labelsHidden()
                    Spacer()
                }
            }
        }
        // Load + downscale the picked image off the pick, then persist via CoreService.
        // `.task(id:)` re-runs whenever the selection changes (and no-ops on nil).
        .task(id: pickedPhoto) {
            guard let item = pickedPhoto else { return }
            if let data = try? await item.loadTransferable(type: Data.self),
               let jpeg = downscaledAvatarJPEG(data) {
                core.avatarPhoto = jpeg
            }
        }
    }

    /// Whether `swatch` is the currently-chosen block colour (hex-string compare).
    private func isSelected(_ swatch: Color) -> Bool {
        core.avatarColorHex.caseInsensitiveCompare(swatch.hexString) == .orderedSame
    }
}

/// Your display name — the one field every peer sees first. Editable now: the name
/// is a profile field, not the identity (that is the key), so changing it costs
/// nothing but a Delta to everyone who holds you.
struct NameEditor: View {
    @EnvironmentObject private var core: CoreService
    @State private var draft = ""

    var body: some View {
        ProfileSection(title: "Name", systemImage: "person.text.rectangle") {
            TextField("Name", text: $draft)
                .textFieldStyle(.plain)
                .font(.body)
                .textInputAutocapitalization(.words)
                .autocorrectionDisabled()
                .accessibilityIdentifier("profile-name")
                .onChange(of: draft) { _, new in core.rename(to: new) }
        }
        // Seed from the core, and re-seed if it changes underneath us (first launch
        // resolves the name asynchronously). Guarded so it never clobbers typing.
        .onAppear { if draft.isEmpty { draft = core.displayName } }
        .onChange(of: core.displayName) { _, name in
            if draft.isEmpty, !name.isEmpty { draft = name }
        }
    }
}

/// Who you are in words: organisation, title, a short note, and where you are.
///
/// The CITY is a text field, never a GPS grab — the same rule the first-boot BIO
/// step follows, and the reason `ContactCard.city` is a display string rather
/// than a coordinate. This is also where that step is re-openable, which is what
/// lets first boot skip anything without losing it.
struct BioEditor: View {
    @EnvironmentObject private var core: CoreService

    var body: some View {
        ProfileSection(title: "Bio", systemImage: "text.alignleft") {
            VStack(alignment: .leading, spacing: 10) {
                TextField("Organisation", text: core.cardBinding(\.org))
                    .textInputAutocapitalization(.words)
                    .accessibilityIdentifier("profile-org")
                Divider()
                TextField("Title", text: core.cardBinding(\.title))
                    .textInputAutocapitalization(.words)
                    .accessibilityIdentifier("profile-title")
                Divider()
                TextField("Note", text: core.cardBinding(\.note), axis: .vertical)
                    .lineLimit(1...4)
                    .accessibilityIdentifier("profile-note")
                Divider()
                TextField("City", text: core.cardBinding(\.city))
                    .textInputAutocapitalization(.words)
                    .accessibilityIdentifier("profile-city")
            }
            .textFieldStyle(.plain)
            .autocorrectionDisabled()
        }
    }
}

/// THE RECOVERY KEY, re-readable. First boot shows it once; this is where it can
/// be read again — because a phrase you cannot look up is a phrase you will lose.
///
/// Hidden until asked for: it is the one secret in the app that restores an
/// entire identity, so it does not sit on a screen someone can glance at. Absent
/// entirely for an identity minted before seeds existed — the section says so
/// rather than showing a placeholder for a thing that does not exist.
///
/// Not to be confused with the three words on your QR: those are PUBLIC and meant
/// to be read aloud. These are 24 and must never be spoken.
struct RecoveryKeyPanel: View {
    @EnvironmentObject private var core: CoreService
    @State private var shown = false

    var body: some View {
        ProfileSection(title: "Recovery key", systemImage: "key.horizontal") {
            let words = core.recoveryKey()
            if words.isEmpty {
                Text("This identity predates recovery keys — it has none.")
                    .font(.subheadline).foregroundStyle(.secondary)
            } else if shown {
                VStack(alignment: .leading, spacing: 10) {
                    Text("Never say these. Anyone holding them is you.")
                        .font(.caption).foregroundStyle(.secondary)
                    Text(words)
                        .font(.footnote.monospaced())
                        .textSelection(.enabled)
                        .accessibilityIdentifier("profile-recovery-key")
                    Button("Hide") { shown = false }.font(.subheadline)
                }
            } else {
                Button("Show 24 words") { shown = true }
                    .font(.subheadline.weight(.semibold))
                    .accessibilityIdentifier("profile-recovery-reveal")
            }
        }
    }
}

/// Emails, phones and links — each a labelled value, the vCard shape the core
/// already speaks, so these export to a .vcf unchanged.
struct ContactEditor: View {
    @EnvironmentObject private var core: CoreService

    var body: some View {
        ProfileSection(title: "Contact", systemImage: "at") {
            VStack(alignment: .leading, spacing: 14) {
                LabeledListEditor(kind: "Email", keyboard: .emailAddress,
                                  items: core.cardBinding(\.emails))
                LabeledListEditor(kind: "Phone", keyboard: .phonePad,
                                  items: core.cardBinding(\.phones))
                LabeledListEditor(kind: "Link", keyboard: .URL,
                                  items: core.cardBinding(\.urls))
            }
        }
    }
}

/// One repeatable labelled-value list (emails / phones / links). Each row is
/// `label · value`; the trailing row adds another.
struct LabeledListEditor: View {
    var kind: String
    var keyboard: UIKeyboardType
    @Binding var items: [LabeledRow]

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(kind).font(.caption).foregroundStyle(.secondary)
            ForEach(items.indices, id: \.self) { i in
                HStack(spacing: 8) {
                    TextField("label", text: Binding(
                        get: { items[i].label },
                        set: { items[i].label = $0 }
                    ))
                    .frame(width: 68)
                    .foregroundStyle(.secondary)
                    TextField(kind, text: Binding(
                        get: { items[i].value },
                        set: { items[i].value = $0 }
                    ))
                    .keyboardType(keyboard)
                    .textInputAutocapitalization(.never)
                    Button(role: .destructive) { items.remove(at: i) } label: {
                        Image(systemName: "minus.circle.fill")
                    }
                    .buttonStyle(.plain)
                    .foregroundStyle(.secondary)
                }
                .textFieldStyle(.plain)
                .autocorrectionDisabled()
            }
            Button { items.append(LabeledRow(label: "", value: "")) } label: {
                Label("Add \(kind.lowercased())", systemImage: "plus.circle")
            }
            .font(.subheadline)
        }
    }
}

/// Free-form tags — how you'd be found in someone's directory.
struct TagsEditor: View {
    @EnvironmentObject private var core: CoreService
    @State private var entry = ""

    var body: some View {
        ProfileSection(title: "Tags", systemImage: "tag") {
            VStack(alignment: .leading, spacing: 10) {
                if !core.card.tags.isEmpty {
                    FlexibleWrap(items: core.card.tags) { tag in
                        Button {
                            core.editCard { $0.tags.removeAll { $0 == tag } }
                        } label: {
                            HStack(spacing: 4) {
                                Text(tag)
                                Image(systemName: "xmark")
                            }
                            .font(.caption)
                            .padding(.horizontal, 9).padding(.vertical, 5)
                            .background(Theme.surface2, in: Capsule())
                        }
                        .buttonStyle(.plain)
                    }
                }
                HStack {
                    TextField("Add tag", text: $entry)
                        .textFieldStyle(.plain)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .onSubmit(add)
                    Button(action: add) { Image(systemName: "plus.circle") }
                        .buttonStyle(.plain)
                        .disabled(entry.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }
        }
    }

    private func add() {
        let tag = entry.trimmingCharacters(in: .whitespaces)
        guard !tag.isEmpty, !core.card.tags.contains(tag) else { return }
        core.editCard { $0.tags.append(tag) }
        entry = ""
    }
}

/// The category-colour palette.
struct PaletteEditor: View {
    @ObservedObject private var palette = PaletteStore.shared

    var body: some View {
        ProfileSection(title: "Palette", systemImage: "paintpalette") {
            VStack(spacing: 6) {
                ForEach(Palette.all) { p in
                    Button { palette.selected = p } label: {
                        HStack(spacing: 10) {
                            HStack(spacing: 3) {
                                ForEach(Array(p.colors.enumerated()), id: \.offset) { _, c in
                                    Circle().fill(c).frame(width: 14, height: 14)
                                }
                            }
                            Text(p.name).font(.subheadline).foregroundStyle(Theme.textPrimary)
                            Spacer()
                            if p.id == palette.selected.id {
                                Image(systemName: "checkmark").foregroundStyle(Brand.accent)
                            }
                        }
                    }
                    .buttonStyle(.plain)
                }
            }
        }
    }
}

/// The calm on-device privacy statement.
struct PrivacyPanel: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Label("On your device", systemImage: "iphone")
            Label("End-to-end encrypted", systemImage: "lock.fill")
            Label("No server has your profile or your graph", systemImage: "network.slash")
        }
        .font(.caption)
        .foregroundStyle(.secondary)
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding()
        .background(Brand.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
    }
}

// MARK: - QR

struct MyQRSheet: View {
    @EnvironmentObject private var core: CoreService
    @Environment(\.dismiss) private var dismiss
    @State private var showKey = false

    /// Connections already on this device when the sheet opened. Anything that
    /// appears AFTER that, un-accepted, is someone who just scanned this code —
    /// which is the only thing this screen needs to notice.
    @State private var known: Set<String> = []
    /// The scan that just landed: (peer id, display name).
    @State private var inbound: (peer: String, name: String)? = nil
    @State private var accepted = false

    var body: some View {
        NavigationStack {
            VStack(spacing: 24) {
                Spacer()
                // Nothing but the QR and the three words. The QR is the REAL signed
                // contact bundle pair-scan consumes, so it genuinely forms the connection.
                let bundle = core.contactBundle()
                if bundle.isEmpty {
                    Label("Couldn’t create your connect code", systemImage: "exclamationmark.triangle")
                        .font(.subheadline).foregroundStyle(.orange).frame(height: 240)
                } else {
                    QRView(payload: bundle, size: 240)
                        .padding().background(.white, in: RoundedRectangle(cornerRadius: 16))
                }

                // The three safety words. Long-press to reveal the full identity key.
                Text(core.sasWords.isEmpty ? "—" : core.sasWordsDotted)
                    .font(.title3.weight(.semibold))
                    .multilineTextAlignment(.center)
                    .contentShape(Rectangle())
                    .onLongPressGesture { withAnimation { showKey.toggle() } }
                    .accessibilityHint("Long press to reveal your full identity key")
                if showKey {
                    Text(core.spaceId)
                        .font(.caption2.monospaced()).foregroundStyle(.secondary)
                        .textSelection(.enabled).multilineTextAlignment(.center)
                        .padding(.horizontal)
                }

                if let inbound { inboundCard(inbound) }

                Spacer()
            }
            .padding()
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .topBarTrailing) { Button("Close") { dismiss() } } }
            // THE SHEET HAS TO WATCH. Holding your code out is the one moment you are
            // certain to be looking at this screen, and a scan used to land silently:
            // the connection appeared in the log, the QR sat there unchanged, and the
            // scanned person had no idea anything had happened. The sync tick already
            // drains the intro; this just gives the moment somewhere to show up, where
            // the person is standing in front of you and consent means something.
            .onAppear { known = Set(core.connectionRows().map(\.peerId)) }
            .onReceive(core.$syncTick) { _ in noticeInbound() }
        }
    }

    /// The scan, answered where it happened. Name + Accept, nothing else.
    @ViewBuilder
    private func inboundCard(_ hit: (peer: String, name: String)) -> some View {
        HStack(spacing: 12) {
            Image(systemName: accepted ? "checkmark.circle.fill" : "person.crop.circle.badge.plus")
                .foregroundStyle(accepted ? Color.green : Brand.accent)
            Text(accepted ? hit.name : "\(hit.name) wants to connect")
                .font(.subheadline)
            Spacer()
            if !accepted {
                Button("Accept") {
                    core.pairAccept(hit.peer)
                    accepted = true
                    UINotificationFeedbackGenerator().notificationOccurred(.success)
                }
                .buttonStyle(.pixel)
                .controlSize(.small)
                .foregroundStyle(Theme.onAccent)
            }
        }
        .padding(.horizontal, 14).padding(.vertical, 10)
        .background(.bar, in: RoundedRectangle(cornerRadius: 12))
        .transition(.move(edge: .bottom).combined(with: .opacity))
    }

    /// A connection that was not here when the sheet opened, and has not been
    /// accepted, is a scan that just landed. First one wins — the person in front
    /// of you is the one you are answering.
    private func noticeInbound() {
        guard inbound == nil else { return }
        guard let hit = core.connectionRows().first(where: { !known.contains($0.peerId) && !$0.connected })
        else { return }
        let name = hit.displayName.isEmpty ? String(hit.peerId.prefix(8)) + "…" : hit.displayName
        withAnimation { inbound = (peer: hit.peerId, name: name) }
        UINotificationFeedbackGenerator().notificationOccurred(.warning)
    }
}

/// Downscale picked image bytes to the avatar's JPEG — keeps the profile photo
/// small (it rides the card into every peer's fold, under the 500 KB cap) while
/// staying crisp on the disc. THE one avatar downscale: the Settings editor and
/// the first-boot FACE step both come through here, so a photo picked at boot and
/// a photo picked later are the same bytes.
func downscaledAvatarJPEG(_ data: Data) -> Data? { downscaledJPEG(data, maxDimension: 512) }

/// Downscale picked image bytes to a modest JPEG. The longest edge is clamped to
/// `maxDimension`; already-smaller images pass through.
private func downscaledJPEG(_ data: Data, maxDimension: CGFloat) -> Data? {
    #if canImport(UIKit)
    guard let image = UIImage(data: data) else { return nil }
    let longest = max(image.size.width, image.size.height)
    let scale = longest > maxDimension ? maxDimension / longest : 1
    let target = CGSize(width: image.size.width * scale, height: image.size.height * scale)
    let format = UIGraphicsImageRendererFormat.default()
    format.scale = 1
    let resized = UIGraphicsImageRenderer(size: target, format: format).image { _ in
        image.draw(in: CGRect(origin: .zero, size: target))
    }
    return resized.jpegData(compressionQuality: 0.85)
    #else
    return data
    #endif
}
