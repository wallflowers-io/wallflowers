//  Shell.swift
//  The root shell: a bottom row of three CIRCLE buttons — NOTE · CHAT · LIFE —
//  floating over the current screen (the native tab "pill" is retired). CHAT and
//  LIFE swap the shown screen (lands on CHAT). NOTE carries BOTH halves of notes:
//  a TAP is an action — it presents the full-screen compose modal (NoteComposeModal,
//  WS-B) over whatever is showing — and a HOLD swaps the screen to the Notes PAGE
//  (NotesView, your notes newest-first), a destination exactly like CHAT and LIFE.
//  The top-left account avatar every screen shows opens the account sheet
//  where Me (profile), the Live core, and connected Systems (Sysiphus) live.

import SwiftUI
import UIKit          // the feedback generators behind a note's departure
import Combine        // forwarding the compose session's changes to the disc row

/// Shared shell state so every tab's avatar button can open the one account sheet — and
/// the home of the open conversation, because the SHELL presents it, not the Chats
/// screen. That is what lets the conversation be drawn ABOVE the NOTE·CHAT·LIFE row: the
/// composer stretching out of the CHAT disc COVERS the row instead of blending with it,
/// and the row can fade where it stands rather than sliding out of the way.
@MainActor
final class ShellState: ObservableObject {
    @Published var showAccount = false

    /// The page currently on the glass, mirrored from the shell. Screens are
    /// PRELOADED — all three stay mounted for the app's lifetime — so a screen
    /// can no longer use `onAppear` to mean "the user arrived": it fires once, at
    /// launch. Parked screens watch this instead, to run their entry work on real
    /// arrival and to stay quiet while parked (LIFE must not fire the axial flip
    /// off-screen, nor leave `suppressPageSwipe` set for a page it isn't on).
    @Published private(set) var screen: ShellScreen = .chat
    func mirrorScreen(_ s: ShellScreen) { if screen != s { screen = s } }

    /// CHAT's two-state scope, owned HERE and not by ChatsView, because the GOLD
    /// disc is now its only control — the chip row that used to set it is gone.
    /// Arrival at CHAT resets it (see `show`), so "first tap lands on Chat,
    /// second lands on Spaces" holds from anywhere in the app.
    @Published var chatScope: ChatScope = .chats

    /// The Settings overlay's disc choreography. Settings is NOT a sheet — it is drawn under
    /// the disc row, because the row is its chrome: NOTE (red) slides to the centre and becomes
    /// the dismiss button while CHAT + LIFE accelerate off to the right. On dismiss NOTE follows
    /// them off, then all three decelerate back in from the left to their resting places.
    enum SettingsPhase: Equatable {
        case closed      // discs at rest
        case open        // NOTE centred (dismiss), CHAT + LIFE gone right
        case leaving     // NOTE accelerating off right
        case returning   // parked off-screen LEFT, about to decelerate home
    }
    @Published private(set) var settingsPhase: SettingsPhase = .closed

    /// WHICH content is drawn under the centred disc. The choreography (phases,
    /// curves, the red disc as the close button) is ONE mechanism — Settings was
    /// its first rider; New chat and the QR/connect screen ride it identically.
    enum RowOverlay: Equatable { case settings, newChat, connect }
    @Published private(set) var rowOverlay: RowOverlay = .settings

    /// Present content under the row: the red disc slides to centre and becomes
    /// the close button while CHAT + LIFE accelerate off to the right.
    func openOverlay(_ o: RowOverlay) {
        guard !showAccount, settingsPhase == .closed else { return }
        rowOverlay = o
        withAnimation(.easeOut(duration: 0.22)) { showAccount = true }
        settingsPhase = .open        // per-disc curves live in `buttonRow`
    }

    /// Swap what's under the ALREADY-open row (New chat → the QR screen). The
    /// discs stay centred; only the content changes.
    func swapOverlay(_ o: RowOverlay) {
        guard showAccount else { return openOverlay(o) }
        withAnimation(.easeOut(duration: 0.2)) { rowOverlay = o }
    }

    /// Present Settings: the row re-forms around NOTE as the dismiss button.
    func openSettings() { openOverlay(.settings) }

    /// Dismiss: NOTE accelerates off right, then the row sweeps back in from the left.
    func dismissSettings() {
        guard showAccount else { return }
        withAnimation(.easeIn(duration: 0.18)) { showAccount = false }
        settingsPhase = .leaving
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.30) { [self] in
            settingsPhase = .returning                       // snap off-screen LEFT (no animation)
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.02) { [self] in
                settingsPhase = .closed                      // decelerate home, staggered
            }
        }
    }
    /// True while an immersive conversation is open. The floating NOTE·CHAT·LIFE row
    /// fades out for the duration — Signal-style, a conversation owns the whole screen
    /// and you leave it with the nav back button.
    @Published var immersive = false

    /// The open conversation, or nil for none.
    @Published private(set) var conversation: Conversation?
    /// The composer's disc⇄field morph (see `Composer.extended`).
    @Published private(set) var composerOpen = false
    /// True while the conversation is fading out but is still MOUNTED, so its composer
    /// can be seen collapsing. Nothing in it is tappable while this is true — an
    /// invisible screen must not keep swallowing taps meant for the list underneath.
    @Published private(set) var leaving = false

    /// The `New` surface that is up, if any (Design/New.swift) — one at a time,
    /// app-wide. The three discs slide from NOTE·CHAT·LIFE into its ✕ · Share · Commit
    /// while it is, because they ARE its buttons.
    ///
    /// This used to be `composing: Bool`, which is all the NOTE needed. Every other mint
    /// was a sheet or a full-screen cover — presented ABOVE the shell, covering the very
    /// discs it would have borrowed — so each hosted a static replica of the row instead.
    /// Holding the SESSION rather than a flag is what lets any kind be a shell overlay,
    /// and `composing` survives below so `buttonRow` is untouched by the change.
    @Published private(set) var compose: (any ShellSurface)?

    /// A surface that has committed and is playing its departure: still MOUNTED so the
    /// exit can be seen, but no longer the active compose (the discs are already sliding
    /// home). Cleared when the teardown lands.
    @Published private(set) var leavingCompose: (any ShellSurface)?

    /// Whether the row is in its compose arrangement. Derived, so the spread in
    /// `buttonRow` reads exactly as it did.
    var composing: Bool { compose != nil }

    /// Suppresses the shell's horizontal page-swipe (NOTE↔CHAT↔LIFE) while a child owns
    /// horizontal drags: LIFE's jiggle-mode reordering, and a list's own swipe-to-delete.
    /// Without this the page-swipe hijacks those gestures.
    @Published var suppressPageSwipe = false

    /// The bearing number printed above the yellow CHAT disc, shown while the top-down map is up.
    @Published var showCompass = false

    // The two-faced LIFE screen (market face + axial spin) was REMOVED 10 Aug
    // (FEED-TRANSMISSION 0-ANSWER-G follow-up): LIFE is one page; TRADE and JOB
    // are jewels on its rail.

    /// THE SPLIT (iPad): LIFE is not a page you turn to — it holds the right two
    /// thirds permanently, and the red/gold discs swap NOTE·CHAT through the left
    /// third. `screen` therefore names only what the LEFT PANEL shows.
    @Published var split = false

    /// A jewel being dragged in LIFE. Drawn at the SHELL level, exactly as the press-bloom
    /// is, so neither the disc row nor a screen's own bounds can clip or cover it. A
    /// reference type observed only by that overlay, so moving it never re-renders the shell.
    let jewelDrag = JewelDragModel()

    // How a surface leaves is `NewExit` (Design/New.swift) — each outcome gets its own
    // edge colour, its own exit curve and its own (or no) haptic, so the surface tells
    // you what happened without a word of copy. It lives with the component now, because
    // the three outcomes were never note-specific.

    private var noteExitGeneration = 0

    /// Guards the delayed teardown. Open a new conversation before the last one has
    /// finished leaving and the stale timer would otherwise close the NEW one.
    private var exitGeneration = 0

    /// The card fades up and the discs slide into their compose arrangement — together,
    /// from this frame. (The card overrides the curve for its own fade; see the overlay.)
    ///
    /// One note at a time: a NOTE tap is IGNORED while a card is up or still animating
    /// out. Re-entering during the ~0.4s exit reused the leaving card's view instance —
    /// so its `onAppear` focus never re-fired and you got a card with no keyboard — and
    /// let that card's pending teardown wipe the new draft. The guard closes both.
    /// How a NOTE session is built. Installed by the shell, which owns the NotesStore
    /// and the Place-publish sheet the note's GOLD raises. Any screen's `+` can then ask
    /// for the SAME note surface the RED disc raises, instead of building a second one
    /// that drifts — the Notes page's `+` is the one caller today.
    var makeNoteSession: (() -> NewSession)?

    /// Raise a note. The Notes page's `+` and the RED disc are the same gesture.
    func beginNote() {
        guard let make = makeNoteSession else { return }
        beginNew(make())
    }

    /// How every OTHER kind's session is built. Installed by LIFE, which owns each kind's
    /// mint and what follows it, so RED mints exactly what LIFE's `+` does rather than a
    /// second copy.
    var makeMintSession: ((NewKind) -> NewSession)?

    /// RED's creator while it is the surface up (and while it leaves). Set on every raise,
    /// so it never outlives its surface.
    @Published private(set) var creator: NewCreator?
    private var creatorSink: AnyCancellable?

    /// RED at rest: the creator, opening on a note. Every kind but the Channel, which is
    /// minted INTO a Space and RED has none; only the note until LIFE has installed the
    /// mints.
    func beginCreator() {
        guard let note = makeNoteSession else { return }
        let mint = makeMintSession
        let kinds = mint == nil ? [NewKind.note] : NewKind.allCases.filter { $0 != .channel }
        let c = NewCreator(kinds: kinds) { k in k == .note ? note() : mint!(k) }
        beginNew(c.session, creator: c)
        guard creator === c else { return }
        // `$active` publishes the NEW kind before it is stored.
        creatorSink = c.$active.dropFirst().sink { [weak self, weak c] k in
            guard let s = c?.sessions[k] else { return }
            self?.swapNew(s)
        }
    }

    /// The creator's card in view changed: that session takes the old one's place under
    /// the same discs, which stay where they are.
    private func swapNew(_ session: NewSession) {
        guard compose is NewSession, compose?.exit == nil else { return }
        composeSink = session.changes.sink { [weak self] _ in
            self?.objectWillChange.send()
        }
        compose = session
    }

    /// Raise ANY shell-hosted surface — a mint, or the detail modal. Named for the
    /// mint because that is what every existing caller raises; what it takes is the
    /// seam (`Design/ShellSurface.swift`), which is what lets the detail modal
    /// inherit the circle choreography instead of drawing a replica of the row.
    func beginNew(_ session: any ShellSurface, creator: NewCreator? = nil) {
        guard compose == nil, leavingCompose == nil else { return }
        // Before `compose`, so the surface's first appearance already knows it is the
        // creator — that is when it decides whether to focus.
        self.creator = creator
        creatorSink = nil

        // FORWARD THE SESSION'S CHANGES TO OURSELVES.
        //
        // `NewSession` is its own ObservableObject, and the disc row reads its `cell`
        // through `shell.compose` — but the shell only observes ShellState. Typing into
        // the draft published on the SESSION and nothing redrew the row, so BLUE kept the
        // cell it was given at open (`canCommit: false`) and only woke up when something
        // unrelated happened to redraw the shell. That is the "send button stays disabled"
        // bug: the gate was right, the row just never re-asked.
        //
        // A nested ObservableObject does not propagate on its own; this is the seam that
        // makes it. Cancelled in `endNew`, so a departed session cannot keep the shell
        // redrawing.
        composeSink = session.changes.sink { [weak self] _ in
            self?.objectWillChange.send()
        }

        // Slide the discs into their compose arrangement on the keyboard's own clock
        // — the surface's field focuses as this runs, so the two move together.
        withAnimation(KeyboardMotion.shared.animation) { compose = session }
    }

    /// Keeps the active session's changes flowing into the disc row. See `beginNew`.
    private var composeSink: AnyCancellable?

    /// And back — all on this frame: the card's edge flashes the outcome's colour and it
    /// leaves, while the discs slide home and reform NOTE·CHAT·LIFE. The card stays
    /// MOUNTED for the length of its departure (dropping it here would delete it
    /// mid-animation); `finish` runs once it is gone, which is where the draft is cleared
    /// so the text never blanks out in front of you.
    func endNew(_ exit: NewExit, then finish: @escaping () -> Void = {}) {
        guard let session = compose else { return }
        noteExitGeneration &+= 1
        let generation = noteExitGeneration

        haptic(for: exit)
        // The surface owns its own departure curves (ground first, canvas after — see
        // `NewSurface`), so this just states the outcome. The discs slide home on the
        // same frame.
        session.beginExit(exit)
        // The session moves to `leavingCompose` so it stays MOUNTED for the length of its
        // departure — dropping it here would delete it mid-animation — while `compose`
        // going nil slides the discs home on the keyboard's dismissal clock (the field
        // resigns as the surface leaves), so the row and the keyboard settle together.
        leavingCompose = session
        composeSink = nil          // a departed session stops driving the row
        withAnimation(KeyboardMotion.shared.animation) { compose = nil }

        DispatchQueue.main.asyncAfter(deadline: .now() + Motion.noteExit) { [self] in
            guard generation == noteExitGeneration else { return }
            leavingCompose = nil
            finish()
        }
    }

    /// Kept is worth feeling; handed-off is worth feeling. Discarded is NOT — a haptic
    /// for throwing something away rewards the one outcome that kept nothing.
    private func haptic(for exit: NewExit) {
        switch exit {
        case .saved:     UINotificationFeedbackGenerator().notificationOccurred(.success)
        case .shared:    UIImpactFeedbackGenerator(style: .medium).impactOccurred()
        case .cancelled: break
        }
    }

    /// Everything on one frame: the transcript fades in, the disc row fades out where it
    /// stands, and the CHAT disc stretches into the composer.
    func open(_ convo: Conversation) {
        NSLog("[shell] open %@ kind=%@", String(convo.id.prefix(12)), convo.kind == .dm ? "dm" : "group")
        exitGeneration &+= 1
        leaving = false
        withAnimation(Motion.screenIn) { conversation = convo }
        withAnimation(Motion.rowOut) { immersive = true }
        // One runloop turn, and ONLY for a mechanical reason: a property that changes in
        // the same frame its view is inserted has no "from" value to travel out of, so
        // the stretch would snap to full width. ~16ms — still one motion to the eye.
        DispatchQueue.main.async { [self] in
            withAnimation(Motion.extend) { composerOpen = true }
        }
    }

    /// The same, reversed and simultaneous. The conversation FADES while still mounted
    /// (dropping it here would tear the composer out of the tree on the spot and there
    /// would be no collapse to see); the teardown that follows is invisible bookkeeping.
    func closeConversation() {
        guard conversation != nil, !leaving else { return }
        exitGeneration &+= 1
        let generation = exitGeneration
        withAnimation(Motion.retract) { composerOpen = false }
        withAnimation(Motion.rowIn) { immersive = false }
        withAnimation(Motion.screenOut) { leaving = true }
        DispatchQueue.main.asyncAfter(deadline: .now() + Motion.exit) { [self] in
            guard generation == exitGeneration else { return }
            conversation = nil
            leaving = false
        }
    }
}

/// The account avatar moved into the TOP-RIGHT CORNER (10 Aug order) — it now
/// rides TabChrome's trailing cluster as the outermost item, so this modifier is
/// a pass-through kept only for its call sites' stability.
struct AccountToolbar: ViewModifier {
    func body(content: Content) -> some View { content }
}

extension View {
    func accountToolbar() -> some View { modifier(AccountToolbar()) }
}

/// The UNIVERSAL tab chrome — one inline top bar shared by every primary tab: the tab
/// name RIGHT-ALIGNED next to a universal `+` for creating that tab's object (note / chat
/// / project). Inline (no large title) so the avatar and the title share one row instead
/// of stacking — reclaiming the vertical the large title used to waste. Pairs with
/// `accountToolbar` (the avatar, top-left); together they are the whole top bar.
struct TabChrome: ViewModifier {
    // The bell opens the deck, so THIS is the view that needs the graph (what was observed),
    // core (what was minted) and the derived proposals between them.
    @EnvironmentObject private var graph: GraphService
    @EnvironmentObject private var proposals: ProposalStore
    @EnvironmentObject private var coreService: CoreService
    let title: String
    let newLabel: String
    let showsTitle: Bool
    let showsNotifications: Bool
    /// Overrides the `+` glyph colour — LIFE passes the selected category's jewel so the
    /// `+` reads as "create one of THESE". nil keeps the app accent.
    let newTint: Color?
    /// nil ⇒ the screen creates nothing and shows NO `+` (the Notes page lists and
    /// searches; composing a note belongs to the shell's NOTE disc).
    let onNew: (() -> Void)?
    @State private var trayOpen = false

    init(title: String, newLabel: String, showsTitle: Bool, showsNotifications: Bool,
         newTint: Color? = nil, onNew: (() -> Void)?) {
        self.title = title
        self.newLabel = newLabel
        self.showsTitle = showsTitle
        self.showsNotifications = showsNotifications
        self.newTint = newTint
        self.onNew = onNew
    }

    func body(content: Content) -> some View {
        content
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                if showsTitle {
                    ToolbarItem(placement: .topBarTrailing) {
                        Text(title).font(.title3.weight(.bold))
                    }
                }
                // THE PRECESSION (10 Aug order): the top bar's three circles — CREATE ·
                // BELL · ACCOUNT — sit on three perches (WEST pill · EAST-INNER · EAST
                // CORNER). Cycling the tabs advances every circle ONE perch clockwise, so
                // a full NOTE→CHAT→LIFE cycle is one full revolution and each circle
                // visits every perch before coming home. A perch whose circle this screen
                // doesn't carry (no `+`, no bell) simply sits empty for that tab.
                ToolbarItem(placement: .topBarLeading) {
                    perchContent(circleAt(0))
                }
                if #available(iOS 26.0, *) {
                    ToolbarSpacer(.fixed, placement: .topBarTrailing)
                }
                ToolbarItem(placement: .topBarTrailing) {
                    perchContent(circleAt(1))
                }
                // The corner keeps its bare-circle treatment (glass ring stripped on
                // iOS 26) for WHICHEVER circle precession parks there.
                if #available(iOS 26.0, *) {
                    ToolbarSpacer(.fixed, placement: .topBarTrailing)
                    ToolbarItem(placement: .topBarTrailing) { perchContent(circleAt(2)) }
                        .sharedBackgroundVisibility(.hidden)
                } else {
                    ToolbarItem(placement: .topBarTrailing) { perchContent(circleAt(2)) }
                }
            }
            // The bell opens the CONVERGENCE deck on REAL proposals: things your graph holds
            // that you have not accepted yet. Accepting mints a Thing GroupObject, which is
            // what puts it in the LIFE market.
            .fullScreenCover(isPresented: $trayOpen) {
                ConvergenceView(proposals.pending,
                                onDecide: { p, d in proposals.decide(p, d, core: coreService, graph: graph) },
                                onDone: { trayOpen = false })
            }
            // Recompute on entry and after every graph change — proposals are DERIVED from
            // (graph entities ⊖ minted things), so there is nothing to keep in sync.
            .onChange(of: trayOpen) { _, open in if open { rebuildProposals() } }
            // A decision taken from the notification banner, applied here because this is where
            // both stores live. The store re-checks that the proposal is still pending, so a
            // banner acted on twice (or after the deck already handled it) is a no-op.
            .onReceive(NotificationCenter.default.publisher(for: .pacificProposalDecision)) { n in
                guard let id = n.userInfo?["proposal"] as? String,
                      let accept = n.userInfo?["accept"] as? Bool else { return }
                proposals.decide(proposalId: id, accept: accept, core: coreService, graph: graph)
                rebuildProposals()
            }
            .task(id: graph.epoch) { rebuildProposals() }
    }

    @EnvironmentObject private var shellState: ShellState

    private var accountCorner: some View {
        AccountBarButton(photo: coreService.avatarPhoto, color: coreService.avatarColor) {
            shellState.openSettings()
        }
        .accessibilityIdentifier("account-button")
    }

    // MARK: - Top-circle precession

    /// The three circles that ride the top bar, in their home order (NOTE tab).
    private enum TopCircle: Int, CaseIterable, Hashable { case create = 0, bell = 1, account = 2 }

    /// NOTE = 0 · CHAT = 1 · LIFE = 2 — one precession step per tab step.
    private var tabIndex: Int {
        switch shellState.screen {
        case .notes: 0
        case .chat: 1
        case .life: 2
        }
    }

    /// The circle perched at `slot` (0 = west · 1 = east-inner · 2 = east corner):
    /// circle c sits at perch (c + tabIndex) mod 3, so every tab step advances the
    /// whole trio one perch and a full tab cycle is one full revolution.
    private func circleAt(_ slot: Int) -> TopCircle {
        TopCircle(rawValue: (((slot - tabIndex) % 3) + 3) % 3)!
    }

    /// One perch's occupant. The swap animates as a scale-through morph keyed to the
    /// tab — the discrete face of the precession (toolbar items cannot glide between
    /// placements, so the revolution reads as each perch handing to its neighbour).
    @ViewBuilder private func perchContent(_ c: TopCircle) -> some View {
        ZStack {
            switch c {
            case .create:
                if let onNew {
                    Button(action: onNew) {
                        Image(systemName: "plus").fontWeight(.semibold)
                            .foregroundStyle(newTint ?? Brand.accent)
                    }
                    .accessibilityLabel(newLabel)
                    .accessibilityIdentifier("tab-new")
                }
            case .bell:
                if showsNotifications {
                    BellButton { trayOpen = true }
                }
            case .account:
                accountCorner
            }
        }
        .id(c)
        .transition(.scale(scale: 0.6).combined(with: .opacity))
        .animation(.spring(response: 0.35, dampingFraction: 0.8), value: tabIndex)
    }

    /// Proposals are DERIVED: graph entities (people, orgs, things),
    /// minus objects already minted under the same name — any kind, because a person
    /// minted as a Group must not be re-proposed as a Thing. Recomputing is cheap and
    /// idempotent, so it runs on every graph change and on every open.
    private func rebuildProposals() {
        coreService.refreshThings()
        let mintedNames = Set(coreService.objects
            .filter { ["group", "thing", "place", "project"].contains($0.kind) }
            .map { $0.name.lowercased() })
        proposals.rebuild(resolved: graph.cachedEntities, mintedNames: mintedNames)
    }
}

extension View {
    /// The shared tab top bar: an optional right-aligned `title`, a universal `+` (its `onNew`
    /// creates that tab's object), and — when `showsNotifications` — the notification-tray bell
    /// to the `+`'s right. `newLabel` is the `+` button's accessibility label; `newTint` colours
    /// the `+` glyph (LIFE tints it to the selected category).
    /// `onNew` is optional: pass nil where the screen has nothing to create (LIFE does this
    /// for categories with no wizard) and the `+` is omitted entirely.
    func tabChrome(_ title: String, newLabel: String,
                   showsTitle: Bool = true, showsNotifications: Bool = false,
                   newTint: Color? = nil,
                   onNew: (() -> Void)? = nil) -> some View {
        modifier(TabChrome(title: title, newLabel: newLabel,
                           showsTitle: showsTitle, showsNotifications: showsNotifications,
                           newTint: newTint, onNew: onNew))
    }

    /// The same top bar for a screen that CREATES nothing — title (and optionally the
    /// bell), no `+`.
    func tabChrome(_ title: String, showsTitle: Bool = true, showsNotifications: Bool = false) -> some View {
        modifier(TabChrome(title: title, newLabel: "",
                           showsTitle: showsTitle, showsNotifications: showsNotifications,
                           newTint: nil, onNew: nil))
    }
}

/// The tab-chrome bell — turns RED when suggested mints are waiting. The bell
/// opens the CONVERGENCE deck (`proposals.pending`), so it colours off the SAME store, not the
/// OutputStore it used to read (which the deck never showed — the badge was dead).
private struct BellButton: View {
    @EnvironmentObject private var proposals: ProposalStore
    let open: () -> Void
    private var pending: Int { proposals.pending.count }
    var body: some View {
        Button(action: open) {
            // An accent circle (white on slate, ink on paper), the radius of the glyph
            // it replaces; the WHOLE disc turns red when there are suggested mints to
            // review — no satellite dot.
            Circle()
                .fill(pending > 0 ? Color.red : Theme.accent)
                .frame(width: 17, height: 17)
        }
        .accessibilityLabel(pending > 0 ? "Notifications, \(pending) to review" : "Notifications")
        .accessibilityIdentifier("tab-notifications")
    }
}

/// The notification tray — where ticket arrivals surface. Confirming opens the wallet;
/// dismissing drops the item. Empty is honest — never a fabricated item.
struct NotificationTray: View {
    @EnvironmentObject private var outputs: OutputStore
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            Group {
                if outputs.outputs.isEmpty {
                    ContentUnavailableView(
                        "No notifications",
                        systemImage: "bell",
                        description: Text("Tickets will appear here.")
                    )
                } else {
                    List {
                        ForEach(outputs.outputs) { output in
                            OutputCard(
                                output: output,
                                confirm: { confirm(output) },
                                dismissAction: { outputs.remove(output) }
                            )
                            .listRowBackground(Brand.card)
                        }
                    }
                    .scrollContentBackground(.hidden)
                }
            }
            .background(Theme.bg)
            .navigationTitle("Notifications")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) { Button("Done") { dismiss() } }
            }
        }
    }

    /// A human confirms ONE tray item. A ticket is a VIEW: the shell presents the wallet (the
    /// credential itself already landed), and the item leaves the tray.
    private func confirm(_ output: Output) {
        switch output {
        case .ticket:
            NotificationCenter.default.post(name: .pacificShowTickets, object: nil)
        }
        outputs.remove(output)
    }
}

/// One tray card — its shape follows the Output kind. A ticket offers VIEW; Dismiss clears it.
/// Label or value only — no captions (see the house UI rule).
private struct OutputCard: View {
    let output: Output
    let confirm: () -> Void
    let dismissAction: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            kind
            HStack {
                Spacer()
                Button("Dismiss", role: .cancel, action: dismissAction)
                    .buttonStyle(.pixelSecondary)
                if let label = confirmLabel {
                    Button(label, action: confirm)
                        .buttonStyle(.pixel)
                        .accessibilityIdentifier("output-confirm")
                }
            }
            .controlSize(.small)
        }
        .padding(.vertical, 4)
        .accessibilityIdentifier("output-card")
    }

    @ViewBuilder private var kind: some View {
        switch output {
        case let .ticket(t):
            tag("Ticket", "ticket", Brand.accent)
            Text(t.title.isEmpty ? String(t.ticketId.prefix(8)) : t.title).font(.headline)
        }
    }

    private func tag(_ text: String, _ symbol: String, _ color: Color) -> some View {
        Label(text, systemImage: symbol)
            .font(.caption.weight(.semibold))
            .foregroundStyle(color)
    }

    private var confirmLabel: String? {
        switch output {
        case .ticket: return "VIEW"
        }
    }
}

// MARK: - Previews for the shared tab chrome

#Preview("Chats top bar — plain +, separate bell") {
    NavigationStack {
        Theme.bg.ignoresSafeArea()
            .tabChrome("Chats", newLabel: "New chat", showsTitle: false, showsNotifications: true) {}
    }
    .environmentObject(OutputStore())
}

#Preview("Notes top bar — title + plus, no bell") {
    NavigationStack {
        Theme.bg.ignoresSafeArea()
            .tabChrome("Notes", newLabel: "New note") {}
    }
}

#Preview("Notification tray") {
    NotificationTray()
        .environmentObject(OutputStore())
}

/// The union of the on-screen frames of rows that own their horizontal swipe (swipe-to-
/// delete), reported in the shell's coordinate space. The shell's page-swipe refuses to page
/// when a drag STARTS inside this region — so a swipe on a row stages that row's delete, and
/// a swipe anywhere else (the search field, the empty space below the last row) still pages.
///
/// A REGION, not a flag. The flag this replaces was set by a `DragGesture` racing the row's
/// own swipe, and inside a `List` cell that race is unwinnable: UIKit's swipe-action
/// recogniser claims the horizontal drag and SwiftUI's gesture never fires at all, so the
/// flag stayed false and the shell paged out from under the delete. Geometry doesn't race —
/// the region is known before the finger lands. Only VISIBLE rows report, which is exactly
/// right: a row that is scrolled off cannot be swiped.
struct PageSwipeBlockKey: PreferenceKey {
    static var defaultValue: CGRect = .zero
    static func reduce(value: inout CGRect, nextValue: () -> CGRect) {
        let next = nextValue()
        guard next != .zero else { return }
        value = value == .zero ? next : value.union(next)
    }
}

extension View {
    /// Mark a ROW that owns its horizontal swipe, so the shell's page-swipe leaves drags
    /// that begin on it alone. It publishes the row's frame, so putting it on a whole list
    /// would block paging across that list's entire area — which is the bug it exists to fix.
    func blocksPageSwipe() -> some View {
        background(
            GeometryReader { p in
                Color.clear.preference(key: PageSwipeBlockKey.self,
                                       value: p.frame(in: .named("shell")))
            }
        )
    }
}

extension View {
    /// SWIPE LEFT TO REMOVE — the one definition, used by a note, a chat and a room.
    ///
    /// It carries all three parts together on purpose. The MASK (`blocksPageSwipe`) has to
    /// ride the same rows as the gesture, or a drag meant for the row pages the tab
    /// instead. The CONFIRM has to be here too: this is the only gesture in the app that
    /// destroys something, a swipe is easy to make by accident on a list you are
    /// scrolling, and a per-site dialog is how three surfaces end up asking three
    /// different questions.
    ///
    /// `title` names the thing ("Leave Ada Okoro?", "Delete this note?") — never "this
    /// item" — and `verb` is both the swipe's word and the dialog's button, so they
    /// cannot drift apart.
    func swipeToRemove(title: String, verb: String = "Delete",
                       perform: @escaping () -> Void) -> some View {
        modifier(SwipeToRemove(title: title, verb: verb, perform: perform))
    }
}

private struct SwipeToRemove: ViewModifier {
    let title: String
    let verb: String
    let perform: () -> Void
    @State private var asking = false

    func body(content: Content) -> some View {
        content
            .blocksPageSwipe()
            .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                Button(role: .destructive) { asking = true } label: {
                    Label(verb, systemImage: "trash")
                }
                .accessibilityIdentifier("row-remove")
            }
            .confirmationDialog(title, isPresented: $asking, titleVisibility: .visible) {
                Button(verb, role: .destructive, action: perform)
                Button("Cancel", role: .cancel) {}
            }
    }
}

/// The floating jewel LIFE drags. Position is the jewel's CENTRE, in global space.
@MainActor final class JewelDragModel: ObservableObject {
    @Published var visible = false
    @Published var point: CGPoint = .zero
    @Published var fading = false
    var glyph = ""
    var color: Color = .clear
    var size: CGFloat = 56

    func begin(glyph: String, color: Color, size: CGFloat, at p: CGPoint) {
        self.glyph = glyph; self.color = color; self.size = size
        point = p; fading = false; visible = true
    }
}

/// Drawn over everything the shell shows.
private struct DraggedJewelOverlay: View {
    @ObservedObject var model: JewelDragModel

    var body: some View {
        GeometryReader { p in
            if model.visible {
                let f = p.frame(in: .global)
                Image(systemName: model.glyph)
                    .font(.system(size: 19, weight: .bold))
                    .foregroundStyle(.white)
                    .frame(width: model.size, height: model.size)
                    .background(Circle().fill(model.color))
                    .scaleEffect(model.fading ? 1 : 1.08)
                    .opacity(model.fading ? 0 : 1)
                    .shadow(color: .black.opacity(0.55), radius: 12, y: 5)
                    .position(x: model.point.x - f.minX, y: model.point.y - f.minY)
            }
        }
        .allowsHitTesting(false)
        .ignoresSafeArea()
    }
}

struct RootShell: View {
    @EnvironmentObject private var enrich: EnrichmentService
    @EnvironmentObject private var core: CoreService
    @EnvironmentObject private var system: SystemService
    @EnvironmentObject private var notes: NotesStore
    @EnvironmentObject private var arcs: ArcStore            // dev seam: arc-join selects/joins an Arc
    @EnvironmentObject private var membership: ArcMembership // dev seam: the headless tether
    @EnvironmentObject private var outputs: OutputStore      // the tray — ticket arrivals land here
    @StateObject private var shell = ShellState()
    /// The ticket wallet, presented by a tray VIEW or the Settings TICKETS card.
    @State private var showTickets = false
    /// A ticket to open full-screen with the wallet (dev seam: `show-tickets?ticket=`).
    @State private var presentTicket: String?
    /// The door, opened from the shell so a scan can be driven from any screen.
    @State private var doorScan: DoorLink?
    @State private var screen: Screen = .chat
    @State private var sharing = false
    /// The text handed to the share sheet, captured when Share is tapped — the draft is
    /// cleared once the card has left, and the sheet must not blank out underneath it.
    @State private var confirmDiscard = false
    @State private var openedRoom: RoomLink?
    /// The shell-level press-bloom, rendered UN-clipped over everything. `heldDisc` is the
    /// disc currently blooming (nil = none); `holdAmp` swells 0→bloomMax while held and
    /// decays on release; `discCenters` carries every disc's LIVE centre so the bloom
    /// follows the disc as the compose row slides.
    @State private var heldDisc: Int?
    @State private var holdAmp: Double = 0
    @State private var bloomColors: [Color] = []
    @State private var discCenters: [Int: CGPoint] = [:]
    /// Where rows that own their own horizontal swipe currently are (see `PageSwipeBlockKey`).
    /// A page-swipe that starts in here belongs to the row, not to the shell.
    @State private var pageSwipeBlock: CGRect = .zero
    /// Bumped on every press/release so a STALE decay completion can't tear down a bloom
    /// that a newer press already owns.
    @State private var bloomGen = 0

    /// The persistent screens the bottom row swaps between — `ShellScreen`, the type the
    /// interface matrix speaks. NOTES is the page of past notes (the old Notes tab) —
    /// reached by HOLDING the NOTE disc, since a tap on it is the compose action
    /// (`showNote`), not a destination.
    typealias Screen = ShellScreen

    /// Row geometry: the gap between the discs at rest, and the margin the outer two
    /// stop at when the row becomes the compose card's buttons — held well clear of the
    /// screen edges rather than pinned to them.
    private static let restGap: CGFloat = DesignTokens.Discs.restGap
    private static let rowEdge: CGFloat = DesignTokens.Discs.rowEdge

    /// Bloom amplitude cap (shader progress). The bloom fires on touch-down and swells
    /// toward this — logarithmically (an easeOut deceleration) for as long as the finger
    /// is held — then decays on release.
    private static let bloomMax: Double = 0.3

    /// Touch-down binds the bloom to a disc and swells it; release decays it. The disc's
    /// position comes from `discCenters` (live), so the bloom tracks it as the row slides.
    private func handlePress(_ id: Int, _ pressed: Bool, _ colors: [Color]) {
        guard id >= 0, !colors.isEmpty else { return }
        bloomGen &+= 1
        let gen = bloomGen

        if pressed {
            heldDisc = id
            bloomColors = colors
            // EVERY press starts from nothing. Without this the swell re-targets from
            // whatever amplitude the last press left behind — press again inside the
            // decay and the bloom appears already at max instead of building. Snap to 0
            // with animations disabled, then swell on the next runloop turn so the zero
            // is actually committed before the animation reads its "from" value.
            var reset = Transaction()
            reset.disablesAnimations = true
            withTransaction(reset) { holdAmp = 0 }
            DispatchQueue.main.async {
                guard gen == bloomGen else { return }   // released / re-pressed already
                withAnimation(.easeOut(duration: 2.0)) { holdAmp = Self.bloomMax }
            }
        } else {
            withAnimation(.easeOut(duration: 0.55)) { holdAmp = 0 } completion: {
                guard gen == bloomGen else { return }   // a newer press owns the bloom now
                heldDisc = nil
            }
        }
    }

    /// The Metal press-bloom, placed at the HELD disc's LIVE centre so it moves with the
    /// disc, and drawn over everything so nothing clips it (see Bloom.metal).
    @ViewBuilder private var bloomOverlay: some View {
        if let id = heldDisc, let center = discCenters[id], holdAmp > 0.001, !bloomColors.isEmpty {
            let box: CGFloat = 240
            Rectangle()
                .fill(.black)
                .frame(width: box, height: box)
                .colorEffect(ShaderLibrary.discBloom(
                    .float2(Float(box), Float(box)),
                    .float(Float(holdAmp)),
                    .color(bloomColors[0]),
                    .color(bloomColors.count > 1 ? bloomColors[1] : bloomColors[0])
                ))
                .position(center)
                .blendMode(.plusLighter)
                .allowsHitTesting(false)
        }
    }

    var body: some View {
        // ONE body expression had grown past the type-checker's budget; splitting
        // it is the documented remedy. Same modifiers, same order — only the
        // expression boundary moved (`shellCore` = layout + chrome; `hooked` =
        // the NotificationCenter hooks and the covers they present).
        hooked(shellCore)
    }

    private var shellCore: some View {
        // The disc row is a SIBLING of the screen, never a modifier on it. It used to ride
        // as a bottom safe-area inset, and that anchored it only by accident: the screens
        // were a paged TabView, which slid its own internals one level BELOW the view the
        // inset hung off. Paging is now a `.transition` on `currentScreen` itself (the
        // TabView had to go — it crashed UIKit's nav bars; see `currentScreen`), so anything
        // attached to that view travels with the slide. A ZStack sibling is out of the
        // transition's reach, which anchors the row by construction rather than by luck.
        // The space the inset used to reserve is reserved by the screen instead — see
        // `shellBottomBarClearance` on `currentScreen`. Inside an immersive conversation the
        // row collapses away entirely — the chat owns the screen, Signal-style, and its
        // composer sits at the natural bottom.
        ZStack(alignment: .bottom) {
            currentScreen
                // The `New` surface goes UNDER the row, because those discs are its
                // buttons — it stops just above them.
                .overlay { newOverlay }
                // Settings goes UNDER the row too — the discs ARE its chrome: NOTE slides to
                // the centre and becomes its dismiss button.
                .overlay { settingsOverlay }

            // The row STAYS PUT and fades where it stands — it never slides away. It is
            // kept in the tree (not removed) so nothing reflows underneath it; the open
            // conversation is drawn OVER it, so the widening composer covers it.
            buttonRow
                .opacity(shell.immersive ? 0 : 1)
                .allowsHitTesting(!shell.immersive)
                // The bearing number (000–360) printed just above the yellow CHAT disc —
                // centred over the row, only on the map, at rest.
                // Printed just above the yellow CHAT disc, so it travels with the
                // row's band — centred on the left third on the split.
                .overlay(alignment: .topLeading) {
                    if compassVisible {
                        GeometryReader { g in
                            CompassReadout()
                                .frame(width: Self.isPad ? g.size.width / 3 : g.size.width)
                                .offset(y: -8)
                        }
                        .allowsHitTesting(false)
                    }
                }
        }
        // The row's fade is the other half of the composer's disc⇄field morph and starts
        // on the same frame (see Motion).
        .animation(shell.immersive ? Motion.rowOut : Motion.rowIn, value: shell.immersive)
        // ONE ground under the whole shell, the row's band included. Without it the band
        // is not the app's background at all: a screen's own background stops at its
        // content, so the row's fade-to-`Theme.bg` gradient was compositing over the
        // window's BLACK and landing visibly darker than the screen above it. Over this,
        // the gradient is the same colour at every opacity — invisible except where
        // scrolling content passes under it, which is the only thing it is there for.
        .background(Theme.bg.ignoresSafeArea())
        // The conversation goes ABOVE the row — applied after the inset, so the composer
        // stretching out of the CHAT disc covers NOTE and LIFE rather than blending with
        // them. This is also why the shell owns the conversation and not ChatsView.
        .overlay { conversationOverlay }
        // The press-bloom is drawn OVER everything at the shell level so the disc row's
        // bounds can't clip it; discs report their centre in this "shell" space.
        .overlay { bloomOverlay }
        // Above everything, including the disc row — a dragged jewel must never be clipped.
        .overlay { DraggedJewelOverlay(model: shell.jewelDrag) }
        .coordinateSpace(.named("shell"))
        // The preloaded screens' arrival signal (see ShellState.screen).
        .onChange(of: screen) { _, s in shell.mirrorScreen(s) }
        .onPreferenceChange(DiscCenterKey.self) { discCenters = $0 }   // discs' live centres
        .onPreferenceChange(PageSwipeBlockKey.self) { pageSwipeBlock = $0 }   // swipeable rows
        // A horizontal swipe pages between screens (see `pageSwipe`). Simultaneous +
        // low-distance so taps and vertical scrolls still work.
        .simultaneousGesture(pageSwipe)
        .environmentObject(shell)
        .sheet(item: $enrich.inspected) { entity in EntityRouter(entity: entity) }
        .sheet(isPresented: $sharing) {
            PlacePublishSheet { placeId in publishFinished(to: placeId) }
        }
        .confirmationDialog(discardPrompt, isPresented: $confirmDiscard, titleVisibility: .visible) {
            Button("Discard", role: .destructive) { discardNew() }
            Button("Keep editing", role: .cancel) {}
        }
        .onOpenURL { DevLinks.handle($0, core: core, notes: notes, arcs: arcs, membership: membership) }   // dev/demo seam — real core calls only
        #if DEBUG
        // The same seam, without SpringBoard: iOS 26 confirms every `simctl openurl` with
        // an alert no script can tap, so a host harness appends its links to a file in the
        // container and this drains them into the identical handler. See DevCommands.
        .task {
            await DevCommands.watch {
                DevLinks.handle($0, core: core, notes: notes, arcs: arcs, membership: membership)
            }
        }
        #endif
        .task {
            shell.split = Self.isPad     // LIFE is permanent furniture on the split
            _ = KeyboardMotion.shared   // observe keyboard timing early
            // The shell owns NotesStore and the Place-publish sheet, so it is the one
            // place a NOTE session can be built. Installed here so the Notes page's `+`
            // and the RED disc raise the identical surface.
            shell.makeNoteSession = { noteSession() }
            #if DEBUG
            // Headless drive: seed the two real launch events (real mints, real posters)
            // and land on EVENTS when asked — simctl can't tap the LIFE disc.
            EventSeeder.seedIfRequested(core: core)
            if EventSeeder.wantsEventsScreen { show(.life) }
            #endif
            core.warmCaches()           // re-ask the graph (n-2) so every shelf is warm
            await core.autoSyncLoop()   // background poll so messages arrive on their own
        }
    }

    /// The back half of `body`'s modifier chain: every NotificationCenter hook and
    /// the covers they present. Split from `body` so neither expression overwhelms
    /// the type-checker; behaviour is byte-identical to the single chain.
    private func hooked(_ v: some View) -> some View {
        v
        .onReceive(NotificationCenter.default.publisher(for: .pacificOpenProject)) { _ in
            // Projects now live under LIFE (it absorbed Work) — jump to that screen.
            show(.life)
        }
        .onReceive(NotificationCenter.default.publisher(for: .pacificOpenGroup)) { _ in
            // A room title's deep link: the group's page lives on LIFE — jump there;
            // LifeView pushes the page itself.
            show(.life)
        }
        .onReceive(NotificationCenter.default.publisher(for: .pacificOpenEventRow)) { _ in
            // An RSVP reminder's tap: the row lives on LIFE's events page — jump there;
            // LifeView opens the row itself.
            show(.life)
        }
        .onReceive(NotificationCenter.default.publisher(for: .pacificShowLife)) { _ in
            show(.life)   // headless drive → LIFE, for the graph screenshot
        }
        .onReceive(NotificationCenter.default.publisher(for: .pacificShowNotes)) { _ in
            show(.notes)  // headless drive → the Notes list, to confirm the note landed
        }
        .onReceive(NotificationCenter.default.publisher(for: .pacificShowAccount)) { _ in
            shell.openSettings()   // headless drive → the account/Settings overlay
        }
        .onReceive(NotificationCenter.default.publisher(for: .pacificShowTickets)) { note in
            // A tray VIEW → the wallet. `ticket` (dev seam) opens one full-screen, since
            // a headless drive cannot tap a row.
            presentTicket = note.userInfo?["ticket"] as? String
            showTickets = true
        }
        // The ticket sweeps ride the sync tick: new deliveries → the tray; pending free
        // requests against listings this device owns → fulfilled (the phone-fulfilled path).
        .onReceive(core.$syncTick) { _ in
            core.refreshEvents()
            TicketSweep.arrivals(core: core, outputs: outputs)
            Task { await TicketSweep.fulfilFree(core: core) }
        }
        .sheet(isPresented: $showTickets) {
            NavigationStack { TicketWalletView(present: presentTicket) }
        }
        .onReceive(NotificationCenter.default.publisher(for: .pacificDoorScan)) { note in
            // The door, opened at the shell so it works from any screen — and carrying the
            // scanned payload when a simulator (no camera) is standing in for the lens.
            guard let id = note.userInfo?["object"] as? String else { return }
            doorScan = DoorLink(id: id, qr: note.userInfo?["qr"] as? String)
        }
        .fullScreenCover(item: $doorScan) { link in
            DoorScanSheet(objectId: link.id, injectedQR: link.qr)
        }
        .onReceive(NotificationCenter.default.publisher(for: .pacificOpenRoom)) { note in
            if let id = note.userInfo?["object"] as? String {
                openedRoom = RoomLink(id: id,
                                      title: note.userInfo?["title"] as? String,
                                      group: note.userInfo?["group"] as? String)
            }
        }
        .onReceive(NotificationCenter.default.publisher(for: .pacificSysiphusSignIn)) { note in
            if let u = note.userInfo?["u"] as? String {
                let p = note.userInfo?["p"] as? String ?? "aries26"
                Task { await system.signIn(username: u, password: p) }
            }
        }
        .onReceive(NotificationCenter.default.publisher(for: .pacificTestBloom)) { _ in
            // Dev seam: simulate a press-bloom on LIFE (at its live centre) so it can be
            // screenshotted headlessly — swell while "held", then release to decay.
            handlePress(2, true, [core.avatarColor, DiscSlot.life.blockColor])
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { handlePress(2, false, [core.avatarColor, DiscSlot.life.blockColor]) }
        }
        .fullScreenCover(item: $openedRoom) { link in
            NavigationStack {
                ThreadView(objectId: link.id, title: link.title ?? "Channel")
                    .toolbar {
                        // A HOSTED room's title is the way back to its group: it
                        // deep-links to the group's page on LIFE (the cover closes;
                        // `.pacificOpenGroup` flips the screen and pushes the page).
                        // Same words the plain title wears — a door, not a label.
                        if let group = link.group {
                            ToolbarItem(placement: .principal) {
                                Button {
                                    openedRoom = nil
                                    NotificationCenter.default.post(
                                        name: .pacificOpenGroup, object: nil,
                                        userInfo: ["object": group])
                                } label: {
                                    Text(link.title ?? "Channel")
                                        .font(.headline)
                                        .foregroundStyle(Theme.textPrimary)
                                }
                                .accessibilityIdentifier("room-group-link")
                            }
                        }
                        ToolbarItem(placement: .topBarTrailing) {
                            Button("Done") { openedRoom = nil }
                        }
                    }
                    // The room cover registered NO destinations, so a post's
                    // author had nowhere to lead even once it became a link.
                    // Same two the conversation overlay carries.
                    .navigationDestination(for: PersonLink.self) { link in
                        PersonProfilePage(id: link.id,
                                          source: RealProfileSource(core: core, subject: .peer(link.id)))
                    }
                    .navigationDestination(for: GroupLink.self) { link in
                        GroupDetailView(objectId: link.id)
                    }
            }
        }
    }

    /// The order the discs sit in, which is the order a swipe moves through.
    private static let screenOrder: [Screen] = [.notes, .chat, .life]

    /// The screen the bottom row lands you on. NOTES is `NotesView` (your notes, newest
    /// first — the page the retired Notes tab used to be); CHAT is `ChatsView` (chats +
    /// group chats, unchanged); LIFE is `LifeView` (people · places · groups · projects ·
    /// events — it replaced Work and absorbed the Outline).
    ///
    /// ONE screen mounted at a time (a switch), NOT a paged `TabView`. A paged TabView
    /// wraps the screens in a `UIPageViewController` and, because each screen carries its
    /// own `NavigationStack` + toolbar, UIKit recycled the navigation bars into an
    /// inconsistent state and aborted in `-[UINavigationBar layoutSubviews]` — the
    /// confirmed cause of the on-device NOTE-tap crash. A swipe still moves between them
    /// (see the drag gesture on the body), but as a slide between single-mounted screens,
    /// so only one navigation bar is ever alive.
    /// The iPad's layout: LIFE is permanent furniture, not a page. Decided by the
    /// IDIOM rather than the size class — a size class also turns .regular on a Max
    /// phone in landscape, where a three-column split has no room to be anything but
    /// cramped, and "on iPad" is the actual rule.
    static let isPad = UIDevice.current.userInterfaceIdiom == .pad

    /// The two pages that share the split's left third.
    private static let panelOrder: [Screen] = [.notes, .chat]

    /// Set `screen` safely under either layout. On the split, LIFE is always up and
    /// `screen` names only the left panel — so "show LIFE" is not a page turn, it is
    /// the blue disc turning the face over (its phone second-tap behaviour, which on
    /// iPad is its ONLY behaviour). Every caller goes through here so no headless
    /// drive or deep link can park `.life` in a panel that cannot render it.
    private func show(_ s: Screen) {
        // BLUE pressed while LIFE is already up: back to the Feed (10 Aug order) —
        // LIFE minimises whatever jewel owns the ground and returns to its ground
        // state. The second tap is a HOME gesture, not a no-op.
        if s == .life && screen == .life {
            NotificationCenter.default.post(name: .pacificLifeHome, object: nil)
        }
        // Arriving at CHAT from anywhere else always lands on CHATS. Without this
        // the gold disc's two-stop rotary would remember where it was left, and
        // "first tap lands on Chat" would be false on every return visit.
        if s == .chat && screen != .chat { shell.chatScope = .chats }
        // On the split LIFE is always up — "show LIFE" swaps no panel, but the home
        // gesture above still fires.
        if Self.isPad && s == .life { return }
        screen = s
    }

    @ViewBuilder
    private var currentScreen: some View {
        if Self.isPad { splitScreen } else { pagedScreen }
    }

    /// iPad: NOTE·CHAT in the left third, LIFE holding the right two thirds for good.
    /// All three stay mounted exactly as the phone's pager keeps them — the panel's
    /// parked page is inert (not hittable, invisible to accessibility) so UI-test
    /// "does not exist" assertions stay honest.
    private var splitScreen: some View {
        GeometryReader { geo in
            HStack(spacing: 0) {
                ZStack {
                    ForEach(Self.panelOrder, id: \.self) { s in
                        pageContent(s)
                            .shellBottomBarClearance()
                            .opacity(screen == s ? 1 : 0)
                            .allowsHitTesting(screen == s)
                            .accessibilityHidden(screen != s)
                    }
                }
                .frame(width: geo.size.width / 3)

                Rectangle()
                    .fill(Theme.hairline)
                    .frame(width: 1)
                    .ignoresSafeArea()

                LifeView()
                    .shellBottomBarClearance()
                    .frame(width: geo.size.width * 2 / 3)
            }
        }
    }

    @ViewBuilder
    private var pagedScreen: some View {
        // ALL THREE screens stay mounted, always — swapping used to rebuild a screen
        // from scratch on every page turn (`switch` + `.id(screen)`), and LIFE's mount
        // is expensive, which made CHAT↔LIFE feel heavy. Preloaded, a page turn is
        // just this offset animating; a screen's state — scroll positions, pushed
        // navigation, the LIFE graph snapshot, loaded images — survives parking.
        // Parked screens are inert: not hittable, invisible to accessibility (which
        // keeps "does not exist" UI-test assertions honest). Direction falls out of
        // the geometry, so there is no `pageForward` to set any more.
        GeometryReader { geo in
            ZStack {
                ForEach(Array(Self.screenOrder.enumerated()), id: \.element) { i, s in
                    pageContent(s)
                        // The row is a ZStack sibling, so it reserves no space of its
                        // own — each screen reserves it (see `ShellMetrics`); pushed
                        // screens re-reserve it themselves.
                        .shellBottomBarClearance()
                        .offset(x: CGFloat(i - currentPageIndex) * geo.size.width)
                        .allowsHitTesting(screen == s)
                        .accessibilityHidden(screen != s)
                }
            }
        }
    }

    @ViewBuilder private func pageContent(_ s: Screen) -> some View {
        switch s {
        case .notes: NotesView()
        case .chat:  ChatsView()
        case .life:  LifeView()
        }
    }

    private var currentPageIndex: Int {
        Self.screenOrder.firstIndex(of: screen) ?? 1
    }

    /// A horizontal swipe pages between the screens, in disc order. Attached to the whole
    /// shell as a low-priority simultaneous gesture so it never pre-empts a tap; the
    /// distance + horizontal-dominance thresholds keep a deliberate page-fling distinct
    /// from a list's own short swipe-to-delete or vertical scroll. Off while composing a
    /// note or inside a conversation — those own the screen.
    private var pageSwipe: some Gesture {
        // In "shell" space, so `startLocation` can be tested against the swipeable-row
        // region reported in that same space (see `PageSwipeBlockKey`).
        DragGesture(minimumDistance: 24, coordinateSpace: .named("shell"))
            .onEnded { v in
                // Nothing to page on the split: LIFE is furniture and the panel is
                // swapped by the discs, so a horizontal drag belongs to whatever
                // content it started on.
                guard !Self.isPad else { return }
                guard !shell.suppressPageSwipe,
                      !shell.composing, !shell.immersive, shell.conversation == nil else { return }
                // A drag that BEGAN on a row which owns its own swipe belongs to that row —
                // it is staging a delete, and paging out from under it loses the gesture.
                // Tested on the start point, not the current one: a swipe that travels off
                // the row is still the row's.
                guard !pageSwipeBlock.contains(v.startLocation) else { return }
                guard abs(v.translation.width) > 80,
                      abs(v.translation.width) > 2 * abs(v.translation.height) else { return }
                page(v.translation.width < 0 ? 1 : -1)
            }
    }

    /// Move `step` screens along the disc order, clamped at the ends. The slide
    /// direction is geometric now — every screen sits at its own offset.
    private func page(_ step: Int) {
        guard let i = Self.screenOrder.firstIndex(of: screen) else { return }
        let j = i + step
        guard Self.screenOrder.indices.contains(j) else { return }
        withAnimation(.easeInOut(duration: 0.28)) { screen = Self.screenOrder[j] }
    }

    /// The open conversation, drawn over everything the shell shows — including the disc
    /// row, which is the point (see the `.overlay` above). It fades while `leaving` runs
    /// the collapse, and stops taking taps the moment it starts to go.
    @ViewBuilder
    private var conversationOverlay: some View {
        if let convo = shell.conversation {
            NavigationStack {
                ChatView(conversation: convo, composerExtended: shell.composerOpen)
                    .toolbar {
                        ToolbarItem(placement: .topBarLeading) {
                            Button { shell.closeConversation() } label: {
                                // A bare chevron is a ~17pt glyph — a tap target small
                                // enough to miss. Give it the 44pt minimum, and a
                                // contentShape so the whole square is live, not just
                                // where the strokes are.
                                Image(systemName: "chevron.left")
                                    .fontWeight(.semibold)
                                    .frame(width: 44, height: 44)
                                    .contentShape(Rectangle())
                            }
                            .accessibilityLabel("Back to chats")
                            .accessibilityIdentifier("chat-back")
                        }
                    }
            }
            // A back-swipe that BEGINS at the very left of the screen leaves the chat —
            // the Signal/iOS gesture. A UIKit pan on an ancestor (not a SwiftUI drag) so it
            // fires over the transcript's own ScrollView instead of losing the horizontal
            // drag to it; it only acts on a fling that STARTED in the left edge band, which
            // is dead margin the transcript never fights for. See `EdgeBackSwipe`.
            .background(EdgeBackSwipe { shell.closeConversation() })
            .opacity(shell.leaving ? 0 : 1)
            .allowsHitTesting(!shell.leaving)
            .transition(.opacity)
        }
    }

    /// The floating bottom row — THREE discs that never come and go, they only move and
    /// change identity:
    ///  • at rest — NOTE · CHAT · LIFE, evenly spaced and centred. NOTE composes on a tap
    ///    and opens your notes on a HOLD; CHAT and LIFE swap the shown screen, and each
    ///    disc lights up when its screen is the one showing.
    ///  • composing — X · Share · Send, X sliding out to the left edge and the other two
    ///    to the right, as the compose card fades up above them. They are that card's
    ///    buttons; there are none inside it.
    ///
    /// Each disc is ONE `CircleButton` whose arguments change, not two in an if/else —
    /// same view, same identity, so SwiftUI moves it rather than swapping one out for
    /// another. That is what makes it read as sliding.
    ///
    /// A soft slate gradient seats the discs over scrolling content without stealing taps.
    /// Settings, drawn UNDER the disc row (the discs are its chrome). Deliberately not a sheet:
    /// a sheet would cover the row, and the row is what dismisses it.
    @ViewBuilder private var settingsOverlay: some View {
        if shell.showAccount {
            Group {
                switch shell.rowOverlay {
                case .settings: MyProfileOverlay()
                case .newChat:  NewChatSheet(onOpen: { convo in
                                    shell.dismissSettings()
                                    shell.open(convo)
                                })
                case .connect:  ConnectSheet()
                }
            }
            .shellBottomBarClearance()
            .background(Theme.bg.ignoresSafeArea())
            .transition(.opacity)
        }
    }

    /// NOTE's resting centre → the row's centre (it is one disc + one gap left of centre).
    private var discToCentre: CGFloat { CircleButton.discSize + Self.restGap }

    /// Where each disc sits for the current Settings phase. 0 = NOTE, 1 = CHAT, 2 = LIFE.
    private func settingsOffset(_ i: Int, _ geo: GeometryProxy) -> CGFloat {
        let off = geo.size.width + CircleButton.discSize        // comfortably off-screen
        switch shell.settingsPhase {
        case .closed:    return 0
        case .open:      return i == 0 ? discToCentre : off     // NOTE centres, others exit right
        case .leaving:   return off                             // NOTE follows them off
        case .returning: return -off                            // parked left, no animation
        }
    }

    /// The curve each disc travels on. Exits ACCELERATE (easeIn); the return DECELERATES
    /// (easeOut) and is staggered LIFE → CHAT → NOTE, so the row arrives from the left in order.
    private func settingsAnimation(_ i: Int) -> Animation? {
        switch shell.settingsPhase {
        case .open:      return i == 0 ? .spring(response: 0.44, dampingFraction: 0.84)
                                       : .easeIn(duration: 0.34)
        case .leaving:   return .easeIn(duration: 0.30)
        case .returning: return nil                              // snap; nothing to see off-screen
        case .closed:    return .easeOut(duration: 0.55).delay([0.14, 0.07, 0.0][i])
        }
    }

    @EnvironmentObject private var location: LocationService

    /// The compass number shows only on the top-down map (LIFE → PLACES), at rest.
    private var compassVisible: Bool {
        shell.showCompass && !shell.composing && shell.settingsPhase == .closed && !shell.immersive
    }

    /// The interface context the row is in — DERIVED from shell state each render (never
    /// stored), so the row can never disagree with the screen it chromes. Settings wins
    /// over composing because `openSettings` only fires from rest.
    private var interfaceContext: InterfaceMatrix.Context {
        if shell.settingsPhase == .open { return .settings }
        if let session = shell.compose { return .composing(session.cell) }
        return .rest(screen, shell.chatScope)
    }

    /// The ONE router from a matrix cell's declared intent to its effect. The matrix
    /// stays a value table; everything a disc can do runs through here.
    private func perform(_ intent: DiscIntent) {
        switch intent {
        case .show(let s):     show(s)
        case .beginNew(let k): beginNew(k)
        case .newCancel:       cancelNew()
        // GOLD turns the surface to its VISIBILITY face and back. Universal across every
        // kind (Ralph, 18 Aug) — privacy/sharing is the same question whatever you are
        // making, so it is the same component and the same circle everywhere.
        case .newShare:        shell.compose?.gold()
        case .newCommit:       commitNew()
        case .dismissSettings: shell.dismissSettings()
        case .openNotesPage:   show(.notes)
        case .advanceChatScope:
            withAnimation(.easeOut(duration: 0.18)) { shell.chatScope = shell.chatScope.next }
        }
    }

    /// One disc, dressed by the matrix for the current context. Position, bloom and
    /// accessibility ids stay at the call site — choreography is the shell's, identity
    /// is the matrix's.
    private func disc(_ slot: DiscSlot, bloomColors: [Color]? = nil) -> some View {
        let spec = InterfaceMatrix.spec(slot, in: interfaceContext)
        return CircleButton(label: spec.label,
                            systemImage: spec.glyph,
                            blockColor: slot.blockColor,
                            showsGlyph: spec.showsGlyph,
                            bloomColors: bloomColors,
                            bloomID: slot.rawValue,
                            onPress: { id, p, cols in handlePress(id, p, cols) },
                            isActive: spec.isActive,
                            enabled: spec.enabled,
                            longPressAction: spec.hold.map { intent in { perform(intent) } },
                            longPressLabel: spec.holdLabel) {
            perform(spec.tap)
        }
    }

    private var buttonRow: some View {
        let composing = shell.composing
        return GeometryReader { geo in
            // The BAND the row lives in: the whole width on a phone, the LEFT THIRD on
            // the split — the discs belong over the panel they swap, not adrift across
            // LIFE. Everything below is measured against the band, so the composing
            // spread reaches the band's edges rather than the screen's.
            let band = Self.isPad ? geo.size.width / 3 : geo.size.width
            // The gaps carry the whole slide: widen the first one and the discs spread to
            // the edges; return it to the resting gap and they gather back to centre.
            let usable = band - 2 * Self.rowEdge
            let closing: CGFloat = composing ? DesignTokens.Discs.composeGap : Self.restGap
            let opening = composing
                ? max(Self.restGap, usable - 3 * CircleButton.discSize - closing)
                : Self.restGap

            HStack(spacing: 0) {
                disc(.note)
                    .accessibilityIdentifier("shell-note")
                    .offset(x: settingsOffset(0, geo))
                    .animation(settingsAnimation(0), value: shell.settingsPhase)

                gap(opening)

                disc(.chat)
                    .accessibilityIdentifier("shell-chat")
                    .offset(x: settingsOffset(1, geo))
                    .animation(settingsAnimation(1), value: shell.settingsPhase)

                gap(closing)

                // LIFE blooms with the profile block colour as well as blue.
                disc(.life, bloomColors: [core.avatarColor, DiscSlot.life.blockColor])
                    .accessibilityIdentifier("shell-life")
                    .offset(x: settingsOffset(2, geo))
                    .animation(settingsAnimation(2), value: shell.settingsPhase)
            }
            // Gathered at rest, edge-to-edge (of the band) when composing; the band
            // then sits leading, which is a no-op on a phone (band == full width).
            .frame(width: usable)
            .padding(.horizontal, Self.rowEdge)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .frame(height: CircleButton.discSize)
        .padding(.top, 24)
        .padding(.bottom, 6)
        .frame(maxWidth: .infinity)
        .background(
            LinearGradient(colors: [Theme.bg.opacity(0), Theme.bg.opacity(0.92)],
                           startPoint: .top, endPoint: .bottom)
                .allowsHitTesting(false)
                .ignoresSafeArea(edges: .bottom)
        )
        // Stay pinned to the physical bottom when a chat composer's keyboard rises — the
        // discs tuck behind the keyboard rather than floating up over the input. While
        // COMPOSING A NOTE they must do the opposite and ride above it: they are the
        // card's Send and Share, and a button you cannot reach while typing is no button.
        .ignoresSafeArea(.keyboard, edges: shell.composing ? [] : .bottom)
    }

    /// The space between the discs — which is what carries the slide, and is also OFF
    /// CANVAS while a note is being composed: the band the row sits in is not the card,
    /// so a tap here dismisses an empty note exactly as the ground above it does.
    ///
    /// It has to say so itself rather than letting the tap fall through to the card's
    /// ground beneath: the row is the shell's safe-area inset, and a touch landing in
    /// that region does not reach the content underneath it.
    private func gap(_ width: CGFloat) -> some View {
        Color.clear
            .frame(width: width)
            .contentShape(Rectangle())
            .onTapGesture {
                if let s = shell.compose, !s.needsDiscardWarning { cancelNew() }
            }
    }

    // MARK: - `New` actions (the row's three discs, while a surface is up)

    /// The `New` surface, under the disc row. Its own fade, not the discs' spring. It
    /// stays mounted while it is LEAVING so its departure can be seen (see `endNew`).
    ///
    /// This one line is what buys the circle choreography for EVERY kind: attached to
    /// `currentScreen` as an overlay, it sits under the row rather than over it, which
    /// is the whole difference between the note card and the five sheets it replaced.
    @ViewBuilder
    private var newOverlay: some View {
        if let surface = shell.compose ?? shell.leavingCompose {
            // One branch per surface the shell hosts. A short explicit table, not a
            // `body` on the protocol: a view is not something a value type can carry,
            // and two entries read better than the erasure that would avoid them.
            Group {
                if let session = surface as? NewSession {
                    NewSurface(session: session, onCancel: cancelNew,
                               creator: shell.creator?.sessions[session.kind] === session ? shell.creator : nil)
                } else if let detail = surface as? DetailSession {
                    DetailSurface(session: detail, onClose: cancelNew)
                }
            }
            .transition(.opacity)
            .animation(Motion.screenIn, value: shell.composing)
        }
    }

    /// RED at rest — the universal creator, opening on a note. Other screens' `+` still
    /// raise their own one kind through `shell.beginNew`.
    private func beginNew(_ kind: NewKind) {
        guard kind == .note else { return }
        shell.beginCreator()
    }

    /// The NOTE's session — the origin surface, rebuilt as one `NewSession` so it goes
    /// through exactly the same path as every other kind.
    ///
    /// GOLD is no longer "publish at a Place" here: it is the VISIBILITY face, like
    /// everywhere else. Publishing a note AT somewhere is a Place `post`, which is a
    /// different act from deciding who may see the note — conflating them was what made
    /// GOLD mean two things. The publish flow keeps its own path (`publishFinished`).
    private func noteSession() -> NewSession {
        NewSession(
            kind: .note,
            commit: { d in
                // NotesStore, not the core: a note is genuinely device-local. Saying so
                // in the one place that writes it is better than a `.note` special case
                // threaded through the mint seam.
                persistNote(title: d.name, body: d.body)
                return .saved
            }
        )
    }

    /// ✕ — discard. An empty surface just goes, quietly: it fades, no flash, no haptic.
    /// One with content in it warns first, because this is the one action here that
    /// destroys something and cannot be undone.
    private func cancelNew() {
        guard let session = shell.compose else { return }
        if session.needsDiscardWarning {
            confirmDiscard = true
        } else {
            shell.endNew(.cancelled)
        }
    }

    private func discardNew() { shell.endNew(.cancelled) }

    /// The warning names WHAT is about to be thrown away. "Discard this note?" was right
    /// when there was one kind; asking it over a half-written Space would be wrong.
    private var discardPrompt: String {
        "Discard this \(shell.compose?.discardNoun ?? "draft")?"
    }

    /// → — commit. The session owns what committing MEANS; the shell owns the departure.
    /// A failed commit (nil) leaves the surface up with its content intact, which is the
    /// only honest thing to do with a mint that did not happen.
    private func commitNew() {
        guard let session = shell.compose else { return }
        Task { @MainActor in
            guard let outcome = await session.performCommit() else { return }
            shell.endNew(outcome)
        }
    }

    /// …and only once a Place has ACTUALLY been chosen does the note leave: saved
    /// (publishing something is no reason to lose it), published AT the place as a
    /// signed `place.post`, and gone upward. A cancelled sheet never runs this.
    private func publishFinished(to placeId: String) {
        guard let session = shell.compose as? NewSession, session.kind == .note else { return }
        // The persisted note is the one source for the post — never the draft,
        // which clears under the departing card.
        let note = persistNote(title: session.draft.name, body: session.draft.body)
        Task {
            await core.publishNote(to: placeId,
                                   noteId: note.id.uuidString,
                                   title: note.title,
                                   text: note.body.isEmpty ? note.title : note.body)
        }
        shell.endNew(.shared)
    }

    @discardableResult
    private func persistNote(title: String, body: String) -> Note {
        var note = notes.create()
        note.title = title
        note.body = body
        notes.update(note)
        return note
    }
}

/// Shared shell geometry. The floating NOTE·CHAT·LIFE row is drawn once, over everything,
/// as an ANCHORED SIBLING of the current screen — so it reserves no space of its own.
/// Everything that has to clear it reserves this height at its own bottom instead: a tab's
/// root screen does it in `currentScreen`, and screens pushed inside a tab's NavigationStack
/// (e.g. a conversation's composer) do it for themselves, since a reservation made outside
/// the stack does not reach them. Derived, not restated: `buttonRow` is disc + top padding
/// (24) + bottom padding (6), so a resized disc carries its clearance with it.
enum ShellMetrics {
    static let bottomBarHeight: CGFloat = CircleButton.discSize + 30

    /// Where the shared search field's TOP lands, measured from the safe-area top: the
    /// nav-bar band (44) + the NavigationStack's own ~13pt margin under it — the same
    /// measurement `SearchField` documents. The NOTE compose card keys off this so its
    /// top edge sits exactly on the search bar it covers.
    static let searchFieldTop: CGFloat = 57
}

extension View {
    /// Applied to screens pushed inside a tab's NavigationStack so their bottom
    /// content clears the shell's floating NOTE·CHAT·LIFE discs (see `ShellMetrics`).
    func shellBottomBarClearance() -> some View {
        safeAreaInset(edge: .bottom, spacing: 0) {
            Color.clear
                .frame(height: ShellMetrics.bottomBarHeight)
                .ignoresSafeArea(.keyboard, edges: .bottom)
        }
    }

    /// The app's top bar is GLASS: the title and back button float over the
    /// screen's own full-bleed ground, and the system never paints the bar's
    /// scroll-edge material — the opaque slab that otherwise appears the moment
    /// content sits or scrolls under the bar, reading as "the background stops
    /// under the top bar". Pair with a ground that ignores the top safe area
    /// (every screen's ground must reach the physical top of the glass).
    func glassTopBar() -> some View {
        toolbarBackground(.hidden, for: .navigationBar)
    }
}

/// Identifies the ChatRoom the open-room dev link asked the shell to present.
struct RoomLink: Identifiable {
    let id: String
    /// The room's title as presented ("Group / Room" for a hosted forum);
    /// nil falls back to the generic "Channel".
    var title: String? = nil
    /// The hosting Group's object id, for a group-hosted forum — what makes the
    /// cover's title a deep link to the group's page on LIFE. nil = free-standing.
    var group: String? = nil
}

/// Leaves the open conversation on a back-swipe that BEGINS at the very left of the screen.
///
/// The chat is a shell OVERLAY, not a real navigation push, so the system's interactive-pop
/// gesture isn't available to it. And the transcript is a `ScrollView`: a SwiftUI `DragGesture`
/// racing it for a horizontal drag in the content area loses, so `onEnded` never fires — which
/// is why every SwiftUI attempt at this did nothing on device.
///
/// So this is a UIKit `UIPanGestureRecognizer` installed on an ANCESTOR of the chat. An ancestor
/// recogniser receives touches that occur in its descendants (the scroll view included), and set
/// to recognise simultaneously it never blocks the scroll or a tap — it only WATCHES. It fires
/// `onTrigger` when a drag that STARTED inside the left edge band travels left far enough to be a
/// deliberate back-fling. Starting in that band is the whole point: it's dead margin the
/// transcript doesn't fight for, so the recogniser wins it cleanly.
struct EdgeBackSwipe: UIViewRepresentable {
    /// Fraction of the width that counts as "the very left side" — where the fling must begin.
    private static let bandFraction: CGFloat = 0.32
    /// How far left the drag must travel to leave (points), and how strongly horizontal.
    private static let minLeft: CGFloat = 64

    var onTrigger: () -> Void

    func makeCoordinator() -> Coordinator { Coordinator(onTrigger: onTrigger) }

    func makeUIView(context: Context) -> UIView {
        // Non-interactive: this view exists only to reach an ancestor to hang the recogniser on.
        let v = UIView()
        v.isUserInteractionEnabled = false
        v.backgroundColor = .clear
        return v
    }

    func updateUIView(_ view: UIView, context: Context) {
        context.coordinator.onTrigger = onTrigger
        // Install once. The superview is the container the chat content also lives in, so a
        // recogniser on it sees the transcript's touches.
        guard context.coordinator.pan == nil, let host = view.superview else { return }
        let pan = UIPanGestureRecognizer(target: context.coordinator,
                                         action: #selector(Coordinator.handle(_:)))
        pan.delegate = context.coordinator
        pan.cancelsTouchesInView = false      // never eat a tap or a scroll
        host.addGestureRecognizer(pan)
        context.coordinator.pan = pan
    }

    final class Coordinator: NSObject, UIGestureRecognizerDelegate {
        var onTrigger: () -> Void
        weak var pan: UIPanGestureRecognizer?
        private var startX: CGFloat = 0
        private var fired = false

        init(onTrigger: @escaping () -> Void) { self.onTrigger = onTrigger }

        @objc func handle(_ g: UIPanGestureRecognizer) {
            guard let v = g.view else { return }
            switch g.state {
            case .began:
                startX = g.location(in: v).x
                fired = false
            case .changed:
                guard !fired, startX <= v.bounds.width * EdgeBackSwipe.bandFraction else { return }
                let t = g.translation(in: v)
                // A leftward fling (t.x negative) that is clearly horizontal.
                guard t.x < -EdgeBackSwipe.minLeft, abs(t.x) > abs(t.y) else { return }
                fired = true
                onTrigger()
            default:
                break
            }
        }

        // Watch alongside the scroll view / nav gestures — never replace them.
        func gestureRecognizer(_ g: UIGestureRecognizer,
                               shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool { true }
    }
}


// MARK: - The own-profile overlay (11 Aug order)

/// The top-left circle opens YOUR OWN PROFILE — the universal component on Me —
/// not the settings tree. Settings sit one gear-tap deep on this page; the
/// pins/lanes push inside this overlay's own stack (it lives outside LIFE's).
struct MyProfileOverlay: View {
    @EnvironmentObject private var core: CoreService

    var body: some View {
        NavigationStack {
            PersonProfilePage(id: core.myIdentityHex,
                              source: RealProfileSource(core: core, subject: .peer(core.myIdentityHex)))
                .toolbar {
                    ToolbarItem(placement: .topBarTrailing) {
                        NavigationLink { AccountSheet() } label: {
                            Image(systemName: "gearshape")
                        }
                        .accessibilityIdentifier("account-settings")
                    }
                }
                .navigationDestination(for: PersonLink.self) { link in
                    PersonProfilePage(id: link.id,
                                      source: RealProfileSource(core: core, subject: .peer(link.id)))
                }
                .navigationDestination(for: GroupLink.self) { link in
                    GroupDetailView(objectId: link.id)
                }
        }
    }
}
