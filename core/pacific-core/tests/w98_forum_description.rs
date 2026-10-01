//! W-98 Rooms: `forum.editDescription {description}`. Its id, args and authority are the ICD's
//! (`coordination/delta-graph.icd.json`). The room's owner, or an admin of THIS room
//! (`base.setRole admin` on the forum itself), writes one register, LWW by (gen, author); anyone
//! else is refused, at fold and at write. Empty clears. The forum view shows it as `description`,
//! null when there is none.

use pacific_core::authoring::{self, Ctx, Refusal};
use pacific_core::coordinator::{ArgVal, Args, ConversationType, Delta, ForumState, ForumType};
use pacific_core::object::{build_delta, DeltaRejection, MemberId, ObjectKind, ObjectType, Op, ReduceContext};
use serde_json::Value;

/// The room's owner.
const ADA: MemberId = [0xad; 32];
/// Made an admin of the room, where a test says so.
const BO: MemberId = [0xb0; 32];
/// A member.
const CY: MemberId = [0xc4; 32];
const MEMBERS: [MemberId; 3] = [ADA, BO, CY];

fn icd() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../coordination/delta-graph.icd.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("the ICD")).expect("the ICD is JSON")
}

fn decl() -> Value {
    icd()["kinds"]["forum"]["ops"]["forum.editDescription"].clone()
}

fn op() -> u32 {
    decl()["op"].as_u64().expect("the ICD declares forum.editDescription on the forum") as u32
}

fn role_op(name: &str) -> u32 {
    icd()["facets"]["roles"]["ops"][name]["op"].as_u64().unwrap_or_else(|| panic!("the ICD declares {name}")) as u32
}

/// Every arg the ICD declares for forum.editDescription.
fn args(description: &str, gen: i64) -> Args {
    decl()["args"]
        .as_object()
        .expect("forum.editDescription declares args")
        .keys()
        .map(|k| {
            let v = match k.as_str() {
                "description" => ArgVal::Text(description.into()),
                "gen" => ArgVal::Int(gen),
                other => panic!("the ICD declares `{other}` on forum.editDescription; teach this test what it means"),
            };
            (k.clone(), v)
        })
        .collect()
}

/// Every arg the ICD declares for a roles op on `member`.
fn role_args(name: &str, member: &MemberId, role: &str) -> Args {
    icd()["facets"]["roles"]["ops"][name]["args"]
        .as_object()
        .expect("the roles op declares args")
        .keys()
        .map(|k| {
            let v = match k.as_str() {
                "member" => ArgVal::Text(hex::encode(member)),
                "role" => ArgVal::Text(role.into()),
                other => panic!("the ICD declares `{other}` on {name}; teach this test what it means"),
            };
            (k.clone(), v)
        })
        .collect()
}

fn fold(state: &mut ForumState, op_id: u32, args: &Args, author: &MemberId) -> Result<(), DeltaRejection> {
    let ctx = ReduceContext { members: &MEMBERS, owner: ADA, epoch: 0 };
    ForumType::reduce(state, &Op { op_id, args, author, pos: None, ctx: &ctx })
}

fn describe(state: &mut ForumState, by: &MemberId, text: &str, gen: i64) -> Result<(), DeltaRejection> {
    fold(state, op(), &args(text, gen), by)
}

fn role(state: &mut ForumState, name: &str, who: &MemberId) {
    fold(state, role_op(name), &role_args(name, who, "admin"), &ADA).expect("the owner sets the room's roles");
}

/// The room as its view shows it, from a log in which the owner's role ops form the spine.
fn view(log: &[(MemberId, Delta)]) -> Result<Value, String> {
    let enc: Vec<(MemberId, Vec<u8>)> = log.iter().map(|(a, d)| (*a, d.canonical_bytes())).collect();
    pacific_core::fold::view_of("forum", ADA, MEMBERS.to_vec(), vec![], enc, None)
        .map(|v| serde_json::from_str(&v).expect("the view is JSON"))
        .map_err(|e| e.to_string())
}

fn described(by: MemberId, text: &str, gen: i64) -> (MemberId, Delta) {
    (by, build_delta(ObjectKind::Forum, op(), args(text, gen), 0, Some(gen as u64)))
}

/// The owner's roles ops on the room's spine, each linked to the one before.
fn spine(ops: &[(&str, MemberId)]) -> Vec<(MemberId, Delta)> {
    let mut prev = pacific_core::coordinator::GENESIS_PREV;
    ops.iter()
        .enumerate()
        .map(|(i, (name, who))| {
            let mut d = build_delta(ObjectKind::Forum, role_op(name), role_args(name, who, "admin"), 0, None);
            d.seq = Some(i as u64);
            d.prev = prev;
            prev = d.id();
            (ADA, d)
        })
        .collect()
}

#[test]
fn the_owner_describes_the_room() {
    let v = view(&[described(ADA, "Seed swaps and sowing dates", 1)]).expect("the owner's description folds");
    assert_eq!(v["description"], "Seed swaps and sowing dates");
}

#[test]
fn a_room_with_none_says_none() {
    let v = view(&[]).expect("an empty room folds");
    assert!(v.get("description").is_some_and(Value::is_null), "the view carries description, null: {v}");
}

#[test]
fn an_admin_of_this_room_describes_it() {
    let mut log = spine(&[("base.setRole", BO)]);
    log.push(described(BO, "The tool shed rota", 2));
    assert_eq!(view(&log).expect("an admin's description folds")["description"], "The tool shed rota");
}

/// Who may edit is the page's to know: the view names this room's roles, in the group view's
/// shape, `[member, role]`.
#[test]
fn the_view_names_this_rooms_roles() {
    let v = view(&spine(&[("base.setRole", BO)])).unwrap();
    assert_eq!(v["roles"], serde_json::json!([[hex::encode(BO), "admin"]]));
    assert_eq!(view(&[]).unwrap()["roles"], serde_json::json!([]), "none, as an empty list");
}

#[test]
fn a_member_is_refused_at_fold() {
    let mut s = ForumState::default();
    assert!(describe(&mut s, &CY, "mine now", 1).is_err(), "a member who is not an admin");
    // The view's fold holds the same rule: a refused commutative write is inert (Coordinator::state).
    assert!(view(&[described(CY, "mine now", 1)]).unwrap()["description"].is_null(), "and the view shows none");
}

#[test]
fn an_admin_is_this_rooms_admin_and_a_revoked_one_is_refused() {
    let mut s = ForumState::default();
    assert!(describe(&mut s, &BO, "before", 1).is_err(), "no role in this room yet");
    role(&mut s, "base.setRole", &BO);
    assert_eq!(describe(&mut s, &BO, "while admin", 2), Ok(()));
    fold(&mut s, role_op("base.clearRole"), &role_args("base.clearRole", &BO, "admin"), &ADA).expect("the owner clears it");
    assert!(describe(&mut s, &BO, "after", 3).is_err(), "revoked");
}

/// The ego the ICD states is the rule the fold holds: each standing it names may write, and a
/// member it does not name may not.
#[test]
fn the_icds_ego_is_the_folds() {
    let ego = decl()["ego"].as_str().expect("forum.editDescription states its ego").to_string();
    let names: Vec<&str> = ego.split('|').collect();
    let mut s = ForumState::default();
    role(&mut s, "base.setRole", &BO);
    assert_eq!(describe(&mut s, &ADA, "owner", 1).is_ok(), names.contains(&"owner"), "the owner, against `{ego}`");
    assert_eq!(describe(&mut s, &BO, "admin", 2).is_ok(), names.contains(&"role:admin"), "an admin, against `{ego}`");
    assert_eq!(describe(&mut s, &CY, "member", 3).is_ok(), names.contains(&"member"), "a member, against `{ego}`");
}

#[test]
fn the_latest_counting_write_wins_in_any_order() {
    let a = described(ADA, "first", 1);
    let b = described(ADA, "second", 2);
    let mut spined = spine(&[("base.setRole", BO)]);
    spined.push(described(BO, "third", 3));
    spined.push(b.clone());
    spined.push(a.clone());
    assert_eq!(view(&spined).unwrap()["description"], "third", "the highest gen, whatever order it arrives in");

    // A tie on gen goes to the higher author.
    let mut tie = spine(&[("base.setRole", BO)]);
    tie.push(described(BO, "bo's", 5));
    tie.push(described(ADA, "ada's", 5));
    let mut tie_rev = spine(&[("base.setRole", BO)]);
    tie_rev.push(described(ADA, "ada's", 5));
    tie_rev.push(described(BO, "bo's", 5));
    let winner = if BO > ADA { "bo's" } else { "ada's" };
    assert_eq!(view(&tie).unwrap()["description"], winner);
    assert_eq!(view(&tie_rev).unwrap()["description"], winner);
}

/// A revoked admin's write stops counting, and the description falls back to the latest that does.
#[test]
fn a_revoked_admins_write_is_inert() {
    let mut log = spine(&[("base.setRole", BO), ("base.clearRole", BO)]);
    log.push(described(ADA, "the owner's", 2));
    log.push(described(BO, "the revoked admin's", 3));
    assert_eq!(view(&log).unwrap()["description"], "the owner's");
}

/// A revoked admin's write, the one they made while admin and a hostile one at the highest gen
/// there is, counts for nothing once the role is cleared: the view reads the owner's.
#[test]
fn a_revoked_admins_write_falls_back_at_any_gen() {
    for gen in [3, i64::MAX] {
        let mut log = spine(&[("base.setRole", BO), ("base.clearRole", BO)]);
        log.push(described(ADA, "first", 1));
        log.push(described(BO, "while admin", gen));
        assert_eq!(view(&log).unwrap()["description"], "first", "BO's write at gen {gen}, the role cleared");
    }
}

/// At most the ICD's `maxBytes` of UTF-8: over it is refused, never truncated.
#[test]
fn over_the_cap_is_refused_never_truncated() {
    let cap = decl()["args"]["description"]["maxBytes"].as_u64().expect("the ICD states the description's maxBytes") as usize;
    let mut s = ForumState::default();
    let at = "é".repeat(cap / 2) + &"a".repeat(cap % 2);
    assert_eq!(at.len(), cap);
    assert_eq!(describe(&mut s, &ADA, &at, 1), Ok(()), "exactly the cap");
    assert_eq!(describe(&mut s, &ADA, &(at.clone() + "a"), 2), Err(DeltaRejection::MalformedArgs), "one byte over");
    assert_eq!(view(&[described(ADA, &at, 1), described(ADA, &(at.clone() + "a"), 2)]).unwrap()["description"], at.as_str(), "and the view keeps the last that fitted, whole");
}

#[test]
fn empty_clears() {
    let v = view(&[described(ADA, "Seed swaps", 1), described(ADA, "", 2)]).unwrap();
    assert!(v["description"].is_null(), "cleared: {v}");
}

#[test]
fn refused_at_write_as_at_fold() {
    let owners = [(0u64, ADA)];
    let log: Vec<(Delta, MemberId)> = spine(&[("base.setRole", BO)]).into_iter().map(|(a, d)| (d, a)).collect();
    let ctx = |me| Ctx { me, epoch: 0, members: &MEMBERS, owners: &owners, log: &log, watermark: Some(0) };
    assert!(authoring::build("forum", op(), args("owner", 0), &ctx(ADA)).is_ok(), "the owner");
    assert!(authoring::build("forum", op(), args("admin", 0), &ctx(BO)).is_ok(), "an admin of this room");
    assert!(
        matches!(authoring::build("forum", op(), args("member", 0), &ctx(CY)), Err(Refusal::OwnerOrRoleOnly { .. })),
        "a member, refused before anything is sealed"
    );
}

#[test]
fn a_forum_declares_it_and_a_conversation_does_not() {
    let d = ForumType::op(op()).expect("the Forum's table declares forum.editDescription");
    assert_eq!(d.name, "forum.editDescription");
    assert!(ConversationType::op(op()).is_none(), "the ICD's conversation kind does not declare it");
}
