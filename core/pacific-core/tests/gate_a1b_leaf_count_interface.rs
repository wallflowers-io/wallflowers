//! A1b, AT THE INTERFACE — `Directory::group_leaf_count` beside the deduplicated
//! people rows, and the dedup itself pinned so nobody simplifies it away.
//!
//! `docs/the-build.html` §02 A1b:
//!
//! > Stop conflating two questions in one call. *Who is authorised here* stays
//! > people, deduplicated, feeding the fold. *Do I need the relay* becomes leaves.
//! > … Keep it as a scalar beside the rows, add `group_leaf_count`, point the five
//! > gates at it, leave the ~32 other `group_members` consumers alone.
//!
//! > Do not drop the dedup. That same primary key is what stops two leaves
//! > inflating every membership count in the system — the wallet's quorum bands,
//! > `is_member`, `MembershipLog::divergence`. `mls::roster_identities` returns a
//! > `Vec` with no dedup of its own, so the table is the only thing making a roster
//! > a set of people.
//!
//! THIS BINARY IS SEPARATE FROM THE GATES ON PURPOSE. It names an interface that
//! does not exist yet, so until A1b lands it does not COMPILE — and a compile
//! error takes its whole binary with it. `gate_g1_g2_stoma_multi_device.rs` names
//! nothing new and therefore cannot be stopped by a naming disagreement here.
//!
//! TODAY: does not compile — there is no `Directory::group_leaf_count`. (And the
//! three-leaf premise every test here rests on cannot be built either, because
//! A1a has not admitted the second leaf.)
//! AFTER: passes.

mod common;

use common::Harness;
use pacific_core::mls;

/// THE ONE DETAIL THE SPEC DOES NOT PIN, absorbed here rather than guessed at.
///
/// `docs/execution.html` §04 pins the NAME and the argument —
/// `Directory::group_leaf_count(group_id) -> leaves` — but not whether the return
/// is `usize`, a narrower integer, or (as every other `Directory` method) wrapped
/// in `Result<_, CoreError>`. Rather than pick one and fail the suite over a
/// disagreement that is not the thing under test, this normalises every plausible
/// shape to `usize`. It is reported as a finding; it is not an invention, because
/// it commits to no shape.
///
/// What it deliberately does NOT absorb: the name, the receiver, or the argument.
/// Those are pinned, and a mismatch there must fail loudly.
trait LeafCount {
    fn leaves(self) -> usize;
}
impl LeafCount for usize {
    fn leaves(self) -> usize {
        self
    }
}
impl LeafCount for u32 {
    fn leaves(self) -> usize {
        self as usize
    }
}
impl LeafCount for u64 {
    fn leaves(self) -> usize {
        self as usize
    }
}
impl LeafCount for i64 {
    fn leaves(self) -> usize {
        assert!(self >= 0, "a leaf count cannot be negative");
        self as usize
    }
}
impl<E: std::fmt::Debug> LeafCount for Result<usize, E> {
    fn leaves(self) -> usize {
        self.expect("group_leaf_count failed")
    }
}
impl<E: std::fmt::Debug> LeafCount for Result<u32, E> {
    fn leaves(self) -> usize {
        self.expect("group_leaf_count failed") as usize
    }
}
impl<E: std::fmt::Debug> LeafCount for Result<u64, E> {
    fn leaves(self) -> usize {
        self.expect("group_leaf_count failed") as usize
    }
}
impl<E: std::fmt::Debug> LeafCount for Result<i64, E> {
    fn leaves(self) -> usize {
        let n = self.expect("group_leaf_count failed");
        assert!(n >= 0, "a leaf count cannot be negative");
        n as usize
    }
}

/// `group_leaf_count` on device `u` for object `obj`, read through the same
/// `Directory` handle every `node.rs` gate holds.
fn leaf_count(h: &Harness, u: usize, obj: &str) -> usize {
    let gid = hex::decode(obj).expect("an object id is hex");
    h.with_sync(u, |n| n.dir.group_leaf_count(&gid).leaves())
}

/// THE SEPARATION, stated as the two calls the five gates have to choose between.
///
/// One object with ONE member and TWO leaves (axel's own notebook) and one with
/// TWO members and THREE leaves (the cookbook thread). In both, the people count
/// and the leaf count are DIFFERENT NUMBERS, and each call must return its own.
///
/// The leaf count is asserted against the live ratchet tree (`Harness::leaves`,
/// which reads `group.roster().members().len()` off the MLS group itself), not
/// against a literal. A scalar cached in the directory that has drifted from the
/// tree is exactly the bug this pins.
///
/// TODAY: does not compile.
/// AFTER: passes.
///
/// HOW THIS COULD BE FAKED. (a) `group_leaf_count` returning
/// `group_members(..).len()` — closed, because the notebook's two numbers differ
/// and the test compares against the tree. (b) Hard-coding a leaf count at insert
/// time that never updates — closed by re-reading after a SECOND add, below.
/// (c) Making `group_members` return leaves so both agree — closed by asserting
/// the people count separately, and by `g2_*`.
#[tokio::test]
async fn group_leaf_count_counts_leaves_where_group_members_counts_people() {
    let mut h = Harness::people(&[("axel", &["phone"]), ("szonja", &["phone"])]).await;
    let phone = h.device("axel", "phone");
    let szonja = h.device("szonja", "phone");
    let laptop = h.add_device("axel", "laptop");

    // ── a person's OWN object: 1 member, 2 leaves ─────────────────────────────
    let notebook = h.form_named_forum(phone, "axel's notebook");
    assert_eq!(leaf_count(&h, phone, &notebook), 1, "one leaf to start");
    h.add_to_forum(phone, laptop, &notebook).await;

    for u in [phone, laptop] {
        assert_eq!(
            h.roster(u, &notebook).len(),
            1,
            "{}: ONE PERSON — the deduplicated rows are untouched by A1b",
            h.device_name(u)
        );
        // The gate reads every leaf, pool leaves included (resumption.md §5); the
        // DEVICE leaves are the claim here.
        assert_eq!(
            leaf_count(&h, u, &notebook) - h.pool_leaves(u, &notebook),
            2,
            "{}: TWO LEAVES — this is the number the five node.rs gates must read, \
             and the reason a person's own groups stopped reaching the relay",
            h.device_name(u)
        );
        assert_eq!(
            leaf_count(&h, u, &notebook),
            h.leaves(u, &notebook),
            "{}: the cached scalar must equal the live ratchet tree's width — a \
             leaf count that has drifted from the tree is worse than none",
            h.device_name(u)
        );
    }

    // ── a shared object: 2 members, 3 leaves ──────────────────────────────────
    h.pair(phone, szonja).await;
    let cookbook = h.form_forum(phone, &[szonja]).await;
    assert_eq!(leaf_count(&h, phone, &cookbook) - h.pool_leaves(phone, &cookbook), 2);
    h.add_to_forum(phone, laptop, &cookbook).await;

    for u in [phone, laptop, szonja] {
        assert_eq!(h.roster(u, &cookbook).len(), 2, "{}: two people", h.device_name(u));
        assert_eq!(
            leaf_count(&h, u, &cookbook) - h.pool_leaves(u, &cookbook),
            3,
            "{}: three leaves",
            h.device_name(u)
        );
        assert_eq!(leaf_count(&h, u, &cookbook), h.leaves(u, &cookbook));
    }

    // ── and it MOVES: a third device of axel's, and the scalar follows ────────
    // A count written once at group creation would still read 3 here.
    let tablet = h.add_device("axel", "tablet");
    h.add_to_forum(phone, tablet, &cookbook).await;
    for u in [phone, laptop, tablet, szonja] {
        assert_eq!(
            h.roster(u, &cookbook).len(),
            2,
            "{}: STILL two people — three of axel's leaves are still one axel",
            h.device_name(u)
        );
        assert_eq!(
            leaf_count(&h, u, &cookbook) - h.pool_leaves(u, &cookbook),
            4,
            "{}: four leaves, and the scalar tracked the add",
            h.device_name(u)
        );
        assert_eq!(leaf_count(&h, u, &cookbook), h.leaves(u, &cookbook));
    }
}

/// `mls::roster_identities` RETURNS A VEC WITH NO DEDUP OF ITS OWN, and the
/// directory's `PRIMARY KEY (group_id, member_pk)` is the only thing making a
/// roster a set of people. Both halves pinned, because the danger is that someone
/// "simplifies" either one and the other silently stops holding.
///
/// Seven of the eight `set_group_members` callers pass `roster_identities`
/// straight in — leaves, duplicates included — so if the primary key ever stops
/// deduplicating, every membership count in the system inflates at once: the
/// wallet's quorum bands, `is_member`, `MembershipLog::divergence`. This is the
/// test that fails on the day that happens rather than the quarter after.
///
/// It also pins the A1a half that makes all of it safe: BOTH of axel's leaves
/// carry the SAME `cred_id`. If a future fix gives a device its own credential,
/// `roster_identities` stops containing a duplicate — this test goes red, and it
/// should, because the fold, authorship, authority and `space_id` all key on that
/// value.
///
/// TODAY: does not compile (and its three-leaf premise needs A1a).
/// AFTER: passes.
///
/// HOW THIS COULD BE FAKED. (a) Dedup inside `roster_identities` so the leaf count
/// can be taken from it — closed by asserting the duplicate is present.
/// (b) Count `group_members` rows and call it leaves — closed by asserting the two
/// lengths DIFFER here. (c) Per-device credentials — closed by asserting both of
/// axel's entries equal his identity key.
#[tokio::test]
async fn roster_identities_does_not_dedup_and_the_primary_key_is_what_does() {
    let mut h = Harness::people(&[("axel", &["phone"]), ("szonja", &["phone"])]).await;
    let phone = h.device("axel", "phone");
    let szonja = h.device("szonja", "phone");
    let laptop = h.add_device("axel", "laptop");

    h.pair(phone, szonja).await;
    let cookbook = h.form_forum(phone, &[szonja]).await;
    h.add_to_forum(phone, laptop, &cookbook).await;

    let axel = h.id(phone);
    assert_eq!(h.id(laptop), axel, "the premise: one account, two devices");

    for u in [phone, laptop, szonja] {
        // The Vec, straight off the ratchet tree.
        // Pool leaves (resumption.md §5) carry the person's credential too; the
        // claim is about DEVICE leaves.
        let leaves: Vec<[u8; 32]> = h.device_identities(u, &cookbook);
        assert_eq!(
            h.with_mls_group(u, &cookbook, |g| mls::roster_identities(g).unwrap()).len()
                - h.pool_leaves(u, &cookbook),
            leaves.len(),
            "roster_identities is per-leaf"
        );
        assert_eq!(
            leaves.len(),
            3,
            "{}: roster_identities is per-LEAF and returns three entries",
            h.device_name(u)
        );
        assert_eq!(
            leaves.iter().filter(|pk| **pk == axel).count(),
            2,
            "{}: axel's identity key appears TWICE — roster_identities does no \
             dedup of its own, and it must not start: it is where the leaf count \
             comes from, and its duplicate is what the primary key absorbs",
            h.device_name(u)
        );
        assert_eq!(
            leaves.iter().filter(|pk| **pk == h.id(szonja)).count(),
            1,
            "{}: szonja once",
            h.device_name(u)
        );

        // The table, which is the same Vec after the primary key has had it.
        let rows = h.roster(u, &cookbook);
        assert_eq!(
            rows.len(),
            2,
            "{}: TWO rows. `set_group_members` is handed the three-entry Vec above \
             and INSERT OR IGNORE against PRIMARY KEY (group_id, member_pk) is the \
             only thing that turns it into a set of people",
            h.device_name(u)
        );
        let mut sorted = rows.clone();
        sorted.sort();
        let before = sorted.len();
        sorted.dedup();
        assert_eq!(before, sorted.len(), "{}: no duplicate rows", h.device_name(u));
        assert!(rows.contains(&axel) && rows.contains(&h.id(szonja)));

        // The two questions, and they have different answers.
        assert_ne!(
            leaves.len(),
            rows.len(),
            "{}: leaves and people are the same number here, which means one of \
             the two calls has been made to answer the other's question",
            h.device_name(u)
        );

        // And the scalar A1b adds is the Vec's length, not the table's.
        assert_eq!(
            leaf_count(&h, u, &cookbook),
            h.with_mls_group(u, &cookbook, |g| mls::roster_identities(g).unwrap()).len(),
            "{}: group_leaf_count IS the length of roster_identities",
            h.device_name(u)
        );
    }
}
