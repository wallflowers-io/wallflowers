//  TicketWallet.swift
//  The tickets this device holds, and the two sweeps that keep them honest.
//
//  A ticket is a signed credential DELIVERED into the buyer↔organizer connection and
//  folded there (`wallet(peer:)`), so the wallet is the union across connections — no
//  store of its own, exactly like the market reads. Presenting is the whole job: the QR
//  is the credential verbatim (never re-serialized), full-screen for the door's camera.
//
//  THE SWEEPS (run on the sync tick, from the shell):
//  · arrivals — a credential not in the persisted seen-set is NEW: it lands in the one
//    notification tray as a `.ticket` Output, and its listing stops being "pending".
//  · free fulfilment — the free path is PHONE-fulfilled (no Arc, no rail): a pending
//    request against a free listing this device OWNS is answered here — recordSale +
//    issue + deliver — guarded by a persisted fulfilled-set so a request is answered once.

import SwiftUI
import PacificCore

extension Notification.Name {
    /// Posted when a tray `.ticket` is confirmed (VIEW); the shell presents the wallet.
    static let pacificShowTickets = Notification.Name("pacificShowTickets")
}

// MARK: - Persisted sets
// NOTE: `pending` shadows the ticket ledger the same way the going-set shadowed
// the invite replies — see the UserDefaults audit. It is a checkout-resume cache,
// so it survives; if it ever answers "do I hold a ticket", the fold must.

/// Listings with a request/checkout in flight — what disables the capsule (9A). Cleared
/// when the credential arrives.
@MainActor
final class PendingTicketStore: ObservableObject {
    static let shared = PendingTicketStore()
    @Published private(set) var pending: Set<String>
    private static let key = "pacific.tickets.pending"

    private init() {
        pending = Set(UserDefaults.standard.stringArray(forKey: Self.key) ?? [])
    }

    func add(_ listing: String) {
        pending.insert(listing)
        UserDefaults.standard.set(Array(pending).sorted(), forKey: Self.key)
    }

    func clear(_ listing: String) {
        guard pending.remove(listing) != nil else { return }
        UserDefaults.standard.set(Array(pending).sorted(), forKey: Self.key)
    }
}

/// Ticket ids already surfaced in the tray (arrivals) and request pis already answered
/// (free fulfilment) — plain persisted sets, no observation needed.
enum TicketSets {
    static func load(_ key: String) -> Set<String> {
        Set(UserDefaults.standard.stringArray(forKey: key) ?? [])
    }
    static func save(_ set: Set<String>, _ key: String) {
        UserDefaults.standard.set(Array(set).sorted(), forKey: key)
    }
    static let seenKey = "pacific.tickets.seen"
    static let fulfilledKey = "pacific.tickets.fulfilled"
}

// MARK: - The sweeps

@MainActor
enum TicketSweep {

    /// New deliveries → the tray. A credential's first sighting appends ONE `.ticket`
    /// Output and releases its listing from the pending set.
    static func arrivals(core: CoreService, outputs: OutputStore) {
        var seen = TicketSets.load(TicketSets.seenKey)
        var changed = false
        // includeBlocked (here and below): a ticket is a credential you HOLD — blocking
        // the organizer must not vanish it, and a sale already made must still fulfil.
        for c in core.connectionRows(includeBlocked: true) where c.connected {
            for t in core.walletTickets(peer: c.peerId) where !seen.contains(t.ticket) {
                seen.insert(t.ticket)
                changed = true
                outputs.append(.ticket(TicketArrival(title: t.title, ticketId: t.ticket,
                                                     listing: t.listing)))
                PendingTicketStore.shared.clear(t.listing)
            }
        }
        if changed { TicketSets.save(seen, TicketSets.seenKey) }
    }

    /// The free path's organizer half: answer pending requests against free listings this
    /// device owns. Once per pi (persisted), capacity-gated, oldest state first.
    static func fulfilFree(core: CoreService) async {
        var mine: [String: EventRow] = [:]   // listing → the free event we own
        for row in core.events
        where row.listed && row.priceCents == 0 && row.open && core.isGroupOwner(row.id) {
            guard row.capacity == 0 || row.sold < row.capacity else { continue }
            mine[row.listing] = row
        }
        guard !mine.isEmpty else { return }

        var done = TicketSets.load(TicketSets.fulfilledKey)
        for c in core.connectionRows(includeBlocked: true) where c.connected {
            for r in core.pendingTicketRequests(peer: c.peerId) {
                guard let row = mine[r.listing], !done.contains(r.pi) else { continue }
                if await core.recordFreeSale(objectId: row.id, pi: r.pi,
                                             buyer: r.author, qty: r.qty) {
                    done.insert(r.pi)
                    TicketSets.save(done, TicketSets.fulfilledKey)
                }
            }
        }
    }
}

// MARK: - The wallet

struct TicketWalletView: View {
    /// A ticket id to open full-screen on arrival — the dev seam's `show-tickets?ticket=`,
    /// which is how a headless drive reaches a cover that otherwise needs a tap.
    var present: String? = nil
    @EnvironmentObject private var core: CoreService
    @State private var tickets: [WalletTicketRow] = []
    @State private var presented: WalletTicketRow?

    var body: some View {
        Group {
            if tickets.isEmpty {
                Text("NO TICKETS")
                    .font(.system(size: 18, weight: .semibold))
                    .tracking(1.4)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                List(tickets, id: \.ticket) { t in
                    Button { presented = t } label: {
                        HStack {
                            Text(t.title.isEmpty ? String(t.ticket.prefix(8)) : t.title)
                                .font(.system(size: 17, weight: .bold))
                                .foregroundStyle(Theme.textPrimary)
                            Spacer()
                            if t.admitted {
                                // The row states the fact, not a sentence about it.
                                Text("ADMITTED")
                                    .font(.system(size: 13, weight: .heavy))
                                    .tracking(1.2)
                                    .foregroundStyle(Self.admittedGreen)
                                    .accessibilityIdentifier("ticket-row-admitted")
                            } else {
                                Text(Self.day.string(from: Date(timeIntervalSince1970: Double(t.startMs) / 1000)))
                                    .font(.system(size: 14))
                                    .foregroundStyle(Theme.textSecondary)
                            }
                        }
                        .padding(.vertical, 6)
                    }
                    .listRowBackground(Brand.card)
                }
                .scrollContentBackground(.hidden)
            }
        }
        .background(Theme.bg)
        .navigationTitle("TICKETS")
        .navigationBarTitleDisplayMode(.inline)
        .onAppear(perform: reload)
        .onReceive(core.$syncTick) { _ in reload() }
        .fullScreenCover(item: $presented) { t in
            TicketQRView(ticket: t) { presented = nil }
        }
    }

    /// The union across connections — the wallet has no store of its own.
    ///
    /// `presented` is re-read from the fresh rows, not left as the snapshot it was opened
    /// with: the admit receipt lands on a sync tick WHILE the ticket is on screen, and a
    /// stale value copy is exactly how "it went green a screen too late" happens.
    private func reload() {
        var seen = Set<String>()
        tickets = core.connectionRows(includeBlocked: true)
            .flatMap { core.walletTickets(peer: $0.peerId) }
            .filter { seen.insert($0.ticket).inserted }
            .sorted { $0.startMs < $1.startMs }
        if let open = presented, let fresh = tickets.first(where: { $0.ticket == open.ticket }) {
            presented = fresh
        }
        if presented == nil, let want = present, !want.isEmpty {
            presented = tickets.first { $0.ticket == want } ?? (want == "first" ? tickets.first : nil)
        }
    }

    /// The one green in the app that means "you are in" — the door's green, on the
    /// holder's side of the same fact.
    static let admittedGreen = Color(red: 0.16, green: 0.78, blue: 0.42)

    private static let day: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "EEE d MMM"
        return f
    }()
}

extension WalletTicketRow: @retroactive Identifiable {
    public var id: String { ticket }

    /// The door told this device the holder was let in (`contact.admitTicket`). Monotone
    /// and first-wins in the fold, so it never flickers back.
    public var admitted: Bool { admittedMs > 0 }
}

/// The credential, presented: the QR verbatim, the ticket id under it. White ground —
/// the door's camera reads contrast, not taste.
///
/// ONCE ADMITTED the QR has done its job and the screen becomes the receipt: a full green
/// ground and one word. That green is authored by the door, not by this device — it exists
/// because a `contact.admitTicket` Delta arrived on the organizer's channel.
struct TicketQRView: View {
    let ticket: WalletTicketRow
    let done: () -> Void

    var body: some View {
        Group {
            if ticket.admitted { admittedScreen } else { credential }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .onTapGesture(perform: done)
    }

    private var credential: some View {
        VStack(spacing: 24) {
            Spacer()
            QRView(payload: ticket.qr, size: 300)
                .padding(20)
                .background(.white, in: RoundedRectangle(cornerRadius: 16))
            Text(ticket.ticket)
                .font(.caption2.monospaced())
                .foregroundStyle(.secondary)
                .textSelection(.enabled)
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Theme.bg.ignoresSafeArea())
        .accessibilityIdentifier("ticket-credential")
    }

    private var admittedScreen: some View {
        VStack(spacing: 18) {
            Image(systemName: "checkmark.circle.fill")
                .font(.system(size: 96, weight: .semibold))
                .foregroundStyle(.white)
            Text("ADMITTED")
                .font(.system(size: 40, weight: .heavy))
                .tracking(1.2)
                .foregroundStyle(.white)
            if !ticket.title.isEmpty {
                Text(ticket.title)
                    .font(.system(size: 20, weight: .semibold))
                    .foregroundStyle(.white.opacity(0.85))
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(TicketWalletView.admittedGreen.ignoresSafeArea())
        .contentShape(Rectangle())
        .accessibilityIdentifier("ticket-admitted")
    }
}
