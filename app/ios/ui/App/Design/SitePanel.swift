//  SitePanel.swift
//  GOLD ^ on anything with a public page: whether it is PUBLISHED and at what address,
//  and its LINK and QR code. What wallflowers.io's sign-up does, on the same core calls
//  (`Node::site_*`) and the same face specification; a new Site starts with the
//  schema's opening face. Editing the face is #70.
//
//  WHICH KINDS. Any kind the ICD makes of a `host` part (`kindsWithAPage()`), so a
//  Space today; another kind gets this panel when the ICD gives it a page (#62), with
//  no change here.

import SwiftUI
import PacificCore

/// Whether an address can be had, as the Arc's `/auth/site/check` answers.
enum AddressCheck: Equatable {
    case unchecked, checking, free
    case refused(String)
}

/// A Site's page while it is being made or published: the face, the address.
/// What has already been written is kept, so a retry writes only what is left.
@MainActor
final class SiteDraft: ObservableObject {
    @Published var face: JSON
    @Published var slug: String
    @Published var publish: Bool
    @Published var address: AddressCheck = .unchecked
    /// The address it is published at, once it is.
    @Published var published: String?

    /// Made so far, for a retry.
    var site: String?
    var host: String?
    var faceWritten = false
    var claimed: String?

    private let opening: (JSON, String, Bool)

    init(face: JSON = FaceSpec.shared.opening, slug: String = "", published: String? = nil) {
        self.face = face
        self.slug = slug
        self.publish = published != nil
        self.published = published
        opening = (face, slug, published != nil)
    }

    var edited: Bool { (face, slug, publish) != opening }

    static func kindHasAPage(_ kind: String) -> Bool { kindsWithAPage().contains(kind) }

    /// `wallflowers.io/<slug>`, as the schema says pages are served.
    static var root: String { FaceSpec.shared.schema["x-wf-address"].string ?? "https://wallflowers.io/" }
    func link(_ slug: String) -> String { Self.root + slug }
}

/// The rows under the audience dial on the card's gold face.
struct SitePanel: View {
    @EnvironmentObject private var core: CoreService
    @ObservedObject var site: SiteDraft
    var tint: Color
    /// Where the panel is its own act (a Space's Manage → Page), Publish is a button that
    /// does it; on a card it is a switch, and BLUE does it.
    var publishNow: (() async -> Void)? = nil

    @State private var copied = false
    @State private var publishing = false
    /// The Arc holds no account for this key, so no address can be claimed: said, and
    /// Publish off, before BLUE rather than at it.
    @State private var noAccount = false
    @State private var checking: Task<Void, Never>?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Divider().padding(.vertical, 8)
            publishRow
            linkRow
        }
        .onChange(of: site.slug) { _, s in check(s) }
        .task { noAccount = await !core.arcHoldsAccount() }
    }

    // MARK: Publish

    private var publishRow: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 14) {
                Image(systemName: "globe").frame(width: 20)
                Text(SiteDraft.root.replacingOccurrences(of: "https://", with: ""))
                    .font(.system(.footnote, design: .monospaced))
                    .foregroundStyle(Theme.textTertiary)
                TextField("address", text: $site.slug)
                    .font(.system(.body, design: .monospaced))
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .accessibilityIdentifier("site-slug")
                if let publishNow {
                    if site.published != site.slug {
                        Button(publishing ? "Publishing…" : "Publish") {
                            site.publish = true
                            publishing = true
                            Task { await publishNow(); publishing = false }
                        }
                        .buttonStyle(.borderedProminent)
                        .tint(tint)
                        .disabled(publishing || !canPublish)
                        .accessibilityIdentifier("site-publish")
                    }
                } else {
                    Toggle("Publish", isOn: $site.publish)
                        .labelsHidden()
                        .tint(tint)
                        .disabled(!canPublish && !site.publish)
                        .accessibilityIdentifier("site-publish")
                }
            }
            .padding(.horizontal, 20)
            .frame(minHeight: CircleButton.discSize)
            if noAccount {
                Text("Publishing needs your account kept at the Arc, which a passkey does. This device has no passkey.")
                    .font(.footnote).foregroundStyle(Theme.textSecondary).padding(.horizontal, 20)
                    .accessibilityIdentifier("site-no-account")
            } else if case .refused(let why) = site.address {
                Text(why).font(.footnote).foregroundStyle(Theme.danger).padding(.horizontal, 20)
                    .accessibilityIdentifier("site-slug-why")
            }
        }
    }

    private var canPublish: Bool { !noAccount && (site.address == .free || site.slug == site.claimed) }

    private func check(_ slug: String) {
        checking?.cancel()
        guard !slug.isEmpty, slug != site.claimed else {
            site.address = slug.isEmpty ? .unchecked : .free
            return
        }
        site.address = .checking
        checking = Task {
            try? await Task.sleep(nanoseconds: 350_000_000)
            guard !Task.isCancelled else { return }
            let answer = await core.checkAddress(slug)
            guard !Task.isCancelled, site.slug == slug else { return }
            site.address = answer
            if answer != .free { site.publish = false }
        }
    }

    // MARK: Link

    @ViewBuilder private var linkRow: some View {
        if let slug = site.published {
            let url = site.link(slug)
            VStack(alignment: .leading, spacing: 12) {
                HStack(spacing: 14) {
                    Image(systemName: "link").frame(width: 20)
                    Text(url.replacingOccurrences(of: "https://", with: ""))
                        .font(.system(.body, design: .monospaced))
                        .accessibilityIdentifier("site-link")
                }
                HStack(alignment: .center, spacing: 16) {
                    QRView(payload: url, size: 120)
                        .padding(8)
                        .background(Color.white, in: RoundedRectangle(cornerRadius: 10))
                        .accessibilityIdentifier("site-qr")
                    VStack(alignment: .leading, spacing: 10) {
                        Button(copied ? "Copied" : "Copy link") {
                            UIPasteboard.general.string = url
                            copied = true
                        }
                        .accessibilityIdentifier("site-copy")
                        ShareLink("Share", item: URL(string: url)!)
                            .accessibilityIdentifier("site-share")
                        if let qr = QRView.image(url) {
                            ShareLink("Save QR code", item: Image(uiImage: qr),
                                      preview: SharePreview(slug, image: Image(uiImage: qr)))
                                .accessibilityIdentifier("site-save-qr")
                        }
                    }
                    .font(.callout)
                }
            }
            .padding(.horizontal, 20)
            .padding(.vertical, 8)
        }
    }
}

/// A Space's page from its Manage sheet: the same rows, Publish their one act.
struct SitePageView: View {
    @EnvironmentObject private var core: CoreService
    let objectId: String

    @State private var site: SiteDraft?

    var body: some View {
        VStack(spacing: 0) {
            if let site {
                SitePanel(site: site, tint: Theme.accent) { _ = await core.finishSite(objectId, site) }
                Spacer(minLength: 0)
            } else {
                ProgressView().frame(maxHeight: .infinity)
            }
        }
        .background(Theme.bg.ignoresSafeArea())
        .navigationTitle("Page")
        .navigationBarTitleDisplayMode(.inline)
        .task { site = await core.siteDraft(objectId) }
    }
}
