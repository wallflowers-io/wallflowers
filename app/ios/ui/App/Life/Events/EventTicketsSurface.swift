//  EventTicketsSurface.swift
//  LIST — putting tickets on a night you are hosting.
//
//  WHY THIS EXISTS AT ALL. Ticket authoring used to live in EventComposer, which
//  minted the event and set its tickets in one save. The composer was retired into
//  the universal `New` surface, and `EVENT_FIELDS` (pacific-core/src/mint.rs)
//  declares Name · Icon · Banner · Describe · When · Recurrence · Place — no price
//  and no capacity. So the app can mint an event and cannot list it. The seeder
//  still calls `setEventTickets` directly, which is why the seeded nights have
//  capsules nobody could have produced by hand.
//
//  WHY IT IS SHAPED LIKE A LIST. Because it is one: an event's tickets are that
//  event ON OFFER, and Pacific already has a surface for putting something on
//  offer. What it reuses is the SURFACE, not the primitive — the compose ground,
//  the glass, and the RED · GOLD · BLUE row every mint in the app wears
//  (`Design/ComposeSurface.swift`, the one compose row). BLUE authors
//  `event.setTickets` on the event that is already there.
//
//  WHAT IT IS NOT. It does not mint a Thing. A ticket is not a Thing listing: the
//  ticketing spine is a listing key of sha256(group_id ‖ owner), a sale ledger,
//  signed credentials in the buyer's wallet, `event.redeem` and the door's scan —
//  none of which a Thing has. Reusing the route means reusing the way it LOOKS and
//  the way it is REACHED, not swapping the object underneath it.
//
//  WHY THERE IS NO PRICE FIELD, stated rather than quietly omitted. `event.setTickets`
//  refuses a priced listing without `acct` (a connected rail account) and `delegate`
//  (the fulfilment Arc's pk) — correctly, since a price nobody can collect is a
//  promise to a buyer. The onboarding that produced those two values lived in the
//  retired composer and has no home yet. A price row here would therefore be a
//  control that cannot ever save, which is the exact failure `Carriage::Deferred`
//  was invented to prevent. THING_FIELDS reaches the same conclusion about price
//  for the same underlying reason (selling is V2). So: capacity now, price when
//  there is a rail onboarding to stand behind it.
//
//  IT IS ALSO A SHEET, NOT A SHELL OVERLAY. `New` is presented UNDER the shell's
//  disc row so it can borrow those three circles. The event page is a
//  fullScreenCover ABOVE the shell — it covers the discs and cannot borrow them —
//  so, exactly as ComposeSurface's own header prescribes, it HOSTS the row instead.

import SwiftUI
import PacificCore

struct EventTicketsSurface: View {
    let objectId: String
    let title: String
    /// The listing as it stands, so re-opening edits rather than starts again.
    let capacity: Int64

    @EnvironmentObject private var core: CoreService
    @Environment(\.dismiss) private var dismiss

    @State private var entry: String
    @State private var canvas: CGRect = .zero
    @State private var flash: Color?

    init(objectId: String, title: String, capacity: Int64) {
        self.objectId = objectId
        self.title = title
        self.capacity = capacity
        // An existing listing opens at its own number; a new one opens EMPTY rather
        // than at a suggested crowd size. How many people fit is a fact about a room,
        // and the app does not know it.
        _entry = State(initialValue: capacity > 0 ? String(capacity) : "")
    }

    /// EVENTS' own palette slot — the same one the jewel and the `New Event`
    /// surface are cast in (`NewKind.colorIndex` = 0). A local constant since the
    /// event page stopped being a profile and its binder went with it.
    private static var eventTint: Color { PaletteStore.base(0) }

    /// The one rule: `event.setTickets` refuses capacity 0 at the fold, so BLUE is
    /// dead until the number is one the reducer will actually take.
    private var count: Int64? {
        guard let n = Int64(entry.trimmingCharacters(in: .whitespaces)), n > 0 else { return nil }
        return n
    }

    var body: some View {
        ZStack {
            ComposeGround(tint: Self.eventTint, opaque: true, canvas: canvas) {
                dismiss()
            }
            VStack(spacing: 0) {
                Spacer(minLength: 0)
                card
                Spacer(minLength: 0)
                ComposeDiscs(canCommit: count != nil,
                             commitLabel: "List",
                             cancel: { dismiss() },
                             commit: list)
                    .padding(.horizontal, DesignTokens.Life.gutter)
                    .padding(.bottom, 18)
            }
        }
        .accessibilityIdentifier("event-tickets-surface")
    }

    private var card: some View {
        VStack(alignment: .leading, spacing: 16) {
            // The night this is about — a value, so the surface says what it will
            // change without a sentence explaining itself.
            Text(title)
                .font(.system(size: 20, weight: .heavy))
                .tracking(-0.2)
                .foregroundStyle(Theme.textPrimary)
                .lineLimit(2)
            InlineFieldRow(label: "Capacity") {
                TextField("", text: $entry)
                    .keyboardType(.numberPad)
                    .multilineTextAlignment(.trailing)
                    .font(.system(size: 17, weight: .semibold, design: .monospaced))
                    .foregroundStyle(Theme.textPrimary)
                    .accessibilityIdentifier("event-tickets-capacity")
            }
            // FREE is stated, because it is the listing's actual price and the one
            // thing a guest will want to know. Not a caption on the capacity field —
            // a value of its own (see the price note in this file's header).
            Text("FREE")
                .font(.system(size: 13, weight: .bold))
                .tracking(1.1)
                .foregroundStyle(EventsChannel.accentSoft)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(20)
        .modifier(ComposeGlass(tint: Self.eventTint, flash: flash, opaque: true))
        .padding(.horizontal, DesignTokens.Life.gutter)
        .background(
            GeometryReader { geo in
                Color.clear.onAppear { canvas = geo.frame(in: .global) }
            }
        )
    }

    /// BLUE. The listing is FREE — `priceCents: 0` — which is the only listing this
    /// device can honestly author (header). A refusal leaves the surface up with the
    /// number intact, so nothing typed is lost to a failed write.
    private func list() {
        guard let count else { return }
        guard core.setEventTickets(objectId, priceCents: 0, currency: "usd",
                                   capacity: count) else { return }
        flash = Self.eventTint
        dismiss()
    }
}
