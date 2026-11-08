//  SystemsView.swift
//  The System surface: sign in with your Sysiphus credential (each employee their
//  own → their own group-of-1 System), see the Fields it pulled, and prove the
//  core requirement — resolve a design "in human" back to its Sysiphus entry.
//  Embeddable (parent provides the navigation stack).

import SwiftUI

struct SystemsView: View {
    @EnvironmentObject private var system: SystemService
    @EnvironmentObject private var resolution: ResolutionService

    var body: some View {
        Group {
            if system.isSignedIn { connected } else { signIn }
        }
        .background(Theme.bg)
        .navigationTitle("Systems")
        .navigationBarTitleDisplayMode(.inline)
    }

    // MARK: signed out — the credential form

    @State private var username = ""
    @State private var password = ""

    private var signIn: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Label("Sysiphus", systemImage: "cylinder.split.1x2.fill")
                    .font(.title3.bold()).foregroundStyle(Theme.textPrimary)
                Text("Aries' single source of truth — Shopify, BrightPearl and Warewolf as one. Sign in with your own login; your System is scoped to that credential.")
                    .font(.subheadline).foregroundStyle(.secondary)

                VStack(spacing: 10) {
                    TextField("Username", text: $username)
                        .textInputAutocapitalization(.never).autocorrectionDisabled()
                        .padding(12).background(Theme.fill, in: RoundedRectangle(cornerRadius: 10))
                        .accessibilityIdentifier("sysiphus-username")
                    SecureField("Password", text: $password)
                        .padding(12).background(Theme.fill, in: RoundedRectangle(cornerRadius: 10))
                        .accessibilityIdentifier("sysiphus-password")
                    Button {
                        Task { await system.signIn(username: username, password: password) }
                    } label: {
                        HStack {
                            if system.status == .connecting { ProgressView().tint(Theme.onAccent) }
                            Text(system.status == .connecting ? "Connecting…" : "Connect System")
                        }
                        .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.pixel)
                    .foregroundStyle(Theme.onAccent)
                    .disabled(username.isEmpty || system.status == .connecting)
                    .accessibilityIdentifier("sysiphus-connect")
                }

                if let e = system.lastError {
                    Label(e, systemImage: "exclamationmark.triangle").font(.caption).foregroundStyle(.orange)
                }
            }
            .padding()
        }
    }

    // MARK: signed in — the System + resolution playground

    private var connected: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                systemHeader
                resolvePlayground
                Button(role: .destructive) { system.signOut() } label: {
                    Label("Sign out of Sysiphus", systemImage: "rectangle.portrait.and.arrow.right")
                }
                .padding(.top, 4)
            }
            .padding()
        }
    }

    private var systemHeader: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 10) {
                Image(systemName: "cylinder.split.1x2.fill").foregroundStyle(Brand.accent)
                VStack(alignment: .leading, spacing: 1) {
                    Text(system.employee?.name ?? "Sysiphus").font(.headline)
                    Text("\(system.employee?.role ?? "") · System scoped to this credential (group-of-1)")
                        .font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
            }
            HStack(spacing: 14) {
                stat("\(fieldCount)", "Fields")
                stat("\(resolution.indexedCount)", "indexed")
            }
        }
        .padding()
        .background(Brand.card, in: RoundedRectangle(cornerRadius: 14))
    }

    private var fieldCount: Int {
        if case .connected(let n) = system.status { return n }
        return system.fields.count
    }

    private func stat(_ value: String, _ label: String) -> some View {
        VStack(alignment: .leading, spacing: 1) {
            Text(value).font(.subheadline.weight(.semibold)).foregroundStyle(Theme.textPrimary)
            Text(label).font(.caption2).foregroundStyle(.secondary)
        }
    }

    @State private var query = ""

    private var resolvePlayground: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Resolve a design — in human").font(.subheadline.weight(.semibold))
            Text("No codes. Try “ram cargos”, “the ripstop trouser”, “oversized hoodie”, “the fleece”.")
                .font(.caption).foregroundStyle(.secondary)

            HStack {
                Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                TextField("say it however you'd say it…", text: $query)
                    .textInputAutocapitalization(.never).autocorrectionDisabled()
            }
            .padding(12).background(Theme.fill, in: RoundedRectangle(cornerRadius: 10))

            let examples = ["ram cargos", "the ripstop trouser", "oversized hoodie", "the fleece", "sigil", "astral jacket"]
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(examples, id: \.self) { ex in
                        Button { query = ex } label: {
                            Text(ex).font(.caption).padding(.horizontal, 10).padding(.vertical, 6)
                                .background(Theme.surface2, in: Capsule())
                                .foregroundStyle(Theme.textSecondary)
                        }
                        .buttonStyle(.plain)
                    }
                }
            }

            let results = resolution.resolve(query, k: 3)
            if query.isEmpty {
                Text("Type above — matches resolve to the Sysiphus entry.").font(.caption).foregroundStyle(.tertiary)
            } else if results.isEmpty {
                Label("No design resolved.", systemImage: "questionmark.circle").font(.caption).foregroundStyle(.orange)
            } else {
                ForEach(Array(results.enumerated()), id: \.offset) { i, r in
                    LiveDesignCard(cached: r.field, primary: i == 0, score: r.score)
                }
            }
        }
        .padding()
        .background(Brand.card, in: RoundedRectangle(cornerRadius: 14))
    }
}

/// The Sysiphus entry for a resolved design — the content that will also back the
/// universal contextual panel. Shows the unified record across Shopify / BrightPearl
/// / Warewolf.
struct DesignEntryCard: View {
    let field: SysiphusField
    var primary: Bool = true
    var score: Float? = nil
    var isLive: Bool = false   // freshened from source (WS-D lookup live tier)

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline) {
                Text(field.name).font(primary ? .headline : .subheadline.weight(.semibold))
                if isLive {
                    Image(systemName: "dot.radiowaves.left.and.right")
                        .font(.caption2).foregroundStyle(.green)
                }
                Spacer()
                if let s = field.season { Text(s).font(.caption).foregroundStyle(.secondary) }
            }
            let subtitle = [field.collection, field.category].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · ")
            if !subtitle.isEmpty {
                Text(subtitle).font(.caption).foregroundStyle(.secondary)
            }

            HStack(spacing: 14) {
                if let p = field.wholesalePrice { kv("£\(Int(p))", "wholesale") }
                kv("\(field.stock.available)/\(field.stock.onHand)", "avail / on-hand")
                kv("\(field.colourways.count)×\(field.sizes.count)", "colour × size")
            }
            .font(.caption)

            if !field.openOrders.isEmpty {
                let orders = Dictionary(grouping: field.openOrders, by: \.order)
                WrapLayout(spacing: 6) {
                    ForEach(orders.keys.sorted(), id: \.self) { oid in
                        let first = orders[oid]!.first!
                        Text("\(first.stockist) · \(first.status)")
                            .font(.caption2).padding(.horizontal, 8).padding(.vertical, 4)
                            .background(Theme.surface2, in: Capsule()).foregroundStyle(.secondary)
                    }
                }
            }

            Label("Sysiphus · \(field.systems["shopify"] ?? "-") · \(field.systems["brightpearl"] ?? "-") · \(field.systems["warewolf"] ?? "-")",
                  systemImage: "cylinder.split.1x2.fill")
                .font(.system(.caption2, design: .monospaced)).foregroundStyle(.tertiary)
                .lineLimit(1).truncationMode(.middle)
        }
        .padding(12)
        .background(Theme.surface2, in: RoundedRectangle(cornerRadius: 12))
        .overlay(primary ? RoundedRectangle(cornerRadius: 12).strokeBorder(Brand.accent.opacity(0.4), lineWidth: 1) : nil)
    }

    private func kv(_ v: String, _ l: String) -> some View {
        VStack(alignment: .leading, spacing: 1) {
            Text(v).fontWeight(.semibold).foregroundStyle(Theme.textPrimary)
            Text(l).font(.caption2).foregroundStyle(.secondary)
        }
    }
}
