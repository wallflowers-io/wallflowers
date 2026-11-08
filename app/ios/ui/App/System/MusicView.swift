//  MusicView.swift
//  The Music System's surface: the scope (an RA area) and the listings it pulled.
//  Mirrors SystemsView's place in Settings ▸ SYSTEMS — but RA is public, so where
//  Sysiphus asks for a credential this asks only WHERE. Label-or-value rows,
//  monochrome; the flyer thumb is the one image. Embeddable (parent provides the
//  navigation stack).

import SwiftUI

struct MusicView: View {
    @EnvironmentObject private var music: MusicService

    @State private var query = ""
    @State private var results: [MusicArea] = []
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
        .navigationTitle("Music")
        .navigationBarTitleDisplayMode(.inline)
    }

    // MARK: the System header — scope + count + status, values only

    private var header: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 10) {
                Image(systemName: "music.note").foregroundStyle(Theme.textPrimary)
                Text(music.area?.name ?? "Resident Advisor")
                    .font(.headline).foregroundStyle(Theme.textPrimary)
                Spacer()
                Button {
                    Task { await music.refresh() }
                } label: {
                    if music.status == .refreshing {
                        ProgressView().tint(Theme.textSecondary)
                    } else {
                        Image(systemName: "arrow.clockwise").foregroundStyle(Theme.textSecondary)
                    }
                }
                .buttonStyle(.plain)
                .disabled(music.status == .refreshing)
                .accessibilityLabel("Refresh")
                .accessibilityIdentifier("music-refresh")
            }
            if case .loaded(let n) = music.status {
                Text("\(n) listings").font(.caption).foregroundStyle(.secondary)
            }
            if case .error(let e) = music.status {
                Label(e, systemImage: "exclamationmark.triangle")
                    .font(.caption).foregroundStyle(.secondary)
                    .accessibilityIdentifier("music-error")
            }
        }
        .padding()
        .background(Brand.card, in: RoundedRectangle(cornerRadius: 14))
    }

    // MARK: the scope picker — search RA's areas, tap one to re-scope

    private var scopePicker: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                TextField("Area", text: $query)
                    .textInputAutocapitalization(.never).autocorrectionDisabled()
                    .accessibilityIdentifier("music-area-search")
                    .onSubmit { search() }
            }
            .padding(12).background(Theme.fill, in: RoundedRectangle(cornerRadius: 10))

            if let searchError {
                Label(searchError, systemImage: "exclamationmark.triangle")
                    .font(.caption).foregroundStyle(.secondary)
            }
            ForEach(results) { a in
                Button {
                    results = []
                    query = ""
                    music.setScope(a)
                } label: {
                    HStack {
                        Text(a.name).foregroundStyle(Theme.textPrimary)
                        Spacer()
                        if a.id == music.area?.id {
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
            do { results = try await music.searchAreas(term) }
            catch {
                results = []
                searchError = (error as? LocalizedError)?.errorDescription ?? String(describing: error)
            }
        }
    }

    // MARK: the listings — title · venue · date, flyer thumb

    private var listings: some View {
        VStack(spacing: 0) {
            ForEach(music.events) { e in
                HStack(spacing: 12) {
                    thumb(e.flyerURL)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(e.title)
                            .font(.subheadline.weight(.semibold))
                            .foregroundStyle(Theme.textPrimary)
                            .lineLimit(2)
                        if let v = e.venueName {
                            Text(v).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                        }
                        Text(MusicEnrichmentBridge.day(e))
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    Spacer(minLength: 0)
                }
                .padding(.vertical, 10).padding(.horizontal, 12)
                .accessibilityIdentifier("music-listing")
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
