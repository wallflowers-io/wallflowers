//
//  PosterImage.swift
//  Posters, flyers and thumbs that never pop.
//
//  AsyncImage restarts from `.empty` every time a lazy list recycles a row, so a
//  poster the user has already seen "pops" back in on every pass. This store is
//  the fix: a plain dictionary — deliberately NOT an NSCache — held for the app's
//  lifetime, so an image that has rendered once renders SYNCHRONOUSLY forever
//  after. The URLSession beneath keeps a fat disk cache, so even a fresh launch
//  mostly serves from disk. `prefetch` lets a feed warm its whole page of posters
//  the moment its rows exist, before any card scrolls into view.
//

import SwiftUI

@MainActor
final class PosterStore: ObservableObject {
    static let shared = PosterStore()

    /// Never evicted by design: a poster that has rendered must never pop again.
    private var images: [URL: UIImage] = [:]
    private var inflight = Set<URL>()
    private let session: URLSession

    private init() {
        let cfg = URLSessionConfiguration.default
        cfg.requestCachePolicy = .returnCacheDataElseLoad
        cfg.urlCache = URLCache(memoryCapacity: 32 << 20, diskCapacity: 256 << 20)
        session = URLSession(configuration: cfg)
    }

    func image(for url: URL) -> UIImage? { images[url] }

    /// Idempotent; a failed fetch clears its in-flight mark so the next appearance
    /// retries.
    func fetch(_ url: URL) {
        guard images[url] == nil, !inflight.contains(url) else { return }
        inflight.insert(url)
        Task {
            defer { inflight.remove(url) }
            guard let (data, _) = try? await session.data(from: url) else { return }
            // Decode off the main actor — a page of posters decoding on main is a hitch.
            guard let img = await Task.detached(operation: { UIImage(data: data) }).value else { return }
            objectWillChange.send()
            images[url] = img
        }
    }

    /// Warm a whole feed the moment its rows exist, before anything scrolls.
    func prefetch(_ urls: [URL?]) {
        urls.compactMap { $0 }.forEach(fetch)
    }
}

/// Drop-in for the poster/thumb `AsyncImage` sites: renders synchronously from the
/// store when the image is known (no pop), shows the placeholder and fetches when
/// it is not. `imageID` is stamped only on the LOADED image, so an existence
/// assertion on it still means "the picture is actually up".
struct PosterImage<Placeholder: View>: View {
    let url: URL
    var imageID: String? = nil
    @ViewBuilder var placeholder: () -> Placeholder
    @ObservedObject private var store = PosterStore.shared

    var body: some View {
        if let img = store.image(for: url) {
            Image(uiImage: img)
                .resizable()
                .scaledToFill()
                .accessibilityIdentifier(imageID ?? "")
        } else {
            placeholder()
                .onAppear { store.fetch(url) }
        }
    }
}
