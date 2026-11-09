//  NewComponentTests.swift
//  The universal `New` component (App/Design/New.swift) — the two things that can go
//  quietly wrong when one surface serves every mint:
//
//    1. the DISC CELL, which decides what the three circles are. It replaced the note's
//       single `hasContent` Bool, and a regression here changes what a button does
//       without changing what it looks like.
//    2. the DRAFT CROSSING, where one shared draft becomes each kind's own op-0 arg
//       vocabulary. The Rust side pins the arg NAMES (pacific-core/src/mint.rs); this
//       pins that the Swift side hands over the right VALUES.

import XCTest
import PacificCore
@testable import WallFlowers

@MainActor
final class NewComponentTests: XCTestCase {

    // MARK: - The kind is anchored to the core, not to the rail

    /// `NewKind.rawValue` IS `ObjectKind::name()`. This is the invariant that makes the
    /// wrong-species bug unrepresentable: `CoreService.mint` passes the raw value
    /// straight through, and `pacific_core::mint::kind_from_name` refuses anything else.
    func testKindRawValuesAreTheCoresOwnNames() {
        XCTAssertEqual(NewKind.space.rawValue, "group")
        XCTAssertEqual(NewKind.listing.rawValue, "thing")
        XCTAssertEqual(NewKind.channel.rawValue, "forum")
        XCTAssertEqual(NewKind.event.rawValue, "event")
        XCTAssertEqual(NewKind.place.rawValue, "place")
        XCTAssertEqual(NewKind.project.rawValue, "project")
    }

    /// The note is the ONE device-local surface, and says so rather than pretending to
    /// have an object.
    func testOnlyTheNoteHasNoCoreKind() {
        for kind in NewKind.allCases {
            XCTAssertEqual(kind.coreKind == nil, kind == .note, "\(kind) coreKind")
        }
    }

    // MARK: - The disc cell

    /// A blank surface: RED and GOLD are live, BLUE is not. GOLD is the VISIBILITY face
    /// and is deliberately NOT gated on the commit rule — who this is for is answerable
    /// before it has a name, and is often the first thing you know about it.
    func testBlankSurfaceGatesCommitButNotVisibility() {
        let s = NewSession(kind: .space) { _ in .saved }
        XCTAssertFalse(s.cell.canCommit, "BLUE must be dead with no name")
        XCTAssertTrue(s.cell.canShare, "GOLD is always live — it is the audience dial")
        XCTAssertTrue(s.cell.hasShare)
    }

    // MARK: - What RED's creator mints

    /// LIFE's `+` went on 9 Nov because RED's creator covers it: every kind the `+`
    /// minted from a jewel is a card on RED's ring. PEOPLE was never a mint — a
    /// connection is a double opt-in formed by pairing (Chats · Connect).
    @MainActor
    func testRedsCreatorOffersEveryKindLifesPlusMinted() {
        let shell = ShellState()
        shell.makeNoteSession = { NewSession(kind: .note) { _ in .saved } }
        shell.makeMintSession = { k in NewSession(kind: k) { _ in .saved } }
        shell.beginCreator()
        let kinds = shell.creator?.kinds ?? []
        for k in [NewKind.post, .event, .space, .listing, .place, .project] {
            XCTAssertTrue(kinds.contains(k), "RED's ring carries \(k)")
        }
    }

    /// A Post carries the universal rows and no switches — the simplest kind in the
    /// catalog, which is what a primitive for "a published thing" should be.
    func testPostAsksForNameMediaAndProse() {
        let s = NewSession(kind: .post) { _ in .saved }
        // Plus its FORM and LINK, which the core's Post declaration gained after this
        // test was written — the list follows the core, never the other way round.
        XCTAssertEqual(s.fields.map(\.key), [.name, .icon, .banner, .descriptor, .form, .link])
        XCTAssertEqual(s.kind.rawValue, "post", "the wire name the core parses")
    }

    // MARK: - VISIBILITY — the universal audience dial on GOLD

    /// GOLD is present on EVERY kind, note included. Privacy/sharing is the same question
    /// whatever you are making, so it is the same circle and the same component.
    func testVisibilityIsOnEveryKind() {
        for kind in NewKind.allCases {
            let s = NewSession(kind: kind) { _ in .saved }
            XCTAssertTrue(s.cell.hasShare, "\(kind) must carry the audience dial")
            let gold = InterfaceMatrix.spec(.chat, in: .composing(s.cell))
            XCTAssertTrue(gold.enabled, "\(kind) GOLD must be tappable")
            XCTAssertEqual(gold.label, "Visibility")
        }
    }

    /// GOLD TURNS the card between its two faces rather than covering it, and says which
    /// way it will go next — the same contract the NOTE disc's hold label has.
    func testGoldTurnsTheCardBetweenItsTwoFaces() {
        let s = NewSession(kind: .event) { _ in .saved }
        XCTAssertEqual(s.face, .fields)

        s.toggleVisibility()
        XCTAssertEqual(s.face, .visibility)
        XCTAssertEqual(s.cell.shareLabel, "Back", "the label names where the next tap goes")

        s.toggleVisibility()
        XCTAssertEqual(s.face, .fields)
        XCTAssertEqual(s.cell.shareLabel, "Visibility")
    }

    /// The audience opens at ONLY ME — the safe default, and the one that needs no op at
    /// all (a GroupObject with no additional members is what a mint already produces).
    func testAudienceDefaultsToOnlyMe() {
        for kind in NewKind.allCases {
            XCTAssertEqual(NewSession(kind: kind) { _ in .saved }.audience, .onlyMe)
        }
    }

    /// Three of the five are `Visibility` levels the core already names, and their raw
    /// values ARE the wire values — so the dial cannot offer a level the core will not
    /// parse. The other two are roster choices and say they need a selection.
    func testAudienceOptionsSplitIntoVisibilityAndMembership() {
        XCTAssertEqual(Audience.onlyMe.rawValue, "private")
        XCTAssertEqual(Audience.connections.rawValue, "connections")
        XCTAssertEqual(Audience.network.rawValue, "network")

        XCTAssertTrue(Audience.onlyMe.isVisibilityLevel)
        XCTAssertTrue(Audience.connections.isVisibilityLevel)
        XCTAssertTrue(Audience.network.isVisibilityLevel)

        XCTAssertTrue(Audience.people.needsSelection)
        XCTAssertTrue(Audience.spaces.needsSelection)
        XCTAssertFalse(Audience.people.isVisibilityLevel)
        XCTAssertFalse(Audience.spaces.isVisibilityLevel)

        XCTAssertEqual(Audience.allCases.count, 5)
    }

    // MARK: - The type declares its own required fields

    /// Every kind that mints an object carries a declaration, read from the core. The
    /// note does not — it is device-local and mints nothing.
    func testEveryMintingKindCarriesTheTypesDeclaration() {
        for kind in NewKind.allCases {
            let s = NewSession(kind: kind) { _ in .saved }
            XCTAssertEqual(s.fields.isEmpty, kind == .note,
                           "\(kind): declaration presence should follow coreKind")
        }
    }

    /// The commit gate IS the declaration: every required field must be answered. Not a
    /// per-kind condition in the UI that can disagree with what the reducer demands.
    func testCommitGateIsTheDeclaredRequiredFields() {
        let s = NewSession(kind: .event) { _ in .saved }
        let required = s.fields.filter(\.required).map(\.key)
        XCTAssertTrue(required.contains(.name))
        XCTAssertTrue(required.contains(.start), "event.setProfile takes startMs with req_int")

        XCTAssertFalse(s.cell.canCommit, "unnamed ⇒ dead")
        s.draft.name = "gig"
        XCTAssertTrue(s.cell.canCommit, "named + seeded date ⇒ live")
        s.draft.date = nil
        XCTAssertFalse(s.cell.canCommit, "an unanswered required date must block BLUE")
    }

    /// **The 1970 bug.** `req_int` is satisfied by 0, so an untouched date picker minted
    /// an event at the unix epoch — accepted by the reducer, wrong in the world. The
    /// session seeds every declared Date field, so a required field is never silently
    /// unanswered behind a control that already looks filled in.
    func testEventStartIsSeededSoItIsNeverEpochZero() {
        let s = NewSession(kind: .event) { _ in .saved }
        XCTAssertNotNil(s.draft.date, "a declared Date field must open answered")
        s.draft.name = "gig"
        XCTAssertGreaterThan(s.draft.mint(for: .event).startMs, 0,
                             "startMs must never cross as 0 — that is 1970")
    }

    /// A declared Choice opens on a real value from the reducer's OWN vocabulary, so the
    /// picker can never mint something the fold rejects.
    func testChoiceFieldsSeedFromTheReducersVocabulary() {
        let s = NewSession(kind: .space) { _ in .saved }
        guard let shape = s.fields.first(where: { $0.key == .shape }) else {
            return XCTFail("a Space declares its shape — group.setProfile requires it")
        }
        XCTAssertEqual(shape.choices, ["community", "team", "organisation"])
        XCTAssertTrue(shape.choices.contains(s.draft.shape),
                      "the seeded shape must be one the reducer parses")
    }

    /// Fields the mint would drop must not be offered. Place and Event each used to
    /// collect a "Note" that never reached a delta, and Project an "Objective" that
    /// needed two args it was never asked for.
    func testNoKindOffersAFieldTheMintWouldDrop() {
        // Pinned against the core's own declaration, which `fields_carry_to_the_wire`
        // proves reaches the wire. Here we only assert the UI asks for nothing beyond it.
        for kind in NewKind.allCases where kind != .note {
            let s = NewSession(kind: kind) { _ in .saved }
            let declared = Set(s.fields.map(\.key))
            XCTAssertFalse(declared.isEmpty, "\(kind) must declare something")
            XCTAssertTrue(declared.isSubset(of: [.name, .descriptor, .shape, .category,
                                                 .stance, .start, .fix, .icon, .banner,
                                                 .price, .recurrence, .place, .form, .link]))
        }
        // Project asks for its NAME and nothing else. No OBJECTIVE (op 12 takes two
        // args), and no MEDIA either — Project has no media op, and the row is offered
        // exactly where a kind can store what you put in it.
        let project = NewSession(kind: .project) { _ in .saved }
        XCTAssertEqual(project.fields.map(\.key), [.name])
    }

    // MARK: - BLUE's gate is the declaration's `required` flag, and nothing else

    /// The criteria, stated once: BLUE is live when EVERY field the type declares
    /// `required` is answered, and no commit is already in flight. Nothing else gates it.
    ///
    /// Today that is Name on every kind, plus Kind on a Space and When on an Event — and
    /// both of those open already answered, so in practice a name is the whole gate.
    func testCommitGateIsExactlyTheRequiredFlag() {
        for kind in NewKind.allCases where kind != .note {
            let s = NewSession(kind: kind) { _ in .saved }
            let required = s.fields.filter(\.required).map(\.key)
            XCTAssertTrue(required.contains(.name), "\(kind) should require a name")

            XCTAssertFalse(s.cell.canCommit, "\(kind): blank ⇒ BLUE dead")
            s.draft.name = "something"
            XCTAssertTrue(s.cell.canCommit,
                          "\(kind): every required field answered ⇒ BLUE live. Required = \(required)")
        }
    }

    /// No OPTIONAL field may hold BLUE back. This is the assertion that would have caught
    /// a `required: true` slipped onto a media slot or a placeholder.
    func testOptionalFieldsNeverBlockCommit() {
        for kind in NewKind.allCases where kind != .note {
            let s = NewSession(kind: kind) { _ in .saved }
            s.draft.name = "something"
            for f in s.fields where !f.required {
                XCTAssertTrue(s.cell.canCommit,
                              "\(kind): optional \(f.key) must not gate the commit")
            }
        }
    }

    /// A DEFERRED field can never be required — it cannot be saved, so gating on it would
    /// make BLUE unreachable. The core pins this too; this is the app-side half.
    func testNoDeferredFieldIsRequired() {
        for kind in NewKind.allCases where kind != .note {
            for f in NewSession(kind: kind, commit: { _ in .saved }).fields where f.deferred {
                XCTAssertFalse(f.required, "\(kind) \(f.key): deferred AND required is unreachable")
            }
        }
    }

    /// A named surface lights BLUE.
    func testNamingASurfaceLightsCommit() {
        let s = NewSession(kind: .space) { _ in .saved }
        s.draft.name = "Aries"
        XCTAssertTrue(s.cell.canCommit)
    }

    /// Whitespace is not a name. The gate trims, so a spacebar cannot mint an object
    /// you will never find again.
    func testWhitespaceIsNotAName() {
        let s = NewSession(kind: .space) { _ in .saved }
        s.draft.name = "   \n "
        XCTAssertFalse(s.cell.canCommit)
    }

    /// GOLD's gate is genuinely independent: New Space's gold opens the member list,
    /// which is answerable BEFORE the Space has a name. If `canShare` collapsed back
    /// into `canCommit` this would be the one disc greyed out when it is the only one
    /// you can use yet.
    func testShareGateIsIndependentOfCommitGate() {
        let s = NewSession(kind: .space) { _ in .saved }
        XCTAssertFalse(s.cell.canCommit, "still unnamed")
        XCTAssertTrue(s.cell.canShare, "but GOLD is answerable already")
    }

    /// GOLD now has a compose identity on every surface, because every surface has an
    /// audience. This is the shape that used to be `hasShare == false`.
    func testGoldAlwaysHasAComposeIdentity() {
        let s = NewSession(kind: .place) { _ in .saved }
        s.draft.name = "the kitchen"
        let gold = InterfaceMatrix.spec(.chat, in: .composing(s.cell))
        XCTAssertTrue(gold.showsGlyph)
        XCTAssertTrue(gold.enabled)
        XCTAssertEqual(gold.glyph, "chevron.up")
    }

    /// The matrix's no-share arm is still a real branch and still correct: a cell that
    /// declares no GOLD action leaves the disc glyphless and disabled at the row's own
    /// inactive opacity, rather than removing it. The three circles never come and go —
    /// that persistence is what makes them slide instead of cross-fading.
    ///
    /// Exercised on a hand-built cell because no session produces one today; the branch
    /// exists so `NewCell` stays a total description of the row.
    func testACellWithNoShareLeavesGoldWithoutAnIdentity() {
        let cell = NewCell(canCommit: true, canShare: false, hasShare: false)
        let gold = InterfaceMatrix.spec(.chat, in: .composing(cell))
        XCTAssertFalse(gold.showsGlyph, "no compose glyph without a share")
        XCTAssertFalse(gold.enabled, "and it must not be tappable")
        XCTAssertFalse(gold.isActive)
    }

    /// The matrix reads the commit LABEL off the cell, so BLUE's spoken name is the
    /// surface's own verb. This is also what keeps `app.buttons["Save Place"]` working
    /// for the UI tests now that the button is the shell's disc.
    func testCommitDiscTakesTheKindsLabel() {
        let s = NewSession(kind: .place) { _ in .saved }
        s.draft.name = "the bar"
        let blue = InterfaceMatrix.spec(.life, in: .composing(s.cell))
        XCTAssertEqual(blue.label, "Save Place")
        XCTAssertTrue(blue.enabled)

        let space = NewSession(kind: .space) { _ in .saved }
        space.draft.name = "Aries"
        XCTAssertEqual(InterfaceMatrix.spec(.life, in: .composing(space.cell)).label,
                       "Create Space")
    }

    /// RED cancels in every context, named or blank — it is the one disc that is always
    /// live while a surface is up.
    func testCancelDiscIsAlwaysLive() {
        for kind in NewKind.allCases {
            let s = NewSession(kind: kind) { _ in .saved }
            let red = InterfaceMatrix.spec(.note, in: .composing(s.cell))
            XCTAssertTrue(red.isActive, "\(kind) RED")
            XCTAssertTrue(red.enabled, "\(kind) RED")
            XCTAssertEqual(red.glyph, "xmark")
        }
    }

    /// The NOTE's cell, preserved exactly. This is the behaviour the generalisation had
    /// to keep: Share and Send light up together, only once there is content.
    func testNoteCellMatchesTheOriginalNoteBehaviour() {
        XCTAssertEqual(NewCell.note(hasContent: false),
                       NewCell(canCommit: false, canShare: false, hasShare: true,
                               commitLabel: "Send"))
        let live = NewCell.note(hasContent: true)
        XCTAssertTrue(live.canCommit)
        XCTAssertTrue(live.canShare)
    }

    /// A note has no name — its BODY is its gate, which is the one place the
    /// name-is-required rule does not apply.
    func testANoteCommitsOnItsBodyAlone() {
        let s = NewSession(kind: .note) { _ in .saved }
        XCTAssertFalse(s.cell.canCommit)
        s.draft.body = "a thought"
        XCTAssertTrue(s.cell.canCommit, "a titleless note is still a note")
    }

    // MARK: - The draft crossing

    /// One draft name crosses ONCE and the core lands it on each kind's own arg key.
    /// Here we pin the Swift half: the value arrives trimmed, in `name`.
    func testDraftCrossingTrimsTheName() {
        var d = NewDraft()
        d.name = "  Aries  "
        XCTAssertEqual(d.mint(for: .space).name, "Aries")
    }

    /// A Space's purpose crosses as `descriptor`; the CORE folds it into the ContactCard
    /// `note`. The UI hand-built that JSON before, which is exactly how the declared
    /// field and the delta drifted apart — so the crossing must send no card at all.
    func testTheUiNeverHandBuildsTheCard() {
        var d = NewDraft()
        d.name = "Aries"
        d.descriptor = "we build things"
        let m = d.mint(for: .space)
        XCTAssertEqual(m.card, "", "card is the core's job, from descriptor")
        XCTAssertEqual(m.descriptor, "we build things")
    }

    /// An event's dates cross as unix MILLIseconds, and an absent end is 0 (open-ended)
    /// rather than a fabricated hour.
    func testEventDatesCrossAsMilliseconds() {
        var d = NewDraft()
        d.name = "gig"
        d.date = Date(timeIntervalSince1970: 1_700_000_000)
        let m = d.mint(for: .event)
        XCTAssertEqual(m.startMs, 1_700_000_000_000)
        XCTAssertEqual(m.endMs, 0, "no end ⇒ open-ended, not an invented duration")
    }

    /// A lineup crosses newline-separated, billing order preserved — the shape
    /// `event::set_profile_args` expects.
    func testLineupCrossesNewlineSeparated() {
        var d = NewDraft()
        d.name = "gig"
        d.lineup = ["Headliner", "Support"]
        XCTAssertEqual(d.mint(for: .event).lineup, "Headliner\nSupport")
    }

    /// An unplaced Place still crosses — `placed: false` mints it UNPLACED rather than
    /// refusing, so slow indoor GPS never costs the user their typing.
    func testUnplacedPlaceStillCrosses() {
        var d = NewDraft()
        d.name = "the allotment"
        let m = d.mint(for: .place)
        XCTAssertFalse(m.placed)
        XCTAssertEqual(m.name, "the allotment")
    }

    // MARK: - Commit

    /// A failed commit returns nil so the shell leaves the surface UP with its content
    /// intact — the only honest thing to do with a mint that did not happen.
    func testFailedCommitReportsNil() async {
        let s = NewSession(kind: .space) { _ in nil }
        s.draft.name = "Aries"
        let outcome = await s.performCommit()
        XCTAssertNil(outcome)
    }

    /// A blank surface cannot commit even if something calls it directly — the gate is
    /// in the session, not only on the disc.
    func testCommitRefusesABlankDraft() async {
        var ran = false
        let s = NewSession(kind: .space) { _ in ran = true; return .saved }
        let outcome = await s.performCommit()
        XCTAssertNil(outcome)
        XCTAssertFalse(ran, "the commit closure must not run on a blank draft")
    }
}
