//! Arc membership verification, reproduced END TO END in-process.
//!
//! WHY THIS EXISTS. A real device's membership credential was 401ing at a live Arc. The Arc's
//! `verify_member` accepts a caller iff (1) the signature verifies, (2) it is fresh, and (3) the
//! signer is on the roster of a `member-tether` — and then reads their ROLE. Server logs proved
//! the signature verified (so the client-side mint is correct) but the request still 401'd with
//! no membership-decision log — i.e. an ERROR was thrown AFTER the signature check and silently
//! mapped to "not a member". The only ops past that point are the roster read and the ROLE read
//! (`member_role` → `group_view` → fold the tether's group log).
//!
//! This test rebuilds the Arc's exact signup sequence with two real nodes over a real relay —
//! `pair_scan_kind("member-tether")` then `group.setMemberRole` (grant_default_role) — and then
//! reads the roster + role back the way `verify_member` does. If the fold poisons, the role read
//! throws HERE, in milliseconds, with the real `CoreError` — no Railway, no log streaming.
//!
//! `verify_member` itself lives in the arc `node` crate (a bin, not a lib), so its POST-signature
//! logic is transcribed here against the same public `Node` API it calls. The signature half is
//! already pinned by pacific-ffi's `minted_credential_satisfies_verify_member`.

use pacific_core::coordinator::{ArgVal, Args};
use pacific_core::group::GroupRole;
use pacific_core::roles::OP_SET_ROLE;
use pacific_core::Node;

const MEMBER_TETHER_KIND: &str = "member-tether";

async fn spawn_relay() -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { relay::serve(l).await });
    format!("ws://{addr}")
}

/// Pin the state dir to `relay` before its Node exists: its routes and its default Arc, or it
/// dials production's (`paths::DEFAULT_RELAY_URL`), and the self record minted with the
/// identity is stamped with production's Arc.
fn pin(relay: &str) {
    pacific_core::router::Routes::parse(relay).save().unwrap();
    pacific_core::node::set_default_arc(relay).unwrap();
}

/// Open the node rooted at `home` (PACIFIC_STATE_DIR is process-global, so every op re-points it
/// first — the same discipline the shared harness uses).
fn node_at(home: &std::path::Path) -> Node {
    std::env::set_var("PACIFIC_STATE_DIR", home);
    Node::open().unwrap()
}

/// verify_member's roster+role tail, transcribed. Returns:
///   Ok(Some(role)) — on the roster, role read cleanly (the ALLOW path),
///   Ok(None)       — signature-valid but on no member-tether roster (genuine non-member),
///   Err(e)         — a roster/role READ threw (the swallowed-error path we are hunting).
fn verify_roster_and_role(
    arc: &Node,
    member_pk: &[u8; 32],
) -> Result<Option<GroupRole>, pacific_core::CoreError> {
    let me = pacific_core::identity::parse_identity_key(&arc.identity_key())?;
    for (gid, kind) in arc.all_groups()? {
        if kind != MEMBER_TETHER_KIND {
            continue;
        }
        if arc
            .group_roster(&gid)?
            .into_iter()
            .any(|pk| pk != me && pk == *member_pk)
        {
            // The exact op that is reached ONLY after a roster hit — the suspect.
            let role = arc
                .member_role(&gid, member_pk)?
                .unwrap_or(GroupRole::Member);
            return Ok(Some(role));
        }
    }
    Ok(None)
}

#[tokio::test]
async fn arc_signup_then_verify_reads_member_role() {
    let relay = spawn_relay().await;

    let arc_home = tempfile::tempdir().unwrap();
    let dev_home = tempfile::tempdir().unwrap();

    // Mint the two identities (Arc = the server side, Device = the signing-up user). A fresh
    // state dir has no identity, so each is minted with `init_identity` before it can be opened.
    let arc_pk = {
        std::env::set_var("PACIFIC_STATE_DIR", arc_home.path());
        pin(&relay);
        let n = Node::init_identity("Arc").unwrap();
        n.id.identity_pk()
    };
    let (dev_pk, dev_bundle) = {
        std::env::set_var("PACIFIC_STATE_DIR", dev_home.path());
        pin(&relay);
        let n = Node::init_identity("Device").unwrap();
        (n.id.identity_pk(), n.build_contact_bundle().unwrap())
    };
    assert_ne!(arc_pk, dev_pk);

    // ── SIGNUP, exactly as arc-node does it ──────────────────────────────────────────────
    // 1) The Arc scans the device's bundle into a 2-member `member-tether` it owns.
    let tether = {
        let arc = node_at(arc_home.path());
        let peer = arc
            .pair_scan_kind(&dev_bundle, MEMBER_TETHER_KIND)
            .await
            .expect("pair_scan_kind (member-tether) should establish the tether");
        assert_eq!(peer, dev_pk, "the tether peer is the device");
        // member_tether_of's logic: find the member-tether whose roster holds the device.
        let me = pacific_core::identity::parse_identity_key(&arc.identity_key()).unwrap();
        arc.all_groups()
            .unwrap()
            .into_iter()
            .find(|(gid, kind)| {
                kind == MEMBER_TETHER_KIND
                    && arc
                        .group_roster(gid)
                        .unwrap()
                        .into_iter()
                        .any(|pk| pk != me && pk == dev_pk)
            })
            .map(|(gid, _)| gid)
            .expect("the device is on the tether roster right after pair_scan")
    };

    // DIAGNOSTIC: what is already in the tether's log right after pair_scan_kind? A
    // `Coordinator<GroupType>` fold rejects any delta whose type_id != Group, so if signup seeded
    // a delta of another type this is where it shows.
    {
        let arc = node_at(arc_home.path());
        match arc.object_log(&tether) {
            Ok(log) => {
                eprintln!(
                    "[repro] tether log has {} delta(s) after pair_scan_kind:",
                    log.len()
                );
                for (op_id, gen, author, id, len) in &log {
                    eprintln!(
                        "        op_id={op_id} gen={gen} author={author} id={id} bytes={len}"
                    );
                }
            }
            Err(e) => eprintln!("[repro] object_log(tether) itself errored: {e}"),
        }
    }

    // 2) grant_default_role: author `group.setMemberRole(device, member)` onto the tether.
    //    BEST-EFFORT, exactly like the signup handler — a failure here leaves the member on the
    //    roster (admitted) and is only logged, never propagated. So we mirror that: capture the
    //    outcome, do NOT abort, and carry the error into the verify assertion below.
    let grant_outcome = {
        let arc = node_at(arc_home.path());
        let identity_key = format!("space1{}", hex::encode(dev_pk));
        let mut args = Args::new();
        args.insert("space".into(), ArgVal::Text(identity_key));
        args.insert(
            "role".into(),
            ArgVal::Text(GroupRole::Member.as_str().to_string()),
        );
        arc.apply(&tether, OP_SET_ROLE, args).await
    };
    if let Err(e) = &grant_outcome {
        eprintln!("[repro] grant_default_role FAILED (best-effort, member still admitted): {e}");
    }

    // ── VERIFY, exactly as the gateway's /verify does it ─────────────────────────────────
    // This is the read that 401'd in production. If the tether's log poisons on fold, the
    // member_role read throws here with the real CoreError.
    let arc = node_at(arc_home.path());
    let outcome = verify_roster_and_role(&arc, &dev_pk);

    match &outcome {
        Ok(Some(role)) => {
            eprintln!("[repro] verify ALLOW: role = {role:?}");
            assert_eq!(*role, GroupRole::Member, "signup default role is Member");
        }
        Ok(None) => panic!(
            "REPRODUCED (roster-miss): signature-valid but device is on no member-tether roster \
             — signup did not leave the device on the roster"
        ),
        Err(e) => panic!(
            "REPRODUCED (swallowed error → silent 401): the roster/role read threw AFTER the \
             signature check. grant_default_role outcome was {grant:?}. Real verify error: {e}",
            grant = grant_outcome
                .as_ref()
                .map(|_| "ok")
                .map_err(|e| e.to_string()),
        ),
    }
}
