//  KindPage.swift
//  The maximised detail — a card, grown to the whole screen.
//
//  THE CARD MAXIMISES. Tapping an ObjectCard does not raise a modal over the feed:
//  the card's own square grows from where it sits until it is the top of the
//  object's page, and the page fills in beneath it. RED shrinks it back into the
//  slot it came from. One move, the same one a jewel makes on the rail, so the
//  page is visibly THAT card and not a screen about it.
//
//  IT IS A ShellSurface, hosted UNDER the disc row, so RED · GOLD · BLUE stay the
//  page's own buttons — close · share · say something — exactly as on the modal it
//  replaces (`KindSession`).
//
//  WHAT THE PAGE IS. Media, the title, the kind's rows, its pictures and document,
//  whose it is, and what people said. A VIEWER sees values only. The OWNER sees
//  each writable row in the creator's row grammar (`FieldRow`) and writes it in
//  place; the creator takes three fields, and this is where the rest are written.
//
//  WHY THE COMMENTS ARE FLAT. The object's log carries a real threaded projection
//  (`ForumThreadRow.depth`, what ThreadView renders). This shows it flat, newest
//  last, because a nesting tree is a ROOM's information architecture and this is a
//  detail page with comments under it.

import SwiftUI
import PhotosUI
import PDFKit
import UniformTypeIdentifiers

struct KindPage: View {
    @ObservedObject var session: KindSession
    /// Close without keeping. Routed to the SAME action RED takes, so a swipe and
    /// the disc can never disagree about what closing means.
    var onClose: () -> Void

    @FocusState private var typing: Bool
    /// The page at full size. False on the first frame and again on the way out, so
    /// the square starts and ends where the card is.
    @State private var grown = false
    @State private var picked: PhotosPickerItem?
    @State private var importing = false
    @State private var inside = false

    private var exit: NewExit? { session.exit }

    /// RED's own colour — the disc that closes this.
    private var tint: Color { DiscSlot.note.blockColor }

    var body: some View {
        GeometryReader { geo in
            let width = geo.size.width
            let page = geo.frame(in: .global)
            ZStack(alignment: .top) {
                Theme.bg
                    .opacity(grown ? 1 : 0)
                    .animation(grown ? Motion.screenIn : Motion.noteGroundOut, value: grown)
                content(width: width)
                    .frame(width: width, height: geo.size.height, alignment: .top)
                    // At rest the page shows only its square, at the card's size and
                    // place; grown it is the screen. The mask and the transform move
                    // together on one spring.
                    .mask(alignment: .top) {
                        RoundedRectangle(cornerRadius: grown ? 0 : DesignTokens.Skin.radius / scale(width),
                                         style: .continuous)
                            .frame(height: grown ? geo.size.height : width)
                    }
                    .scaleEffect(grown ? 1 : scale(width), anchor: .topLeading)
                    .offset(grown ? .zero : offset(in: page))
                    .opacity(grown || session.origin != nil ? 1 : 0)
            }
        }
        // The screen's edges, not the keyboard's: the field you are typing in, and
        // what you just said above it, stay above the keyboard.
        .ignoresSafeArea(.container)
        .allowsHitTesting(exit == nil)
        .task { await session.load() }
        .onAppear { withAnimation(Self.grow) { grown = true } }
        .onChange(of: exit) { _, leaving in
            guard leaving != nil else { return }
            typing = false
            withAnimation(Self.grow) { grown = false }
        }
        .onChange(of: picked) { _, item in
            guard let item else { return }
            picked = nil
            Task {
                if let data = try? await item.loadTransferable(type: Data.self) {
                    await session.addPicture(data)
                }
            }
        }
        .fileImporter(isPresented: $importing, allowedContentTypes: [.pdf]) { result in
            guard case .success(let url) = result else { return }
            let scoped = url.startAccessingSecurityScopedResource()
            defer { if scoped { url.stopAccessingSecurityScopedResource() } }
            guard let data = try? Data(contentsOf: url) else { return }
            Task { await session.setDocument(data, name: url.lastPathComponent) }
        }
        .fullScreenCover(isPresented: $inside) {
            session.headline?.inside?.view { inside = false }
        }
    }

    /// Inside the shell's departure window (`Motion.noteExit`), so the square is home
    /// before the surface unmounts.
    static let grow = Animation.spring(response: 0.36, dampingFraction: 0.86)

    private func scale(_ width: CGFloat) -> CGFloat {
        guard let o = session.origin, width > 0 else { return 0.94 }
        return o.width / width
    }

    private func offset(in page: CGRect) -> CGSize {
        guard let o = session.origin else { return CGSize(width: page.width * 0.03, height: 40) }
        return CGSize(width: o.minX - page.minX, height: o.minY - page.minY)
    }

    // MARK: The page

    private func content(width: CGFloat) -> some View {
        ScrollViewReader { proxy in
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                media(width: width)
                Group {
                    if session.sharing {
                        shareFace
                    } else {
                        title
                        rows
                        if let way = session.headline?.inside { insideBar(way) }
                        adds
                        gallery
                        document(width: width)
                        if let author = session.author { authorBar(author) }
                        comments
                    }
                }
                .opacity(grown ? 1 : 0)
                .animation(grown ? Motion.screenIn.delay(0.08) : Motion.noteGroundOut, value: grown)
            }
            // Clear of the disc row, which floats over the page's foot.
            .padding(.bottom, ShellMetrics.bottomBarHeight + 24)
            .overlay(alignment: .bottom) { Color.clear.frame(height: 1).id(Self.composerID) }
        }
        .scrollIndicators(.hidden)
        .scrollDisabled(!grown)
        .scrollDismissesKeyboard(.interactively)
        .background(Theme.bg.opacity(grown ? 1 : 0))
        // What you just said is on screen, above the field you said it in.
        .onChange(of: session.comments.count) { before, after in
            guard after > before else { return }
            withAnimation(Motion.screenIn) { proxy.scrollTo(Self.composerID, anchor: .bottom) }
        }
        }
    }

    /// The page's foot, below the disc row's clearance: scrolled to, the field and
    /// the comment above it sit clear of the row.
    private static let composerID = "detail-foot"

    /// MEDIA — the square the card showed, at the screen's width: the picture, a
    /// Place's area, or the stripes, which say THERE IS NO PICTURE rather than
    /// implying one is on its way.
    @ViewBuilder private func media(width: CGFloat) -> some View {
        Color.clear
            .frame(width: width, height: width)
            .overlay {
                if let bytes = session.headline?.photo, let image = UIImage(data: bytes) {
                    Image(uiImage: image).resizable().scaledToFill()
                } else if let url = session.headline?.photoURL {
                    PosterImage(url: url, imageID: "detail-media-remote") { FlyerStripes() }
                } else if let cells = session.headline?.area, !cells.isEmpty {
                    MapCard(cells: cells, color: tint, width: width, tall: width)
                } else {
                    FlyerStripes()
                }
            }
            .clipped()
            // One element: the square, valued with the card's square it grew from,
            // "x,y,w,h" in points — what a test holds the grow to. A container's
            // identifier would override its children's.
            .accessibilityElement()
            .accessibilityLabel(mediaLabel)
            .accessibilityValue(session.origin.map {
                "\(Int($0.minX)),\(Int($0.minY)),\(Int($0.width)),\(Int($0.height))"
            } ?? "")
            .accessibilityIdentifier("detail-media")
    }

    /// The title, and what the square shows when it is not a picture of it.
    private var mediaLabel: String {
        let title = session.headline?.title ?? ""
        return session.headline?.area.isEmpty == false ? "\(title), map of its area" : title
    }

    @ViewBuilder private var title: some View {
        Group {
            if let edit = session.headline?.titleEdit {
                KindTitleField(edit: edit, busy: session.saving == edit.op + "." + edit.arg) { value in
                    await session.edit(edit, to: value)
                }
            } else {
                Text(session.headline?.title ?? "")
            }
        }
        .font(.system(size: 24, weight: .heavy))
        .tracking(-0.2)
        .foregroundStyle(Theme.textPrimary)
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 18)
        .padding(.top, 18)
        .padding(.bottom, 6)
        .accessibilityIdentifier("detail-title")
    }

    // MARK: Rows — values for a viewer, fields for the owner

    private var rows: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(session.headline?.rows ?? []) { row in
                if let edit = row.edit {
                    KindEditRow(row: row, edit: edit, busy: session.saving == row.id) { value in
                        await session.edit(edit, to: value)
                    }
                } else if !row.value.isEmpty {
                    valueRow(row)
                }
            }
        }
        .padding(.horizontal, 18)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    @ViewBuilder private func valueRow(_ row: KindRow) -> some View {
        let text = Text(row.markdown
                        ? (try? AttributedString(markdown: row.value,
                                                 options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace)))
                            ?? AttributedString(row.value)
                        : AttributedString(row.value))
            .font(.system(size: 15))
            .fixedSize(horizontal: false, vertical: true)
        Group {
            if let link = row.link {
                Link(destination: link) { text.foregroundStyle(Theme.accent) }
            } else if let place = row.place {
                Button { openPlace(place) } label: {
                    HStack(spacing: 6) {
                        text.foregroundStyle(Theme.textPrimary)
                        Image(systemName: "chevron.right").font(.system(size: 11, weight: .semibold))
                            .foregroundStyle(Theme.textTertiary)
                    }
                }
                .buttonStyle(.plain)
            } else {
                text.foregroundStyle(Theme.textSecondary)
            }
        }
        .accessibilityIdentifier("detail-row-\(row.id)")
    }

    private func insideBar(_ way: KindInside) -> some View {
        Button { inside = true } label: {
            HStack(spacing: 12) {
                Image(systemName: way.glyph)
                    .font(.system(size: 15, weight: .semibold))
                    .foregroundStyle(tint)
                Text(way.label.uppercased())
                    .font(.system(size: 12, weight: .semibold, design: .monospaced))
                    .kerning(1.2)
                    .foregroundStyle(Theme.textPrimary)
                Spacer()
                Image(systemName: "chevron.right")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(Theme.textTertiary)
            }
            .padding(.horizontal, 18)
            .frame(height: ProfileGeometry.bar)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .padding(.top, 14)
        .accessibilityIdentifier(way.id)
    }

    /// The owner's adds — a picture into the kind's set, a document onto a Post.
    @ViewBuilder private var adds: some View {
        let offered = session.headline?.adds ?? []
        if !offered.isEmpty {
            HStack(spacing: 10) {
                ForEach(offered) { add in
                    switch add {
                    case .picture:
                        PhotosPicker(selection: $picked, matching: .images) {
                            addLabel("Add picture", glyph: "photo.badge.plus")
                        }
                        .accessibilityIdentifier("detail-add-picture")
                    case .document:
                        Button { importing = true } label: { addLabel("Attach PDF", glyph: "doc.badge.plus") }
                            .accessibilityIdentifier("detail-add-document")
                    }
                }
            }
            .disabled(session.saving != nil)
            .padding(.horizontal, 18)
            .padding(.top, 12)
        }
    }

    private func addLabel(_ text: String, glyph: String) -> some View {
        Label(text, systemImage: glyph)
            .font(.system(size: 13, weight: .semibold))
            .foregroundStyle(Theme.textPrimary)
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(Capsule().fill(Theme.fill.opacity(0.6)))
    }

    /// The rest of the pictures, in the order the fold gives them.
    @ViewBuilder private var gallery: some View {
        let pictures = session.headline?.gallery ?? []
        if !pictures.isEmpty {
            ScrollView(.horizontal) {
                HStack(spacing: 8) {
                    ForEach(pictures) { p in
                        if let image = UIImage(data: p.data) {
                            Image(uiImage: image).resizable().scaledToFill()
                                .frame(width: 132, height: 132)
                                .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
                                .accessibilityLabel(p.alt)
                        }
                    }
                }
                .padding(.horizontal, 18)
            }
            .scrollIndicators(.hidden)
            .padding(.top, 16)
            .accessibilityIdentifier("detail-gallery")
        }
    }

    /// A Post's PDF, read in place.
    @ViewBuilder private func document(width: CGFloat) -> some View {
        if let doc = session.headline?.document, let pdf = PDFDocument(data: doc.data) {
            VStack(alignment: .leading, spacing: 6) {
                Text(doc.name)
                    .font(.system(size: 12, weight: .semibold, design: .monospaced))
                    .foregroundStyle(Theme.textTertiary)
                KindPDF(document: pdf)
                    .frame(height: (width - 36) * 1.3)
                    .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
            }
            .padding(.horizontal, 18)
            .padding(.top, 16)
            .accessibilityIdentifier("detail-document")
        }
    }

    // MARK: Whose, and what was said

    /// AUTHOR — a door to their profile, which is where the egoic IA lives. It
    /// closes this page first: the profile is a page in LIFE's stack, and a route
    /// fired into a surface that is still leaving is silently dropped.
    private func authorBar(_ author: KindAuthor) -> some View {
        Button {
            let id = author.id, isSpace = author.isSpace
            onClose()
            DispatchQueue.main.asyncAfter(deadline: .now() + Motion.noteExit) {
                NotificationCenter.default.post(
                    name: isSpace ? .pacificOpenGroup : .pacificOpenPerson,
                    object: nil, userInfo: ["object": id])
            }
        } label: {
            ConversationRow(title: author.name,
                            snippet: "",
                            time: author.isSpace ? "SPACE" : "PERSON",
                            tint: author.color ?? tint,
                            initials: author.initials,
                            blockColor: author.color ?? tint,
                            enrichSnippet: false,
                            avatarSize: 36)
                .padding(.horizontal, 18)
                .frame(height: ProfileGeometry.bar)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .padding(.top, 14)
        .accessibilityIdentifier("detail-author")
    }

    private func openPlace(_ id: String) {
        onClose()
        DispatchQueue.main.asyncAfter(deadline: .now() + Motion.noteExit + 0.1) {
            NotificationCenter.default.post(name: .pacificOpenPlace, object: nil, userInfo: ["object": id])
        }
    }

    /// COMMENTS — a section of its own, on the page whenever the device holds the
    /// object's comments section, said-in or not: its heading and count, what was
    /// said (flat, oldest first, each in its author's own face, their name a door),
    /// and the field to say something in. BLUE sends it.
    @ViewBuilder private var comments: some View {
        if session.commentsHeld || !session.comments.isEmpty {
            VStack(alignment: .leading, spacing: 14) {
                HStack(spacing: 8) {
                    Text("COMMENTS")
                    if !session.comments.isEmpty { Text("\(session.comments.count)") }
                }
                .font(.system(size: 11, weight: .semibold, design: .monospaced))
                .kerning(1.4)
                .foregroundStyle(Theme.textTertiary)
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("detail-comments-heading")

                ForEach(session.comments) { comment in
                    commentRow(comment)
                }
                if session.takesComment && session.commentsHeld { composer }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 18)
            .padding(.vertical, 18)
            .background(Theme.surface)
            .overlay(alignment: .top) { Rectangle().fill(Theme.hairline).frame(height: 1) }
            .padding(.top, 22)
        }
    }

    private func commentRow(_ comment: KindComment) -> some View {
        HStack(alignment: .top, spacing: 10) {
            Avatar(initials: comment.initials, tint: Theme.fill, size: 32,
                   photo: comment.photo, color: comment.color)
            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 6) {
                    Button { openPerson(comment.authorID) } label: {
                        Text(comment.author)
                            .font(.system(size: 13, weight: .semibold))
                            .foregroundStyle(Theme.textPrimary)
                    }
                    .buttonStyle(.plain)
                    .disabled(comment.mine)
                    Text(Date(timeIntervalSince1970: Double(comment.whenMs) / 1000),
                         format: .relative(presentation: .named))
                        .font(.system(size: 12))
                        .foregroundStyle(Theme.textTertiary)
                }
                Text(comment.body)
                    .font(.system(size: 15))
                    .foregroundStyle(Theme.textPrimary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 0)
        }
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("detail-comment")
    }

    private func openPerson(_ id: String) {
        onClose()
        DispatchQueue.main.asyncAfter(deadline: .now() + Motion.noteExit) {
            NotificationCenter.default.post(name: .pacificOpenPerson, object: nil, userInfo: ["object": id])
        }
    }

    /// The field the section ends on. BLUE sends it — there is no send button here,
    /// because the disc IS the send button.
    private var composer: some View {
        TextField("Say something", text: $session.draft, axis: .vertical)
            .lineLimit(1...4)
            .font(.system(size: 15))
            .foregroundStyle(Theme.textPrimary)
            .focused($typing)
            .padding(.horizontal, 14)
            .padding(.vertical, 11)
            .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(Theme.fill))
            .accessibilityIdentifier("detail-comment-field")
    }

    // MARK: The share face — GOLD's turn

    /// Where this can go: the Spaces you are actually in. The page under the square
    /// TURNS to it, the same move the mint's audience dial makes, rather than raising
    /// a menu that would cover the discs that opened it.
    private var shareFace: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("SHARE TO")
                .font(.system(size: 11, weight: .semibold, design: .monospaced))
                .kerning(1.4)
                .foregroundStyle(Theme.textTertiary)
                .padding(.horizontal, 18)
                .padding(.top, 20)
                .padding(.bottom, 10)
            ForEach(session.targets) { target in
                Button {
                    Task { await session.share(to: target) }
                } label: {
                    ConversationRow(title: target.name,
                                    snippet: "",
                                    time: "SPACE",
                                    tint: tint,
                                    initials: Avatar.initials(target.name),
                                    blockColor: tint,
                                    enrichSnippet: false,
                                    avatarSize: 32)
                        .padding(.horizontal, 18)
                        .frame(height: ProfileGeometry.bar)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityIdentifier("detail-share-face")
    }
}

// MARK: - The owner's row

/// One writable row, in the creator's grammar. It commits when the person is done
/// with it — Return, leaving the field, a choice picked, a switch thrown, a date
/// SET — and the page reads the value back from the fold. Every commit is a
/// sequenced write that is kept for good, so a value is sent once: never per step
/// of a wheel, and never twice for one Return (which also blurs the field).
private struct KindEditRow: View {
    let row: KindRow
    let edit: KindEdit
    let busy: Bool
    /// Whether it landed.
    var commit: (String) async -> Bool

    @State private var text = ""
    @State private var date = Date()
    @State private var dated = false
    /// The last value sent, so a Return and the blur it causes send one write.
    @State private var sent: String?
    @FocusState private var focused: Bool

    var body: some View {
        FieldRow(label: row.label) {
            Group {
                switch edit.input {
                case .line, .multiline:
                    TextField("", text: $text, axis: edit.input == .multiline ? .vertical : .horizontal)
                        .lineLimit(edit.input == .multiline ? 1...8 : 1...1)
                        .focused($focused)
                        .submitLabel(.done)
                        .onSubmit { send(text) }
                        .onChange(of: focused) { _, now in if !now { send(text) } }
                        .accessibilityIdentifier(id)
                case .choice(let words):
                    Menu {
                        ForEach(words, id: \.self) { w in Button(w) { send(w) } }
                        if !edit.current.isEmpty { Button("Clear", role: .destructive) { send("") } }
                    } label: {
                        Text(edit.current)
                            .frame(maxWidth: .infinity, minHeight: 20, alignment: .leading)
                            .contentShape(Rectangle())
                    }
                    .accessibilityIdentifier(id)
                case .toggle:
                    Toggle("", isOn: Binding(get: { edit.current == "1" },
                                             set: { send($0 ? "1" : "") }))
                        .labelsHidden()
                        .accessibilityIdentifier(id)
                case .date:
                    if dated {
                        // The wheel only moves `date`; SET sends it, once.
                        HStack {
                            DatePicker("", selection: $date).labelsHidden().accessibilityIdentifier(id)
                            Spacer(minLength: 8)
                            if dateValue != edit.current {
                                Button { send(dateValue) } label: {
                                    Image(systemName: "checkmark.circle.fill").foregroundStyle(Theme.accent)
                                }
                                .accessibilityLabel("Set")
                                .accessibilityIdentifier("detail-set-\(row.id)")
                            }
                        }
                    } else {
                        Button { dated = true } label: {
                            Image(systemName: "plus").foregroundStyle(Theme.textSecondary)
                        }
                        .accessibilityIdentifier(id)
                    }
                }
            }
            .font(.system(size: 15))
            .foregroundStyle(Theme.textPrimary)
            .disabled(busy)
        }
        .onAppear(perform: seed)
        .onChange(of: edit.current) { _, _ in seed() }
    }

    /// Each control carries it, not their container: a container's identifier would
    /// override the SET button's own.
    private var id: String { "detail-edit-\(row.id)" }

    /// Rounded, not truncated: ms → Date → ms through a Double can land 1 ms short.
    private var dateValue: String { String(Int64((date.timeIntervalSince1970 * 1000).rounded())) }

    /// A value that did not land clears `sent`, so sending it again is a retry.
    private func send(_ value: String) {
        guard value != (sent ?? edit.current) else { return }
        sent = value
        Task { if !(await commit(value)) { sent = nil } }
    }

    private func seed() {
        text = edit.current
        sent = nil
        if case .date = edit.input, let ms = Int64(edit.current), ms > 0 {
            date = Date(timeIntervalSince1970: Double(ms) / 1000)
            dated = true
        }
    }
}

/// The owner's title: the heading itself is the field.
private struct KindTitleField: View {
    let edit: KindEdit
    let busy: Bool
    /// Whether it landed.
    var commit: (String) async -> Bool

    @State private var text = ""
    @State private var sent: String?
    @FocusState private var focused: Bool

    var body: some View {
        TextField("", text: $text, axis: .vertical)
            .focused($focused)
            .submitLabel(.done)
            .onSubmit { commitIfNamed() }
            .onChange(of: focused) { _, now in if !now { commitIfNamed() } }
            .disabled(busy)
            .onAppear { text = edit.current }
            .onChange(of: edit.current) { _, now in text = now; sent = nil }
    }

    /// A title is required by every kind's profile op: an emptied field puts the
    /// held one back rather than sending a write the fold must refuse.
    private func commitIfNamed() {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if t.isEmpty { text = edit.current; return }
        guard t != (sent ?? edit.current) else { return }
        sent = t
        Task { if !(await commit(t)) { sent = nil } }
    }
}

/// PDFKit's reader, for a Post's document.
private struct KindPDF: UIViewRepresentable {
    let document: PDFDocument

    func makeUIView(context: Context) -> PDFView {
        let view = PDFView()
        view.autoScales = true
        view.displayMode = .singlePageContinuous
        view.backgroundColor = .clear
        view.document = document
        return view
    }

    func updateUIView(_ view: PDFView, context: Context) {
        if view.document !== document { view.document = document }
    }
}
