//  InviteSheet.swift
//  Piggyback iOS Contacts to INVITE peers to Pacific.
//
//  Pacific has no central directory — no phone-number → account lookup — so we
//  can't show "which of your contacts are on Pacific" the way Signal/WhatsApp do
//  (that needs a server). What we CAN do is use your address book to INVITE:
//  pick who you're inviting (CNContactPickerViewController is user-confirmed, so
//  it needs NO contacts permission and runs on the Simulator), personalise the
//  message, and share an invite carrying a FRESH single-use pairing bundle. When
//  they open the link in Pacific it runs the exact same `pair_scan` handshake as
//  the QR — the connection forms once they accept. No servers, no phone numbers.

import SwiftUI
import Contacts
import ContactsUI
import UIKit

struct InviteSheet: View {
    @EnvironmentObject private var core: CoreService
    @Environment(\.dismiss) private var dismiss

    @State private var invitedName: String? = nil
    /// One FRESH bundle per invite: contactBundle() mints a new single-use key
    /// package each call, so each person you invite gets their own.
    @State private var bundle = ""
    @State private var pickContact = false
    @State private var share: ShareText? = nil

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    Text("Invite someone to connect with you on WallFlowers. They get a link to install the app and pair with you — no phone-number lookup, no servers.")
                        .font(.subheadline).foregroundStyle(.secondary)

                    Button { pickContact = true } label: {
                        Label(invitedName.map { "Inviting \($0)" } ?? "Choose from Contacts",
                              systemImage: "person.crop.circle.badge.plus")
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.vertical, 4)
                    }
                    .buttonStyle(.pixelSecondary)

                    VStack(alignment: .leading, spacing: 6) {
                        Text("Invite preview").font(.caption).foregroundStyle(.secondary)
                        Text(inviteBody(name: invitedName, bundle: bundle))
                            .font(.footnote)
                            .textSelection(.enabled)
                            .padding(12)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(Theme.surface2, in: RoundedRectangle(cornerRadius: 12))
                    }

                    Button {
                        share = ShareText(text: inviteBody(name: invitedName, bundle: bundle))
                    } label: {
                        Label("Share invite", systemImage: "square.and.arrow.up")
                            .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.pixel)
                    .foregroundStyle(Theme.onAccent)
                    .disabled(bundle.isEmpty)

                    TrustLine(text: "The invite carries a one-time pairing code · your connection forms when they accept")
                }
                .padding()
            }
            .background(Theme.bg)
            .navigationTitle("Invite to WallFlowers")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .topBarTrailing) { Button("Done") { dismiss() } } }
            .onAppear { if bundle.isEmpty { bundle = core.contactBundle() } }
            .background(ContactPicker(isPresented: $pickContact) { invitedName = $0 })
            .sheet(item: $share) { s in ShareSheet(items: [s.text]) }
        }
    }
}

struct ShareText: Identifiable { let id = UUID(); let text: String }

/// The invite message. Carries a FRESH single-use pairing bundle as a
/// `pacific://pair-scan` deep link (the same handshake the QR runs) plus an
/// install link. wallflowers.io is a placeholder until the universal-link + App
/// Store listing land; the pacific:// link already works app-to-app.
func inviteBody(name: String?, bundle: String) -> String {
    let hi = name.map { "Hi \($0)! " } ?? ""
    let code = bundle.addingPercentEncoding(withAllowedCharacters: .alphanumerics) ?? bundle
    return """
    \(hi)I'm on WallFlowers — private, on-device messaging with no servers and no phone numbers.

    Get it: https://wallflowers.io

    Then tap to connect with me:
    pacific://pair-scan?bundle=\(code)
    """
}

// MARK: - UIKit bridges

/// The system contact picker, presented from an invisible host so its built-in
/// self-dismissal and our SwiftUI state stay in sync (no double-dismiss). It is
/// user-confirmed, so NO contacts permission is requested — works on the Simulator.
struct ContactPicker: UIViewControllerRepresentable {
    @Binding var isPresented: Bool
    var onPick: (String) -> Void

    func makeUIViewController(context: Context) -> UIViewController { UIViewController() }

    func updateUIViewController(_ host: UIViewController, context: Context) {
        if isPresented, host.presentedViewController == nil {
            let picker = CNContactPickerViewController()
            picker.delegate = context.coordinator
            host.present(picker, animated: true)
        }
    }

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    final class Coordinator: NSObject, CNContactPickerDelegate {
        let parent: ContactPicker
        init(_ p: ContactPicker) { parent = p }

        func contactPicker(_ picker: CNContactPickerViewController, didSelect contact: CNContact) {
            let name = CNContactFormatter.string(from: contact, style: .fullName)
                ?? [contact.givenName, contact.familyName].joined(separator: " ")
                    .trimmingCharacters(in: .whitespaces)
            parent.onPick(name.isEmpty ? "your contact" : name)
            parent.isPresented = false
        }
        func contactPickerDidCancel(_ picker: CNContactPickerViewController) {
            parent.isPresented = false
        }
    }
}

/// The system share sheet (UIActivityViewController).
///
/// `onFinish` reports what the user actually did: `true` only when a share really
/// happened, `false` when the sheet was dismissed without one. Callers that act on the
/// share (the note compose card leaves upward once its text is away) need to tell those
/// two apart — dismissing on mere presentation would throw the note away on a cancel.
struct ShareSheet: UIViewControllerRepresentable {
    let items: [Any]
    var onFinish: ((Bool) -> Void)? = nil

    func makeUIViewController(context: Context) -> UIActivityViewController {
        let vc = UIActivityViewController(activityItems: items, applicationActivities: nil)
        vc.completionWithItemsHandler = { _, completed, _, _ in onFinish?(completed) }
        return vc
    }
    func updateUIViewController(_ vc: UIActivityViewController, context: Context) {}
}
