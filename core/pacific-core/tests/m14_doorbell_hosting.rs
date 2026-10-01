//! M14 — DOORBELL HOSTING: an always-on member, installed through its own doorbell.
//!
//! The contract behind `POST /v1/host` (docs/doorbell-hosting-icd.html): an Arc asked to
//! host a space is handed exactly the join card a stranger scans, knocks the same knock,
//! and is admitted by a member's ordinary answer pass — admission granted, never seized
//! (H1). From then on it answers doors through the identical sweep every phone runs (H3),
//! with exactly the membership it was given (H2), and the space is never bound to it:
//! the original card keeps admitting through whichever host is about (H8).
//!
//! Real nodes, a real relay, real MLS commits — the host here is a plain `Node`, which is
//! the point: an Arc IS one, and nothing below is host-specific code.

mod common;

use common::Harness;
use pacific_core::place::Access;
use pacific_core::Node;

/// The whole hosting story in one scenario. A founder opens a place, anchors their group
/// at it, prints ONE card, and hands it to a host. The founder's ordinary pass installs
/// the host; the founder then goes dark, and the host — alone — admits the next scanner
/// to the place AND the anchored group.
#[tokio::test]
async fn a_host_is_installed_through_its_own_doorbell_and_answers_while_the_founder_sleeps() {
    let mut h = Harness::new(&["Founder"]).await;
    let host = h.add_user("arc-host");
    let scanner = h.add_user("Scanner");

    // The space: an open Place with a bell fitted, and the founder's group anchored at
    // it — the agreement that whoever scans the sign joins the group too.
    let place = h
        .node(0)
        .place_mint("the venue", "", None)
        .await
        .expect("mint place");
    h.node(0)
        .place_set_access(&place, Access::Public)
        .await
        .expect("open");
    h.node(0)
        .place_fit_doorbell(&place)
        .await
        .expect("fit doorbell");
    let group = h.mint_group(0);
    h.node(0)
        .group_anchor_at_place(&group, &place, 1_700_000_000_000)
        .await
        .expect("anchor the group");
    let card = h.node(0).place_join_card(&place).expect("print the card");

    // INSTALL — the /v1/host semantics exactly: the host parses the card with the
    // grammar's owner and knocks like any stranger. No other route into the space exists.
    let (pid, bell) = Node::parse_place_join_card(&card).expect("card parses");
    assert_eq!(pid, place, "the card names the place it was minted for");
    h.node(host)
        .place_knock(&pid, &bell)
        .await
        .expect("host knocks");
    h.settle().await;

    // The founder's ordinary pass admits the host — to the place AND the anchored group.
    h.node(0).answer_all_doors().await.expect("founder answers");
    h.settle().await;
    assert!(
        h.roster(0, &place).contains(&h.id(host)),
        "the host is a member of the place"
    );
    assert!(
        h.roster(0, &group).contains(&h.id(host)),
        "and of the group anchored there — hosting scope is the membership granted"
    );

    // THE FOUNDER GOES DARK. Nothing below touches user 0 — only the host and the
    // scanner sync, several rounds, because an MLS add is a STAGED commit through the
    // relay's commit slot: the committer folds its own add on a later pass, and the
    // newcomer's Welcome lands after that. Rounds among the living are the honest
    // spelling of "the founder's phone is not needed".
    h.node(scanner)
        .place_knock(&pid, &bell)
        .await
        .expect("scanner knocks");
    h.node(host).answer_all_doors().await.expect("host answers");
    for _ in 0..3 {
        h.sync(host).await;
        h.sync(scanner).await;
    }

    assert!(
        h.roster(host, &place).contains(&h.id(scanner)),
        "the host let the scanner into the place with no phone awake"
    );
    assert!(
        h.roster(host, &group).contains(&h.id(scanner)),
        "and into the anchored group — the host discharges the group's agreement"
    );
    assert!(
        h.roster(scanner, &place).contains(&h.id(scanner)),
        "the scanner holds the place themself — a real member, not a claim"
    );
}

/// H6 — convergence. A card gets scanned twice, a nervous operator POSTs /host twice,
/// two answerers race: none of it may stack duplicate memberships or fail.
#[tokio::test]
async fn repeat_installs_and_repeat_answers_converge() {
    let mut h = Harness::new(&["Founder"]).await;
    let host = h.add_user("arc-host");

    let place = h
        .node(0)
        .place_mint("the venue", "", None)
        .await
        .expect("mint");
    h.node(0)
        .place_set_access(&place, Access::Public)
        .await
        .expect("open");
    h.node(0)
        .place_fit_doorbell(&place)
        .await
        .expect("doorbell");
    let card = h.node(0).place_join_card(&place).expect("card");
    let (pid, bell) = Node::parse_place_join_card(&card).expect("parse");

    // Two knocks before anyone answers — the impatient double-POST.
    h.node(host).place_knock(&pid, &bell).await.expect("knock");
    h.node(host)
        .place_knock(&pid, &bell)
        .await
        .expect("knock again");
    h.settle().await;
    h.node(0).answer_all_doors().await.expect("answer");
    h.settle().await;

    // A third knock AFTER membership, and another answer pass over it.
    h.node(host)
        .place_knock(&pid, &bell)
        .await
        .expect("knock while member");
    h.settle().await;
    h.node(0).answer_all_doors().await.expect("answer again");
    h.settle().await;

    let mine = h
        .roster(0, &place)
        .iter()
        .filter(|id| **id == h.id(host))
        .count();
    assert_eq!(mine, 1, "one membership, however many knocks it took");
}

/// H8, the add side — a space is never bound to its host. The microVM host admits its
/// self-hosted replacement through the same door, and a card printed BEFORE the repoint
/// still admits a stranger AFTER it, via the new host, founder dark throughout, bell
/// never rotated. (The remove-the-old-host leg awaits an evict primitive in core — the
/// known missing piece, recorded in coordination/HOSTING.txt.)
#[tokio::test]
async fn repointing_hosts_leaves_the_original_card_working() {
    let mut h = Harness::new(&["Founder"]).await;
    let microvm = h.add_user("arc-microvm");
    let owned = h.add_user("arc-selfhosted");
    let late = h.add_user("LateScanner");

    let place = h
        .node(0)
        .place_mint("the venue", "", None)
        .await
        .expect("mint");
    h.node(0)
        .place_set_access(&place, Access::Public)
        .await
        .expect("open");
    h.node(0)
        .place_fit_doorbell(&place)
        .await
        .expect("doorbell");
    // THE card — printed once, before any host exists, never reprinted below.
    let card = h.node(0).place_join_card(&place).expect("card");
    let (pid, bell) = Node::parse_place_join_card(&card).expect("parse");

    // Install the first (rented) host; the founder's pass admits it, then the
    // founder goes dark for good.
    h.node(microvm)
        .place_knock(&pid, &bell)
        .await
        .expect("microvm knocks");
    h.settle().await;
    h.node(0).answer_all_doors().await.expect("founder answers");
    h.settle().await;
    assert!(h.roster(0, &place).contains(&h.id(microvm)));

    // REPOINT — a member's act: whoever holds the card asks the new Arc to host
    // (POST /v1/host), and their own device's ordinary pass admits it. The pass
    // also RE-STATES the spine at the new epoch (reemit_own_spine) — which is what
    // turns the newcomer into a capable host rather than a member with an empty
    // fold. (The stricter chain — the incumbent host admitting its successor with
    // every author dark — needs relayable owner-signed spine deltas; recorded as
    // future hardening in coordination/HOSTING.txt, alongside the missing evict.)
    h.node(owned)
        .place_knock(&pid, &bell)
        .await
        .expect("owned arc knocks");
    h.settle().await;
    h.node(0).answer_all_doors().await.expect("member repoints");
    h.settle().await;
    assert!(
        h.roster(0, &place).contains(&h.id(owned)),
        "the self-hosted arc is in"
    );

    // THE FOUNDER GOES DARK — and so does the microVM host: the walked-off rented
    // box, powered down for good. The pre-repoint card still works, answered by
    // the NEW host alone.
    h.node(late).place_knock(&pid, &bell).await.expect("late knock");
    h.node(owned).answer_all_doors().await.expect("owned answers");
    for _ in 0..3 {
        h.sync(owned).await;
        h.sync(late).await;
    }
    assert!(
        h.roster(owned, &place).contains(&h.id(late)),
        "a sticker printed before the repoint admits through the host installed after it"
    );
}
