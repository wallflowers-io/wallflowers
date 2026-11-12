//  MediaImage.swift
//  A detached picture (#112, O-79): the fold carries its reference, core fetches and opens the
//  bytes off the main thread (`Core.mediaGet`, which keeps them on disk), and this decodes
//  them once and keeps the image by digest. An object made before the media facet carries
//  its picture inline instead, and that still draws.

import SwiftUI
import UIKit
import PacificCore

@MainActor
final class MediaCache: ObservableObject {
    static let shared = MediaCache()

    /// Core's fetch. CoreService sets it: it holds the one Core.
    var fetch: ((MediaRefRow) throws -> Data)?
    private let images = NSCache<NSString, UIImage>()
    /// Digests asked for once. A fetch that failed is not asked again this launch: a draw
    /// that retried would retry on every frame while offline.
    private var asked = Set<String>()
    /// Bumped as a fetch lands, so a view that drew a miss draws again.
    @Published private(set) var landed = 0

    /// The picture for a slot: the detached ref where there is one, else the inline base64,
    /// else nil. A ref not yet fetched is nil now and draws once it lands.
    func picture(ref: MediaRefRow?, b64: String = "", id: String = "") -> UIImage? {
        guard let ref else { return BannerCache.shared.image(id: id, b64: b64) }
        let key = ref.digest as NSString
        if let hit = images.object(forKey: key) { return hit }
        guard let fetch, asked.insert(ref.digest).inserted else { return nil }
        Task {
            let got = await Task.detached(priority: .utility) { Result { try fetch(ref) } }.value
            switch got {
            case .success(let data):
                guard let ui = UIImage(data: data) else {
                    return Trace.shared.log("media", object: ref.digest, "not a picture (\(ref.mime))", ok: false)
                }
                images.setObject(ui, forKey: key)
                landed &+= 1
            case .failure(let e):
                Trace.shared.log("media", object: ref.digest, "fetch failed: \(e)", ok: false)
            }
        }
        return nil
    }
}

/// One picture slot, filled: the detached ref where the fold has one, else the inline base64
/// it replaced, else `placeholder`.
struct MediaImage<Placeholder: View>: View {
    let ref: MediaRefRow?
    var b64: String = ""
    var id: String = ""
    @ViewBuilder var placeholder: () -> Placeholder
    @ObservedObject private var media = MediaCache.shared

    var body: some View {
        if let ui = media.picture(ref: ref, b64: b64, id: id) {
            Image(uiImage: ui).resizable().scaledToFill()
        } else {
            placeholder()
        }
    }
}
