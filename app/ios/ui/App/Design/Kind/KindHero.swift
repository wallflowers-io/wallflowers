//  KindHero.swift
//  The hero: an object's pictures and clip, paged in the square the card showed.
//
//  It PAGES, it does not scroll: one square, one picture at a time, the clip last. A tap
//  opens them full screen. In Edit, the square carries its own writes: the cover
//  replaced, a picture added or removed, the clip set, each offered only where the
//  kind's binder says it takes it (`KindHeadline.adds`).

import SwiftUI
import PhotosUI
import CoreImage

struct KindHero: View {
    let headline: KindHeadline?
    let tint: Color
    let width: CGFloat
    let editing: Bool
    let busy: Bool
    var cover: (Data) async -> Void
    var add: (Data) async -> Void
    var remove: (String) async -> Void
    var clip: (Data, String) async -> Void

    @State private var page = 0
    @State private var full = false
    @State private var pickedCover: PhotosPickerItem?
    @State private var pickedPicture: PhotosPickerItem?
    @State private var pickedClip: PhotosPickerItem?

    private var pages: [KindHeroPage] { KindHeroPage.of(headline) }
    private var adds: [KindAdd] { headline?.adds ?? [] }

    var body: some View {
        ZStack(alignment: .bottom) {
            if pages.isEmpty {
                empty
            } else {
                TabView(selection: $page) {
                    ForEach(Array(pages.enumerated()), id: \.offset) { i, p in
                        p.view(fill: true).tag(i)
                            .contentShape(Rectangle())
                            .onTapGesture { full = true }
                    }
                }
                .tabViewStyle(.page(indexDisplayMode: pages.count > 1 ? .always : .never))
                .accessibilityIdentifier("detail-hero-pager")
            }
            if editing { controls }
        }
        .onChange(of: pages.count) { _, n in if page >= n { page = max(0, n - 1) } }
        .fullScreenCover(isPresented: $full) {
            KindHeroViewer(pages: pages, page: page) { full = false }
        }
        .onChange(of: pickedCover) { _, item in take(item) { await cover($0) }; pickedCover = nil }
        .onChange(of: pickedPicture) { _, item in take(item) { await add($0) }; pickedPicture = nil }
        .onChange(of: pickedClip) { _, item in
            guard let item else { return }
            pickedClip = nil
            let mime = item.supportedContentTypes.first?.preferredMIMEType ?? "video/mp4"
            Task { if let data = try? await item.loadTransferable(type: Data.self) { await clip(data, mime) } }
        }
    }

    @ViewBuilder private var empty: some View {
        if let url = headline?.photoURL {
            PosterImage(url: url, imageID: "detail-media-remote") { FlyerStripes() }
        } else if let cells = headline?.area, !cells.isEmpty {
            MapCard(cells: cells, color: tint, width: width, tall: width)
        } else {
            FlyerStripes()
        }
    }

    /// The square's own writes, in Edit.
    private var controls: some View {
        HStack(spacing: 8) {
            if adds.contains(.cover) {
                PhotosPicker(selection: $pickedCover, matching: .images) { chip("Cover", "photo") }
                    .accessibilityIdentifier("detail-hero-cover")
            }
            if adds.contains(.picture) {
                PhotosPicker(selection: $pickedPicture, matching: .images) { chip("Add", "photo.badge.plus") }
                    .accessibilityIdentifier("detail-add-picture")
            }
            if pages.indices.contains(page), let key = pages[page].key {
                Button { Task { await remove(key) } } label: { chip("Remove", "trash") }
                    .accessibilityIdentifier("detail-hero-remove")
            }
            if adds.contains(.clip) {
                PhotosPicker(selection: $pickedClip, matching: .videos) { chip("Clip", "film") }
                    .accessibilityIdentifier("detail-hero-clip")
            }
        }
        .disabled(busy)
        .padding(.bottom, pages.count > 1 ? 34 : 14)
    }

    private func chip(_ text: String, _ glyph: String) -> some View {
        Label(text, systemImage: glyph)
            .font(.system(size: 13, weight: .semibold))
            .foregroundStyle(.white)
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(Capsule().fill(.black.opacity(0.55)))
    }

    private func take(_ item: PhotosPickerItem?, _ run: @escaping (Data) async -> Void) {
        guard let item else { return }
        Task { if let data = try? await item.loadTransferable(type: Data.self) { await run(data) } }
    }
}

/// One page of the hero: a picture (with its set's key when it can be removed), or the clip.
enum KindHeroPage {
    case picture(Data, key: String?, alt: String)
    case clip(URL)

    var key: String? { if case .picture(_, let key, _) = self { key } else { nil } }

    static func of(_ h: KindHeadline?) -> [KindHeroPage] {
        guard let h else { return [] }
        var pages: [KindHeroPage] = []
        if let photo = h.photo { pages.append(.picture(photo, key: nil, alt: h.title)) }
        pages += h.gallery.map { .picture($0.data, key: $0.key, alt: $0.alt) }
        if let c = h.clip, let url = KindClipFile.url(for: c) { pages.append(.clip(url)) }
        return pages
    }

    @ViewBuilder func view(fill: Bool) -> some View {
        switch self {
        case .picture(let data, _, let alt):
            if let image = UIImage(data: data) {
                Image(uiImage: image).resizable()
                    .aspectRatio(contentMode: fill ? .fill : .fit)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .clipped()
                    .accessibilityLabel(alt)
            }
        case .clip(let url):
            LoopingClipPlayer(url: url)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .accessibilityLabel("Clip")
        }
    }
}

/// The hero full screen: the same pages, fitted, on black.
private struct KindHeroViewer: View {
    let pages: [KindHeroPage]
    @State var page: Int
    var close: () -> Void

    var body: some View {
        ZStack(alignment: .topTrailing) {
            Color.black.ignoresSafeArea()
            TabView(selection: $page) {
                ForEach(Array(pages.enumerated()), id: \.offset) { i, p in p.view(fill: false).tag(i) }
            }
            .tabViewStyle(.page(indexDisplayMode: pages.count > 1 ? .always : .never))
            .ignoresSafeArea()
            Button(action: close) {
                Image(systemName: "xmark")
                    .font(.system(size: 15, weight: .bold))
                    .foregroundStyle(.white)
                    .frame(width: 40, height: 40)
                    .background(Circle().fill(.white.opacity(0.18)))
            }
            .padding(16)
            .accessibilityLabel("Close")
            .accessibilityIdentifier("detail-hero-close")
        }
        .accessibilityIdentifier("detail-hero-viewer")
    }
}

/// A clip's bytes, written once to a file a player can open, named by their own hash.
enum KindClipFile {
    static func url(for clip: KindClip) -> URL? {
        let ext = clip.mime.contains("quicktime") ? "mov" : "mp4"
        let name = String(format: "%016llx", UInt64(bitPattern: Int64(clip.data.hashValue)))
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("clip-\(name).\(ext)")
        if !FileManager.default.fileExists(atPath: url.path) {
            guard (try? clip.data.write(to: url)) != nil else { return nil }
        }
        return url
    }
}

/// The cover's average colour, for the page's ground. Computed on read; nothing stored.
enum KindCast {
    static func of(_ data: Data?) async -> Color? {
        guard let data else { return nil }
        return await Task.detached(priority: .utility) { () -> Color? in
            guard let image = CIImage(data: data) else { return nil }
            let filter = CIFilter(name: "CIAreaAverage",
                                  parameters: [kCIInputImageKey: image,
                                               kCIInputExtentKey: CIVector(cgRect: image.extent)])
            guard let out = filter?.outputImage else { return nil }
            var px = [UInt8](repeating: 0, count: 4)
            CIContext(options: [.workingColorSpace: NSNull()])
                .render(out, toBitmap: &px, rowBytes: 4, bounds: CGRect(x: 0, y: 0, width: 1, height: 1),
                        format: .RGBA8, colorSpace: nil)
            return Color(red: Double(px[0]) / 255, green: Double(px[1]) / 255, blue: Double(px[2]) / 255)
        }.value
    }
}

/// A value its author wrote in markdown, rendered whole: headings, lists and quotes as
/// blocks, emphasis and links inline (CommonMark through `AttributedString`).
struct KindMarkdown: View {
    let source: String

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(Array(blocks.enumerated()), id: \.offset) { _, b in
                Text(b.text)
                    .font(b.heading ? .system(size: 17, weight: .bold) : .system(size: 15))
                    .foregroundStyle(Theme.textSecondary)
                    .tint(Theme.accent)
                    .padding(.leading, b.indent)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .accessibilityElement(children: .combine)
    }

    private struct Block { let text: AttributedString; let heading: Bool; let indent: CGFloat }

    /// Split at the presentation intents the parser marks: one Text per paragraph,
    /// heading, list item or quote.
    private var blocks: [Block] {
        guard let doc = try? AttributedString(markdown: source,
                                              options: .init(interpretedSyntax: .full)) else {
            return [Block(text: AttributedString(source), heading: false, indent: 0)]
        }
        var out: [Block] = []
        for run in doc.runs[\.presentationIntent] {
            let (intent, range) = run
            var text = AttributedString(doc[range])
            let kinds = intent?.components.map(\.kind) ?? []
            let heading = kinds.contains { if case .header = $0 { true } else { false } }
            let item = kinds.contains { if case .listItem = $0 { true } else { false } }
            let quote = kinds.contains { if case .blockQuote = $0 { true } else { false } }
            if item { text = AttributedString("•  ") + text }
            out.append(Block(text: text, heading: heading, indent: (item || quote) ? 12 : 0))
        }
        return out
    }
}
