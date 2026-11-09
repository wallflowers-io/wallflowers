//  NewSpaceFromChat.swift
//  "New Space with…" — WhatsApp's "New group with…", except it constitutes a
//  SPACE (a real Group GroupObject with offices), not another chat.
//
//  WHAT THIS IS NOT. It does not promote, migrate or convert the conversation.
//  The DM or group chat stays exactly what it was, with its own MLS group, its
//  own roster and its whole transcript untouched — nothing is copied and no edge
//  is drawn between the two. The ONLY thing carried across is WHO: the people in
//  the chat are offered as the founding members of the new Space.
//
//  That ruling matters because the obvious alternative — folding the chat's
//  history into the Space — would mean a second MLS group holding a copy of a
//  conversation, which is the merge this codebase deliberately avoids elsewhere
//  ("join-not-merge"). A Space made this way starts empty and says so.
//
//  IT IS A COMPOSE, NOT A FORM. Making a Space is the same act as making a Note
//  or minting a Place, so it wears the same surface: `ComposeGround` behind,
//  `.composeGlass` on the card, `ComposeDiscs` for the buttons — RED cancel,
//  BLUE commit, the shell's own disc language (Design/ComposeSurface.swift, and
//  its header explains why three copies of this became one). It was an iOS
//  `Form` in a NavigationStack, which is a settings screen wearing a create
//  flow's job — a fourth dialect for a surface that had just been unified.
//
//  THE THREE CIRCLES (beat 8 of the one-pager journey — 15 Aug order). All three
//  are live here, which is what makes this the NOTE card's surface rather than a
//  form that borrowed its paint:
//
//    RED   cancels the new Space. An empty compose goes quietly; one you have
//          started warns first, exactly as the note's ✕ does — this is the only
//          action here that destroys something.
//    GOLD  DRAWS UP the list of people to add. The members are not a field on the
//          card any more: the card carries the SUMMARY ("Ada ✓ Jimmy ✓") and gold
//          raises the list over it. Gold's own gate, not the commit gate — you can
//          answer who is coming before the Space has a name.
//    BLUE  creates the Space.
//
//  And it LEAVES the way the note leaves (Motion.note*): the ground goes first and
//  fast, the card follows a beat later, and the edge flashes the outcome's colour —
//  the SPACES jewel, the colour of the thing just made — as the card opens out past
//  the screen edges. Created is a release, not a dismissal.
//
//  HOW THE MEMBERS ARRIVE. `core.addContact(objectId:peerId:)` — the tap-to-add
//  path, which consumes one of the peer's stocked prekeys and commits a real MLS
//  Add. No connect code, no code exchange: that is what makes this a one-tap flow
//  rather than AddPeopleSheet's paste-a-bundle. A member with no prekeys left
//  cannot be added, and is reported rather than silently dropped.
//
//  WHAT THIS DOES NOT DO. It does not link the Space to anything. A Space is
//  affiliated with an Event by SHARING that event TO it (EventDetailPage's
//  "Share to"), which is the direction the act actually runs in: you are looking
//  at a night and choosing who to bring it to. Offering a link list at birth
//  would ask the question in the wrong place, before the Space is anything.

import SwiftUI

struct NewSpaceFromChat: View {
    @EnvironmentObject private var core: CoreService
    @Environment(\.dismiss) private var dismiss

    /// The conversation whose members seed the Space. Never modified.
    let conversation: Conversation
    var onCreated: (String) -> Void = { _ in }

    @State private var name = ""
    /// The Space's descriptor — what the InfoCard shows on its profile. Carried
    /// into `mintOrg(purpose:)`, so it is set at birth rather than edited in later.
    @State private var bio = ""
    @State private var chosen: Set<String> = []
    @State private var busy = false
    /// Members the Add could not reach — no prekeys left. Named, not swallowed.
    @State private var unreachable: [String] = []
    /// GOLD's list, drawn up over the card. Not a sheet: a sheet would come from
    /// outside this surface and cover the compose it belongs to.
    @State private var picking = false
    /// Non-nil once the compose is on its way out — see `Exit`.
    @State private var exit: Exit?
    /// RED on a compose you have started asks before binning it.
    @State private var confirmDiscard = false

    /// How the compose left, which is how it plays its departure — the note's own
    /// two outcomes (`ShellState.NoteExit`), minus the hand-off this surface has no
    /// equivalent of.
    private enum Exit { case created, cancelled }

    /// The chat's people, minus me. A DM has exactly one; a group chat has its
    /// roster. These are candidates, not a fait accompli — the sheet lets you
    /// drop anyone before constituting.
    private var candidates: [String] {
        let mine = core.myIdentityHex
        if conversation.kind == .dm { return [conversation.id] }
        return core.objectMembers(conversation.id)
            .filter { CoreService.identityHex($0) != mine }
    }

    private func label(_ peer: String) -> String {
        let n = core.connection(peer)?.displayName ?? ""
        return n.isEmpty ? String(peer.prefix(8)) + "…" : n
    }

    /// SPACES' own jewel colour, so the surface is tinted like the thing it makes
    /// — the same source the rail and the `+` fill with.
    private var gloss: (hi: Color, lo: Color) {
        let base = PaletteStore.base(LifeFilter.groups.colorIndex)
        return (base.lightened(0.16), base.darkened(0.32))
    }

    /// There is something to lose — RED's warn gate. A name, a purpose, or a member
    /// list you have already edited away from "everyone".
    private var started: Bool {
        !name.trimmingCharacters(in: .whitespaces).isEmpty
            || !bio.trimmingCharacters(in: .whitespaces).isEmpty
            || chosen != Set(candidates)
    }

    private var canCommit: Bool {
        !name.trimmingCharacters(in: .whitespaces).isEmpty && !busy && exit == nil
    }

    var body: some View {
        ZStack {
            // The ground goes FIRST and fast — the transcript underneath is back
            // before the card in front of it has finished leaving.
            ComposeGround(tint: gloss.hi, onTapOutside: { if !started { cancel() } })
                .opacity(exit == nil ? 1 : 0)
                .animation(Motion.noteGroundOut, value: exit)

            card
                .padding(.horizontal, 16).padding(.vertical, 44)
                // CREATED opens out past the screen edges as it fades — the Space
                // going INTO the app, not off it. CANCELLED just fades.
                .scaleEffect(exit == .created ? 1.3 : 1)
                .opacity(exit == nil ? 1 : 0)
                .animation(exit == .created ? Motion.noteExpand : Motion.noteFade, value: exit)

            if picking { peopleDrawer }
        }
        .allowsHitTesting(exit == nil)      // a leaving card takes no more taps
        .onAppear { chosen = Set(candidates) }   // everyone, opt-out
        .confirmationDialog("Discard this Space?", isPresented: $confirmDiscard,
                            titleVisibility: .visible) {
            Button("Discard", role: .destructive) { leave(.cancelled) }
            Button("Keep editing", role: .cancel) {}
        }
    }

    private var card: some View {
        VStack(spacing: 0) {
            header
            ScrollView { fields.padding(.horizontal, 20).padding(.bottom, 8) }
            // RED cancel · GOLD people · BLUE create. Gold carries its OWN gate:
            // who is coming is answerable before the Space has a name.
            ComposeDiscs(canCommit: canCommit,
                         commitLabel: "Create Space",
                         canShare: !candidates.isEmpty && exit == nil,
                         shareGlyph: "person.2.fill",
                         shareLabel: "Add people",
                         cancel: cancel,
                         share: { withAnimation(Motion.screenIn) { picking.toggle() } },
                         commit: { Task { await create() } })
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .composeGlass(tint: gloss.hi, flash: exit == .created ? gloss.hi : nil)
    }

    private var header: some View {
        HStack(spacing: 12) {
            Image(systemName: "person.3.fill")
                .font(.headline).foregroundStyle(.white)
                .frame(width: 40, height: 40)
                .background(Circle().fill(LinearGradient(colors: [gloss.hi, gloss.lo],
                                                         startPoint: .top, endPoint: .bottom)))
            Text("New Space")
                .font(.system(.title2, design: .serif).weight(.bold))
                .foregroundStyle(Theme.textPrimary)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 20)
        .padding(.top, 20)
        .padding(.bottom, 4)
    }

    /// Name, bio, members. Nothing else: a Space is linked to events by SHARING
    /// to it, not by a picker here.
    @ViewBuilder private var fields: some View {
        FieldRow(label: "Name") {
            TextField("", text: $name)
                .textFieldStyle(.plain).foregroundStyle(Theme.textPrimary)
                .accessibilityIdentifier("new-space-name")
        }
        FieldRow(label: "What it's for") {
            TextField("", text: $bio, axis: .vertical)
                .lineLimit(1...3)
                .textFieldStyle(.plain).foregroundStyle(Theme.textPrimary)
                .accessibilityIdentifier("new-space-bio")
        }
        // MEMBERS is a SUMMARY here, not the list — the list is GOLD's, drawn up
        // over the card. The card states who is coming ("Ada ✓  Jimmy ✓"), which is
        // the one thing you need to see while naming the Space.
        FieldRow(label: "Members") {
            Text(memberSummary)
                .font(.subheadline)
                .foregroundStyle(chosen.isEmpty ? Theme.textSecondary : Theme.textPrimary)
                .accessibilityIdentifier("new-space-members-summary")
        }
        if !unreachable.isEmpty {
            FieldRow(label: "Could not add") {
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(unreachable, id: \.self) { p in
                        Text(label(p)).font(.subheadline).foregroundStyle(Theme.textSecondary)
                    }
                    Text("No prekeys left for them. They can be added later from the Space.")
                        .font(.caption).foregroundStyle(Theme.textTertiary)
                }
            }
        }
    }

    /// The card's own line about who is coming — the diagram's `MEMBERS · Ada ✓
    /// Jimmy ✓`. Honest when there is nobody: a DM with someone who left you no way
    /// to add them says so rather than showing an empty tick list.
    private var memberSummary: String {
        if candidates.isEmpty { return "No one to add yet" }
        if chosen.isEmpty { return "Nobody — just you" }
        return candidates.filter { chosen.contains($0) }
            .map { label($0) + " ✓" }
            .joined(separator: "  ")
    }

    // MARK: - GOLD's list

    /// The people to add, DRAWN UP over the card from the bottom edge. It wears the
    /// compose's own glass, so it reads as the same surface rising rather than a
    /// sheet arriving from outside it. Tapping off it puts it back down.
    private var peopleDrawer: some View {
        ZStack(alignment: .bottom) {
            Color.black.opacity(0.28)
                .ignoresSafeArea()
                .contentShape(Rectangle())
                .onTapGesture { withAnimation(Motion.screenIn) { picking = false } }

            VStack(alignment: .leading, spacing: 0) {
                Text("ADD PEOPLE")
                    .font(.system(.caption2, design: .monospaced)).tracking(1.5)
                    .foregroundStyle(Theme.textTertiary)
                    .padding(.horizontal, 20).padding(.top, 20).padding(.bottom, 10)

                ScrollView {
                    VStack(alignment: .leading, spacing: 8) {
                        ForEach(candidates, id: \.self) { peer in
                            Button {
                                if chosen.contains(peer) { chosen.remove(peer) }
                                else { chosen.insert(peer) }
                            } label: {
                                HStack(spacing: 10) {
                                    Image(systemName: chosen.contains(peer)
                                          ? "checkmark.circle.fill" : "circle")
                                        .foregroundStyle(chosen.contains(peer) ? gloss.hi
                                                                               : Theme.textTertiary)
                                    Text(label(peer)).font(.subheadline)
                                        .foregroundStyle(Theme.textPrimary)
                                    Spacer(minLength: 0)
                                }
                                .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                            .accessibilityIdentifier("new-space-member")
                        }
                    }
                    .padding(.horizontal, 20)
                }
                .frame(maxHeight: 260)
                .padding(.bottom, 20)
            }
            .frame(maxWidth: .infinity)
            .composeGlass(tint: gloss.hi)
            .padding(.horizontal, 16)
            .padding(.bottom, 16)
            // A CONTAINER, or the identifier is invisible to XCUITest — an id on a
            // plain layout view surfaces nothing (FeedView's cards learned the same
            // lesson). `.contain` keeps the member rows queryable inside it.
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("new-space-people")
        }
        .transition(.opacity)
    }

    // MARK: - Leaving

    /// RED. An untouched compose goes quietly; one you have started warns first.
    private func cancel() {
        if started { confirmDiscard = true } else { leave(.cancelled) }
    }

    /// Play the departure, THEN tear down — dropping the view here would delete it
    /// mid-animation, which is the note card's own rule (`ShellState.endCompose`).
    private func leave(_ how: Exit, then finish: @escaping () -> Void = {}) {
        guard exit == nil else { return }
        if how == .created { UINotificationFeedbackGenerator().notificationOccurred(.success) }
        withAnimation { exit = how }
        DispatchQueue.main.asyncAfter(deadline: .now() + Motion.noteExit) {
            finish()
            dismiss()
        }
    }

    /// Constitute the Space, then Add each chosen member through the prekey path.
    /// The Space is minted FIRST and kept even if some Adds fail: a group of one
    /// that you can add to later is a real object; an all-or-nothing mint would
    /// throw away a constituted body because one peer was out of prekeys.
    private func create() async {
        let trimmed = name.trimmingCharacters(in: .whitespaces)
        guard !trimmed.isEmpty, !busy else { return }
        busy = true
        defer { busy = false }

        guard let id = core.mintOrg(name: trimmed, shape: "organisation",
                                    purpose: bio.trimmingCharacters(in: .whitespaces)) else {
            return   // core.error is set and surfaced by the shell
        }
        var failed: [String] = []
        for peer in candidates where chosen.contains(peer) {
            if await core.addContact(id, peerId: peer) == false { failed.append(peer) }
        }
        core.refreshOrgs()
        if failed.isEmpty {
            // BLUE's outcome: the edge flashes the SPACES jewel and the card opens
            // out, then the Space it made takes the screen.
            leave(.created) { onCreated(id) }
        } else {
            unreachable = failed   // stay open and say who, rather than half-succeeding in silence
        }
    }
}
