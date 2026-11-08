//  PersonProfilePage.swift
//  The PERSON page — a bare host over the universal Profile component.
//
//  It was not always this. T1 needed HeadlineCard's onConnect/onIssue seam,
//  ProfileStack did not forward it, and ProfileStack.swift was not T1's file —
//  so this page composed the component's own cards by hand, mirroring the
//  person contract card-for-card, with an ON RECORD note asking M1 to "teach
//  ProfileStack action-forwarding and re-thin this page to a bare host". That
//  is what happened: `ProfileStack(source:subject:)` forwards the seam, and
//  everything this file used to duplicate now lives once, in the component.
//
//  The copy had already drifted before it was retired — its jewels row was
//  left-aligned behind a Spacer where the component centres them (the 11 Aug
//  order). Nobody changed it on purpose; that is simply what a second
//  implementation does. Adopting the component fixes it by deletion.
//
//  What remains here is the one thing this page genuinely owns: which peer it
//  is about, and the `person-profile` identifier the UI tests address it by.

import SwiftUI

struct PersonProfilePage: View {
    let id: String
    /// The injected binder. Defaults to the fixture so Previews stand alone;
    /// every real route passes `RealProfileSource` (Life.swift, Shell.swift).
    let source: any ProfileSource

    init(id: String, source: (any ProfileSource)? = nil) {
        self.id = id
        self.source = source ?? FixtureProfileSource.person(id)
    }

    var body: some View {
        // `subject: id` is what turns the person actions on — the Connect and
        // Issue circles, and the status value beside them. A Space passes none.
        ProfileStack(source: source, subject: id)
            .accessibilityIdentifier("person-profile")
    }
}

// MARK: - Previews (fixture-driven)

#Preview("Person page") {
    NavigationStack {
        PersonProfilePage(id: "fixture-dare")
    }
    .preferredColorScheme(.dark)
}
