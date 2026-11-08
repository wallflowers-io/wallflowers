//  EventInvite.swift
//  Inviting someone to an event, and the card that lands in the chat when you do.
//
//  An invitation rides the PAIRWISE Contact channel — `contact.invite` (op 9) and
//  `contact.inviteReply` (op 10), spliced onto Contact exactly like the ticket legs.
//  A guest never joins the Event GroupObject (the ticketing model keeps buyers out on
//  purpose), so the channel the two of you already share is the only place an
//  invitation can live.
//
//  WHY THE LEG CARRIES THE FACTS. The invitee does not hold the Event object, so an
//  invite carrying only an object id would render as an opaque handle and could not be
//  answered without a round-trip to a node nobody has. The leg carries title, when,
//  where and your note, so the card says something the moment it lands — offline,
//  forever. It is a SNAPSHOT: if the organiser later moves the event, this invite
//  still says what was sent, and the event id joins back to the live object for
//  anyone who does hold it.
//
//  This is also why the card needs nothing from `forum.post`. A chat message carries
//  one argument, `text` — there are no attachments — so the card renders from the
//  Contact fold beside the messages rather than from a message.

import SwiftUI
import PacificCore

// MARK: - Sending

/// Pick who to invite. One tap per person: the leg is authored immediately, so there
/// is no compose step and nothing to submit.
struct InviteToEventSheet: View {
    @EnvironmentObject private var core: CoreService
    @Environment(\.dismiss) private var dismiss

    let eventId: String
    let title: String
    let startMs: Int64
    let venue: String

    @State private var sent: Set<String> = []
    @State private var busy: String?

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 0) {
                    ForEach(core.connectionRows(), id: \.peerId) { row in
                        Button {
                            Task { await invite(row.peerId) }
                        } label: {
                            HStack(spacing: 14) {
                                Avatar(initials: Avatar.initials(row.displayName),
                                       tint: EventsPage.accent, size: 36)
                                Text(row.displayName.isEmpty ? String(row.peerId.prefix(8)) : row.displayName)
                                    .font(.system(size: 17))
                                    .foregroundStyle(Theme.textPrimary)
                                    .lineLimit(1)
                                Spacer(minLength: 12)
                                if sent.contains(row.peerId) {
                                    Image(systemName: "checkmark")
                                        .font(.system(size: 14, weight: .bold))
                                        .foregroundStyle(EventsPage.accentSoft)
                                } else if busy == row.peerId {
                                    ProgressView().controlSize(.small)
                                }
                            }
                            .padding(.horizontal, 20)
                            .padding(.vertical, 9)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .disabled(sent.contains(row.peerId) || busy != nil)
                        .accessibilityIdentifier("invite-peer-\(row.peerId.prefix(12))")
                        .overlay(alignment: .top) {
                            Rectangle().fill(Theme.hairline.opacity(0.6)).frame(height: 0.5)
                                .padding(.horizontal, 20)
                        }
                    }
                }
                .padding(.top, 8)
            }
            .scrollIndicators(.hidden)
            .background(Theme.bg.ignoresSafeArea())
            .navigationTitle("Invite")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
    }

    private func invite(_ peer: String) async {
        busy = peer
        defer { busy = nil }
        if await core.sendInvite(peer: peer, event: eventId, title: title,
                                 startMs: startMs, venue: venue) {
            sent.insert(peer)
        }
    }
}

// MARK: - The card in the chat

/// One invitation, in the thread with the person it is between.
///
/// Renders from the Contact fold, so it survives independently of anything either of
/// you said — the invitation is a thing that HAPPENED between you, not a thing one of
/// you typed. Received invites carry the two answers; sent ones show what came back.
struct InviteCard: View {
    @EnvironmentObject private var core: CoreService

    let invite: InviteRow
    /// The channel this card belongs to — who to author the answer to.
    let peer: String

    private var mine: Bool { invite.author == core.myIdentityHex }
    private var answered: Bool { !invite.reply.isEmpty }
    private var going: Bool { invite.reply == "going" }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Image(systemName: "calendar")
                    .font(.system(size: 12, weight: .bold))
                Text(mine ? "INVITED" : "INVITATION")
                    .font(.system(size: 11, weight: .heavy))
                    .tracking(1.2)
            }
            .foregroundStyle(EventsPage.accentSoft)

            Text(invite.title)
                .font(.system(size: 19, weight: .bold))
                .foregroundStyle(Theme.textPrimary)
                .lineLimit(2)

            // When and where, from the leg — not looked up, because the invitee has
            // no Event object to look anything up in.
            Text(Self.when(invite.startMs) + (invite.venue.isEmpty ? "" : " · \(invite.venue)"))
                .font(.system(size: 14))
                .foregroundStyle(Theme.textSecondary)

            if !invite.note.isEmpty {
                Text(invite.note)
                    .font(.system(size: 14))
                    .foregroundStyle(Theme.textSecondary)
            }

            if mine {
                // Your own invitation: show what came back, or that it is outstanding.
                Text(answered ? (going ? "Going" : "Not going") : "No answer yet")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(answered && going ? EventsPage.accentSoft : Theme.textTertiary)
            } else {
                answerButtons
            }
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .composeGlass(tint: EventsPage.accent)
        .accessibilityIdentifier("invite-card-\(invite.event.prefix(12))")
    }

    /// Going / Not going. Both stay live after answering — changing your mind is just
    /// a later delta, so the UI must not lock you out of doing it.
    private var answerButtons: some View {
        HStack(spacing: 10) {
            answer(label: "Going", on: answered && going, value: true)
            answer(label: "Can't", on: answered && !going, value: false)
        }
    }

    private func answer(label: String, on: Bool, value: Bool) -> some View {
        Button {
            Task { _ = await core.replyInvite(peer: peer, event: invite.event, going: value) }
        } label: {
            Text(label)
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(on ? Theme.onAccent : Theme.textPrimary)
                .padding(.horizontal, 18)
                .padding(.vertical, 8)
                .background(Capsule().fill(on ? EventsPage.accent : Theme.fill))
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("invite-\(value ? "going" : "decline")-\(invite.event.prefix(12))")
    }

    private static func when(_ ms: Int64) -> String {
        let f = DateFormatter()
        f.dateFormat = "EEE d MMM · HH:mm"
        return f.string(from: Date(timeIntervalSince1970: Double(ms) / 1000))
    }
}
