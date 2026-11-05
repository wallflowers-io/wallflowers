//  ArtView.swift
//  The Art System's surface: the scope (a geocoded place) and the listings it pulled.
//  Mirrors MusicView's place in Settings — ArtRabbit is public, so this too asks only
//  WHERE. Label-or-value rows, monochrome; the listing image is the one image.
//  Embeddable (parent provides the navigation stack).

import SwiftUI

struct ArtView: View {
    @EnvironmentObject private var art: ArtService

    @State private var query = ""
    @State private var results: [ArtPlace] = []
    @State private var searchError: String?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                header
                scopePicker
                listings
            }
            .padding()
        }
        .background(Theme.bg)
        .navigationTitle("Art")
        .navigationBarTitleDisplayMode(.inline)
    }

    // MARK: the System header — scope + count + status, values only

    private var header: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 10) {
                Image(systemName: "paintpalette").foregroundStyle(Theme.textPrimary)
                Text(art.place?.name ?? "ArtRabbit")
                    .font(.headline).foregroundStyle(Theme.textPrimary)
                Spacer()
                Button {
                    Task { await art.refresh() }
                } label: {
                    if art.status == .refreshing {
                        ProgressView().tint(Theme.textSecondary)
                    } else {
                        Image(systemName: "arrow.clockwise").foregroundStyle(Theme.textSecondary)
                    }
                }
                .buttonStyle(.plain)
                .disabled(art.status == .refreshing)
                .accessibilityLabel("Refresh")
                .accessibilityIdentifier("art-refresh")
            }
            if case .loaded(let n) = art.status {
                Text("\(n) listings").font(.caption).foregroundStyle(.secondary)
            }
            if case .error(let e) = art.status {
                Label(e, systemImage: "exclamationmark.triangle")
                    .font(.caption).foregroundStyle(.secondary)
                    .accessibilityIdentifier("art-error")
            }
        }
        .padding()
        .background(Brand.card, in: RoundedRectangle(cornerRadius: 14))
    }

    // MARK: the scope picker — geocode a typed place, tap a candidate to re-scope

    private var scopePicker: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                TextField("Place", text: $query)
                    .textInputAutocapitalization(.never).autocorrectionDisabled()
                    .accessibilityIdentifier("art-place-search")
                    .onSubmit { search() }
            }
            .padding(12).background(Theme.fill, in: RoundedRectangle(cornerRadius: 10))

            if let searchError {
                Label(searchError, systemImage: "exclamationmark.triangle")
                    .font(.caption).foregroundStyle(.secondary)
            }
            ForEach(results) { p in
                Button {
                    results = []
                    query = ""
                    art.setScope(p)
                } label: {
                    HStack {
                        Text(p.name).foregroundStyle(Theme.textPrimary)
                        Spacer()
                        if p.id == art.place?.id {
                            Image(systemName: "checkmark").foregroundStyle(Theme.textSecondary)
                        }
                    }
                    .padding(.vertical, 10).padding(.horizontal, 12)
                    .background(Theme.surface2, in: RoundedRectangle(cornerRadius: 10))
                }
                .buttonStyle(.plain)
            }
        }
        .padding()
        .background(Brand.card, in: RoundedRectangle(cornerRadius: 14))
    }

    private func search() {
        let term = query.trimmingCharacters(in: .whitespaces)
        guard !term.isEmpty else { return }
        searchError = nil
        Task {
            do { results = try await art.searchPlaces(term) }
            catch {
                results = []
                searchError = (error as? LocalizedError)?.errorDescription ?? String(describing: error)
            }
        }
    }

    // MARK: the listings — title · venue · run, image thumb

    private var listings: some View {
        VStack(spacing: 0) {
            ForEach(art.events) { e in
                HStack(spacing: 12) {
                    thumb(e.imageURL)
                    VStack(alignment: .leading, spacing: 2) {
                        // The title wears its OWN identifier: the row's flattening is
                        // SwiftUI's to change (and it has), so tests read the value here.
                        Text(e.title)
                            .font(.subheadline.weight(.semibold))
                            .foregroundStyle(Theme.textPrimary)
                            .lineLimit(2)
                            .accessibilityIdentifier(
                                e.venueName == nil ? "art-listing-title" : "art-listing-title-venued")
                        if let v = e.venueName {
                            Text(v).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                        }
                        Text(ArtEnrichmentBridge.run(e))
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    Spacer(minLength: 0)
                }
                .padding(.vertical, 10).padding(.horizontal, 12)
                .overlay(alignment: .bottom) {
                    Rectangle().fill(Theme.hairline.opacity(0.5)).frame(height: 0.5)
                }
            }
        }
        .background(Brand.card, in: RoundedRectangle(cornerRadius: 14))
    }

    @ViewBuilder private func thumb(_ url: URL?) -> some View {
        Group {
            if let url {
                PosterImage(url: url) { Theme.surface2 }
            } else {
                Theme.surface2
            }
        }
        .frame(width: 52, height: 52)
        .clipShape(RoundedRectangle(cornerRadius: 8))
    }
}
