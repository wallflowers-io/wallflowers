//  GroupsPageUITests.swift
//  The GROUPS page contract on the REAL app: the GROUPS jewel puts the groups
//  list on the ground (search pill + rows), the ＋ constitutes a real Group
//  GroupObject through OrgWizard, the group's page lists its FORUMS, and
//  minting a forum opens a working ThreadView chatroom. Data is whatever this
//  device truly has: an empty world shows the honest empty word, never a fixture.

import XCTest

final class GroupsPageUITests: XCTestCase {

    private let app = XCUIApplication()
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        addUIInterruptionMonitor(withDescription: "system-alert") { [weak self] _ in
            self?.tapSpringboard(["Allow", "OK", "Don’t Allow", "Don't Allow"]) ?? false
        }
    }

    func test_groupsPage_constituteGroup_mintForum_openThread() {
        // No wake banners: on a device with accumulated runs a banner can land
        // mid-flow, and XCTest's interruption handling taps it — opening an old
        // room's cover over the surface this test is driving.
        app.launchArguments += ["-pacificNoWake"]
        app.launch()
        _ = tapSpringboard(["Allow", "Don’t Allow", "Don't Allow"], waitFor: 4)

        // A FRESH install fronts the Welcome gate (name → begin) ahead of the
        // shell; walk it the way a person does. A seasoned device skips this.
        let welcome = app.textFields["welcome-name"]
        if welcome.waitForExistence(timeout: 4) {
            welcome.tap()
            welcome.typeText("QA")
            app.buttons["welcome-begin"].tap()
        }

        let life = app.buttons["shell-life"]
        XCTAssertTrue(life.waitForExistence(timeout: 10), "LIFE disc")
        // The axial flip is GONE (10 Aug rail simplification) — a second tap on
        // the LIFE disc is the HOME gesture now, safe to make blindly.
        if !life.isSelected { life.tap() }

        // LIFE lands on the FEED; the GROUP jewel maximises to the flat typed
        // list (S27 — the old GroupsPage ground left the rail with the config
        // pages). Buttons, narrowly: the jewels wear `.isButton` (RibbonRail),
        // and a broad `.any` descendants query snapshots the whole LIFE
        // hierarchy — heavy enough to take the test runner down.
        let jewel = app.buttons["jewel-groups"]
        XCTAssertTrue(jewel.waitForExistence(timeout: 10), "GROUP jewel")
        jewel.tap()
        let groupsList = app.scrollViews["typed-list-groups"]
        XCTAssertTrue(groupsList.waitForExistence(timeout: 10)
                        || app.otherElements["typed-list-groups"].exists,
                      "GROUP maximised puts its typed list on the ground")
        shoot("groups-page")

        // ＋ constitutes a real Group GroupObject through OrgWizard (the LIFE
        // chrome's one `+`, minting the selected category). The name is unique
        // per run — repeated runs leave real groups on the device (there is
        // deliberately no delete), and a duplicate name makes `firstMatch`
        // ambiguous.
        let groupName = "QA Club \(Int(Date().timeIntervalSince1970) % 100_000)"
        app.buttons["tab-new"].tap()
        let nameField = app.textFields["Butterrow Community Garden"]
        XCTAssertTrue(nameField.waitForExistence(timeout: 6), "OrgWizard's name field")
        nameField.tap()
        nameField.typeText(groupName)
        // The wizard's commit is a ComposeDiscs CircleButton wearing its label
        // ("Constitute" for the default community type).
        let constitute = app.buttons["Constitute"]
        XCTAssertTrue(constitute.waitForExistence(timeout: 6), "the wizard's commit disc")
        constitute.tap()
        let done = app.buttons["org-done"]
        XCTAssertTrue(done.waitForExistence(timeout: 8), "the office board's Done")
        done.tap()
        // Let the wizard's cover finish dismissing — its header carries the group
        // name too, and a match against that mid-animation is not hittable.
        XCTAssertTrue(waitGone(done, timeout: 6), "the wizard dismisses")

        // The new group re-lists without a manual refresh. The typed list's row
        // is ONE combined card element (ObjectCard via TypedListPage) whose
        // label carries the title — query the card identifier space, never a
        // bare staticText that could also match chrome.
        let rowMatch = NSPredicate(format: "identifier BEGINSWITH 'list-row-group:' AND label CONTAINS %@",
                                   groupName)
        var row = app.otherElements.matching(rowMatch).firstMatch
        if !row.waitForExistence(timeout: 8) {
            row = app.staticTexts.matching(rowMatch).firstMatch
        }
        XCTAssertTrue(row.exists, "the minted group's row")
        shoot("groups-listed")

        // The group's page: FORUMS (empty, honest) + the owner's ＋. Tap by
        // COORDINATE: the row is visibly on the ground, but the rail's full-canvas
        // sibling makes the accessibility hittability oracle flaky here (isHittable
        // false while real touches land fine — same class of problem as the jewels
        // needing explicit button traits). A coordinate tap is what a finger does.
        row.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        // The clean Space Profile (11 Aug order): headline card, then jewels.
        // Channels live behind the CHANNELS jewel; the ＋ rides that lane.
        // (They used to share the POSTS lane — a channel is a Forum object,
        //  a post is a delta inside one, and one lane could not name both.)
        XCTAssertTrue(app.buttons["group-jewel-channels"].waitForExistence(timeout: 8),
                      "the space profile's jewels row")
        app.buttons["group-jewel-channels"].tap()
        let newForum = app.buttons["group-forum-new"]
        XCTAssertTrue(newForum.waitForExistence(timeout: 6), "owner sees the channel ＋ on CHANNELS")

        // MEMBER → PERSON: the PEOPLE jewel maximises the roster lane, and a
        // member row routes to the universal person profile (PersonLink →
        // ProfileStack). Tap the row by COORDINATE — the same hittability-oracle
        // lesson as the card→detail tap above.
        let peopleJewel = app.buttons["group-jewel-people"]
        XCTAssertTrue(peopleJewel.waitForExistence(timeout: 6), "the PEOPLE jewel on the space profile")
        peopleJewel.tap()
        let memberMatch = NSPredicate(format: "identifier BEGINSWITH 'group-member-'")
        var member = app.otherElements.matching(memberMatch).firstMatch
        if !member.waitForExistence(timeout: 6) {
            member = app.buttons.matching(memberMatch).firstMatch
        }
        XCTAssertTrue(member.exists, "my own member row on the PEOPLE lane")
        XCTAssertTrue(scrollTo(member), "the member row on screen")
        member.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        XCTAssertTrue(app.otherElements["profile-headline"].waitForExistence(timeout: 8)
                        || app.otherElements["profile-stack"].exists,
                      "the member row lands on the person profile page")
        shoot("member-person-page")
        app.navigationBars.buttons.firstMatch.tap()
        XCTAssertTrue(app.buttons["group-jewel-channels"].waitForExistence(timeout: 8),
                      "back on the space profile")

        // Mint a room — a real forum GroupObject plus its base.setPart edge.
        app.buttons["group-jewel-channels"].tap()
        XCTAssertTrue(newForum.waitForExistence(timeout: 6), "the CHANNELS lane ＋ again")
        newForum.tap()
        let alertField = app.alerts["New channel"].textFields.firstMatch
        XCTAssertTrue(alertField.waitForExistence(timeout: 6), "the naming alert")
        alertField.typeText("general")
        app.alerts["New channel"].buttons["Create"].tap()

        let room = app.staticTexts["general"].firstMatch
        XCTAssertTrue(room.waitForExistence(timeout: 10), "the room's row folds in")
        shoot("group-detail")

        // The room opens in the same ThreadView every chat surface uses, wearing
        // CHAT's one composer (the shared `Composer` slot, id "composer-field").
        room.tap()
        XCTAssertTrue(app.textFields["composer-field"].waitForExistence(timeout: 8),
                      "the room types through CHAT's composer")
        shoot("group-forum-thread")

        // ROOMS — the channel's constituent tabs, each a Forum of its own. The
        // owner's ＋ mints one; creating it opens it, and the channel tab leads back.
        let newRoom = app.buttons["room-new"]
        XCTAssertTrue(newRoom.waitForExistence(timeout: 8), "owner sees the room ＋ on the tab band")
        newRoom.tap()
        let roomField = app.alerts["New room"].textFields.firstMatch
        XCTAssertTrue(roomField.waitForExistence(timeout: 6), "the room naming alert")
        roomField.typeText("sub")
        app.alerts["New room"].buttons["Create"].tap()
        let subTab = app.buttons.matching(
            NSPredicate(format: "identifier BEGINSWITH 'room-tab-' AND label == 'sub'")).firstMatch
        XCTAssertTrue(subTab.waitForExistence(timeout: 10), "the room's tab folds in")
        // Creating OPENS the room, but only once the mint's relay flush returns —
        // wait for the tab's selected trait, or the next post races into the
        // still-bound channel (an empty channel and an empty room look identical).
        XCTAssertTrue(waitSelected(subTab, timeout: 10), "creating a room opens it")
        shoot("channel-room-tab")

        // The surface rebinds per tab: a post lands in the ROOM's log, and the
        // channel tab flips back to a channel this post never touched.
        let roomComposer = app.textFields["composer-field"]
        roomComposer.tap()
        roomComposer.typeText("room first!")
        app.buttons["composer-send"].tap()
        XCTAssertTrue(app.staticTexts["room first!"].waitForExistence(timeout: 8),
                      "the post folds into the room")
        let channelTab = app.buttons["room-tab-channel"]
        channelTab.tap()
        XCTAssertTrue(waitSelected(channelTab, timeout: 6), "the channel tab takes the surface back")
        XCTAssertTrue(app.staticTexts["No posts yet"].waitForExistence(timeout: 8),
                      "the channel's own thread is untouched — the post went to the room")
        subTab.tap()
        XCTAssertTrue(app.staticTexts["room first!"].waitForExistence(timeout: 8),
                      "the room's post is where it was left")
        shoot("channel-room-posted")

        // Pop the room before leaving LIFE — a parked stack must not keep a second
        // composer in the tree while CHAT's room cover presents its own.
        app.navigationBars.buttons.firstMatch.tap()

        // And in CHAT the room wears its group — the forum dress: the row reads
        // "QA Club … / general", never a free-standing "general".
        //
        // Arrival at CHAT always lands on CHATS (ShellState resets the scope on
        // the way in), and a hosted room is NOT a DM — so the row is legitimately
        // absent here and one tap of the gold disc brings it in.
        let gold = app.buttons["shell-chat"]
        gold.tap()
        let forumRow = app.buttons.matching(NSPredicate(
            format: "label CONTAINS %@ AND label CONTAINS %@", groupName, "/ general")).firstMatch
        XCTAssertTrue(waitGone(forumRow, timeout: 4),
                      "CHATS is DMs only, so the room is not on this stop")
        gold.tap()
        _ = forumRow.waitForExistence(timeout: 6)
        XCTAssertTrue(scrollTo(forumRow),
                      "SPACES shows the room as Group / Room")
        shoot("chat-forum-row")

        // Round the rotary once more. It has exactly TWO stops — the four chips
        // (All · Chats · Groups · Channels) became one disc — so the next tap
        // must drop the room again and the one after must bring it back. This
        // leaves the list on SPACES, which is where the tap-through below needs
        // it.
        gold.tap()
        XCTAssertTrue(waitGone(forumRow, timeout: 4), "back to CHATS — the room drops again")
        shoot("chat-scope-chats")
        gold.tap()
        _ = forumRow.waitForExistence(timeout: 3)
        XCTAssertTrue(scrollTo(forumRow), "and back to SPACES: exactly two stops, no third")

        // Tapping it RENDERS AS A FORUM — the threaded room ("No posts yet" is
        // ThreadView's empty word; a flat chat would say "No messages yet").
        forumRow.tap()
        XCTAssertTrue(app.staticTexts["No posts yet"].waitForExistence(timeout: 8),
                      "a hosted room opens threaded, never as a flat chat")
        XCTAssertTrue(app.staticTexts["\(groupName) / general"].firstMatch.exists,
                      "the room's title carries its group")
        shoot("chat-forum-opened")

        // The title is a DOOR, not a label: tapping it closes the room and lands on
        // the group's own page over on LIFE (the deep link the cover's title carries).
        app.buttons["room-group-link"].tap()
        XCTAssertTrue(app.navigationBars[groupName].waitForExistence(timeout: 8),
                      "the room's title deep-links to the group's page on LIFE")
        shoot("room-title-deep-link")

        // Return to CHAT and re-enter the room — the flow below happens inside it.
        app.buttons["shell-chat"].tap()
        _ = forumRow.waitForExistence(timeout: 6)
        XCTAssertTrue(scrollTo(forumRow), "back on the chat list")
        forumRow.tap()
        XCTAssertTrue(app.staticTexts["No posts yet"].waitForExistence(timeout: 8))

        // Post, then VOTE — the Reddit half: the cluster arrives with the post,
        // and my upvote folds into the score.
        let composer = app.textFields["composer-field"]
        composer.tap()
        composer.typeText("first! #qa")
        app.buttons["composer-send"].tap()
        let voteUp = app.buttons["vote-up"].firstMatch
        XCTAssertTrue(voteUp.waitForExistence(timeout: 8), "the post wears its vote cluster")
        voteUp.tap()
        let score = app.staticTexts["vote-score"].firstMatch
        XCTAssertTrue(score.waitForExistence(timeout: 6), "the score renders")
        XCTAssertTrue(waitLabel(score, "1", timeout: 6), "my upvote folds into the score")
        shoot("forum-voted")

        // REPLY is a tight strip on the composer (WhatsApp's shape) — the quote
        // block with its ✕, never a panel. The keyboard rises with it.
        app.buttons["Reply"].firstMatch.tap()
        XCTAssertTrue(app.buttons["reply-cancel"].waitForExistence(timeout: 6),
                      "the quote strip with its cancel")
        shoot("forum-reply-strip")
        app.typeText("welcome!")
        app.buttons["composer-send"].tap()
        XCTAssertTrue(app.staticTexts["welcome!"].waitForExistence(timeout: 8),
                      "the reply nests into the thread")
        shoot("forum-replied")

        // #TAGS in SEARCH: back on the list, "#qa" (typed into the post above)
        // reaches the room through the tag lane — a lane over the transcripts'
        // parsed tags, prefix-matched; a tag nobody used finds nothing.
        app.buttons["Done"].tap()
        // Land on CHAT explicitly — the cover can leave the shell elsewhere.
        // Both list screens wear a "Search" field and the slide keeps the
        // outgoing screen mounted for a beat, so take the HITTABLE one — and
        // restore hidden chrome (the field hides on scroll-down) on the way.
        app.buttons["shell-chat"].tap()
        var search = app.textFields["Search"].firstMatch
        _ = search.waitForExistence(timeout: 6)
        for _ in 0..<3 where !search.isHittable {
            app.swipeDown()
            if let visible = app.textFields.matching(identifier: "Search")
                .allElementsBoundByIndex.first(where: { $0.isHittable }) { search = visible }
        }
        XCTAssertTrue(search.isHittable, "the CHAT search field")
        search.tap()
        search.typeText("#qa")
        XCTAssertTrue(forumRow.waitForExistence(timeout: 8) || scrollTo(forumRow),
                      "the #tag lane finds the room")
        shoot("chat-tag-search")
        search.typeText("nope")   // "#qanope" — nobody used it
        XCTAssertTrue(waitGone(forumRow, timeout: 6), "an unused tag finds nothing")
    }

    // MARK: - Helpers

    /// Wait for an element to leave the hierarchy (there is no waitForNonExistence).
    /// PEOPLE RUNS UNDER ROLE HEADINGS. Walks to a Space this device already holds
    /// rather than constituting one: the roster grouping is a READ, and the mint
    /// path above is the other test's subject.
    ///
    /// A group of one is not a thin case — it is the case every Space starts in, and
    /// the one the lane could most easily get wrong: nobody has authored a
    /// `setMemberRole` overlay, so an OWNER heading here can only have come off the
    /// object's owner. A device with no Space of its own has nothing to assert on and
    /// says so, rather than passing on an empty screen.
    func test_spaceProfile_peopleLane_runsUnderRoleHeadings() {
        app.launchArguments += ["-pacificNoWake"]
        app.launch()
        _ = tapSpringboard(["Allow", "Don’t Allow", "Don't Allow"], waitFor: 4)

        let welcome = app.textFields["welcome-name"]
        if welcome.waitForExistence(timeout: 4) {
            welcome.tap()
            welcome.typeText("QA")
            app.buttons["welcome-begin"].tap()
        }

        let life = app.buttons["shell-life"]
        XCTAssertTrue(life.waitForExistence(timeout: 10), "LIFE disc")
        if !life.isSelected { life.tap() }
        let jewel = app.buttons["jewel-groups"]
        XCTAssertTrue(jewel.waitForExistence(timeout: 10), "GROUP jewel")
        jewel.tap()

        // Any element type: the row has been an Other and a StaticText (run 5), and a
        // lookup by one type misses it the day it becomes the other.
        let rowMatch = NSPredicate(format: "identifier BEGINSWITH 'list-row-group:'")
        let rows = app.descendants(matching: .any).matching(rowMatch)
        var row = rows.firstMatch
        _ = row.waitForExistence(timeout: 8)
        // A DEVICE WITH NO SPACE constitutes one, the way a person does: the shell's
        // ＋ mints the lit jewel's species, and GROUPS is lit. Installing a build
        // replaces the app's data container, so "there is already a Space here" is
        // not a thing a test may assume between runs.
        if !row.exists {
            app.buttons["tab-new"].tap()
            let nameField = app.textFields["new-field-name"]
            XCTAssertTrue(nameField.waitForExistence(timeout: 8), "the New surface's name field")
            nameField.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
            XCTAssertTrue(app.keyboards.element.waitForExistence(timeout: 6), "the keyboard")
            nameField.typeText("QA Space \(Int(Date().timeIntervalSince1970) % 100_000)")
            app.buttons["Create Space"].tap()
            row = rows.firstMatch
            XCTAssertTrue(row.waitForExistence(timeout: 12), "the constituted Space lists")
            shoot("space-constituted")
        }
        XCTAssertTrue(row.exists, "a Space on this device to open")
        XCTAssertTrue(scrollTo(row), "the group row on screen")
        row.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()

        let peopleJewel = app.buttons["group-jewel-people"]
        XCTAssertTrue(peopleJewel.waitForExistence(timeout: 8), "the PEOPLE jewel on the space profile")
        peopleJewel.tap()

        let heading = app.staticTexts["group-role-owner"]
        XCTAssertTrue(heading.waitForExistence(timeout: 6), "the OWNER heading over the roster's owner run")
        XCTAssertEqual(heading.label, "OWNER")
        let memberMatch = NSPredicate(format: "identifier BEGINSWITH 'group-member-'")
        var member = app.otherElements.matching(memberMatch).firstMatch
        if !member.waitForExistence(timeout: 6) {
            member = app.buttons.matching(memberMatch).firstMatch
        }
        XCTAssertTrue(member.exists, "a member card under the heading")
        // WHICH ONE IS ME. This device is the Space's only member, so the one card
        // on the lane is mine and has to say so — a roster that renders me exactly
        // like everyone else is the thing this marker exists to fix.
        XCTAssertTrue(member.label.contains("(me)"),
                      "my own card names me: \(member.label)")
        shoot("people-by-role")

        // AND WHAT AN EMPTY LANE SAYS: its own word, not a sentence about the
        // absence. Nothing rolls posts up across a Space's channels, so POSTS is
        // empty on every device — the honest lane to assert on.
        let posts = app.buttons["group-jewel-posts"]
        posts.tap()
        XCTAssertTrue(app.staticTexts["POSTS"].waitForExistence(timeout: 6),
                      "the empty POSTS lane says POSTS")
        shoot("empty-lane-word")

        // A LIT JEWEL BECOMES A ＋ WHERE THE SPACE CAN AUTHOR THAT KIND. POSTS is a
        // delta inside a channel, so the lane a Space cannot mint into keeps its
        // jewel even while it is the selected one — the negative case matters as
        // much as the positive, since this rail is read by non-owners too.
        XCTAssertEqual(posts.label, "POSTS", "POSTS is not a kind a Space mints")

        // CHANNELS is. I own this Space, so selecting it turns the tile into the ＋
        // and the tile mints — the create affordance the retired lane head row lost.
        let channels = app.buttons["group-jewel-channels"]
        channels.tap()
        let plus = NSPredicate(format: "label == %@", "New CHANNEL")
        XCTAssertTrue(app.buttons.matching(plus).firstMatch.waitForExistence(timeout: 6),
                      "the lit CHANNELS tile spins into a ＋")
        shoot("jewel-plus")
        channels.tap()
        // The New surface names itself in its header — the kind's noun, in the same
        // type an empty screen wears (New.swift). `new-surface-forum` is on the
        // surface's root, which XCUITest does not always publish as its own element;
        // the word it puts on screen is the honest thing to assert on.
        XCTAssertTrue(app.staticTexts["CHANNEL"].waitForExistence(timeout: 8),
                      "the ＋ raises the universal New surface for a Channel")
        shoot("jewel-plus-mints")

        // Mint it for real, then COUNT it. A held channel used to be drawn twice —
        // once as a pin bar above the rail, which showed on every lane, and once in
        // the CHANNELS lane below it. The pins are gone (26 Aug), so a room appears
        // exactly once, and only on its own lane.
        let name = "QA Room \(Int(Date().timeIntervalSince1970) % 100_000)"
        let field = app.textFields["new-field-name"]
        XCTAssertTrue(field.waitForExistence(timeout: 6), "the New surface's name field")
        // By COORDINATE, and wait for the keyboard before typing: the surface arrives
        // on an animation and does not auto-focus a Channel's name (only a Note's),
        // so a tap dispatched into the moving layout leaves nothing focused.
        field.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        XCTAssertTrue(app.keyboards.element.waitForExistence(timeout: 6),
                      "the keyboard follows the tap into the name field")
        field.typeText(name)
        app.buttons["Create Channel"].tap()

        // Count THIS room, not every room: the Space accumulates real channels across
        // runs (there is deliberately no delete), so a global count would measure the
        // device's history rather than the duplication this asserts against. A room's
        // row is a Button (run 4's hierarchy).
        let roomMatch = NSPredicate(format: "identifier BEGINSWITH 'group-forum-'")
        let mine = NSPredicate(format: "identifier BEGINSWITH 'group-forum-' AND label CONTAINS %@", name)
        XCTAssertTrue(app.buttons.matching(mine).firstMatch.waitForExistence(timeout: 10),
                      "the minted room lands on the CHANNELS lane")
        XCTAssertEqual(app.buttons.matching(mine).count, 1,
                       "the room is drawn ONCE on its own lane")
        shoot("channels-lane-once")

        // OPEN IT. The room bar routes to ThreadView, which now clears the same read
        // watermark the bar's count is drawn from (and sends the receipts) — so this
        // also proves the store it needs is in the environment on this route: a
        // missing EnvironmentObject is a crash, not a warning.
        app.buttons.matching(mine).firstMatch
            .coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        XCTAssertTrue(app.staticTexts["No posts yet"].waitForExistence(timeout: 8),
                      "the room opens on its own thread")
        app.navigationBars.buttons.firstMatch.tap()
        XCTAssertTrue(app.buttons.matching(mine).firstMatch.waitForExistence(timeout: 8),
                      "back on the CHANNELS lane")

        // And nowhere else: the pins used to ride above the rail whatever was lit. The
        // control first: the same query SEES the row here, so the 0 below is a finding
        // and not a query that matches nothing.
        let anyRoom = app.descendants(matching: .any).matching(roomMatch)
        XCTAssertGreaterThanOrEqual(anyRoom.count, 1, "the room query finds the row on the CHANNELS lane")
        peopleJewel.tap()
        XCTAssertTrue(app.staticTexts["group-role-owner"].waitForExistence(timeout: 6),
                      "back on the PEOPLE lane")
        // Any element type: a count of one type is vacuous the day the row becomes another
        // (the rows became Buttons and this read `otherElements`, which is always 0).
        XCTAssertEqual(anyRoom.count, 0,
                       "no channel bar survives above the rail on another lane")
        shoot("no-pins-above-rail")
    }

    /// THE WALL ROW NEVER SHARES THE STATUS STRIP. Two collisions, one contract:
    /// at rest the face, the name and the manage ellipsis sit BELOW the clock, on
    /// the back chevron's line; scrolled, they FADE OUT rather than riding up behind
    /// it. Both were real — the first shipped, the second is what the first one's
    /// fix would have become the moment anybody scrolled.
    ///
    /// The oracle is the screenshot pair, because this is a geometry and opacity
    /// contract and neither is queryable through the accessibility hierarchy: a
    /// faded view still `exists`. The assertion here is that the page survives the
    /// scroll and the row is gone from the hierarchy's hit-testable surface; the
    /// shots carry the rest for a human to read.
    func test_spaceProfile_wallRow_clearsTheStatusStrip() {
        app.launchArguments += ["-pacificNoWake"]
        app.launch()
        _ = tapSpringboard(["Allow", "Don’t Allow", "Don't Allow"], waitFor: 4)

        let welcome = app.textFields["welcome-name"]
        if welcome.waitForExistence(timeout: 4) {
            welcome.tap()
            welcome.typeText("QA")
            app.buttons["welcome-begin"].tap()
        }

        let life = app.buttons["shell-life"]
        XCTAssertTrue(life.waitForExistence(timeout: 10), "LIFE disc")
        if !life.isSelected { life.tap() }
        let jewel = app.buttons["jewel-groups"]
        XCTAssertTrue(jewel.waitForExistence(timeout: 10), "GROUP jewel")
        jewel.tap()

        let rowMatch = NSPredicate(format: "identifier BEGINSWITH 'list-row-group:'")
        var row = app.otherElements.matching(rowMatch).firstMatch
        if !row.waitForExistence(timeout: 8) {
            row = app.staticTexts.matching(rowMatch).firstMatch
        }
        XCTAssertTrue(row.exists, "a Space on this device to open")
        XCTAssertTrue(scrollTo(row), "the group row on screen")
        row.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()

        let cover = app.otherElements["group-cover"]
        XCTAssertTrue(cover.waitForExistence(timeout: 8), "the Space's wall")
        // AT REST: the row is below the strip, on the chevron's line.
        shoot("wall-row-at-rest")

        // SCROLLED: one deliberate drag, far enough that the row would be behind the
        // clock if it were still drawn. A swipe would fling the page and shoot at an
        // arbitrary offset; this lands somewhere the contract can be read.
        let top = app.windows.firstMatch.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.42))
        let up = app.windows.firstMatch.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.22))
        top.press(forDuration: 0.1, thenDragTo: up)
        shoot("wall-row-scrolled")

        // The page is still the page — the drag scrolled it, it did not dismiss it.
        XCTAssertTrue(app.buttons["group-back"].exists, "the chevron stays pinned")
        XCTAssertTrue(app.buttons["group-jewel-people"].exists, "the rail survives the scroll")
    }

    private func waitGone(_ element: XCUIElement, timeout: TimeInterval) -> Bool {
        let gone = NSPredicate(format: "exists == false")
        let exp = expectation(for: gone, evaluatedWith: element)
        return XCTWaiter().wait(for: [exp], timeout: timeout) == .completed
    }

    /// Wait for an element's label to become `value` — folds land a beat after the tap.
    private func waitLabel(_ element: XCUIElement, _ value: String, timeout: TimeInterval) -> Bool {
        let match = NSPredicate(format: "label == %@", value)
        let exp = expectation(for: match, evaluatedWith: element)
        return XCTWaiter().wait(for: [exp], timeout: timeout) == .completed
    }

    /// Wait for an element's `.isSelected` trait — a tab switch lands async.
    private func waitSelected(_ element: XCUIElement, timeout: TimeInterval) -> Bool {
        let match = NSPredicate(format: "selected == true")
        let exp = expectation(for: match, evaluatedWith: element)
        return XCTWaiter().wait(for: [exp], timeout: timeout) == .completed
    }

    /// A SwiftUI List materialises rows lazily, so an off-screen row does not
    /// EXIST yet — and on a device that accumulates QA runs (no delete, by
    /// design) a fresh no-message room sinks below the fold. Swipe until it
    /// turns up and is tappable; a no-op when it is already on screen.
    @discardableResult
    private func scrollTo(_ element: XCUIElement, tries: Int = 6) -> Bool {
        for _ in 0..<tries {
            if element.exists && element.isHittable { return true }
            app.swipeUp()
        }
        return element.exists
    }


    private func shoot(_ name: String) {
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }

    @discardableResult
    private func tapSpringboard(_ labels: [String], waitFor timeout: TimeInterval = 0) -> Bool {
        for label in labels {
            let b = springboard.buttons[label]
            let present = timeout > 0 ? b.waitForExistence(timeout: timeout) : b.exists
            if present { b.tap(); return true }
        }
        return false
    }
}
